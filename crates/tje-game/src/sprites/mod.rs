//! Faithful sprite translations. G source paths and lines identify the behavioral
//! authority; no graphics data is copied into Rust. See docs/TRANSLATION.md.

use tje_mem::{Address, Bus, ObjectState, Platform, Revision, ram};

mod dma_upload;
mod frame_rendering;
mod lifecycle;
mod links;
mod queue_update;
mod rendering;
mod slots;
mod unpack;
mod updates;

#[cfg(test)]
mod leaf_tests;

/// G/src/sprites/entity_sprite_rendering.c: UpdateAttachedEntityPosition
/// (REV 02 0x00C170). Signed parent offsets deliberately differ from camera offsets.
pub fn update_attached_entity_position(
    bus: &mut (impl Bus + ?Sized),
    revision: Revision,
    entity: Address<ObjectState>,
) {
    let owner = entity.owner().read(bus);
    if owner >= 0 {
        let parent = ram::g_player_info(revision).at(i32::from(owner));
        let x = (i32::from(entity.attachment_x_or_latch().read(bus))
            + i32::from(parent.x().read(bus))) as i16;
        entity.x().write(bus, x);
        let y = (i32::from(entity.attachment_y_or_latch().read(bus))
            + i32::from(parent.y().read(bus))) as i16;
        entity.y().write(bus, y);
        let height = (i32::from(entity.attachment_height().read(bus) as i8)
            + i32::from(parent.height().read(bus))) as i16;
        entity.height().write(bus, height);
        let tile = entity.tile().read(bus);
        let parent_priority = parent.tile().read(bus) & 0x8000 != 0;
        entity.tile().write(
            bus,
            if parent_priority {
                tile | 0x8000
            } else {
                tile & !0x8000
            },
        );
    } else if owner == -1 {
        let x = (i32::from(ram::g_camera1_pending_x(revision).read(bus))
            + i32::from(entity.attachment_x_or_latch().read(bus) as u8)) as i16;
        entity.x().write(bus, x);
        let y = (i32::from(ram::g_camera1_pending_y(revision).read(bus))
            + i32::from(entity.attachment_y_or_latch().read(bus) as u8)) as i16;
        entity.y().write(bus, y);
    } else if owner == -2 {
        let x = (i32::from(ram::g_camera2_pending_x(revision).read(bus))
            + i32::from(entity.attachment_x_or_latch().read(bus) as u8)) as i16;
        entity.x().write(bus, x);
        let y = (i32::from(ram::g_camera2_pending_y(revision).read(bus))
            + i32::from(entity.attachment_y_or_latch().read(bus) as u8)) as i16;
        entity.y().write(bus, y);
    }
}

/// G/src/sprites/select_first_link.c: SelectSpriteChainStart (REV 02 0x00CFBA).
/// Hardware writes precede the RAM mirror update, preserving the original order.
pub fn select_sprite_chain_start(
    bus: &mut (impl Bus + Platform + ?Sized),
    revision: Revision,
    primary_only: i16,
) {
    let link = if primary_only != 0 { 1 } else { 11 };
    bus.write_vdp_command(0x78020003);
    bus.write_vdp_data(u16::from(link));
    ram::g_hardware_sprite_root_link(revision).write(bus, link);
}
/// G/src/sprites/entity_sprite_rendering.c: UpdateAttachedEntityPosition bridge.
pub fn attachment_hook(
    context: &mut dyn crate::Context,
    arguments: &[u32],
) -> crate::Result<Option<u32>> {
    update_attached_entity_position(context, context.revision(), Address::new(arguments[0]));
    Ok(None)
}
/// G/src/sprites/select_first_link.c: SelectSpriteChainStart bridge.
pub fn chain_hook(
    context: &mut dyn crate::Context,
    arguments: &[u32],
) -> crate::Result<Option<u32>> {
    select_sprite_chain_start(context, context.revision(), arguments[0] as i16);
    Ok(None)
}

/// Available Rust entries owned by this subsystem.
pub static TRANSLATIONS: &[crate::Translation] = &[
    crate::Translation {
        id: crate::ids::UPDATE_ATTACHED_ENTITY_POSITION,
        function: attachment_hook,
    },
    crate::Translation {
        id: crate::ids::SELECT_SPRITE_CHAIN_START,
        function: chain_hook,
    },
    crate::Translation {
        id: crate::ids::IS_ENTITY_VISIBLE_IN_VIEWPORT,
        function: rendering::is_entity_visible_in_viewport,
    },
    crate::Translation {
        id: crate::ids::SORT_ENTITIES_FOR_SPRITE_DRAW,
        function: rendering::sort_entities_for_sprite_draw,
    },
    crate::Translation {
        id: crate::ids::CLIP_SPRITE_PIXELS_BELOW_CUTOFF,
        function: rendering::clip_sprite_pixels_below_cutoff,
    },
    crate::Translation {
        id: crate::ids::CLEAR_SPRITE_MASK_PIXELS,
        function: rendering::clear_sprite_mask_pixels,
    },
    crate::Translation {
        id: crate::ids::APPEND_SPRITE_TILES_TO_BUFFER,
        function: rendering::append_sprite_tiles_to_buffer,
    },
    crate::Translation {
        id: crate::ids::SET_SPRITE,
        function: rendering::set_sprite,
    },
    crate::Translation {
        id: crate::ids::TERMINATE_SPRITE_CHAIN_AT_HEAD,
        function: links::terminate_sprite_chain_at_head,
    },
    crate::Translation {
        id: crate::ids::TERMINATE_PRIMARY_SPRITE_CHAIN,
        function: links::terminate_primary_sprite_chain,
    },
    crate::Translation {
        id: crate::ids::SELECT_PRIMARY_VIEWPORT_SPRITES,
        function: links::select_primary_viewport_sprites,
    },
    crate::Translation {
        id: crate::ids::SELECT_SECONDARY_VIEWPORT_SPRITES,
        function: links::select_secondary_viewport_sprites,
    },
    crate::Translation {
        id: crate::ids::HIDE_FLAGGED_PLAYER_SPRITES,
        function: links::hide_flagged_player_sprites,
    },
    crate::Translation {
        id: crate::ids::SET_RAM_INDEXES,
        function: rendering::set_ram_indexes,
    },
    crate::Translation {
        id: crate::ids::FREE_SPRITE_SLOTS,
        function: slots::free_sprite_slots,
    },
    crate::Translation {
        id: crate::ids::FREE_SPRITE_SLOT,
        function: slots::free_sprite_slot,
    },
    crate::Translation {
        id: crate::ids::IS_SPRITE_SLOT_LOCKED,
        function: slots::is_sprite_slot_locked,
    },
    crate::Translation {
        id: crate::ids::ALLOCATE_SPRITE_TILE_RUN,
        function: slots::allocate_sprite_tile_run,
    },
    crate::Translation {
        id: crate::ids::FREE_SPRITE_TILE_RUN,
        function: slots::free_sprite_tile_run,
    },
    crate::Translation {
        id: crate::ids::RESET_ENTITY_RUNTIME_STATE,
        function: slots::reset_entity_runtime_state,
    },
    crate::Translation {
        id: crate::ids::SPRITE_UNPACK_INTERLEAVED_RLE,
        function: unpack::sprite_unpack_interleaved_rle,
    },
    crate::Translation {
        id: crate::ids::SPRITE_UNPACK_RLE,
        function: unpack::sprite_unpack_rle,
    },
    crate::Translation {
        id: crate::ids::SPRITE_UNPACK_RAW,
        function: unpack::sprite_unpack_raw,
    },
    crate::Translation {
        id: crate::ids::ALLOCATE_ENTITY,
        function: lifecycle::allocate_entity,
    },
    crate::Translation {
        id: crate::ids::ALLOCATE_ENTITY_VIEWPORT_SPRITES,
        function: lifecycle::allocate_entity_viewport_sprites,
    },
    crate::Translation {
        id: crate::ids::RELEASE_ENTITY_SHADOWS,
        function: lifecycle::release_entity_shadows,
    },
    crate::Translation {
        id: crate::ids::RELEASE_ENTITY_VIEWPORT_SPRITES,
        function: lifecycle::release_entity_viewport_sprites,
    },
    crate::Translation {
        id: crate::ids::POSITION_ENTITY_SPRITES,
        function: frame_rendering::position_entity_sprites,
    },
    crate::Translation {
        id: crate::ids::QUEUE_SPRITE_PIECE_TILES,
        function: frame_rendering::queue_sprite_piece_tiles,
    },
    crate::Translation {
        id: crate::ids::FREE_ENTITY,
        function: updates::free_entity,
    },
    crate::Translation {
        id: crate::ids::UPDATE_ENTITY_SHADOWS,
        function: updates::update_entity_shadows,
    },
    crate::Translation {
        id: crate::ids::UPDATE_ALL_ENTITY_SHADOWS,
        function: updates::update_all_entity_shadows,
    },
    crate::Translation {
        id: crate::ids::UPDATE_ENTITY_VIEWPORT_SPRITES,
        function: updates::update_entity_viewport_sprites,
    },
    crate::Translation {
        id: crate::ids::UPDATE_ALL_ENTITY_VIEWPORT_SPRITES,
        function: updates::update_all_entity_viewport_sprites,
    },
    crate::Translation {
        id: crate::ids::PROCESS_ENTITY_ANIMATION_MOTION,
        function: updates::process_entity_animation_motion,
    },
    crate::Translation {
        id: crate::ids::LINK_PLAYER_SPRITES_FOR_SPLIT_WIPE,
        function: dma_upload::link_player_sprites_for_split_wipe,
    },
];

/// Reviewed bodies/entries not yet eligible for production replacement.
pub static PENDING: &[crate::FunctionId] = &[
    // Source-backed frame bodies await two-revision local/slot/callsite proof.
    crate::ids::ALLOCATE_SPRITE_SLOTS,
    crate::ids::BUILD_VIEWPORT_SPRITE_LINKS,
    crate::ids::ALLOCATE_SPRITE_SLOT,
    // The original loads a dead high byte immediately discarded by EXT.W.
    // Await the shared semantic-dependency audit before claiming both-ROM proof.
    crate::ids::PREPARE_ENTITY_SPRITE_FRAME,
    // Zero-length DMA exposes original call-frame RAM; shared frame/callsite proof is required.
    crate::ids::UPLOAD_QUEUED_SPRITE_GRAPHICS,
    // Real-IRQ wait/pipeline body awaits TimeAware and original-callee dependency proof.
    crate::ids::UPDATE_AND_QUEUE_ENTITY_SPRITES,
];

/// Explicit proof candidates, never consulted by production dispatch.
pub static PENDING_TRANSLATIONS: &[crate::Translation] = &[
    crate::Translation {
        id: crate::ids::PREPARE_ENTITY_SPRITE_FRAME,
        function: frame_rendering::prepare_entity_sprite_frame,
    },
    crate::Translation {
        id: crate::ids::UPLOAD_QUEUED_SPRITE_GRAPHICS,
        function: dma_upload::upload_queued_sprite_graphics,
    },
    crate::Translation {
        id: crate::ids::UPDATE_AND_QUEUE_ENTITY_SPRITES,
        function: queue_update::update_and_queue_entity_sprites,
    },
    crate::Translation {
        id: crate::ids::ALLOCATE_SPRITE_SLOTS,
        function: slots::allocate_sprite_slots,
    },
    crate::Translation {
        id: crate::ids::BUILD_VIEWPORT_SPRITE_LINKS,
        function: rendering::build_viewport_sprite_links,
    },
    crate::Translation {
        id: crate::ids::ALLOCATE_SPRITE_SLOT,
        function: slots::allocate_sprite_slot,
    },
];

/// Reviewed original entries that return without clock/interrupt progress.
pub static ATOMIC_CAPABLE: &[crate::FunctionId] = &[
    crate::ids::UPDATE_ATTACHED_ENTITY_POSITION,
    crate::ids::SELECT_SPRITE_CHAIN_START,
    // All W0 leaves below have no original callees, hardware polls, or waits.
    // The live VDP patches perform only bounded command/data writes.
    crate::ids::IS_ENTITY_VISIBLE_IN_VIEWPORT,
    crate::ids::SORT_ENTITIES_FOR_SPRITE_DRAW,
    crate::ids::CLIP_SPRITE_PIXELS_BELOW_CUTOFF,
    crate::ids::CLEAR_SPRITE_MASK_PIXELS,
    crate::ids::APPEND_SPRITE_TILES_TO_BUFFER,
    crate::ids::SET_SPRITE,
    crate::ids::TERMINATE_SPRITE_CHAIN_AT_HEAD,
    crate::ids::TERMINATE_PRIMARY_SPRITE_CHAIN,
    crate::ids::SELECT_PRIMARY_VIEWPORT_SPRITES,
    crate::ids::SELECT_SECONDARY_VIEWPORT_SPRITES,
    crate::ids::HIDE_FLAGGED_PLAYER_SPRITES,
    crate::ids::SET_RAM_INDEXES,
    crate::ids::FREE_SPRITE_SLOTS,
    crate::ids::FREE_SPRITE_SLOT,
    crate::ids::IS_SPRITE_SLOT_LOCKED,
    crate::ids::ALLOCATE_SPRITE_TILE_RUN,
    crate::ids::FREE_SPRITE_TILE_RUN,
    crate::ids::RESET_ENTITY_RUNTIME_STATE,
    crate::ids::SPRITE_UNPACK_INTERLEAVED_RLE,
    crate::ids::SPRITE_UNPACK_RLE,
    crate::ids::SPRITE_UNPACK_RAW,
    // Original-only classification, independent of translation readiness.
    // G/src/sprites/sprite_slots.c:10-276: RAM pool scans, byte/word stores,
    // and the already-reviewed reset/free/query helpers; no hardware or waits.
    // Stack-local and residual-register bugs affect translation, not scheduling.
    crate::ids::ALLOCATE_SPRITE_SLOTS,
    crate::ids::ALLOCATE_SPRITE_SLOT,
    crate::ids::ALLOCATE_ENTITY,
    crate::ids::ALLOCATE_ENTITY_VIEWPORT_SPRITES,
    crate::ids::RELEASE_ENTITY_SHADOWS,
    crate::ids::RELEASE_ENTITY_VIEWPORT_SPRITES,
    crate::ids::FREE_ENTITY,
    // G/src/sprites/entity_sprite_rendering.c:43-488: RAM lists, positioning,
    // decoding and tile queues only. Motion's four original callback targets
    // perform bounded RAM/ROM terrain/collision work, without device progress.
    crate::ids::POSITION_ENTITY_SPRITES,
    crate::ids::BUILD_VIEWPORT_SPRITE_LINKS,
    crate::ids::QUEUE_SPRITE_PIECE_TILES,
    crate::ids::PREPARE_ENTITY_SPRITE_FRAME,
    crate::ids::PROCESS_ENTITY_ANIMATION_MOTION,
    // G/src/sprites/entity_visibility.c:45-94: visibility plus RAM allocation
    // and release; IsSpeechBubbleEntity is a four-slot RAM scan.
    crate::ids::UPDATE_ENTITY_VIEWPORT_SPRITES,
    crate::ids::UPDATE_ALL_ENTITY_VIEWPORT_SPRITES,
    // G/src/sprites/entity_depth.c:10-80: RAM shadow attributes and allocation,
    // using only the reviewed sprite-slot query/allocation/release closure.
    crate::ids::UPDATE_ENTITY_SHADOWS,
    crate::ids::UPDATE_ALL_ENTITY_SHADOWS,
];

/// Reviewed non-atomic original entries. Classification is not translation proof.
pub static EXECUTION_CONTRACTS: &[(crate::FunctionId, crate::execution::ExecutionClass)] = &[
    // G/src/sprites/load_sprites.c:8-31: closed stable-RAM DMA sources and
    // ordered register writes, with no status/raster read or concurrent observer.
    // DmaCopyWordsToVram (G/src/asm/dma.s:15-75) needs transaction completion.
    (
        crate::ids::UPLOAD_QUEUED_SPRITE_GRAPHICS,
        crate::execution::ExecutionClass::Functional,
    ),
    // G/src/sprites/link_player_sprites.c:9-38: finish all table RAM mutations,
    // then one stable-source DMA through the same reviewed primitive.
    (
        crate::ids::LINK_PLAYER_SPRITES_FOR_SPLIT_WIPE,
        crate::execution::ExecutionClass::Functional,
    ),
    // G/src/sprites/entity_sprite_rendering.c:503-584: the pending-upload byte
    // at lines 513-514 is cleared by interrupt-driven work, so real IRQ/clock
    // progress is required. The remaining original callees have no waits.
    (
        crate::ids::UPDATE_AND_QUEUE_ENTITY_SPRITES,
        crate::execution::ExecutionClass::TimeAware,
    ),
];
#[cfg(test)]
mod tests {
    use super::*;
    use tje_mem::{AddressSpace, RAM_START};

    fn memory(revision: Revision) -> AddressSpace {
        AddressSpace::new(vec![0; 64], revision)
    }

    #[test]
    fn parent_attachment_preserves_signed_views_and_narrowing() {
        for revision in [Revision::Rev02, Revision::Rev00] {
            let mut bus = memory(revision);
            let parent = ram::g_player_info(revision).at(1);
            let entity = ram::g_player_info(revision).at(6);
            parent.x().write(&mut bus, 32760);
            parent.y().write(&mut bus, -32760);
            parent.height().write(&mut bus, 100);
            parent.tile().write(&mut bus, 0x8123);
            entity.owner().write(&mut bus, 1);
            entity.attachment_x_or_latch().write(&mut bus, 127);
            entity.attachment_y_or_latch().write(&mut bus, -128);
            entity.attachment_height().write(&mut bus, 254);
            entity.tile().write(&mut bus, 0x7fff);
            update_attached_entity_position(&mut bus, revision, entity);
            assert_eq!(entity.x().read(&bus), -32649);
            assert_eq!(entity.y().read(&bus), 32648);
            assert_eq!(entity.height().read(&bus), 98);
            assert_eq!(entity.tile().read(&bus), 0xffff);
            assert_eq!(parent.x().read(&bus), 32760);
            parent.tile().write(&mut bus, 0x123);
            update_attached_entity_position(&mut bus, revision, entity);
            assert_eq!(entity.tile().read(&bus), 0x7fff);
        }
    }

    #[test]
    fn owner_zero_is_a_real_parent_and_can_alias_the_entity() {
        let revision = Revision::Rev02;
        let mut bus = memory(revision);
        let entity = ram::g_player_info(revision).at(0);
        entity.owner().write(&mut bus, 0);
        entity.x().write(&mut bus, 32767);
        entity.y().write(&mut bus, -32768);
        entity.height().write(&mut bus, 1);
        entity.attachment_x_or_latch().write(&mut bus, 1);
        entity.attachment_y_or_latch().write(&mut bus, -1);
        entity.attachment_height().write(&mut bus, 255);
        entity.tile().write(&mut bus, 0x8123);
        update_attached_entity_position(&mut bus, revision, entity);
        assert_eq!(entity.x().read(&bus), -32768);
        assert_eq!(entity.y().read(&bus), 32767);
        assert_eq!(entity.height().read(&bus), 0);
        assert_eq!(entity.tile().read(&bus), 0x8123);
    }

    #[test]
    fn camera_attachment_reinterprets_offsets_as_unsigned() {
        let revision = Revision::Rev02;
        let mut bus = memory(revision);
        let entity = ram::g_player_info(revision).at(6);
        entity.owner().write(&mut bus, -1);
        entity.attachment_x_or_latch().write(&mut bus, -1);
        entity.attachment_y_or_latch().write(&mut bus, -128);
        entity.height().write(&mut bus, -7);
        entity.tile().write(&mut bus, 0x1234);
        ram::g_camera1_pending_x(revision).write(&mut bus, 100);
        ram::g_camera1_pending_y(revision).write(&mut bus, -100);
        update_attached_entity_position(&mut bus, revision, entity);
        assert_eq!(entity.x().read(&bus), 355);
        assert_eq!(entity.y().read(&bus), 28);
        assert_eq!(entity.height().read(&bus), -7);
        assert_eq!(entity.tile().read(&bus), 0x1234);
        entity.owner().write(&mut bus, -2);
        ram::g_camera2_pending_x(revision).write(&mut bus, 32760);
        ram::g_camera2_pending_y(revision).write(&mut bus, 20);
        update_attached_entity_position(&mut bus, revision, entity);
        assert_eq!(entity.x().read(&bus), -32521);
        assert_eq!(entity.y().read(&bus), 148);
        entity.owner().write(&mut bus, -3);
        let before = bus.ram().to_vec();
        update_attached_entity_position(&mut bus, revision, entity);
        assert_eq!(bus.ram().as_slice(), before);
    }

    #[derive(Debug, PartialEq, Eq)]
    enum Write {
        Control(u16),
        Data(u16),
        Ram(u32, u8),
    }
    struct Trace {
        memory: AddressSpace,
        events: Vec<Write>,
    }
    impl Bus for Trace {
        fn read_u8(&self, address: u32) -> u8 {
            self.memory.read_u8(address)
        }
        fn write_u8(&mut self, address: u32, value: u8) {
            self.events.push(Write::Ram(address, value));
            self.memory.write_u8(address, value);
        }
    }
    impl Platform for Trace {
        fn write_vdp_control(&mut self, value: u16) {
            self.events.push(Write::Control(value));
        }
        fn write_vdp_data(&mut self, value: u16) {
            self.events.push(Write::Data(value));
        }
    }

    #[test]
    fn vdp_example_preserves_port_width_order_and_ram_alias() {
        for revision in [Revision::Rev02, Revision::Rev00] {
            let mut bus = memory(revision);
            bus.ram_mut().fill(0xa5);
            let mut trace = Trace {
                memory: bus,
                events: Vec::new(),
            };
            for (argument, expected) in [(0, 11), (1, 1), (-1, 1), (i16::MIN, 1)] {
                trace.events.clear();
                select_sprite_chain_start(&mut trace, revision, argument);
                assert_eq!(
                    trace.events,
                    [
                        Write::Control(0x7802),
                        Write::Control(3),
                        Write::Data(u16::from(expected)),
                        Write::Ram(
                            ram::g_hardware_sprite_root_link(revision).address(),
                            expected
                        )
                    ]
                );
                assert_eq!(
                    ram::g_hardware_sprite_root_link(revision).read(&trace),
                    expected
                );
                assert_eq!(
                    ram::g_hardware_sprite_table(revision)
                        .at(0)
                        .link()
                        .read(&trace),
                    expected
                );
            }
            let offset =
                (ram::g_hardware_sprite_root_link(revision).address() - RAM_START) as usize;
            assert!(
                trace
                    .memory
                    .ram()
                    .iter()
                    .enumerate()
                    .all(|(i, byte)| i == offset || *byte == 0xa5)
            );
        }
    }
}
