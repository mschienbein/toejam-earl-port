use super::random_generation::random;
use crate::{Context, Result, call_frame::FrameLayout, ids};
use tje_mem::{arithmetic::expr_rem_u32, ram, rom};

/// G/src/map/level_generation_and_changes.c:292 (`MapChooseMetatileAlternatives`).
/// Matching LINK -8, MOVEM D2-D6/A2-A4. A missing descriptor match retains the
/// previous choice word at A6-4, including original residual state on the first cell.
pub fn map_choose_metatile_alternatives(c: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    super::frame_support::in_frame(c, FrameLayout::link(8, 0x7c, 0x1c), |c, frame| {
        let r = c.revision();
        let map = ram::g_player1_level_map(r);
        let row = frame.local::<i16>(-6);
        let column = frame.local::<i16>(-2);
        let choice = frame.local::<i16>(-4);
        let mut choice_written = false;
        row.write(c, 0);
        while row.read(c) < 26 {
            column.write(c, 0);
            while column.read(c) < 19 {
                let cell = map.at_word_index(row.read(c)).at_word_index(column.read(c));
                let old_shape = cell.read(c) as u8;
                let mut new_cell = cell.read(c) & 0xc000;
                let mut alternatives;
                if cell.read(c) & 0x1f00 == 0 {
                    alternatives = rom::tile_alternative_pointers(r)
                        .at(i32::from(old_shape))
                        .read(c)
                        .target();
                    let value = random(c)?;
                    let value = expr_rem_u32(value, u32::from(alternatives.read(c)))? as i16;
                    choice.write(c, value);
                    choice_written = true;
                    choice.write(c, choice.read(c).wrapping_add(1));
                } else {
                    let transition = (((cell.read(c) & 0x1f00) >> 8) as u8).wrapping_sub(1);
                    let descriptor =
                        rom::map_transition_descriptors(r).at_word_index(i16::from(transition));
                    let mut bank = descriptor.bank().read(c);
                    let mut shapes = descriptor.shapes().read(c).target();
                    alternatives = descriptor.alternatives().read(c).target();
                    let count = i16::from(shapes.read(c));
                    shapes = shapes.offset(1);
                    for shape in 0..i32::from(count) {
                        if old_shape == shapes.read(c) {
                            let value = random(c)? as i16;
                            choice.write(c, value);
                            choice_written = true;
                            choice.write(c, choice.read(c) & 1);
                            if shape >= i32::from(count) / 2 && bank & 15 != 15 {
                                bank = bank.wrapping_add(7);
                            }
                            new_cell |= u16::from(bank) << 8;
                            break;
                        }
                        shapes = shapes.offset(1);
                        alternatives = alternatives.offset(2);
                    }
                }
                // Both c_level_generation_and_changes.s:1566 consume A6-4
                // even when the first descriptor has no matching shape. Until
                // an actual choice store, every such read is its own input.
                let selected = if choice_written {
                    choice.read(c)
                } else {
                    c.read_uninitialized_local(frame.entry().uninitialized_local(
                        ids::MAP_CHOOSE_METATILE_ALTERNATIVES,
                        -8,
                        crate::stack_read::LocalWidth::Word,
                        "G/build/{rev00,rev02}/asm/c_level_generation_and_changes.s:1518-1566; no shape match skips 1524; MOVE.W -4(A6),A1; LINK -8; own entry SP-8 word",
                    ))? as i16
                };
                new_cell |= u16::from(alternatives.offset(i32::from(selected)).read(c));
                if new_cell & 0x4000 != 0 {
                    new_cell |= 0x2000;
                }
                map.at_word_index(row.read(c))
                    .at_word_index(column.read(c))
                    .write(c, new_cell);
                column.write(c, column.read(c).wrapping_add(1));
            }
            row.write(c, row.read(c).wrapping_add(1));
        }
        Ok(None)
    })
}
