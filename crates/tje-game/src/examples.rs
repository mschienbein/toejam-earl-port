//! Worked translations, with synthetic-memory tests. Names identify G's source,
//! not newly claimed original historical C spellings. See docs/TRANSLATION.md.

use tje_mem::{Address, Bus, ObjectState, Platform, Revision, ram};

/// G/src/core/startup_and_main_loop.c: GetLevelMusicIndex (REV 02 0x001262).
/// The subtraction is promoted to 32-bit int before the signed remainder.
pub fn get_level_music_index(selection: i16) -> i16 {
    ((i32::from(selection) - 1) % 6) as i16
}

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

/// G/src/core/startup_and_main_loop.c: GetLevelMusicIndex, MCC68K entry bridge.
pub fn music_hook(_: &mut dyn crate::Context, arguments: &[u32]) -> crate::Result<Option<u32>> {
    Ok(Some(
        i32::from(get_level_music_index(arguments[0] as i16)) as u32
    ))
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

#[cfg(test)]
mod tests {
    use super::*;
    use tje_mem::{AddressSpace, RAM_START};

    fn memory(revision: Revision) -> AddressSpace {
        AddressSpace::new(vec![0; 64], revision)
    }

    #[test]
    fn leaf_arithmetic_reads_and_writes_synthetic_memory() {
        let mut bus = memory(Revision::Rev02);
        for (argument, expected) in [
            (i16::MIN, -3),
            (-1, -2),
            (0, -1),
            (1, 0),
            (2, 1),
            (6, 5),
            (7, 0),
            (i16::MAX, 0),
        ] {
            bus.write_i16(RAM_START, argument);
            let result = get_level_music_index(bus.read_i16(RAM_START));
            bus.write_i16(RAM_START + 2, result);
            assert_eq!(bus.read_i16(RAM_START + 2), expected);
        }
        assert_eq!(bus.ram()[0..2], [0x7f, 0xff]);
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
