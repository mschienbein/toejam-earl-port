//! TJE reference proof: passive actual-read metadata, never execution, source,
//! readiness or reset authority. The optional bus journal is absent when this
//! proof-only feature is not compiled and defaults to None when it is.
use bincode::{Decode, Encode};
use m68000_emu::ReadPurpose;

/// One completed main-CPU read. Address/width/purpose/value bind this context to
/// the same raw native issue in order; no sample is masked or substituted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct ReadContext {
    pub address: u32,
    pub word: bool,
    /// Same stable tags as native::CpuAccess: semantic=0, prefetch=1, discarded=2.
    pub purpose: u8,
    pub value: u16,
    pub driven_mask: u16,
    pub z80_busack: bool,
}
impl ReadContext {
    pub(crate) fn new(
        address: u32,
        word: bool,
        purpose: ReadPurpose,
        value: u16,
        driven_mask: u16,
        z80_busack: bool,
    ) -> Self {
        Self {
            address: address & 0xffffff,
            word,
            purpose: match purpose {
                ReadPurpose::Semantic => 0,
                ReadPurpose::Prefetch => 1,
                ReadPurpose::Discarded => 2,
            },
            value,
            driven_mask,
            z80_busack,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GenesisBus;
    use genesis_config::GenesisEmulatorConfig;
    use jgenesis_common::frontend::TimingMode;
    use m68000_emu::BusInterface;

    fn bus() -> GenesisBus {
        let mut bus = GenesisBus::new(TimingMode::Ntsc, None, &GenesisEmulatorConfig::default());
        bus.native_cpu_accesses = Some(Vec::new());
        bus
    }
    fn write(bus: &mut GenesisBus, address: u32, value: u16) {
        bus.write_word(address, value);
        bus.apply_pending_writes();
    }
    fn reads(bus: &GenesisBus) -> Vec<(u32, bool, u8, u16)> {
        bus.native_cpu_accesses
            .as_ref()
            .unwrap()
            .iter()
            .filter_map(|access| match *access {
                crate::native::CpuAccess::Read { address, word, purpose, value } => {
                    Some((address, word, purpose, value))
                }
                _ => None,
            })
            .collect()
    }
    #[test]
    fn mask_context_is_bound_to_each_actual_read_and_not_the_final_bus_state() {
        let mut bus = bus();
        assert!(bus.reference_read_contexts.is_none());
        bus.reference_read_contexts = Some(Vec::new());
        write(&mut bus, 0xa11100, 0x0100);
        write(&mut bus, 0xa11200, 0x0100);
        assert!(bus.z80_signals.busack());
        bus.write_byte(0xa00000, 0xff);
        bus.apply_pending_writes();
        let _ = bus.read_byte_tagged(0x1a00000, ReadPurpose::Semantic);
        write(&mut bus, 0xa11100, 0);
        assert!(!bus.z80_signals.busack());
        let _ = bus.read_byte_tagged(0xa00000, ReadPurpose::Discarded);
        let _ = bus.read_word_tagged(0xc00004, ReadPurpose::Semantic);
        let _ = bus.read_byte_tagged(0x1c00004, ReadPurpose::Semantic);
        let _ = bus.read_byte_tagged(0xc00005, ReadPurpose::Semantic);
        let _ = bus.read_word_tagged(0xa11100, ReadPurpose::Semantic);
        let _ = bus.read_word_tagged(0xff1234, ReadPurpose::Prefetch);
        let contexts = bus.reference_read_contexts.as_ref().unwrap();
        assert_eq!(
            contexts
                .iter()
                .map(|context| (context.address, context.word, context.purpose, context.value))
                .collect::<Vec<_>>(),
            reads(&bus)
        );
        assert_eq!(contexts[0].driven_mask, 0xff);
        assert!(contexts[0].z80_busack);
        assert_eq!(contexts[1].driven_mask, 0);
        assert!(!contexts[1].z80_busack);
        assert_eq!(
            contexts.iter().map(|context| context.driven_mask).collect::<Vec<_>>(),
            [0xff, 0, 0x03ff, 0x03, 0xff, 0x0100, 0xffff]
        );
        assert_eq!((contexts[0].address, contexts[3].address), (0xa00000, 0xc00004));
    }
    #[test]
    fn enabling_read_contexts_changes_no_read_value_side_effect_cycle_or_native_issue() {
        let mut observed = bus();
        let mut ordinary = bus();
        observed.reference_read_contexts = Some(Vec::new());
        for bus in [&mut observed, &mut ordinary] {
            write(bus, 0xff1234, 0x4567);
            write(bus, 0xa11100, 0x0100);
            write(bus, 0xa11200, 0x0100);
            for (address, purpose) in [
                (0xe01234, ReadPurpose::Semantic),
                (0xc00004, ReadPurpose::Discarded),
                (0xa11100, ReadPurpose::Semantic),
                (0xa00000, ReadPurpose::Semantic),
                (0x100, ReadPurpose::Prefetch),
            ] {
                let _ = bus.read_word_tagged(address, purpose);
            }
        }
        assert_eq!(reads(&observed), reads(&ordinary));
        assert_eq!(observed.reference_read_contexts.take().unwrap().len(), 5);
        assert!(ordinary.reference_read_contexts.is_none());
        assert_eq!(
            bincode::encode_to_vec(&observed, bincode::config::standard()).unwrap(),
            bincode::encode_to_vec(&ordinary, bincode::config::standard()).unwrap()
        );
    }
    #[test]
    fn absent_cartridge_and_rom_holes_never_certify_open_bus_as_driven_data() {
        let mut absent = bus();
        absent.reference_read_contexts = Some(Vec::new());
        let _ = absent.read_byte_tagged(0x100, ReadPurpose::Semantic);
        let _ = absent.read_word_tagged(0x100, ReadPurpose::Prefetch);
        assert_eq!(
            absent
                .reference_read_contexts
                .as_ref()
                .unwrap()
                .iter()
                .map(|context| context.driven_mask)
                .collect::<Vec<_>>(),
            [0, 0]
        );

        let mut rom = vec![0; 1024];
        rom[..2].copy_from_slice(&0x1234_u16.to_be_bytes());
        rom[1022..].copy_from_slice(&0xabcd_u16.to_be_bytes());
        let cartridge = genesis_components::cartridge::Cartridge::new(rom, None, None, &[]);
        let mut mapped =
            GenesisBus::new(TimingMode::Ntsc, Some(cartridge), &GenesisEmulatorConfig::default());
        mapped.native_cpu_accesses = Some(Vec::new());
        mapped.reference_read_contexts = Some(Vec::new());
        assert_eq!(mapped.read_word_tagged(0x1000000, ReadPurpose::Prefetch), 0x1234);
        assert_eq!(mapped.read_byte_tagged(1, ReadPurpose::Semantic), 0x34);
        assert_eq!(mapped.read_byte_tagged(1023, ReadPurpose::Semantic), 0xcd);
        assert_eq!(mapped.read_word_tagged(1022, ReadPurpose::Semantic), 0xabcd);
        let _ = mapped.read_byte_tagged(1024, ReadPurpose::Semantic);
        let _ = mapped.read_word_tagged(1024, ReadPurpose::Discarded);
        let _ = mapped.read_word_tagged(0x400000, ReadPurpose::Semantic);
        let contexts = mapped.reference_read_contexts.as_ref().unwrap();
        assert_eq!(
            contexts.iter().map(|context| context.driven_mask).collect::<Vec<_>>(),
            [0xffff, 0xff, 0xff, 0xffff, 0, 0, 0]
        );
        assert_eq!(
            contexts
                .iter()
                .map(|context| (context.address, context.word, context.purpose, context.value))
                .collect::<Vec<_>>(),
            reads(&mapped)
        );
    }
    #[test]
    fn positive_basic_mapping_query_is_pure_and_a_different_mapper_is_not_rom_evidence() {
        use genesis_components::cartridge::Cartridge;
        let cartridge = Cartridge::new(vec![0; 1024], None, None, &[]);
        let before = bincode::encode_to_vec(&cartridge, bincode::config::standard()).unwrap();
        assert!(cartridge.reference_basic_rom_is_mapped(0, true));
        assert!(cartridge.reference_basic_rom_is_mapped(0x10003ff, false));
        assert!(!cartridge.reference_basic_rom_is_mapped(1024, false));
        assert!(!cartridge.reference_basic_rom_is_mapped(1024, true));
        assert!(!cartridge.reference_basic_rom_is_mapped(1023, true));
        assert!(!cartridge.reference_basic_rom_is_mapped(0x400000, false));
        assert_eq!(
            before,
            bincode::encode_to_vec(&cartridge, bincode::config::standard()).unwrap()
        );
        let mut banked = vec![0; 1024];
        banked[0x100..0x110].copy_from_slice(b"SEGA SSF        ");
        let cartridge = Cartridge::new(banked, None, None, &[]);
        assert!(!cartridge.reference_basic_rom_is_mapped(0, true));
        let mut unknown =
            GenesisBus::new(TimingMode::Ntsc, Some(cartridge), &GenesisEmulatorConfig::default());
        unknown.reference_read_contexts = Some(Vec::new());
        let _ = unknown.read_word_tagged(0, ReadPurpose::Semantic);
        assert_eq!(unknown.reference_read_contexts.as_ref().unwrap()[0].driven_mask, 0);
    }
}
