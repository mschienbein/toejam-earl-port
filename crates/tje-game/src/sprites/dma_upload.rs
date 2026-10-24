//! Stable-source DMA transaction streams; these functions are Functional, not Atomic.
use crate::{
    Context, Result, call, call_at,
    call_frame::{CallSite, FrameLayout, LocalFrame},
    ids,
};
use tje_mem::ram;

fn copy_words(context: &mut dyn Context, source: u32, destination: u32, count: u32) -> Result<()> {
    call(
        context,
        ids::DMA_COPY_WORDS_TO_VRAM,
        &[source, destination, count],
    )?;
    Ok(())
}

/// G/src/sprites/load_sprites.c:8, UploadQueuedSpriteGraphics.
/// Source RAM is stable during each ordered DMA; no raster/status read is performed.
pub fn upload_queued_sprite_graphics(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    // G c_load_sprites.s:4; the all-RAM zero-length transfer observes this save area.
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x1c, 0x04))?;
    let result = upload_inner(context, arguments);
    let result = result.map_err(|error| context.protect_timing_failure(error));
    if result
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        return result;
    }
    let leave = frame.leave(context);
    match (result, leave) {
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(value), Ok(())) => Ok(value),
    }
}

fn upload_inner(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let pending = ram::g_sprite_graphics_upload_pending(revision);
    let force = arguments[0] as i16;
    let tiles_only = arguments[1] as i16;
    context.set_address_register(2, ram::g_hardware_sprite_table(revision).address());
    if pending.read(context) == 0 && force == 0 && tiles_only == 0 {
        // c_load_sprites.s:8-14: zero TST sets Z, clears N/V/C, and retains X.
        context.set_status_register((context.status_register() & !0x0f) | 4);
        return Ok(None);
    }
    if tiles_only == 0 {
        // REV02 CF2C / REV00 CF0C: real JSR return, caller offset 0x3c.
        let site = CallSite::relative(ids::UPLOAD_QUEUED_SPRITE_GRAPHICS, revision, 0x3c);
        call_at(
            context,
            ids::DMA_COPY_WORDS_TO_VRAM,
            &[
                ram::g_hardware_sprite_table(revision).address(),
                0xf800,
                320,
            ],
            site,
        )?;
        let primary = ram::g_hardware_sprite_table(revision)
            .at(23)
            .link()
            .read(context);
        ram::g_primary_viewport_uploaded_link(revision).write(context, primary);
        let secondary = ram::g_hardware_sprite_table(revision)
            .at(24)
            .link()
            .read(context);
        ram::g_secondary_viewport_uploaded_link(revision).write(context, secondary);
    }
    let mut source = ram::g_sprite_tile_upload_buffer(revision).address();
    let mut index = 0i16;
    let d3_high = context.data_register(3) & 0xffff_0000;
    while (index as i32 as u32)
        < u32::from(ram::g_sprite_tile_upload_queue_word_count(revision).read(context))
    {
        let runs = ram::g_sprite_tile_upload_runs(revision);
        let word_count = runs.at(i32::from(index) + 1).read(context).wrapping_shl(4);
        let destination = u32::from(runs.at(i32::from(index)).read(context)).wrapping_shl(5);
        // At the second JSR, the original keeps source/index/count in D2/D3.W/D4.W.
        context.set_data_register(2, source);
        context.set_data_register(3, d3_high | u32::from(index as u16));
        context.set_data_register(
            4,
            (context.data_register(4) & 0xffff_0000) | u32::from(word_count),
        );
        // REV02 CF84 / REV00 CF64: real JSR return, caller offset 0x94.
        let site = CallSite::relative(ids::UPLOAD_QUEUED_SPRITE_GRAPHICS, revision, 0x94);
        call_at(
            context,
            ids::DMA_COPY_WORDS_TO_VRAM,
            &[source, destination, u32::from(word_count)],
            site,
        )?;
        source = source.wrapping_add(u32::from(word_count) * 2);
        index = index.wrapping_add(2);
    }
    ram::g_sprite_tile_buffer_offset(revision).write(context, 0);
    ram::g_sprite_tile_upload_queue_word_count(revision).write(context, 0);
    pending.write(context, 0);
    // c_load_sprites.s:64-66: the final CLR.W/CLR.B/CLR.B set Z and clear N/V/C.
    // X survives (zero after DMA/index increments; entry X on an empty tiles-only path).
    context.set_status_register((context.status_register() & !0x0f) | 4);
    Ok(None)
}

/// G/src/sprites/link_player_sprites.c:9, LinkPlayerSpritesForSplitWipe.
/// Signed-char piece counts and handles retain their original unchecked indexing.
pub fn link_player_sprites_for_split_wipe(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let table = ram::g_hardware_sprite_table(revision);
    let first = ram::g_player_info(revision).at(0);
    let mut count = first
        .current_frame()
        .read(context)
        .target()
        .piece_count()
        .read(context) as i8;
    let mut list = first.primary_sprites();
    let mut sprite = list.at(0).read(context);
    table.at(23).link().write(context, sprite as u8);
    for index in 0..i16::from(count) {
        sprite = list.at(i32::from(index)).read(context);
        if i32::from(index) >= i32::from(count) - 1 {
            break;
        }
        let next = list.at(i32::from(index) + 1).read(context);
        table
            .at(i32::from(sprite))
            .link()
            .write(context, next as u8);
    }
    let second = ram::g_player_info(revision).at(1);
    count = second
        .current_frame()
        .read(context)
        .target()
        .piece_count()
        .read(context) as i8;
    list = second.primary_sprites();
    table
        .at(i32::from(sprite))
        .link()
        .write(context, list.at(0).read(context) as u8);
    for index in 0..i16::from(count) {
        sprite = list.at(i32::from(index)).read(context);
        let next = if i32::from(index) < i32::from(count) - 1 {
            list.at(i32::from(index) + 1).read(context)
        } else {
            79
        };
        table
            .at(i32::from(sprite))
            .link()
            .write(context, next as u8);
    }
    copy_words(context, table.address(), 0xf800, 320)?;
    Ok(None)
}
