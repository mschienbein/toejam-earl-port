use super::get_level_music_index;
use tje_mem::{AddressSpace, Bus, RAM_START, Revision};

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
fn reset_vdp_preserves_register_write_order() {
    use super::test_support::{TestContext, Write};
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = TestContext::new(revision);
        super::reset_vdp(&mut context, &[]).unwrap();
        let expected = [
            0x8004, 0x8144, 0x8228, 0x8334, 0x8407, 0x857c, 0x8700, 0x8a00, 0x8b00, 0x8c89, 0x8d3f,
            0x8f02, 0x9001, 0x9100, 0x9200, 0x9302, 0x9400, 0x9500, 0x9600, 0x9700,
        ]
        .into_iter()
        .map(Write::Control)
        .collect::<Vec<_>>();
        assert_eq!(context.writes, expected);
        assert!(context.memory.ram().iter().all(|&byte| byte == 0));
    }
}

#[test]
fn palette_animation_keeps_byte_overflow_and_write_order() {
    use super::test_support::{TestContext, Write};
    use tje_mem::{ram, rom};
    for revision in [Revision::Rev02, Revision::Rev00] {
        for level in [-1, 25, 26, 127] {
            for delay in [i8::MIN, -1, 0, 1, i8::MAX] {
                let mut bytes = vec![0; 0x100000];
                for (table, marker) in [
                    (rom::palette_animation_frames_a(revision), 0x100_u16),
                    (rom::palette_animation_frames_b(revision), 0x200),
                ] {
                    for colour in 0..4 {
                        let address = table.at(0).at(colour).address() as usize;
                        bytes[address..address + 2]
                            .copy_from_slice(&(marker + colour as u16).to_be_bytes());
                    }
                }
                let mut context = TestContext::with_rom(revision, bytes);
                ram::g_player1_map_level(revision).write(&mut context, level);
                ram::g_level_palette_animation_delay(revision).write(&mut context, delay);
                ram::g_level_palette_animation_frame(revision).write(&mut context, -1);
                context.writes.clear();
                super::draw_level_graphics(&mut context, &[]).unwrap();
                let delay_view = ram::g_level_palette_animation_delay(revision);
                assert_eq!(
                    context.writes[0],
                    Write::Ram(delay_view.address(), delay.wrapping_sub(1) as u8)
                );
                if delay <= 0 {
                    assert_eq!(delay_view.read(&context), 3);
                    assert_eq!(
                        ram::g_level_palette_animation_frame(revision).read(&context),
                        0
                    );
                    let marker = if level < 26 { 0x100 } else { 0x200 };
                    assert_eq!(
                        &context.writes[3..],
                        &[
                            Write::Control(0xc034),
                            Write::Control(0),
                            Write::Data(marker),
                            Write::Data(marker + 1),
                            Write::Data(marker + 2),
                            Write::Data(marker + 3),
                        ]
                    );
                } else {
                    assert_eq!(delay_view.read(&context), delay - 1);
                    assert_eq!(context.writes.len(), 1);
                }
            }
        }
    }
}

#[test]
fn view_separation_preserves_thresholds_flight_and_word_overflow() {
    use super::test_support::TestContext;
    use tje_mem::ram;
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = TestContext::new(revision);
        let toejam = ram::g_player_info(revision).at(0);
        let earl = ram::g_player_info(revision).at(1);
        let check = |context: &mut TestContext| {
            super::players_require_separate_views(context, &[])
                .unwrap()
                .unwrap()
        };
        assert_eq!(check(&mut context), 0);
        earl.level().write(&mut context, 1);
        assert_eq!(check(&mut context), 1);
        ram::g_player2_lives(revision).write(&mut context, -1);
        assert_eq!(check(&mut context), 0);
        ram::g_player2_lives(revision).write(&mut context, 0);
        earl.level().write(&mut context, 0);
        for (x, y, expected) in [(220, 124, 0), (221, 124, 1), (220, 125, 1)] {
            earl.x().write(&mut context, x);
            earl.y().write(&mut context, y);
            assert_eq!(check(&mut context), expected);
        }
        earl.x().write(&mut context, 0);
        earl.y().write(&mut context, 124);
        toejam.height().write(&mut context, 1);
        toejam.animation_group_mask().write(&mut context, 0x8000);
        assert_eq!(check(&mut context), 1);
        toejam.animation_group_mask().write(&mut context, 0);
        assert_eq!(check(&mut context), 0);
        toejam.y().write(&mut context, i16::MIN);
        earl.y().write(&mut context, 1);
        assert_eq!(check(&mut context), 0);
        toejam.y().write(&mut context, 0);
        earl.y().write(&mut context, 0);
        toejam.x().write(&mut context, i16::MIN);
        earl.x().write(&mut context, 1);
        assert_eq!(check(&mut context), 0);
    }
}

#[test]
fn finite_iteration_delay_has_no_memory_or_hardware_effect() {
    use super::test_support::TestContext;
    let mut context = TestContext::new(Revision::Rev02);
    for argument in [0, 1, 10, 32767, 32768, 65535, u32::MAX] {
        assert_eq!(
            super::delay_iterations(&mut context, &[argument]).unwrap(),
            None
        );
    }
    assert!(context.writes.is_empty());
}
