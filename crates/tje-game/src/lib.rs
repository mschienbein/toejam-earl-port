//! Address-preserving game translations, independent of the emulator and frontends.
//!
//! Storage is accessed through `tje_mem::Bus`/typed views; device accesses use
//! `Platform`. Calls use inventory identities, never host pointers or ROM offsets.

use std::fmt;
use tje_mem::{Bus, Revision, arithmetic::ArithmeticTrap};

pub mod examples;
pub mod leaves;
mod metadata;
pub mod random;
pub mod runtime;
pub use metadata::ids;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FunctionId(u16);

impl FunctionId {
    pub fn metadata(self) -> &'static FunctionMetadata {
        let index = usize::from(self.0);
        &metadata::FUNCTION_CHUNKS[index / 108][index % 108]
    }
}

#[derive(Debug, Clone)]
pub struct Abi {
    pub kind: &'static str,
    pub signature: &'static str,
    /// Complete inventory ABI, including assembly input/output registers and evidence.
    pub details_json: &'static str,
    pub argument_count: u8,
    pub returns_value: bool,
    pub preserved_data: u8,
    pub preserved_address: u8,
    /// Handwritten assembly has no compiler-level scratch-register abstraction.
    pub strict_registers: bool,
}

#[derive(Debug, Clone)]
pub struct FunctionMetadata {
    pub id: FunctionId,
    pub inventory_id: &'static str,
    pub name: &'static str,
    pub addresses: [u32; 2],
    pub sizes: [u32; 2],
    pub classification: &'static str,
    pub kind: &'static str,
    pub subsystem: &'static str,
    pub source: &'static str,
    pub source_line: u32,
    pub wave: u8,
    pub callees: &'static [FunctionId],
    /// Explicit reviewed contract: no clock/interrupt progress is needed to return.
    pub atomic_capable: bool,
    pub abi: Abi,
}

impl FunctionMetadata {
    pub fn address(&self, revision: Revision) -> u32 {
        self.addresses[revision_index(revision)]
    }
    pub fn size(&self, revision: Revision) -> u32 {
        self.sizes[revision_index(revision)]
    }
}

const fn revision_index(revision: Revision) -> usize {
    match revision {
        Revision::Rev02 => 0,
        Revision::Rev00 => 1,
    }
}

pub fn functions() -> impl Iterator<Item = &'static FunctionMetadata> + Clone {
    metadata::FUNCTION_CHUNKS
        .iter()
        .flat_map(|chunk| chunk.iter())
}

pub fn function_at(revision: Revision, address: u32) -> Option<FunctionId> {
    functions()
        .find(|function| function.address(revision) == address)
        .map(|function| function.id)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Untranslated(FunctionId),
    Arithmetic(ArithmeticTrap),
    Platform(String),
}
impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Untranslated(id) => {
                write!(formatter, "untranslated function: {}", id.metadata().name)
            }
            Self::Arithmetic(trap) => write!(formatter, "68000 arithmetic exception: {trap:?}"),
            Self::Platform(message) => formatter.write_str(message),
        }
    }
}
impl std::error::Error for Error {}
impl From<ArithmeticTrap> for Error {
    fn from(value: ArithmeticTrap) -> Self {
        Self::Arithmetic(value)
    }
}
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Side-effecting, width-preserving device bus. `Bus` remains ROM/RAM-only so
/// typed storage reads cannot accidentally hide a controller/VDP read side effect.
/// The implementation supplies the actual programmed platform, not fake registers.
pub trait Platform: tje_mem::Platform {
    fn read_device_u8(&mut self, address: u32) -> u8;
    fn read_device_u16(&mut self, address: u32) -> u16;
    fn write_device_u8(&mut self, address: u32, value: u8);
    fn write_device_u16(&mut self, address: u32, value: u16);

    fn read_vdp_status(&mut self) -> u16 {
        self.read_device_u16(0xc00004)
    }
    fn read_vdp_data(&mut self) -> u16 {
        self.read_device_u16(0xc00000)
    }
    fn write_ym2612(&mut self, port: u8, value: u8) {
        self.write_device_u8(0xa04000 + u32::from(port & 3), value);
    }
    fn read_ym2612_status(&mut self) -> u8 {
        self.read_device_u8(0xa04000)
    }
    fn write_psg(&mut self, value: u8) {
        self.write_device_u8(0xc00011, value);
    }
    fn read_z80_ram(&mut self, offset: u16) -> u8 {
        self.read_device_u8(0xa00000 + u32::from(offset & 0x1fff))
    }
    fn write_z80_ram(&mut self, offset: u16, value: u8) {
        self.write_device_u8(0xa00000 + u32::from(offset & 0x1fff), value);
    }
    fn write_z80_bus_request(&mut self, value: u16) {
        self.write_device_u16(0xa11100, value);
    }
    fn read_z80_bus_request(&mut self) -> u16 {
        self.read_device_u16(0xa11100)
    }
    fn write_z80_reset(&mut self, value: u16) {
        self.write_device_u16(0xa11200, value);
    }
    fn read_controller_data(&mut self, second: bool) -> u8 {
        self.read_device_u8(if second { 0xa10005 } else { 0xa10003 })
    }
    fn write_controller_data(&mut self, second: bool, value: u8) {
        self.write_device_u8(if second { 0xa10005 } else { 0xa10003 }, value);
    }
    fn write_controller_control(&mut self, second: bool, value: u8) {
        self.write_device_u8(if second { 0xa1000b } else { 0xa10009 }, value);
    }
}

/// The bridge owns residual register bits and original-call mechanics. Arguments
/// passed to a translation are raw four-byte MCC68K slots, narrowed by the callee.
/// Register-runtime helpers read D0/D1 instead of manufacturing stack arguments.
pub trait Context: Bus + Platform {
    fn revision(&self) -> Revision;
    fn data_register(&self, index: usize) -> u32;
    fn set_data_register(&mut self, index: usize, value: u32);
    fn address_register(&self, index: usize) -> u32;
    fn set_address_register(&mut self, index: usize, value: u32);
    fn status_register(&self) -> u16;
    fn set_status_register(&mut self, value: u16);
    fn dispatcher(&self) -> Dispatcher;
    fn call_original(&mut self, function: FunctionId, arguments: &[u32]) -> Result<u32>;
}

pub type TranslationFn = fn(&mut dyn Context, &[u32]) -> Result<Option<u32>>;
#[derive(Clone, Copy)]
pub struct Translation {
    pub id: FunctionId,
    pub function: TranslationFn,
}

#[derive(Clone, Copy)]
pub struct Dispatcher {
    translations: &'static [Translation],
    original_fallback: bool,
}
impl Dispatcher {
    pub const fn hybrid() -> Self {
        Self::hybrid_with(TRANSLATIONS)
    }
    /// A deliberately bounded hybrid registry is useful when verifying nested
    /// original calls. Missing callees are explicit calls through Context.
    pub const fn hybrid_with(translations: &'static [Translation]) -> Self {
        Self {
            translations,
            original_fallback: true,
        }
    }
    /// Native startup validates the full dependency closure of each requested
    /// entry. Failure lists every missing target; it never enables ROM fallback.
    pub fn native(roots: &[FunctionId]) -> std::result::Result<Self, Vec<FunctionId>> {
        let dispatcher = Self {
            translations: TRANSLATIONS,
            original_fallback: false,
        };
        let mut visited = std::collections::BTreeSet::new();
        let mut pending = roots.to_vec();
        let mut missing = Vec::new();
        while let Some(id) = pending.pop() {
            if !visited.insert(id) {
                continue;
            }
            if dispatcher.translation(id).is_none() {
                missing.push(id);
            }
            pending.extend_from_slice(id.metadata().callees);
        }
        missing.sort_unstable();
        if missing.is_empty() {
            Ok(dispatcher)
        } else {
            Err(missing)
        }
    }
    pub fn translation(self, id: FunctionId) -> Option<Translation> {
        self.translations
            .iter()
            .find(|translation| translation.id == id)
            .copied()
    }
    pub fn translations(self) -> &'static [Translation] {
        self.translations
    }
    pub fn call(self, context: &mut dyn Context, id: FunctionId, arguments: &[u32]) -> Result<u32> {
        if let Some(translation) = self.translation(id) {
            if let Some(value) = (translation.function)(context, arguments)? {
                context.set_data_register(0, value);
            }
            Ok(context.data_register(0))
        } else if self.original_fallback {
            context.call_original(id, arguments)
        } else {
            Err(Error::Untranslated(id))
        }
    }
}

/// A translated caller never chooses its callee's implementation or address.
pub fn call(context: &mut dyn Context, id: FunctionId, arguments: &[u32]) -> Result<u32> {
    context.dispatcher().call(context, id, arguments)
}

pub static TRANSLATIONS: &[Translation] = &[
    Translation {
        id: ids::ABS,
        function: runtime::abs,
    },
    Translation {
        id: ids::LMODT,
        function: runtime::lmodt,
    },
    Translation {
        id: ids::LDIVT,
        function: runtime::ldivt,
    },
    Translation {
        id: ids::LMULT,
        function: runtime::lmult,
    },
    Translation {
        id: ids::ULMODT,
        function: runtime::ulmodt,
    },
    Translation {
        id: ids::ULDIVT,
        function: runtime::uldivt,
    },
    Translation {
        id: ids::GET_RANDOM_NUMBER,
        function: random::get_random_number,
    },
    Translation {
        id: ids::STACK_BASED_RANDOM,
        function: random::stack_based_random,
    },
    Translation {
        id: ids::PRE_GENERATE_RANDOMS,
        function: random::pre_generate_randoms,
    },
    Translation {
        id: ids::DISABLE_BLANKING_GRAPHICS,
        function: leaves::disable_blanking_graphics,
    },
    Translation {
        id: ids::GET_LEVEL_MUSIC_INDEX,
        function: examples::music_hook,
    },
    Translation {
        id: ids::UPDATE_ATTACHED_ENTITY_POSITION,
        function: examples::attachment_hook,
    },
    Translation {
        id: ids::SELECT_SPRITE_CHAIN_START,
        function: examples::chain_hook,
    },
    Translation {
        id: ids::ENABLE_BLANKING_GRAPHICS,
        function: leaves::enable_blanking_graphics,
    },
    Translation {
        id: ids::GET_V_BLANK_GRAPHICS_ENABLED,
        function: leaves::get_vblank_graphics_enabled,
    },
    Translation {
        id: ids::READ_MAPPED_PLAYER_INPUT,
        function: leaves::read_mapped_player_input,
    },
    Translation {
        id: ids::SET_CAMERA_TILE_PRIORITY_ENABLED,
        function: leaves::set_camera_tile_priority_enabled,
    },
    Translation {
        id: ids::ENABLE_TEXT_FONT_ANIMATION,
        function: leaves::enable_text_font_animation,
    },
    Translation {
        id: ids::DISABLE_TEXT_FONT_ANIMATION_IF_MENUS_CLOSED,
        function: leaves::disable_text_font_animation_if_menus_closed,
    },
    Translation {
        id: ids::PLAYER_GET_MAXIMUM_HEALTH,
        function: leaves::player_get_maximum_health,
    },
    Translation {
        id: ids::GET_SCORE_FOR_PROMO,
        function: leaves::get_score_for_promo,
    },
    Translation {
        id: ids::STOP_PLAYER,
        function: leaves::stop_player,
    },
    Translation {
        id: ids::PLAYER_INITIALIZE_PHASED_JUMP_MOTION,
        function: leaves::player_initialize_phased_jump_motion,
    },
    Translation {
        id: ids::ENTITY_HAS_FREE_TIMED_EFFECT_SLOT,
        function: leaves::entity_has_free_timed_effect_slot,
    },
    Translation {
        id: ids::SET_NEXT_SPEECH_LIFETIME,
        function: leaves::set_next_speech_lifetime,
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complete_metadata_and_native_closure() {
        let functions: Vec<_> = functions().collect();
        assert_eq!(functions.len(), 648);
        assert_eq!(
            functions
                .iter()
                .filter(|function| function.kind == "function")
                .count(),
            645
        );
        for function in functions {
            assert_eq!(function.id.metadata().inventory_id, function.inventory_id);
            for revision in [Revision::Rev02, Revision::Rev00] {
                assert_eq!(
                    function_at(revision, function.address(revision)),
                    Some(function.id)
                );
            }
            assert!(!function.abi.signature.is_empty());
            assert!(!function.abi.details_json.is_empty());
            assert!(function.source.starts_with("src/"));
        }
        assert!(Dispatcher::native(&[ids::GET_RANDOM_NUMBER, ids::PRE_GENERATE_RANDOMS]).is_ok());
        let startup = ids::FURTHER_INITIALISATION;
        let missing = Dispatcher::native(&[startup]).err().unwrap();
        assert!(missing.contains(&startup));
        assert!(missing.len() > 100);
        let mut seen = std::collections::BTreeSet::new();
        for translation in TRANSLATIONS {
            assert!(seen.insert(translation.id));
        }
    }
}
