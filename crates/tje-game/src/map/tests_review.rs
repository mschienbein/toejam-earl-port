//! RH04/RH05: synthetic word/long effective-address counterexamples.
use super::*;
#[test]
fn short_map_indexes_wrap_but_promoted_neighbors_remain_long() {
    for r in Revision::ALL {
        for (row, column, bit, dy, dx, output_row, output_column, terrain) in [
            (901_i16, 0_i16, 8_u16, 0, -1, 900, -2, 0x1000_u16),
            (2000, 5, 1, -1, 0, 1998, 4, 0x1100),
            (5, 32767, 2, 0, 1, 4, i16::MIN, 0x1100),
            (-1, 5, 4, 1, 0, 0, 4, 0x1100),
        ] {
            let map = ram::g_player1_level_map(r);
            let y = if dy == 0 {
                i32::from(row.wrapping_mul(38))
            } else {
                (i32::from(row) + dy) * 38
            };
            let x = if dx == 0 {
                i32::from(column.wrapping_mul(2))
            } else {
                (i32::from(column) + dx) * 2
            };
            let neighbor = map.cast::<u16>().byte_offset(y).byte_offset(x);
            let physical = neighbor.address() & 0xffffff;
            let mut bytes = vec![0; 0x100000];
            if physical < 0x100000 {
                bytes[physical as usize..physical as usize + 2]
                    .copy_from_slice(&terrain.to_be_bytes());
            }
            let mut c = Machine::with_rom(r, bytes);
            if physical >= 0xe00000 {
                neighbor.write(&mut c, terrain);
            }
            map.at_word_index(row)
                .at_word_index(column)
                .write(&mut c, bit);
            assert_eq!(
                map_expand_regions_beside_paths(
                    &mut c,
                    &[i32::from(row) as u32, i32::from(column) as u32]
                )
                .unwrap(),
                Some(1)
            );
            assert_eq!(
                map.at_word_index(output_row)
                    .at_word_index(output_column)
                    .read(&c),
                terrain
            );
        }
    }
}

#[test]
fn transition_direct_short_cell_does_not_use_a_long_row_or_column_product() {
    for r in Revision::ALL {
        for (row, column) in [(1000_i16, 0_i16), (2000, 5), (5, 32767), (-1, -1)] {
            let mut bytes = vec![0; 0x100000];
            for i in 0..16 {
                let pointer = rom::map_transition_descriptors(r).at(i).shapes().address() as usize;
                bytes[pointer..pointer + 4].copy_from_slice(&0x6100_u32.to_be_bytes());
            }
            let pointer = rom::map_transition_descriptors(r).at(1).shapes().address() as usize;
            bytes[pointer..pointer + 4].copy_from_slice(&0x6000_u32.to_be_bytes());
            bytes[0x6000..0x6002].copy_from_slice(&[1, 28]);
            let mut c = Machine::with_rom(r, bytes);
            let cell = ram::g_player1_level_map(r)
                .at_word_index(row)
                .at_word_index(column);
            cell.write(&mut c, 28);
            ram::g_transition_path_minimum_visits(r).write(&mut c, 0);
            assert_eq!(
                map_trace_transition_chain(
                    &mut c,
                    &[i32::from(row) as u32, i32::from(column) as u32, 8, 8]
                )
                .unwrap(),
                Some(1)
            );
            assert_eq!(cell.read(&c), 0x21c);
        }
    }
}
