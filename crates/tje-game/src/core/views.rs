use crate::{Context, Result, call, ids};
use tje_mem::{ObjectState, ram, rom};

/// G/src/core/startup_and_main_loop.c:368 (`ReloadActiveCameraViews`, REV 02 0x000BE2).
/// Read each selected coordinate pair before its redraw; the second view tests
/// the split flag after the first original/translated camera call returns.
pub fn reload_active_camera_views(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let y = i32::from(ram::g_camera1_pending_y(revision).read(context)) as u32;
    let x = i32::from(ram::g_camera1_pending_x(revision).read(context)) as u32;
    call(context, ids::SET_CAMERA_POSITION, &[0, x, y])?;
    if ram::g_split_screen_enabled(revision).read(context) != 0 {
        let y = i32::from(ram::g_camera2_pending_y(revision).read(context)) as u32;
        let x = i32::from(ram::g_camera2_pending_x(revision).read(context)) as u32;
        call(context, ids::SET_CAMERA_POSITION, &[1, x, y])?;
    }
    Ok(None)
}

/// G/src/core/startup_and_main_loop.c:376 (`EnterSplitScreen`, REV 02 0x000C2E).
/// The upload-pending wait advances real IRQs, without injecting a device-status
/// read. It does not invent a fixed wait duration or change the pending byte.
pub fn enter_split_screen(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let animate = arguments[0] as i8;
    if ram::g_mode2_reg_softcopy(revision).read(context) & 0x40 != 0 {
        while ram::g_sprite_graphics_upload_pending(revision).read(context) != 0 {
            let counter = ram::g_frame_counter(revision);
            let previous = counter.read(context);
            context.wait_vblank_frame(counter.address(), previous)?;
        }
        call(context, ids::DISABLE_BLANKING_GRAPHICS, &[])?;
        context.write_vdp_control(0x9200);
        call(context, ids::HUD_HIDE_PLAYER_SPRITES, &[2])?;
        if animate != 0 {
            call(context, ids::ANIMATE_SPLIT_SCREEN_WIPE, &[])?;
            call(context, ids::ANIMATE_PLAYERS_INTO_SPLIT_VIEWS, &[])?;
        }
    }
    ram::g_split_screen_enabled(revision).write(context, 1);
    if ram::g_player_inventory_open(revision).at(0).read(context) == 0 {
        let entity = ram::g_player_info(revision).at(0);
        let mut y = entity
            .y()
            .read(context)
            .wrapping_sub(i16::from(entity.height_pixels().read(context) / 2))
            .wrapping_sub(56);
        if entity.height().read(context) > 0
            && entity.animation_group_mask().read(context) & 0x04008000 != 0
        {
            y = y.wrapping_sub(entity.height().read(context) / 2);
        }
        let x = i32::from(entity.x().read(context)) - 160;
        call(
            context,
            ids::SET_CAMERA_POSITION,
            &[0, x as u32, i32::from(y) as u32],
        )?;
    }
    if ram::g_player2_inventory_open(revision).read(context) == 0 {
        let entity = ram::g_player_info(revision).at(1);
        let mut y = entity
            .y()
            .read(context)
            .wrapping_sub(i16::from(entity.height_pixels().read(context) / 2))
            .wrapping_sub(56);
        if entity.height().read(context) > 0
            && entity.animation_group_mask().read(context) & 0x04008000 != 0
        {
            y = y.wrapping_sub(entity.height().read(context) / 2);
        }
        let x = i32::from(entity.x().read(context)) - 160;
        call(
            context,
            ids::SET_CAMERA_POSITION,
            &[1, x as u32, i32::from(y) as u32],
        )?;
        let x = ram::g_camera2_pending_x(revision).read(context);
        ram::g_camera2_committed_x(revision).write(context, x);
        let y = ram::g_camera2_pending_y(revision).read(context);
        ram::g_camera2_committed_y(revision).write(context, y);
    }
    if ram::g_mode2_reg_softcopy(revision).read(context) & 0x40 != 0 {
        call(context, ids::FORCE_TEXT_BACKGROUND_COLOR_TO_BLACK, &[0])?;
        ram::g_hud_redraw_pending(revision).write(context, 1);
        call(context, ids::ENABLE_BLANKING_GRAPHICS, &[])?;
    } else {
        call(context, ids::DRAW_HUD, &[])?;
    }
    Ok(None)
}

/// G/src/core/startup_and_main_loop.c:415 (`EnterSharedScreen`, REV 02 0x000D7E).
/// Keep signed midpoint arithmetic, word narrowing, display-disable ordering,
/// the two pending-camera copies, and the selected ROM palette upload.
pub fn enter_shared_screen(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let distance_y = arguments[0] as i16;
    let distance_x = arguments[1] as i16;
    let toejam = ram::g_player_info(revision).at(0);
    let earl = ram::g_player_info(revision).at(1);
    let mut x = if toejam.x().read(context) > earl.x().read(context) {
        earl.x().read(context)
    } else {
        toejam.x().read(context)
    };
    x = x.wrapping_add((distance_x / 2).wrapping_sub(160));
    let entity = if toejam.y().read(context) > earl.y().read(context) {
        earl
    } else {
        toejam
    };
    let mut y = entity
        .y()
        .read(context)
        .wrapping_sub(i16::from(entity.height_pixels().read(context) / 2));
    if entity.height().read(context) > 0
        && entity.animation_group_mask().read(context) & 0x04008000 != 0
    {
        y = y.wrapping_sub(entity.height().read(context));
    }
    y = y.wrapping_add((distance_y / 2).wrapping_sub(96));
    let display_enabled = ram::g_mode2_reg_softcopy(revision).read(context) & 0x40;
    if display_enabled != 0 {
        call(context, ids::WAIT_FOR_V_BLANK_WITH_GRAPHICS_PAUSED, &[])?;
        call(context, ids::DISABLE_BLANKING_GRAPHICS, &[])?;
        let mode = ram::g_mode2_reg_softcopy(revision).read(context) & 0x3c;
        ram::g_mode2_reg_softcopy(revision).write(context, mode);
        context.write_vdp_control(0x8104 | u16::from(mode));
    }
    ram::g_split_screen_enabled(revision).write(context, 0);
    call(
        context,
        ids::SET_CAMERA_POSITION,
        &[0, i32::from(x) as u32, i32::from(y) as u32],
    )?;
    let x = ram::g_camera1_pending_x(revision).read(context);
    ram::g_camera2_pending_x(revision).write(context, x);
    let y = ram::g_camera1_pending_y(revision).read(context);
    ram::g_camera2_pending_y(revision).write(context, y);
    call(context, ids::DRAW_HUD, &[])?;
    let palette = if ram::g_player1_map_level(revision).read(context) < 26 {
        rom::palette_game_line1(revision).address()
    } else {
        rom::palette_ship_screen_bank3(revision).address()
    };
    call(context, ids::COPY_PALETTE_TO_CRAM_SLOT, &[palette, 0])?;
    if display_enabled != 0 {
        call(context, ids::FORCE_TEXT_BACKGROUND_COLOR_TO_BLACK, &[0])?;
        let mode = ram::g_mode2_reg_softcopy(revision).read(context) | 0x40;
        ram::g_mode2_reg_softcopy(revision).write(context, mode);
        context.write_vdp_control(0x8104 | u16::from(mode));
        call(context, ids::ENABLE_BLANKING_GRAPHICS, &[])?;
    }
    Ok(None)
}

/// G/src/core/startup_and_main_loop.c:460 (`CenterSharedCameraOnPlayer`, REV 02 0x000EE6).
/// The short object index is lowered as ASL.W #7 / ADDA.W, including wraparound.
pub fn center_shared_camera_on_player(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let displacement = (arguments[0] as i16).wrapping_shl(7);
    let entity = ram::g_player_info(revision)
        .cast::<ObjectState>()
        .byte_offset(i32::from(displacement));
    let mut y = entity
        .y()
        .read(context)
        .wrapping_sub(i16::from(entity.height_pixels().read(context) / 2))
        .wrapping_sub(112);
    if entity.height().read(context) > 0
        && entity.animation_group_mask().read(context) & 0x04008000 != 0
    {
        y = y.wrapping_sub(entity.height().read(context) / 2);
    }
    let display_enabled = ram::g_mode2_reg_softcopy(revision).read(context) & 0x40;
    if display_enabled != 0 {
        call(context, ids::WAIT_FOR_V_BLANK_WITH_GRAPHICS_PAUSED, &[])?;
        call(context, ids::DISABLE_BLANKING_GRAPHICS, &[])?;
        let mode = ram::g_mode2_reg_softcopy(revision).read(context) & 0x3c;
        ram::g_mode2_reg_softcopy(revision).write(context, mode);
        context.write_vdp_control(0x8104 | u16::from(mode));
    }
    let x = i32::from(entity.x().read(context)) - 160;
    call(
        context,
        ids::SET_CAMERA_POSITION,
        &[0, x as u32, i32::from(y) as u32],
    )?;
    if display_enabled != 0 {
        call(context, ids::FORCE_TEXT_BACKGROUND_COLOR_TO_BLACK, &[0])?;
        ram::g_hud_redraw_pending(revision).write(context, 1);
        let mode = ram::g_mode2_reg_softcopy(revision).read(context) | 0x40;
        ram::g_mode2_reg_softcopy(revision).write(context, mode);
        context.write_vdp_control(0x8104 | u16::from(mode));
        call(context, ids::ENABLE_BLANKING_GRAPHICS, &[])?;
    } else {
        call(context, ids::DRAW_HUD, &[])?;
    }
    Ok(None)
}

/// G/src/core/startup_and_main_loop.c:514 (`UpdateScreenSplitMode`, REV 02 0x00105A).
/// Keep the original force/early-return gates, SUB.W/BGE absolute-distance
/// overflow behavior, merge hysteresis, and post-call shared-camera mirrors.
pub fn update_screen_split_mode(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let force = arguments[0] as i8;
    let toejam = ram::g_player_info(revision).at(0);
    let earl = ram::g_player_info(revision).at(1);
    if force != 0 {
        if ram::g_player_lives(revision).at(0).read(context) < 0 {
            call(context, ids::CENTER_SHARED_CAMERA_ON_PLAYER, &[1])?;
            return Ok(None);
        } else if ram::g_player2_lives(revision).read(context) < 0 {
            call(context, ids::CENTER_SHARED_CAMERA_ON_PLAYER, &[0])?;
            return Ok(None);
        } else if toejam.level().read(context) != earl.level().read(context) {
            call(context, ids::ENTER_SPLIT_SCREEN, &[0])?;
            return Ok(None);
        }
    } else if ram::g_player_lives(revision).at(0).read(context) < 0
        || ram::g_player2_lives(revision).read(context) < 0
        || toejam.level().read(context) != earl.level().read(context)
    {
        return Ok(None);
    }
    let first_y = toejam.y().read(context);
    let second_y = earl.y().read(context);
    let mut distance_y = first_y.wrapping_sub(second_y);
    if first_y < second_y {
        distance_y = distance_y.wrapping_neg();
    }
    let first_x = toejam.x().read(context);
    let second_x = earl.x().read(context);
    let mut distance_x = first_x.wrapping_sub(second_x);
    if first_x < second_x {
        distance_x = distance_x.wrapping_neg();
    }
    if toejam.height().read(context) > 0
        && toejam.animation_group_mask().read(context) & 0x04008000 != 0
        && toejam.y().read(context) < earl.y().read(context)
    {
        distance_y = distance_y.wrapping_add(toejam.height().read(context));
    } else if earl.height().read(context) > 0
        && earl.animation_group_mask().read(context) & 0x04008000 != 0
        && earl.y().read(context) < toejam.y().read(context)
    {
        distance_y = distance_y.wrapping_add(earl.height().read(context));
    }
    if force != 0 {
        if distance_y > 124
            || distance_x > 220
            || ram::g_player_inventory_open(revision).at(0).read(context) != 0
            || ram::g_player2_inventory_open(revision).read(context) != 0
        {
            call(context, ids::ENTER_SPLIT_SCREEN, &[0])?;
        } else {
            call(
                context,
                ids::ENTER_SHARED_SCREEN,
                &[i32::from(distance_y) as u32, i32::from(distance_x) as u32],
            )?;
        }
    } else if ram::g_split_screen_enabled(revision).read(context) == 0
        && (distance_y > 124 || distance_x > 220)
    {
        call(context, ids::ENTER_SPLIT_SCREEN, &[1])?;
    } else if ram::g_split_screen_enabled(revision).read(context) != 0
        && distance_y < 104
        && distance_x < 200
        && ram::g_player_inventory_open(revision).at(0).read(context) == 0
        && ram::g_player2_inventory_open(revision).read(context) == 0
    {
        call(
            context,
            ids::ENTER_SHARED_SCREEN,
            &[i32::from(distance_y) as u32, i32::from(distance_x) as u32],
        )?;
    }
    if ram::g_split_screen_enabled(revision).read(context) == 0 {
        let x = ram::g_camera1_pending_x(revision).read(context);
        ram::g_camera2_pending_x(revision).write(context, x);
        let y = ram::g_camera1_pending_y(revision).read(context);
        ram::g_camera2_pending_y(revision).write(context, y);
    }
    Ok(None)
}
