use crate::call_frame::{CallSite, FrameLayout, LocalFrame};
use crate::{Context, Error, Result, call, call_at, function_at, ids};
use tje_mem::ram;

fn increment_frame(context: &mut dyn Context) {
    let counter = ram::g_frame_counter(context.revision());
    let next = counter.read(context).wrapping_add(1);
    counter.write(context, next);
}

/// G/src/core/startup_and_main_loop.c:175 (`HandleVBlank`, REV 02 0x0005EC).
/// The installed callback owns its counter; otherwise preserve graphics-service
/// precedence, the priority-update jump over audio, and the final frame increment.
/// IRQ6 device publication time remains the explicit TimeAware proof obligation.
fn handle_vblank_inner(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let callback = ram::g_v_blank_func(revision).read(context).address();
    if callback != 0 {
        let id = function_at(revision, callback)
            .ok_or_else(|| Error::Platform(format!("unmapped VBlank callback {callback:08x}")))?;
        context.set_address_register(0, callback);
        call_at(
            context,
            id,
            &[],
            CallSite::relative(ids::HANDLE_V_BLANK, revision, 0x26),
        )?;
        return Ok(None);
    }
    if ram::g_v_blank_graphics_enabled(revision).read(context) != 0 {
        if ram::g_sprite_graphics_upload_pending(revision).read(context) != 0 {
            let x = ram::g_camera1_pending_x(revision).read(context);
            ram::g_camera1_committed_x(revision).write(context, x);
            let y = ram::g_camera1_pending_y(revision).read(context);
            ram::g_camera1_committed_y(revision).write(context, y);
            let x = ram::g_camera2_pending_x(revision).read(context);
            ram::g_camera2_committed_x(revision).write(context, x);
            let y = ram::g_camera2_pending_y(revision).read(context);
            ram::g_camera2_committed_y(revision).write(context, y);
        }
        let chain = if ram::g_split_screen_enabled(revision).read(context) != 0
            || ram::g_player_inventory_open(revision).at(0).read(context) != 0
            || ram::g_player_inventory_open(revision).at(1).read(context) != 0
        {
            1
        } else {
            0
        };
        call(context, ids::SELECT_SPRITE_CHAIN_START, &[chain])?;
        context.write_vdp_control(if chain != 0 { 0x8a70 } else { 0x8ab6 });
        context.write_vdp_control(0x8014);
        if ram::g_player_inventory_open(revision).at(0).read(context) != 0 {
            context.write_vdp_control(0x920e);
        } else {
            context.write_vdp_control(0x8228);
            context.write_vdp_command(0x40000010);
            context.write_vdp_data(ram::g_camera1_committed_y(revision).read(context) as u16);
            context.write_vdp_command(0x7c000003);
            let x = -i32::from(ram::g_camera1_committed_x(revision).read(context));
            context.write_vdp_data(x as u16);
            if ram::g_player1_map_level(revision).read(context) < 26 {
                context.write_vdp_command(0x40020010);
                let y = i32::from(ram::g_camera1_committed_y(revision).read(context)) / 2;
                context.write_vdp_data(y as u16);
                context.write_vdp_command(0x7c020003);
                let x = -i32::from(ram::g_camera1_committed_x(revision).read(context)) / 2;
                context.write_vdp_data(x as u16);
            }
        }
        call(context, ids::SELECT_VIEWPORT_SPRITE_CHAIN, &[0])?;
        if ram::g_terrain_priority_update_pending(revision).read(context) != 0 {
            call(context, ids::MAP_CLEAR_TILE_PRIORITY_NEAR_PLAYER, &[])?;
            increment_frame(context);
            return Ok(None);
        } else if ram::g_v_blank_graphics_paused(revision).read(context) == 0 {
            if ram::g_inventory_redraw_phase(revision).read(context) != 0 {
                call(context, ids::INVENTORY_SERVICE_REDRAW_PHASE, &[])?;
            } else if ram::g_sprite_graphics_upload_pending(revision).read(context) != 0 {
                call(context, ids::UPLOAD_QUEUED_SPRITE_GRAPHICS, &[0, 0])?;
                call(context, ids::SELECT_VIEWPORT_SPRITE_CHAIN, &[0])?;
            } else if ram::g_hud_redraw_pending(revision).read(context) != 0 {
                call(context, ids::HUD_LOAD_LIVES_SPRITE_TILES, &[0])?;
                call(context, ids::HUD_LOAD_LIVES_SPRITE_TILES, &[1])?;
                call(context, ids::DRAW_HUD, &[])?;
            } else {
                if call(context, ids::TEXT_ANIMATE_FONT_BANKS, &[4])? == 0 {
                    call(context, ids::DRAW_LEVEL_GRAPHICS, &[])?;
                    call(context, ids::UPDATE_CAMERA_Y, &[])?;
                    call(context, ids::UPDATE_CAMERA_X, &[])?;
                    call(context, ids::HUD_ANIMATE_HEALTH_BAR, &[0])?;
                    call(context, ids::HUD_ANIMATE_HEALTH_BAR, &[1])?;
                }
                let parity = ram::g_v_blank_player_update_parity(revision);
                let next = parity.read(context).wrapping_add(1);
                parity.write(context, next);
                call(
                    context,
                    ids::HIDE_FLAGGED_PLAYER_SPRITES,
                    &[if next & 1 != 0 { 0 } else { 1 }],
                )?;
            }
        }
    }
    if ram::g_v_blank_graphics_paused(revision).read(context) == 0
        || ram::g_v_blank_audio_input_override(revision).read(context) != 0
    {
        if ram::g_current_music_selection(revision).read(context)
            != ram::g_requested_music_selection(revision).read(context)
        {
            let selection =
                i32::from(ram::g_requested_music_selection(revision).read(context)) as u32;
            call_at(
                context,
                ids::MUSIC_SELECT_SONG_TEMPLATE_BANK,
                &[selection],
                CallSite::relative(ids::HANDLE_V_BLANK, revision, 0x228),
            )?;
            let selection = ram::g_requested_music_selection(revision).read(context);
            ram::g_current_music_selection(revision).write(context, selection);
        } else {
            call(context, ids::MUSIC_UPDATE_SEQUENCER, &[])?;
        }
        if ram::g_frame_counter_low_byte(revision).read(context) & 1 != 0 {
            let input = call(context, ids::GET_CTRL1_INPUT, &[])?;
            ram::g_controller1_raw_input(revision).write(context, input as u16);
        } else {
            let input = call(context, ids::GET_CTRL2_INPUT, &[])?;
            ram::g_controller2_raw_input(revision).write(context, input as u16);
        }
    }
    increment_frame(context);
    Ok(None)
}

/// G/src/core/startup_and_main_loop.c:175 (`HandleVBlank`), matching
/// MOVEM A2-A4. Dynamic callbacks and the mutable-slot music selector enter
/// through their real JSR frames, with the compiled caller's pointer registers.
pub fn handle_vblank(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0, 0x1c))?;
    context.set_address_register(2, ram::g_player_inventory_open(revision).address());
    context.set_address_register(3, ram::g_camera1_committed_x(revision).address());
    context.set_address_register(4, ram::g_camera1_committed_y(revision).address());
    let result = handle_vblank_inner(context, &[]);
    let result = result.map_err(|error| context.protect_timing_failure(error));
    if result
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        return result;
    }
    let cleanup = frame.leave(context);
    match (result, cleanup) {
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(value), Ok(())) => Ok(value),
    }
}
