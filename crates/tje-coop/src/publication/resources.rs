//! Session-load-only resource import. Logical IDs are deterministic catalogue
//! indices; ROM addresses and decoded graphics never enter authoritative state.
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fmt};
use tje_extended_view::{
    presentation::Piece,
    resources::{MAX_PIECE_BYTES, decode_piece_into, frame_pieces, piece_length},
};
use tje_four_player::Archetype;
use tje_mem::{AddressSpace, rom};
use tje_mode_view::{
    MAX_PIECES,
    animation::{Frame, FrameMotion, Program},
};
use tje_simulation::{
    AnimationId, FrameId,
    population::{DefinitionCatalog, Kind, SLEEP_STATE, rom::RomDefinitions},
};

const MAX_CATALOGUE_BYTES: usize = 48 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaticUpload {
    pub source: u32,
    pub destination: u16,
    pub words: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EffectAnimation {
    Decoy,
    RainCloud,
    Doorway,
    Telephone,
    Boombox,
    TomatoProjectile,
    ControlRemap,
    ControlRemapLeft,
    ControlRemapRight,
    TomatoTargetImpact,
    TomatoGroundImpact,
    TomatoWaterImpact,
    Transient,
    Timed,
    RetiringTerrain,
    TemporaryTerrain,
    HealthDrain,
    ExtraLife,
    RankPromotion,
    ShipCollection,
    Choice,
    Mole,
    Pause,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResourceKey {
    Hero {
        earl: bool,
        descriptor: u8,
    },
    Earthling {
        kind: Kind,
        state: u8,
    },
    Companion(u8),
    Elevator {
        secondary: bool,
        state: u8,
    },
    Wake(u8),
    Terrain(u8),
    Effect {
        animation: EffectAnimation,
        variant: u8,
    },
    Present(u8),
    GroundItem(u8),
    LargeItem(u8),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Resource {
    Animation(AnimationId),
    Frame(FrameId),
}
#[derive(Debug, Clone, Copy)]
enum FrameLocation {
    Program { animation: u16, frame: u16 },
    Static(u16),
}

pub struct ImportedAnimation {
    pub program: Program,
    /// None for eight-byte descriptors: their timing is owned by the kernel,
    /// never guessed by the graphics importer.
    pub period: Option<u16>,
    pub y_offset: i8,
    frames: [Option<FrameId>; 128],
}
#[derive(Debug)]
pub enum ResourceError {
    Invalid(&'static str),
    View(tje_mode_view::Error),
    Decoder(tje_extended_view::RenderError),
    Definition(tje_world::Error),
    Contract(tje_simulation::Error),
}
impl fmt::Display for ResourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(reason) => f.write_str(reason),
            Self::View(error) => error.fmt(f),
            Self::Decoder(error) => error.fmt(f),
            Self::Definition(error) => error.fmt(f),
            Self::Contract(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for ResourceError {}
impl From<tje_mode_view::Error> for ResourceError {
    fn from(error: tje_mode_view::Error) -> Self {
        Self::View(error)
    }
}
impl From<tje_extended_view::RenderError> for ResourceError {
    fn from(error: tje_extended_view::RenderError) -> Self {
        Self::Decoder(error)
    }
}
impl From<tje_world::Error> for ResourceError {
    fn from(error: tje_world::Error) -> Self {
        Self::Definition(error)
    }
}
impl From<tje_simulation::Error> for ResourceError {
    fn from(error: tje_simulation::Error) -> Self {
        Self::Contract(error)
    }
}
pub type Result<T, E = ResourceError> = std::result::Result<T, E>;

pub struct ResourceCatalog {
    keys: BTreeMap<ResourceKey, Resource>,
    animations: Vec<ImportedAnimation>,
    animation_sources: BTreeMap<(u32, u32, Option<u16>, i8), AnimationId>,
    frames: Vec<FrameLocation>,
    frame_addresses: BTreeMap<u32, FrameId>,
    static_frames: Vec<Frame>,
    definitions: DefinitionCatalog,
    reserved_bytes: usize,
    identity: [u8; 32],
    shadow_tiles: [u8; 9],
    companion_offsets: [[i8; 2]; 25],
    static_patterns: [u8; 736],
    static_uploads: [StaticUpload; 2],
    stock_choices: [[Option<Kind>; 20]; 27],
    contracts: tje_simulation::visual::AnimationContracts,
}
impl ResourceCatalog {
    pub fn load(memory: &AddressSpace) -> Result<Self> {
        let imported = RomDefinitions::new(memory)?;
        let mut result = Self {
            keys: BTreeMap::new(),
            animations: Vec::with_capacity(512),
            animation_sources: BTreeMap::new(),
            frames: Vec::with_capacity(4096),
            frame_addresses: BTreeMap::new(),
            static_frames: Vec::with_capacity(64),
            definitions: imported.catalog().clone(),
            reserved_bytes: 0,
            identity: Sha256::new()
                .chain_update(b"tje-coop/resources/1\0")
                .chain_update(memory.rom())
                .finalize()
                .into(),
            // Original depth zero reads lookup[-1] (G/entity_depth.c45-51).
            // Import that finite preceding ROM byte explicitly; no host index
            // underflow and no silent change to the authored tile selection.
            shadow_tiles: bytes(
                memory,
                rom::sprite_tile_allocation_lookup(memory.revision())
                    .address()
                    .checked_sub(1)
                    .ok_or(ResourceError::Invalid("shadow lookup origin"))?,
                9,
            )?
            .try_into()
            .expect("checked shadow lookup"),
            companion_offsets: {
                let offsets = bytes(
                    memory,
                    rom::earthling_coordinate_pairs(memory.revision()).address(),
                    50,
                )?;
                std::array::from_fn(|kind| [offsets[kind * 2] as i8, offsets[kind * 2 + 1] as i8])
            },
            static_patterns: bytes(
                memory,
                rom::hud_additional_tiles(memory.revision()).address(),
                736,
            )?
            .try_into()
            .expect("checked static graphics"),
            static_uploads: [
                StaticUpload {
                    source: rom::hud_additional_tiles(memory.revision()).address(),
                    destination: 0xf400,
                    words: 304,
                },
                StaticUpload {
                    source: rom::hud_additional_tiles_small(memory.revision()).address(),
                    destination: 0xff20,
                    words: 64,
                },
            ],
            stock_choices: {
                let mut choices = [[None; 20]; 27];
                for (level, slots) in choices.iter_mut().enumerate().take(26) {
                    let level = tje_world::LevelId::new(level as u16).expect("bounded stock floor");
                    for (local, slot) in slots.iter_mut().enumerate() {
                        *slot = imported.stock_kind(level, local as u16)?;
                    }
                }
                choices
            },
            contracts: tje_simulation::visual::AnimationContracts::default(),
        };
        let revision = memory.revision();
        for earl in [false, true] {
            let base = if earl {
                rom::player2_animation_descriptors(revision).address()
            } else {
                rom::player_animation_descriptors(revision).address()
            };
            for descriptor in 0..121 {
                result.timed(
                    memory,
                    ResourceKey::Hero { earl, descriptor },
                    base + u32::from(descriptor) * 10,
                )?;
            }
        }
        for kind in Kind::ALL {
            for state in 0..imported.state_count(kind) {
                if !imported.catalog().get(kind).supports_state(kind, state) {
                    continue;
                }
                if let Some(descriptor) = imported.optional_descriptor(kind, state)? {
                    result.animation(
                        memory,
                        ResourceKey::Earthling { kind, state },
                        descriptor.sequence,
                        descriptor.frames,
                        Some(u16::from(descriptor.period)),
                        0,
                    )?;
                }
            }
            if kind.supports_sleep() {
                let descriptor = imported.descriptor(kind, SLEEP_STATE)?;
                result.animation(
                    memory,
                    ResourceKey::Earthling {
                        kind,
                        state: SLEEP_STATE,
                    },
                    descriptor.sequence,
                    descriptor.frames,
                    Some(u16::from(descriptor.period)),
                    0,
                )?;
            }
        }
        for index in 0..9 {
            result.timed(
                memory,
                ResourceKey::Companion(index),
                rom::earthling_companion_animations(revision).address() + u32::from(index) * 10,
            )?;
        }
        for secondary in [false, true] {
            let base = if secondary {
                rom::elevator_secondary_animations(revision).address()
            } else {
                rom::elevator_primary_animations(revision).address()
            };
            for state in 0..13 {
                result.timed(
                    memory,
                    ResourceKey::Elevator { secondary, state },
                    base + u32::from(state) * 10,
                )?;
            }
        }
        for index in 0..2 {
            result.timed(
                memory,
                ResourceKey::Wake(index),
                rom::player_wake_effect_animations(revision).address() + u32::from(index) * 10,
            )?;
        }
        for index in 0..3 {
            result.timed(
                memory,
                ResourceKey::Terrain(index),
                rom::active_terrain_effect_animations(revision).address() + u32::from(index) * 10,
            )?;
        }
        use EffectAnimation::*;
        // Extents are logical declarations/consumers in G/include/assets_sprites.h,
        // not inferred from a neighbouring opaque metadata unit's byte span.
        for (animation, address, count) in [
            (Decoy, rom::player_decoy_animations(revision).address(), 2),
            (
                RainCloud,
                rom::player_rain_cloud_animations(revision).address(),
                2,
            ),
            (Doorway, rom::doorway_animations(revision).address(), 3),
            (
                ControlRemap,
                rom::player_control_remap_animations(revision).address(),
                5,
            ),
            (
                HealthDrain,
                rom::health_drain_display_animation(revision).address(),
                1,
            ),
            (
                ExtraLife,
                rom::extra_life_display_animation(revision).address(),
                1,
            ),
            (
                RankPromotion,
                rom::rank_promotion_display_animation(revision).address(),
                1,
            ),
            (
                Transient,
                rom::transient_effect_animation(revision).address(),
                1,
            ),
            (
                Timed,
                rom::timed_entity_effect_animation(revision).address(),
                1,
            ),
            (
                RetiringTerrain,
                rom::retiring_terrain_effect_animation(revision).address(),
                1,
            ),
            (
                TemporaryTerrain,
                rom::temporary_terrain_effect_animation(revision).address(),
                1,
            ),
            (
                TomatoTargetImpact,
                rom::tomato_target_impact_animation(revision).address(),
                1,
            ),
            (
                TomatoGroundImpact,
                rom::tomato_ground_impact_animation(revision).address(),
                1,
            ),
            (
                TomatoWaterImpact,
                rom::tomato_terrain128_impact_animation(revision).address(),
                1,
            ),
        ] {
            for variant in 0..count {
                result.timed(
                    memory,
                    ResourceKey::Effect { animation, variant },
                    address + u32::from(variant) * 10,
                )?;
            }
        }
        for (animation, address, count) in [
            (Telephone, rom::telephone_animations(revision).address(), 2),
            (Boombox, rom::boom_box_animations(revision).address(), 1),
            (
                TomatoProjectile,
                rom::tomato_projectile_animations(revision).address(),
                3,
            ),
            (
                ControlRemapLeft,
                rom::control_remap_projectile_left_animation(revision).address(),
                1,
            ),
            (
                ControlRemapRight,
                rom::control_remap_projectile_right_animation(revision).address(),
                1,
            ),
            (
                ShipCollection,
                rom::ship_piece_collection_animation(revision).address(),
                1,
            ),
            (
                Choice,
                rom::player_interaction_choice_animations(revision).address(),
                1,
            ),
            (Mole, rom::mole_effect_animations(revision).address(), 1),
            (Pause, rom::pause_display_animation(revision).address(), 1),
        ] {
            for variant in 0..count {
                let descriptor = bytes(memory, address + u32::from(variant) * 8, 8)?;
                result.animation(
                    memory,
                    ResourceKey::Effect { animation, variant },
                    word(descriptor, 0),
                    word(descriptor, 4),
                    None,
                    0,
                )?;
            }
        }
        for index in 0..28 {
            result.static_frame(
                memory,
                ResourceKey::Present(index),
                rom::present_sprite_table(revision).address() + u32::from(index) * 4,
            )?;
        }
        for index in 0..17 {
            result.static_frame(
                memory,
                ResourceKey::GroundItem(index),
                rom::spr_ground_item(revision).address() + u32::from(index) * 4,
            )?;
        }
        for index in 0..3 {
            result.static_frame(
                memory,
                ResourceKey::LargeItem(index),
                rom::large_ground_item_frames(revision).address() + u32::from(index) * 4,
            )?;
        }
        result.contracts = result.build_animation_contracts();
        result.contracts.validate()?;
        Ok(result)
    }
    pub fn definitions(&self) -> &DefinitionCatalog {
        &self.definitions
    }
    pub fn reserved_bytes(&self) -> usize {
        self.reserved_bytes
    }
    pub fn identity(&self) -> [u8; 32] {
        self.identity
    }
    pub fn frame_count(&self) -> u32 {
        self.frames.len() as u32
    }
    pub fn static_uploads(&self) -> &[StaticUpload; 2] {
        &self.static_uploads
    }
    pub fn validate_static_patterns(&self, vram: &[u8]) -> Result<()> {
        if vram.len() != 0x10000
            || vram.get(0xf400..0xf660) != Some(&self.static_patterns[..608])
            || vram.get(0xff20..0xffa0) != Some(&self.static_patterns[608..])
        {
            return Err(ResourceError::Invalid(
                "required original shadow patterns are not initialized",
            ));
        }
        Ok(())
    }
    pub fn animation_contracts(&self) -> tje_simulation::visual::AnimationContracts {
        self.contracts.clone()
    }
    pub fn contracts(&self) -> &tje_simulation::visual::AnimationContracts {
        &self.contracts
    }
    fn build_animation_contracts(&self) -> tje_simulation::visual::AnimationContracts {
        use tje_simulation::visual::{AnimationContracts, EarthlingBinding, ProgramContract};
        let programs = self
            .animations
            .iter()
            .map(|animation| {
                let selections = animation.program.selection_count();
                let mut flips = [0; 4];
                let mut frame_indices = Vec::with_capacity(usize::from(selections));
                for index in 0..selections {
                    let (frame, flip) = animation
                        .program
                        .frame_at(index)
                        .expect("imported finite sequence");
                    frame_indices.push(frame as u8);
                    if flip {
                        flips[usize::from(index) / 64] |= 1 << (usize::from(index) % 64);
                    }
                }
                ProgramContract {
                    selections,
                    flips,
                    frame_indices,
                    period: animation.period,
                }
            })
            .collect();
        let heroes = std::array::from_fn(|earl| {
            (0..121)
                .map(|descriptor| {
                    self.animation_id(ResourceKey::Hero {
                        earl: earl != 0,
                        descriptor,
                    })
                    .expect("complete imported hero catalogue")
                })
                .collect()
        });
        let earthlings = self
            .keys
            .iter()
            .filter_map(|(key, resource)| match (key, resource) {
                (ResourceKey::Earthling { kind, state }, Resource::Animation(animation)) => {
                    Some(EarthlingBinding {
                        kind: kind.index() as u8,
                        state: *state,
                        animation: *animation,
                    })
                }
                _ => None,
            })
            .collect();
        AnimationContracts {
            programs,
            heroes,
            earthlings,
        }
    }
    pub fn stock_kind(
        &self,
        level: tje_world::LevelId,
        local: u16,
    ) -> tje_world::Result<Option<Kind>> {
        level.validate()?;
        self.stock_choices
            .get(usize::from(level.get()))
            .and_then(|slots| slots.get(usize::from(local)))
            .copied()
            .ok_or(tje_world::Error::Bounds("stock Earthling resource choice"))
    }
    pub fn companion_offset(&self, kind: Kind) -> [i8; 2] {
        self.companion_offsets[kind.index()]
    }
    pub fn shadow(&self, elevation: i32, unframed: bool) -> tje_mode_view::packet::VramShadow {
        if elevation < 0 && !unframed {
            return tje_mode_view::packet::VramShadow {
                tile: 0x87f9,
                size: [4, 1],
                offset: [-16, -2],
            };
        }
        let depth = if elevation >= 128 || unframed {
            1
        } else {
            ((127 - elevation) as u8 & 0x70) >> 4
        };
        let size = (depth << 1) & 12;
        tje_mode_view::packet::VramShadow {
            tile: 0xe7a0_u16.wrapping_add(u16::from(self.shadow_tiles[usize::from(depth)])),
            size: [(size >> 2) + 1, 1],
            offset: [-((size + 4) as i8), -4],
        }
    }
    pub fn animation_id(&self, key: ResourceKey) -> Result<AnimationId> {
        match self.keys.get(&key) {
            Some(Resource::Animation(id)) => Ok(*id),
            _ => Err(ResourceError::Invalid(
                "animation resource was not imported",
            )),
        }
    }
    pub fn hero(&self, archetype: Archetype, descriptor: u8) -> Result<AnimationId> {
        self.animation_id(ResourceKey::Hero {
            earl: archetype == Archetype::Earl,
            descriptor,
        })
    }
    pub fn earthling(&self, kind: Kind, state: u8) -> Result<AnimationId> {
        self.animation_id(ResourceKey::Earthling { kind, state })
    }
    pub fn effect(&self, animation: EffectAnimation, variant: u8) -> Result<AnimationId> {
        self.animation_id(ResourceKey::Effect { animation, variant })
    }
    pub fn program(&self, id: AnimationId) -> Result<&ImportedAnimation> {
        self.animations
            .get(usize::from(id.get()))
            .ok_or(ResourceError::Invalid(
                "animation ID is outside imported catalogue",
            ))
    }
    pub fn frame_id(&self, animation: AnimationId, resource_frame: u16) -> Result<FrameId> {
        self.program(animation)?
            .frames
            .get(usize::from(resource_frame))
            .copied()
            .flatten()
            .ok_or(ResourceError::Invalid(
                "frame is outside imported animation",
            ))
    }
    pub fn selected_frame(
        &self,
        animation: AnimationId,
        sequence_index: u16,
    ) -> Result<(FrameId, bool)> {
        let (index, flip) = self.program(animation)?.program.frame_at(sequence_index)?;
        Ok((self.frame_id(animation, index)?, flip))
    }
    pub fn item_frame(&self, key: ResourceKey) -> Result<FrameId> {
        match self.keys.get(&key) {
            Some(Resource::Frame(id)) => Ok(*id),
            _ => Err(ResourceError::Invalid(
                "static frame resource was not imported",
            )),
        }
    }
    pub fn frame(&self, id: FrameId) -> Result<&Frame> {
        match self.frames.get(id.get() as usize) {
            Some(FrameLocation::Program { animation, frame }) => Ok(self.animations
                [usize::from(*animation)]
            .program
            .frame(*frame)?),
            Some(FrameLocation::Static(index)) => self
                .static_frames
                .get(usize::from(*index))
                .ok_or(ResourceError::Invalid("static frame location")),
            None => Err(ResourceError::Invalid(
                "frame ID is outside imported catalogue",
            )),
        }
    }
    fn reserve(&mut self, amount: usize) -> Result<()> {
        let bytes = self
            .reserved_bytes
            .checked_add(amount)
            .ok_or(ResourceError::Invalid("catalogue size overflow"))?;
        if bytes > MAX_CATALOGUE_BYTES {
            return Err(ResourceError::Invalid(
                "resource catalogue exceeds 48 MiB graphics reservation",
            ));
        }
        self.reserved_bytes = bytes;
        Ok(())
    }
    fn timed(&mut self, memory: &AddressSpace, key: ResourceKey, address: u32) -> Result<()> {
        let descriptor = bytes(memory, address, 10)?;
        self.animation(
            memory,
            key,
            word(descriptor, 0),
            word(descriptor, 4),
            Some(u16::from(descriptor[8])),
            descriptor[9] as i8,
        )
    }
    fn animation(
        &mut self,
        memory: &AddressSpace,
        key: ResourceKey,
        sequence: u32,
        frames: u32,
        period: Option<u16>,
        y_offset: i8,
    ) -> Result<()> {
        if self.keys.contains_key(&key) {
            return Err(ResourceError::Invalid("duplicate catalogue key"));
        }
        if let Some(&id) = self
            .animation_sources
            .get(&(sequence, frames, period, y_offset))
        {
            self.reserve(128)?;
            self.keys.insert(key, Resource::Animation(id));
            return Ok(());
        }
        let id = AnimationId::new(
            self.animations
                .len()
                .try_into()
                .map_err(|_| ResourceError::Invalid("animation ID count"))?,
        )
        .ok_or(ResourceError::Invalid("animation catalogue capacity"))?;
        let program = Program::import(memory.rom(), sequence, frames)?;
        self.reserve(program.storage_bytes() + std::mem::size_of::<ImportedAnimation>() + 256)?;
        let mut mapping = [None; 128];
        for (index, frame) in program.imported_frames() {
            let frame_id = if let Some(&id) = self.frame_addresses.get(&frame.address) {
                id
            } else {
                let frame_id = FrameId::new(
                    self.frames
                        .len()
                        .try_into()
                        .map_err(|_| ResourceError::Invalid("frame ID count"))?,
                )
                .ok_or(ResourceError::Invalid("frame catalogue capacity"))?;
                self.reserve(std::mem::size_of::<FrameLocation>() + 128)?;
                self.frames.push(FrameLocation::Program {
                    animation: id.get(),
                    frame: index,
                });
                self.frame_addresses.insert(frame.address, frame_id);
                frame_id
            };
            mapping[usize::from(index)] = Some(frame_id);
        }
        self.animations.push(ImportedAnimation {
            program,
            period,
            y_offset,
            frames: mapping,
        });
        self.animation_sources
            .insert((sequence, frames, period, y_offset), id);
        self.keys.insert(key, Resource::Animation(id));
        Ok(())
    }
    fn static_frame(
        &mut self,
        memory: &AddressSpace,
        key: ResourceKey,
        pointer: u32,
    ) -> Result<()> {
        if self.keys.contains_key(&key) {
            return Err(ResourceError::Invalid("duplicate static catalogue key"));
        }
        let address = word(bytes(memory, pointer, 4)?, 0);
        if let Some(&id) = self.frame_addresses.get(&address) {
            self.reserve(128)?;
            self.keys.insert(key, Resource::Frame(id));
            return Ok(());
        }
        let mut pieces = [Piece::default(); MAX_PIECES];
        let count = frame_pieces(memory.rom(), address, &mut pieces)?;
        let header = bytes(memory, address, 4)?;
        let mut scratch = [0; MAX_PIECE_BYTES];
        for &piece in pieces.iter().take(count) {
            decode_piece_into(memory.rom(), piece, &mut scratch[..piece_length(piece)?])?;
        }
        let id = FrameId::new(
            self.frames
                .len()
                .try_into()
                .map_err(|_| ResourceError::Invalid("static frame count"))?,
        )
        .ok_or(ResourceError::Invalid("frame catalogue capacity"))?;
        let index = self
            .static_frames
            .len()
            .try_into()
            .map_err(|_| ResourceError::Invalid("static frame location count"))?;
        self.reserve(std::mem::size_of::<Frame>() + std::mem::size_of::<FrameLocation>() + 256)?;
        self.static_frames.push(Frame {
            address,
            motion: FrameMotion {
                x: header[1] as i8,
                y: header[2] as i8,
                reuse_tiles: header[3] != 0,
            },
            count: count as u8,
            pieces,
        });
        self.frames.push(FrameLocation::Static(index));
        self.frame_addresses.insert(address, id);
        self.keys.insert(key, Resource::Frame(id));
        Ok(())
    }
}
fn bytes(memory: &AddressSpace, address: u32, count: usize) -> Result<&[u8]> {
    memory
        .rom()
        .get(address as usize..)
        .and_then(|bytes| bytes.get(..count))
        .ok_or(ResourceError::Invalid(
            "truncated ROM presentation resource",
        ))
}
fn word(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("checked resource prefix"),
    )
}

#[cfg(feature = "enhanced-mode")]
impl tje_engine_runtime::enhanced::Resource for ResourceCatalog {
    const KEY: &'static str = "tje-coop/rom-resources";
    const VERSION: u32 = 1;
    fn identity(&self) -> [u8; 32] {
        self.identity
    }
    fn validate_restore(
        &self,
        states: &tje_engine_runtime::enhanced::PreparedSidecars,
        hardware: &tje_engine_runtime::enhanced::SnapshotView<'_>,
    ) -> tje_game::Result<()> {
        let session = states.get::<tje_simulation::session::Session>()?;
        let config = session.config();
        if config.catalogue_identity != self.identity
            || config.frame_count != self.frame_count()
            || config.animations != self.contracts
        {
            return Err(tje_game::Error::Platform(
                "co-op restore resource catalogue/configuration mismatch".into(),
            ));
        }
        if session.published.kind == tje_simulation::publication::PublishedKind::World {
            self.validate_static_patterns(hardware.vram)
                .map_err(|error| tje_game::Error::Platform(error.to_string()))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tje_mem::Revision;
    #[test]
    fn resource_spans_reject_unmapped_and_wrapped_addresses() {
        let memory = AddressSpace::new(vec![0; 32], Revision::Rev02);
        assert!(bytes(&memory, u32::MAX, 4).is_err());
        assert!(bytes(&memory, 31, 4).is_err());
        assert_eq!(bytes(&memory, 28, 4).unwrap(), &[0; 4]);
    }
    #[test]
    #[ignore = "explicit private-ROM catalogue admission; no public assets"]
    fn both_user_revisions_import_all_resources_without_presentation() {
        for revision in tje_rom::Revision::ALL {
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../roms");
            let path = root.join(match revision {
                tje_rom::Revision::Rev00 => "rev00.bin",
                tje_rom::Revision::Rev02 => "rev02.bin",
            });
            let rom = tje_rom::Rom::import(&std::fs::read(path).unwrap()).unwrap();
            let memory = AddressSpace::new(rom.into_bytes(), revision);
            let resources = ResourceCatalog::load(&memory).unwrap();
            assert!(resources.reserved_bytes() < MAX_CATALOGUE_BYTES);
            for archetype in [Archetype::ToeJam, Archetype::Earl] {
                for descriptor in 0..121 {
                    let id = resources.hero(archetype, descriptor).unwrap();
                    let frame = resources.selected_frame(id, 0).unwrap().0;
                    assert!(resources.frame(frame).unwrap().count > 0);
                }
            }
            for kind in Kind::ALL {
                assert!(resources.earthling(kind, 0).is_ok());
            }
        }
    }
}

#[cfg(all(test, feature = "enhanced-mode"))]
#[test]
#[ignore = "explicit private-ROM catalogue-contract transactional restore proof"]
fn contracts_mismatching_loaded_rom_reject_before_hardware_or_sidecar_commit() {
    use tje_engine_runtime::{
        Backend, Bus, CpuState, Engine, StepResult,
        enhanced::{DEBUG_PROFILE, EnhancedMode, Resources},
    };
    use tje_mem::Bus as _;
    use tje_simulation::session::{SessionConfig, SetupTransaction, SimulationState};
    struct NoExecution;
    impl Backend for NoExecution {
        type Error = std::convert::Infallible;
        fn execute(
            &mut self,
            _: &mut CpuState,
            _: &mut dyn Bus,
        ) -> std::result::Result<StepResult, Self::Error> {
            panic!("restore admission must not execute a CPU instruction");
        }
    }
    for revision in tje_rom::Revision::ALL {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../roms")
            .join(match revision {
                tje_rom::Revision::Rev00 => "rev00.bin",
                tje_rom::Revision::Rev02 => "rev02.bin",
            });
        let rom = tje_rom::Rom::import(&std::fs::read(path).unwrap()).unwrap();
        let memory = AddressSpace::new(rom.into_bytes(), revision);
        let catalogue = ResourceCatalog::load(&memory).unwrap();
        let sha1 = revision.sha1();
        let world = tje_world::WorldConfig {
            root_seed: 123,
            rom_sha1: std::array::from_fn(|index| {
                u8::from_str_radix(&sha1[index * 2..index * 2 + 2], 16).unwrap()
            }),
            size: tje_world::LevelSize::Original,
            density: tje_world::Density::Original,
        };
        let original = match revision {
            tje_rom::Revision::Rev00 => tje_world::original::OriginalRevision::Rev00,
            tje_rom::Revision::Rev02 => tje_world::original::OriginalRevision::Rev02,
        };
        let mut config = SessionConfig::stock(
            world,
            tje_four_player::Config::four_seats(123),
            original,
            &[],
        )
        .unwrap();
        config.catalogue_identity = catalogue.identity();
        config.frame_count = catalogue.frame_count();
        config.animations = catalogue.animation_contracts();
        config.animations.programs[0].flips[0] ^= 1;
        // Data-only registration authors a shape-valid bad checkpoint. This
        // does not install or bypass admission for a gameplay profile.
        let state = SimulationState::new(&config).unwrap();
        let session = SetupTransaction::prepare(config, state, vec![])
            .unwrap()
            .into_loading()
            .unwrap();
        let mut resources = Resources::new();
        resources.register(catalogue).unwrap();
        let mut mode = EnhancedMode::new(DEBUG_PROFILE, revision)
            .unwrap()
            .with_resources(resources)
            .unwrap();
        mode.sidecars_mut().register(session).unwrap();
        let mut engine = Engine::from_devices(
            tje_native_platform::DeviceRuntime::new(memory, &Default::default()),
            NoExecution,
        );
        let bad = mode.snapshot(&engine).unwrap();
        engine.devices_mut().memory_mut().write_u8(0xff0200, 73);
        let before = mode.snapshot(&engine).unwrap();
        let error = mode.restore(&mut engine, &bad).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("resource catalogue/configuration mismatch"),
            "{error}"
        );
        assert_eq!(
            mode.snapshot(&engine).unwrap().as_bytes(),
            before.as_bytes()
        );
    }
}
