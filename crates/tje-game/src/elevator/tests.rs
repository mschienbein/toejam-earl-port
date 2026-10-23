use super::*;
use tje_mem::{AddressSpace, Ptr};

#[test]
fn set_state_installs_both_revision_descriptors_and_keeps_early_returns() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut bytes = vec![0; 0x100000];
        for (table, sequence, frames, period, offset) in [
            (
                rom::elevator_primary_animations(revision),
                0x1234_u32,
                0x5678_u32,
                3_u8,
                0_u8,
            ),
            (
                rom::elevator_secondary_animations(revision),
                0x4321,
                0x8765,
                7,
                0xf0,
            ),
        ] {
            let address = table.at(2).address() as usize;
            bytes[address..address + 4].copy_from_slice(&sequence.to_be_bytes());
            bytes[address + 4..address + 8].copy_from_slice(&frames.to_be_bytes());
            bytes[address + 8] = period;
            bytes[address + 9] = offset;
        }
        let mut memory = AddressSpace::new(bytes, revision);
        let elevator = ram::g_elevator_records(revision).at(0).at(0);
        let primary = ram::g_player_info(revision).at(2);
        let secondary = ram::g_player_info(revision).at(4);
        primary.x().write(&mut memory, -123);
        primary.y().write(&mut memory, i16::MIN + 8);
        elevator.entity().write(&mut memory, 2);
        elevator_set_state(&mut memory, revision, elevator, 2);
        assert_eq!(primary.sequence().read(&memory), Ptr::new(0x1234));
        assert_eq!(primary.frames().read(&memory), Ptr::new(0x5678));
        assert_eq!(primary.frame_phase().read(&memory), 3);
        assert_eq!(primary.frame_duration().read(&memory), 1);
        assert_eq!(primary.frame().read(&memory), -1);
        assert_eq!(primary.sprite_state().read(&memory), 2);
        assert_eq!(secondary.sequence().read(&memory), Ptr::new(0x4321));
        assert_eq!(secondary.frames().read(&memory), Ptr::new(0x8765));
        assert_eq!(secondary.frame_phase().read(&memory), 7);
        assert_eq!(secondary.x().read(&memory), -123);
        assert_eq!(secondary.y().read(&memory), i16::MAX - 7);
        elevator.level().write(&mut memory, 9);
        let saved_sequence = primary.sequence().read(&memory);
        elevator_set_state(&mut memory, revision, elevator, 11);
        assert_eq!(elevator.state().read(&memory), 11);
        assert_eq!(elevator.level().read(&memory), -1);
        assert_eq!(primary.sequence().read(&memory), saved_sequence);
        elevator.entity().write(&mut memory, 0);
        elevator_set_state(&mut memory, revision, elevator, -1);
        assert_eq!(elevator.state().read(&memory), -1);
        assert_eq!(primary.sequence().read(&memory), saved_sequence);
    }
}

#[test]
fn map_cell_conversion_sign_extends_and_loads_both_coordinates_before_stores() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut memory = AddressSpace::new(vec![], revision);
        let player = ram::g_player_info(revision).at(0);
        for (map_x, map_y) in [(-128_i8, 127_i8), (-1, -2), (0, 0), (127, -128)] {
            ram::g_arrival_elevator_map_x(revision).write(&mut memory, map_x);
            ram::g_arrival_elevator_map_y(revision).write(&mut memory, map_y);
            player_set_position_from_saved_map_cell(&mut memory, revision, player);
            assert_eq!(
                player.x().read(&memory),
                ((i32::from(map_x) + 2) * 128 - 16) as i16
            );
            assert_eq!(
                player.y().read(&memory),
                ((i32::from(map_y) + 2) * 64 - 5) as i16
            );
        }
        ram::g_arrival_elevator_map_x(revision).write(&mut memory, 1);
        ram::g_arrival_elevator_map_y(revision).write(&mut memory, 2);
        let aliased_player = ram::g_arrival_elevator_map_x(revision).cast::<ObjectState>();
        player_set_position_from_saved_map_cell(&mut memory, revision, aliased_player);
        assert_eq!(aliased_player.x().read(&memory), 368);
        assert_eq!(aliased_player.y().read(&memory), 251);
    }
}

#[test]
fn invalidation_changes_only_arrival_level_and_state() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut memory = AddressSpace::new(vec![], revision);
        let records = ram::g_elevator_records(revision);
        for bank in 0..2 {
            for kind in 0..2 {
                let record = records.at(bank).at(kind);
                record.level().write(&mut memory, 6);
                record.state().write(&mut memory, 3);
                record.entity().write(&mut memory, 2);
            }
        }
        elevator_invalidate_secondary_records(&mut memory, revision);
        for bank in 0..2 {
            assert_eq!(records.at(bank).at(0).state().read(&memory), 3);
            assert_eq!(records.at(bank).at(0).level().read(&memory), 6);
            assert_eq!(records.at(bank).at(1).state().read(&memory), 11);
            assert_eq!(records.at(bank).at(1).level().read(&memory), -1);
            assert_eq!(records.at(bank).at(1).entity().read(&memory), 2);
        }
    }
}

#[test]
fn query_decodes_every_byte_handle_including_wrapped_and_negative_values() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut memory = AddressSpace::new(vec![], revision);
        let records = ram::g_elevator_records(revision);
        for byte in 0..=255_u32 {
            let handle = byte as i8;
            let zero_based = handle.wrapping_sub(1);
            let offset = i32::from(zero_based >> 1) * 16 + i32::from(zero_based & 1) * 8;
            let state = records.cast::<u8>().byte_offset(offset + 5);
            state.write(&mut memory, 3);
            assert_eq!(process_elevator(&memory, revision, handle), 1);
            state.write(&mut memory, 2);
            assert_eq!(process_elevator(&memory, revision, handle), 0);
        }
    }
}

#[test]
fn byte_delay_predecrement_consumes_subtraction_overflow_before_reload() {
    for value in i8::MIN..=i8::MAX {
        let (stored, expired) = decrement_byte_and_test_le_zero(value);
        let overflow = value == i8::MIN;
        let negative = stored < 0;
        let zero = stored == 0;
        let bgt = !zero && negative == overflow;
        assert_eq!(stored, value.wrapping_sub(1));
        assert_eq!(expired, !bgt, "delay {value}");
    }
    assert_eq!(decrement_byte_and_test_le_zero(i8::MIN), (i8::MAX, true));
    assert_eq!(decrement_byte_and_test_le_zero(1), (0, true));
    assert_eq!(decrement_byte_and_test_le_zero(2), (1, false));
}
