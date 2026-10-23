use crate::{Context, Result, call, execution::WaitRequest, ids};
use tje_mem::{Address, ram};

/// Await an IRQ-written byte without fabricating readiness or advancing a fixed
/// number of frames. The aligned word is only the scheduler wake condition;
/// the original byte predicate is rechecked after every real hardware wakeup.
pub(super) fn wait_pending_byte(c: &mut dyn Context, flag: Address<i8>) -> Result<()> {
    while flag.read(c) != 0 {
        let word = flag.address() & !1;
        let previous = c.read_u16(word);
        c.wait_until(WaitRequest::vblank_frame(word, previous))?;
    }
    Ok(())
}

/// G/src/map/land_processing.c:249 (`MapQueueVisibleMetatileRow`).
pub fn map_queue_visible_metatile_row(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let r = c.revision();
    let cell = args[0] as i16;
    let row = (args[1] as i16).wrapping_add(args[3] as i16);
    let column = i32::from(args[2] as i16);
    let local_row = args[3] as i16;
    let camera = i32::from(args[4] as i16);
    let first = i32::from(args[5] as i16);
    let mut count = 0_i16;
    for local_column in 0..16 {
        if first > column + local_column {
            continue;
        }
        if column + local_column > first + 44 {
            break;
        }
        let mut tile = 0;
        if cell & 0x1f00 != 0 {
            tile = call(
                c,
                ids::GET_OVERLAY_METATILE_TILE,
                &[
                    ((cell & 0x1f00) >> 8) as u32,
                    i32::from(local_row) as u32,
                    local_column as u32,
                ],
            )? as u16;
        }
        if tile == 0 {
            tile = call(
                c,
                ids::GET_LEVEL_METATILE_TILE,
                &[
                    u32::from(cell as u8),
                    i32::from(local_row) as u32,
                    local_column as u32,
                ],
            )? as u16;
        }
        if count == 0 {
            ram::g_camera_row_upload_start_column(r)
                .at(camera)
                .write(c, ((column + local_column) & 63) as i8);
        }
        ram::g_camera_row_upload_words(r)
            .at(camera)
            .at(i32::from(count))
            .write(c, tile | 0x8000);
        count = count.wrapping_add(1);
    }
    ram::g_camera_row_upload_vram_row(r)
        .at(camera)
        .write(c, (row & 31) as i8);
    ram::g_camera_row_upload_word_count(r)
        .at(camera)
        .write(c, count as i8);
    let pending = ram::g_camera_row_upload_pending(r).at(camera);
    pending.write(c, 1);
    wait_pending_byte(c, pending)?;
    Ok(None)
}

/// G/src/map/land_processing.c:280 (`MapQueueVisibleMetatileColumn`).
pub fn map_queue_visible_metatile_column(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let r = c.revision();
    let cell = args[0] as i16;
    let row = i32::from(args[1] as i16);
    let column = (args[2] as i16).wrapping_add(args[3] as i16);
    let local_column = args[3] as i16;
    let camera = i32::from(args[4] as i16);
    let first = i32::from(args[5] as i16);
    let mut count = 0_i16;
    for local_row in 0..8 {
        if first > row + local_row {
            continue;
        }
        if row + local_row > first + i32::from(ram::g_camera_column_upload_height(r).read(c)) - 1 {
            break;
        }
        let mut tile = 0;
        if cell & 0x1f00 != 0 {
            tile = call(
                c,
                ids::GET_OVERLAY_METATILE_TILE,
                &[
                    ((cell & 0x1f00) >> 8) as u32,
                    local_row as u32,
                    i32::from(local_column) as u32,
                ],
            )? as u16;
        }
        if tile == 0 {
            tile = call(
                c,
                ids::GET_LEVEL_METATILE_TILE,
                &[
                    u32::from(cell as u8),
                    local_row as u32,
                    i32::from(local_column) as u32,
                ],
            )? as u16;
        }
        if count == 0 {
            ram::g_camera_column_upload_start_row(r)
                .at(camera)
                .write(c, ((row + local_row) & 31) as i8);
        }
        ram::g_camera_column_upload_words(r)
            .at(camera)
            .at(i32::from(count))
            .write(c, tile | 0x8000);
        count = count.wrapping_add(1);
    }
    ram::g_camera_column_upload_vram_column(r)
        .at(camera)
        .write(c, (column & 63) as i8);
    ram::g_camera_column_upload_word_count(r)
        .at(camera)
        .write(c, count as i8);
    let pending = ram::g_camera_column_upload_pending(r).at(camera);
    pending.write(c, 1);
    wait_pending_byte(c, pending)?;
    Ok(None)
}
