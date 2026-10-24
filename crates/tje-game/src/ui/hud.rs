use crate::{Context, Result};
use tje_mem::{ram, rom};

/// G/src/ui/hud.c:56 (`HudPositionLevelNumberSprites`).
pub fn hud_position_level_number_sprites(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let player = i32::from(arguments[0] as i16);
    let revision = context.revision();
    let sprites = ram::g_hardware_sprite_table(revision);
    let (sprite, tens_digit) = if player == 0 {
        (
            11,
            i32::from(ram::g_toejam_level(revision).read(context)) / 10,
        )
    } else {
        (
            13,
            i32::from(ram::g_earl_level(revision).read(context)) / 10,
        )
    };
    let (x, y) = if ram::g_player_lives(revision).at(player).read(context) < 0 {
        (1, 1)
    } else if ram::g_player_inventory_open(revision)
        .at(player)
        .read(context)
        != 0
    {
        (348, if player == 0 { 218 } else { 321 })
    } else if ram::g_split_screen_enabled(revision).read(context) != 0
        || ram::g_player_inventory_open(revision)
            .at(player ^ 1)
            .read(context)
            != 0
    {
        (410, if player == 0 { 136 } else { 249 })
    } else {
        if ram::g_player_lives(revision).at(0).read(context) >= 0 {
            sprites.at(13).y().write(context, 1);
            sprites.at(14).y().write(context, 1);
        }
        (276, 321)
    };
    if tens_digit != 0 {
        sprites.at(sprite).x().write(context, x + 8);
        sprites.at(sprite + 1).x().write(context, x);
        sprites.at(sprite + 1).y().write(context, y);
    } else {
        sprites.at(sprite).x().write(context, x + 4);
        sprites.at(sprite + 1).y().write(context, 1);
    }
    sprites.at(sprite).y().write(context, y);
    Ok(None)
}

/// G/src/ui/hud.c:133 (`HudPositionActionIconSprite`).
/// Icon origins are signed chars read from the revision's ROM pointer table.
/// Pointer-table indexing retains MCC68K's word-sized ASL.W offset.
/// c_hud.s:315-416 (both revisions): retain the volatile register producers read
/// by whole-call return contracts, including ADD.L's X before the final MOVE.W.
pub fn hud_position_action_icon_sprite(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let player = i32::from(arguments[0] as i16);
    let revision = context.revision();
    let powerup = ram::g_player_powerups(revision).at(player);
    context.set_address_register(1, ram::g_player_powerups(revision).address());
    let mut d1_high = context.data_register(1) & 0xffff0000;
    let icon_number = if powerup.read(context) == -1 {
        player as i16
    } else {
        i16::from(powerup.read(context)).wrapping_add(2)
    };
    let icon = rom::spr_action_icons(revision)
        .at(0)
        .byte_offset(i32::from(icon_number.wrapping_shl(2)))
        .read(context)
        .target();
    let (sprite, mut y) = if player == 0 {
        (21, 218_i16)
    } else {
        (22, 321_i16)
    };
    let x = if ram::g_player_lives(revision).at(player).read(context) < 0 {
        y = 1;
        1_i16
    } else if ram::g_player_inventory_open(revision)
        .at(player)
        .read(context)
        != 0
    {
        208
    } else {
        let split = ram::g_split_screen_enabled(revision).read(context);
        if split == 0 {
            // MOVE.W player,D1; EXT.L D1; EOR.L #1,D1 replaces the high word.
            d1_high = (player as u32 ^ 1) & 0xffff0000;
        }
        if split != 0
            || ram::g_player_inventory_open(revision)
                .at(player ^ 1)
                .read(context)
                != 0
        {
            y = 1;
            if powerup.read(context) != -1 {
                y = if player == 0 { 136 } else { 249 };
            }
            392
        } else {
            y = 321;
            if player == 0 { 136 } else { 306 }
        }
    };
    let sprites = ram::g_hardware_sprite_table(revision);
    context.set_address_register(0, sprites.at(0).address());
    context.set_data_register(1, d1_high | ((sprite as u32 * 8) & 0xffff));
    let x = i32::from(icon.x().read(context)) + i32::from(x);
    context.set_data_register(0, x as u32);
    let sprite = sprites.at(sprite);
    sprite.x().write(context, x as i16);
    let (y, carry) =
        (i32::from(icon.y().read(context)) as u32).overflowing_add(i32::from(y) as u32);
    context.set_data_register(0, y);
    sprite.y().write(context, y as i16);
    let nz = if y as u16 == 0 {
        4
    } else if y & 0x8000 != 0 {
        8
    } else {
        0
    };
    context.set_status_register((context.status_register() & !0x1f) | (u16::from(carry) << 4) | nz);
    Ok(None)
}

/// G/src/ui/hud.c:284 (`HudPositionHealthBarSprites`).
pub fn hud_position_health_bar_sprites(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let player = i32::from(arguments[0] as i16);
    let revision = context.revision();
    let (sprite, mut y) = if player == 0 { (15, 229) } else { (17, 332) };
    let x = if ram::g_player_inventory_open(revision)
        .at(player)
        .read(context)
        != 0
    {
        230
    } else if ram::g_split_screen_enabled(revision).read(context) != 0
        || ram::g_player_inventory_open(revision)
            .at(player ^ 1)
            .read(context)
            != 0
    {
        y = if player == 0 { 142 } else { 252 };
        144
    } else {
        y = 333;
        if player == 0 { 160 } else { 330 }
    };
    if ram::g_player_lives(revision).at(player).read(context) < 0 {
        y = 1;
    }
    let sprites = ram::g_hardware_sprite_table(revision);
    sprites.at(sprite).x().write(context, x);
    sprites.at(sprite).y().write(context, y);
    sprites.at(sprite + 1).x().write(context, x + 32);
    sprites.at(sprite + 1).y().write(context, y);
    Ok(None)
}

/// G/src/ui/hud.c:465 (`HudPositionLivesSprite`).
pub fn hud_position_lives_sprite(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let player = i32::from(arguments[0] as i16);
    let revision = context.revision();
    let (sprite, mut y) = if player == 0 {
        (19, 226_i16)
    } else {
        (20, 329_i16)
    };
    let lives = i16::from(ram::g_player_lives(revision).at(player).read(context));
    let x = if ram::g_player_inventory_open(revision)
        .at(player)
        .read(context)
        != 0
    {
        298
    } else if ram::g_split_screen_enabled(revision).read(context) != 0
        || ram::g_player_inventory_open(revision)
            .at(player ^ 1)
            .read(context)
            != 0
    {
        y = 5;
        138
    } else {
        y = 330;
        if player == 0 { 228 } else { 398 }
    };
    if lives < 5 {
        y = y.wrapping_sub(3);
    }
    if ram::g_player_lives(revision).at(player).read(context) < 0 {
        y = 1;
    }
    let sprite = ram::g_hardware_sprite_table(revision).at(sprite);
    sprite.x().write(context, x);
    sprite.y().write(context, y);
    Ok(None)
}
