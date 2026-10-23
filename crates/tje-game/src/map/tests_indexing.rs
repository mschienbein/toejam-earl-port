//! Additional map compiler-index-width regressions.
use super::*;
#[test]
fn range_pair_index_uses_a_signed_word_offset_in_both_revisions() {
    for r in Revision::ALL {
        let mut bytes = vec![0; 0x100000];
        let base = rom::map_generation_range_pairs(r).address() as usize;
        bytes[base..base + 2].copy_from_slice(&[10, 10]);
        bytes[base - 2..base].copy_from_slice(&[30, 30]);
        bytes[base - 65536..base - 65534].copy_from_slice(&[20, 20]);
        bytes[base + 65534..base + 65536].copy_from_slice(&[40, 40]);
        let mut c = Machine::with_rom(r, bytes);
        ram::g_requested_map_level(r).write(&mut c, 2);
        ram::g_rng_state(r).write(&mut c, 1);
        for (index, expected) in [(i16::MIN, 10), (i16::MAX, 30), (-1, 30)] {
            assert_eq!(
                map_get_random_generation_parameter(&mut c, &[i32::from(index) as u32]).unwrap(),
                Some(expected)
            );
        }
    }
}

#[test]
fn path_consumption_keeps_long_flat_map_and_word_direction_bank_indexes_distinct() {
    for r in Revision::ALL {
        for row in [8000_i16, -8000] {
            for bank in [0_u32, 1] {
                let mut bytes = vec![0; 0x100000];
                for shape in 0..16 {
                    let tile = rom::map_generation_lookup_tables(r)
                        .tiles()
                        .at(shape)
                        .address() as usize;
                    bytes[tile] = shape as u8;
                    let slot = rom::tile_alternative_pointers(r).at(shape).address() as usize;
                    let list = 0x4000 + shape as usize * 4;
                    bytes[slot..slot + 4].copy_from_slice(&(list as u32).to_be_bytes());
                    bytes[list..list + 2].copy_from_slice(&[1, 0x20 + shape as u8]);
                }
                let map = if bank == 0 {
                    ram::g_player1_level_map(r)
                } else {
                    ram::g_player2_level_map(r)
                };
                let cell = map.cast::<u16>().offset(i32::from(row) * 19);
                let physical = cell.address() & 0xffffff;
                if physical < 0x100000 {
                    bytes[physical as usize..physical as usize + 2]
                        .copy_from_slice(&0x4021_u16.to_be_bytes());
                }
                let mut c = Machine::with_rom(r, bytes);
                if physical >= 0xe00000 {
                    cell.write(&mut c, 0x4021);
                }
                let mask = ram::g_map_path_direction_banks(r)
                    .cast::<u8>()
                    .byte_offset(i32::from(row.wrapping_mul(19)));
                mask.write(&mut c, if bank == 0 { 0xa1 } else { 0x1a });
                ram::g_land_path_traversal_direction(r).write(&mut c, -1);
                ram::g_rng_state(r).write(&mut c, 1);
                assert_eq!(
                    map_consume_path_cell_directions(&mut c, &[bank, i32::from(row) as u32, 0])
                        .unwrap(),
                    Some(0)
                );
                assert_eq!(mask.read(&c), if bank == 0 { 0xa0 } else { 0x0a });
            }
        }
    }
}

#[test]
fn reveal_level_stride_wraps_as_a_word_not_a_long_product() {
    for r in Revision::ALL {
        for level in [10000_i16, -10000] {
            let mut c = Machine::new(r);
            ram::g_rng_state(r).write(&mut c, 1);
            let flags = ram::g_map_visited_cover_bits(r)
                .cast::<u8>()
                .byte_offset(i32::from(level.wrapping_mul(7)));
            for row in 0..7 {
                flags.offset(row).write(&mut c, 0xff);
            }
            display_map(
                &mut c,
                &[
                    ram::g_player1_level_map(r).address(),
                    ram::g_saved_level_map(r).address(),
                    i32::from(level) as u32,
                ],
            )
            .unwrap();
            assert!(c.writes.is_empty());
        }
    }
}

#[test]
fn terrain_rectangle_rows_use_word_scaled_rom_offsets() {
    for r in Revision::ALL {
        let mut bytes = vec![0; 0x100000];
        bytes[rom::player_tile_property_lookup(r).address() as usize] = 0x10;
        let mut c = Machine::with_rom(r, bytes);
        ram::g_rng_state(r).write(&mut c, 1);
        let x = ram::g_saved_level_map(r).cast::<i16>();
        let y = x.offset(1);
        assert_eq!(
            terrain_validation(&mut c, &[30, 1, 1025, x.address(), y.address(), 1]).unwrap(),
            Some(1)
        );
        assert_eq!((x.read(&c), y.read(&c)), (12, 11582));
    }
}

#[test]
fn signed_join_chance_rejects_negative_percentages_without_skipping_rng_draws() {
    for r in Revision::ALL {
        for join in [-1_i16, i16::MIN] {
            let mut c = Machine::new(r);
            for row in 0..26 {
                for column in 0..19 {
                    ram::g_player1_level_map(r)
                        .at(row)
                        .at(column)
                        .write(&mut c, 0x1200);
                }
            }
            let row = ram::g_saved_level_map(r).cast::<i16>();
            let column = row.offset(1);
            row.write(&mut c, 5);
            column.write(&mut c, 5);
            ram::g_rng_state(r).write(&mut c, 1);
            assert_eq!(
                map_find_region_growth_origin(
                    &mut c,
                    &[
                        row.address(),
                        column.address(),
                        1,
                        0x1100,
                        i32::from(join) as u32
                    ]
                )
                .unwrap(),
                Some(0)
            );
            assert_eq!((row.read(&c), column.read(&c)), (5, 5));
            assert_eq!(ram::g_rng_state(r).read(&c), 1_622_650_073);
        }
    }
}

#[test]
fn edge_palette_compares_narrowed_parameter_as_signed_long() {
    for r in Revision::ALL {
        let mut bytes = vec![0; 0x100000];
        let pair = rom::map_generation_range_pairs(r).at(0).at(5).address() as usize;
        bytes[pair..pair + 2].copy_from_slice(&[255, 0]);
        let plain = rom::map_generation_lookup_tables(r)
            .edge_plain()
            .at(15)
            .address() as usize;
        let alternate = rom::map_generation_lookup_tables(r)
            .edge_alternate()
            .at(15)
            .address() as usize;
        bytes[plain] = 0x11;
        bytes[alternate] = 0x22;
        let mut c = Machine::with_rom(r, bytes);
        ram::g_requested_map_level(r).write(&mut c, 2);
        ram::g_rng_state(r).write(&mut c, 2);
        for row in 0..26 {
            for column in 0..19 {
                ram::g_player1_level_map(r)
                    .at(row)
                    .at(column)
                    .write(&mut c, 0x1000);
            }
        }
        map_build_region_edge_shapes(&mut c, &[1]).unwrap();
        assert_eq!(ram::g_player1_level_map(r).at(5).at(5).read(&c), 0x1811);
    }
}

#[test]
fn rng_preserves_compiled_volatile_residues_for_nonreturning_continuations() {
    for r in Revision::ALL {
        for seed in [0, 1, 12345, i32::MAX, -1, i32::MIN] {
            let mut c = Machine::new(r);
            c.status = 0x271f;
            c.data[1] = 0xdeadbeef;
            c.address[0] = 0xabcdef01;
            c.address[1] = 0x12345678;
            ram::g_rng_state(r).write(&mut c, seed);
            let result = get_random_number(&mut c, &[]).unwrap().unwrap();
            let high = seed / 127_773;
            let low = seed % 127_773;
            let expected = park_miller(seed) as u32;
            assert_eq!(result, expected & 0xffff);
            assert_eq!(c.data[1], (u32::from(high as u16) << 16) | result);
            assert_eq!(c.address[0], high.wrapping_mul(2836) as u32);
            assert_eq!(c.address[1], low.wrapping_mul(16807) as u32);
            assert_eq!(c.status & 0xffe3, 0x2700);
            assert_eq!(
                c.status & 0x0c,
                if result == 0 {
                    4
                } else if result & 0x8000 != 0 {
                    8
                } else {
                    0
                }
            );
        }
    }
}
