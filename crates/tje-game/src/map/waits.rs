use crate::{
    Context, Result, call_at,
    call_frame::{CallSite, FrameLayout},
    execution::WaitRequest,
    ids,
};
use tje_mem::{Address, ram};

/// Source-backed fixed byte TST/BNE poll. The platform owns every sample,
/// including the final zero; an aligned-word proxy adds an invented access.
pub(super) fn wait_pending_byte(c: &mut dyn Context, flag: Address<i8>) -> Result<()> {
    c.wait_until(WaitRequest::ram_byte_equal(flag.address(), 0xff, 0))
}

/// G/src/map/land_processing.c:249 (`MapQueueVisibleMetatileRow`).
pub fn map_queue_visible_metatile_row(c: &mut dyn Context) -> Result<Option<u32>> {
    // Matching c_land_processing.s:752-759. The count/tile words can be
    // reached by the original unchecked upload-buffer addresses.
    super::frame_support::in_frame(c, FrameLayout::link(4, 0x7c, 0), |c, frame| {
        let r = c.revision();
        let cell = frame.argument_word(0).read(c);
        let mut row = frame.argument_word(1).read(c);
        let column = i32::from(frame.argument_word(2).read(c));
        let count = frame.local::<i16>(-2);
        let tile = frame.local::<u16>(-4);
        count.write(c, 0);
        row = row.wrapping_add(frame.argument_word(3).read(c));
        count.write(c, 0);
        for local_column in 0..16_i16 {
            if i32::from(frame.argument_word(5).read(c)) > column + i32::from(local_column) {
                continue;
            }
            if column + i32::from(local_column) > i32::from(frame.argument_word(5).read(c)) + 44 {
                break;
            }
            tile.write(c, 0);
            if cell & 0x1f00 != 0 {
                let values = [
                    ((i32::from(cell) & 0x1f00) >> 8) as u32,
                    i32::from(frame.argument_word(3).read(c)) as u32,
                    i32::from(local_column) as u32,
                ];
                let site = CallSite::relative(ids::MAP_QUEUE_VISIBLE_METATILE_ROW, r, 0x84);
                let value = call_at(c, ids::GET_OVERLAY_METATILE_TILE, &values, site)?;
                tile.write(c, value as u16);
            }
            if tile.read(c) == 0 {
                let values = [
                    u32::from(cell as u8),
                    i32::from(frame.argument_word(3).read(c)) as u32,
                    i32::from(local_column) as u32,
                ];
                let site = CallSite::relative(ids::MAP_QUEUE_VISIBLE_METATILE_ROW, r, 0xb2);
                let value = call_at(c, ids::GET_LEVEL_METATILE_TILE, &values, site)?;
                tile.write(c, value as u16);
            }
            if count.read(c) == 0 {
                let camera = i32::from(frame.argument_word(4).read(c));
                ram::g_camera_row_upload_start_column(r)
                    .at(camera)
                    .write(c, ((column + i32::from(local_column)) & 63) as i8);
            }
            let camera = i32::from(frame.argument_word(4).read(c));
            let old_count = count.read(c);
            count.write(c, old_count.wrapping_add(1));
            let value = tile.read(c) | 0x8000;
            ram::g_camera_row_upload_words(r)
                .at(camera)
                .at_word_index(old_count)
                .write(c, value);
        }
        let camera = i32::from(frame.argument_word(4).read(c));
        ram::g_camera_row_upload_vram_row(r)
            .at(camera)
            .write(c, (row & 31) as i8);
        let camera = i32::from(frame.argument_word(4).read(c));
        let value = count.byte_offset(1).cast::<i8>().read(c);
        ram::g_camera_row_upload_word_count(r)
            .at(camera)
            .write(c, value);
        let camera = i32::from(frame.argument_word(4).read(c));
        let pending = ram::g_camera_row_upload_pending(r).at(camera);
        pending.write(c, 1);
        // Matching asm:872-876 reloads this caller argument on EVERY poll.
        // Upload stores or IRQ work can change it through original aliases.
        loop {
            let camera = i32::from(frame.argument_word(4).read(c));
            if ram::g_camera_row_upload_pending(r).at(camera).read(c) == 0 {
                break;
            }
            c.poll_yield()?;
        }
        Ok(None)
    })
}

/// G/src/map/land_processing.c:280 (`MapQueueVisibleMetatileColumn`).
pub fn map_queue_visible_metatile_column(c: &mut dyn Context) -> Result<Option<u32>> {
    // Matching c_land_processing.s:884-890; camera scales as a long while
    // the local count's doubled element displacement remains a signed word.
    super::frame_support::in_frame(c, FrameLayout::link(4, 0x7c, 0), |c, frame| {
        let r = c.revision();
        let cell = frame.argument_word(0).read(c);
        let row = i32::from(frame.argument_word(1).read(c));
        let mut column = frame.argument_word(2).read(c);
        let count = frame.local::<i16>(-2);
        let tile = frame.local::<u16>(-4);
        count.write(c, 0);
        column = column.wrapping_add(frame.argument_word(3).read(c));
        for local_row in 0..8_i16 {
            if i32::from(frame.argument_word(5).read(c)) > row + i32::from(local_row) {
                continue;
            }
            // The height read precedes the second first-row parameter load.
            let height = i32::from(ram::g_camera_column_upload_height(r).read(c));
            let first = i32::from(frame.argument_word(5).read(c));
            if row + i32::from(local_row) > first + height - 1 {
                break;
            }
            tile.write(c, 0);
            if cell & 0x1f00 != 0 {
                let values = [
                    ((i32::from(cell) & 0x1f00) >> 8) as u32,
                    i32::from(local_row) as u32,
                    i32::from(frame.argument_word(3).read(c)) as u32,
                ];
                let site = CallSite::relative(ids::MAP_QUEUE_VISIBLE_METATILE_COLUMN, r, 0x88);
                let value = call_at(c, ids::GET_OVERLAY_METATILE_TILE, &values, site)?;
                tile.write(c, value as u16);
            }
            if tile.read(c) == 0 {
                let values = [
                    u32::from(cell as u8),
                    i32::from(local_row) as u32,
                    i32::from(frame.argument_word(3).read(c)) as u32,
                ];
                let site = CallSite::relative(ids::MAP_QUEUE_VISIBLE_METATILE_COLUMN, r, 0xb6);
                let value = call_at(c, ids::GET_LEVEL_METATILE_TILE, &values, site)?;
                tile.write(c, value as u16);
            }
            if count.read(c) == 0 {
                let camera = i32::from(frame.argument_word(4).read(c));
                ram::g_camera_column_upload_start_row(r)
                    .at(camera)
                    .write(c, ((row + i32::from(local_row)) & 31) as i8);
            }
            let camera = i32::from(frame.argument_word(4).read(c));
            let old_count = count.read(c);
            count.write(c, old_count.wrapping_add(1));
            let value = tile.read(c) | 0x8000;
            ram::g_camera_column_upload_words(r)
                .at(camera)
                .at_word_index(old_count)
                .write(c, value);
        }
        let camera = i32::from(frame.argument_word(4).read(c));
        ram::g_camera_column_upload_vram_column(r)
            .at(camera)
            .write(c, (column & 63) as i8);
        let camera = i32::from(frame.argument_word(4).read(c));
        let value = count.byte_offset(1).cast::<i8>().read(c);
        ram::g_camera_column_upload_word_count(r)
            .at(camera)
            .write(c, value);
        let camera = i32::from(frame.argument_word(4).read(c));
        let pending = ram::g_camera_column_upload_pending(r).at(camera);
        pending.write(c, 1);
        // Matching asm:999-1003: dynamic address, not a fixed-byte shortcut.
        loop {
            let camera = i32::from(frame.argument_word(4).read(c));
            if ram::g_camera_column_upload_pending(r).at(camera).read(c) == 0 {
                break;
            }
            c.poll_yield()?;
        }
        Ok(None)
    })
}
