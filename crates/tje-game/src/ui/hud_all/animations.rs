use super::super::animations::wait_upload;
use super::super::entities::entity;
use super::frame as source;
use crate::call_frame::{FrameLayout, LocalFrame};
use crate::{Context, Result, call, ids};
use tje_mem::{ram, rom};

/// G/src/ui/hud.c:103 (`HudLoadActionIconSprite`), both c_hud.s:242–311.
/// Default-icon indexing uses ASL.W/Dn.W; powerup indexing uses ASL.L/Dn.L.
/// TimeAware: the graphics-enabled branch waits for the pending-upload ISR.
pub fn hud_load_action_icon_sprite(context: &mut dyn Context) -> Result<Option<u32>> {
    // c_hud.s:243-244: MOVEM D2/D3/A2, then consume only the argument word.
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x0c, 0x04))?;
    let revision = context.revision();
    let player = frame.argument_word(0).read(context);
    let (sprite, tile) = if player == 0 {
        (21, 0x7f1_u16)
    } else {
        (22, 0x7f5_u16)
    };
    let powerup = ram::g_player_powerups(revision)
        .at_long_index(i32::from(player))
        .read(context);
    let icon = if powerup != -1 {
        rom::spr_action_icons(revision)
            .at_long_index(i32::from(powerup) + 2)
            .read(context)
            .target()
    } else {
        rom::spr_action_icons(revision)
            .at_word_index(player)
            .read(context)
            .target()
    };
    // Both linked listings: JSR returns at entry+0x56, pending-byte TST loop.
    if source::call(
        context,
        ids::HUD_LOAD_ACTION_ICON_SPRITE,
        ids::GET_V_BLANK_GRAPHICS_ENABLED,
        0x56,
    )? != 0
    {
        wait_upload(context)?;
    }
    source::push_move(context, u32::from(tile))?;
    source::push_move(context, icon.address())?;
    source::call(
        context,
        ids::HUD_LOAD_ACTION_ICON_SPRITE,
        ids::QUEUE_SPRITE_PIECE_TILES,
        0x70,
    )?;
    let attributes =
        ((u16::from(icon.tile_format().read(context) as u8) & 3) << 13).wrapping_add(0x8000);
    let sprite = ram::g_hardware_sprite_table(revision).at_word_index(sprite);
    source::release(context, 8)?;
    sprite.size().write(context, 5);
    sprite.tile().write(context, tile.wrapping_add(attributes));
    ram::g_sprite_graphics_upload_pending(revision).write(context, 1);
    frame.leave(context)?;
    Ok(None)
}

fn vram_write(address: i16) -> u32 {
    let address = address as u16;
    0x40000000_u32
        .wrapping_add(u32::from(address & 0x3fff) << 16)
        .wrapping_add(u32::from(address & 0xc000) >> 14)
}

/// G/src/ui/hud.c:321 (`HudAnimateHealthBar`), both c_hud.s:720–941.
/// Functional: finite life-icon upload and up to six ordered health pixel words.
/// Preserve signed-byte health, signed-word record displacement and pixel math.
pub fn hud_animate_health_bar(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let player_number = arguments[0] as i16;
    let player = i32::from(player_number);
    if ram::g_hud_displayed_lives(revision)
        .at_long_index(player)
        .read(context)
        != ram::g_player_lives(revision)
            .at_long_index(player)
            .read(context)
    {
        call(context, ids::HUD_LOAD_LIVES_SPRITE_TILES, &[player as u32])?;
        call(context, ids::HUD_POSITION_LIVES_SPRITE, &[player as u32])?;
    }
    let entity = entity(context, player_number);
    if ram::g_player_lives(revision)
        .at_long_index(player)
        .read(context)
        < 0
        || entity.sprite_state().read(context) == 5
        || entity.sprite_state().read(context) == 6
        || ram::g_player1_map_level(revision).read(context) == 26
    {
        return Ok(None);
    }
    let maximum = (if player_number == 0 { 23_i16 } else { 31_i16 }).wrapping_add(
        i16::from(
            ram::g_player_rank(revision)
                .at_long_index(player)
                .read(context),
        )
        .wrapping_mul(4),
    );
    let health = ram::g_player_health(revision).at_long_index(player);
    let saved = ram::g_player_health_before_swimming(revision).at_long_index(player);
    if health.read(context) < 0 {
        health.write(context, 0);
    }
    if i16::from(health.read(context)) > maximum {
        health.write(context, maximum as i8);
    }
    if i16::from(saved.read(context)) > maximum {
        saved.write(context, maximum as i8);
    }
    let displayed = ram::g_hud_displayed_health(revision).at_long_index(player);
    if health.read(context) > displayed.read(context) {
        let next = displayed.read(context).wrapping_add(1);
        displayed.write(context, next);
    } else if health.read(context) < displayed.read(context) {
        let next = displayed.read(context).wrapping_sub(1);
        displayed.write(context, next);
    } else {
        return Ok(None);
    }
    let value = i16::from(displayed.read(context));
    let base = if player_number == 0 {
        0xf680_u16 as i16
    } else {
        0xfa80_u16 as i16
    };
    context.write_vdp_control(0x8f04);
    let offset = (value / 8).wrapping_mul(32).wrapping_add((value >> 1) & 2);
    let mut pattern = 0xdddd_u16;
    if i32::from(value) <= i32::from(maximum) - 4 {
        let address = base
            .wrapping_add(offset)
            .wrapping_add(if value & 4 != 0 { 30 } else { 2 });
        context.write_vdp_command(vram_write(address));
        for _ in 0..3 {
            context.write_vdp_data(pattern);
        }
    }
    if value != 0 {
        let mut pixel = value & 3;
        while pixel >= 0 {
            pattern = (pattern >> 4) | 0xb000;
            pixel = pixel.wrapping_sub(1);
        }
    }
    context.write_vdp_command(vram_write(base.wrapping_add(offset)));
    for _ in 0..3 {
        context.write_vdp_data(pattern);
    }
    context.write_vdp_control(0x8f02);
    Ok(None)
}
