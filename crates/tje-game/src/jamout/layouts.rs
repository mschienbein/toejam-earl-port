use crate::{Context, Result};
use tje_mem::rom;

/// G/src/jamout/jamout.c:635 (`JamOutDrawPackedBackgroundBlock`, REV 02 0x03B988).
/// Functional: read every packed byte from the user's revision-specific ROM,
/// expand flip bits, and issue the original 14-by-20 ordered VDP writes.
pub fn draw_packed_background_block(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let x = i32::from(arguments[0] as i16);
    let y = i32::from(arguments[1] as i16);
    for row in 0..14 {
        let command = 0x60000003_u32
            .wrapping_add((y.wrapping_add(row) as u32) << 23)
            .wrapping_add((x as u32) << 17);
        context.write_vdp_command(command);
        for column in 0..20 {
            let mut tile = i16::from(
                rom::crash_packed_tilemap_b(revision)
                    .at(row)
                    .at(column)
                    .read(context),
            );
            if tile & 0x40 != 0 {
                tile |= 0x0800;
            }
            if tile & 0x80 != 0 {
                tile |= 0x1000;
            }
            tile = (tile as u16 & 0xff3f) as i16;
            context.write_vdp_data((i32::from(tile) + 0xa233) as u16);
        }
    }
    Ok(None)
}
