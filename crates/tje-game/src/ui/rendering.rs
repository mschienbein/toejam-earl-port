use crate::{Context, Result, call, ids};
use tje_mem::{Address, arithmetic, ram, rom};

fn vram_write(address: i32) -> u32 {
    0x40000000_u32
        .wrapping_add(((address as u32) & 0x3fff) << 16)
        .wrapping_add(((address as u32) & 0xc000) >> 14)
}

/// G/src/ui/hud.c:619 (`HudHidePlayerSprites`), Functional transaction contract.
/// Every VDP word precedes its corresponding RAM shadow update.
pub fn hud_hide_player_sprites(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let player = arguments[0] as i16;
    let sprites = ram::g_hardware_sprite_table(context.revision());
    if player == 0 || player == 2 {
        for (command, sprite) in [
            (0x78580003, 11),
            (0x78600003, 12),
            (0x78a80003, 21),
            (0x78780003, 15),
            (0x78800003, 16),
            (0x78980003, 19),
        ] {
            context.write_vdp_command(command);
            context.write_vdp_data(1);
            sprites.at(sprite).y().write(context, 1);
        }
    }
    if player == 1 || player == 2 {
        for (command, sprite) in [
            (0x78680003, 13),
            (0x78700003, 14),
            (0x78b00003, 22),
            (0x78880003, 17),
            (0x78900003, 18),
            (0x78a00003, 20),
        ] {
            context.write_vdp_command(command);
            context.write_vdp_data(1);
            sprites.at(sprite).y().write(context, 1);
        }
    }
    Ok(None)
}

/// G/src/ui/inventory.c:75 (`TextAnimateFontBanks`).
/// The signed-byte tick wraps after increment; the old tick is the dividend.
pub fn text_animate_font_banks(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let bank = ram::g_text_font_bank(revision);
    let tick = ram::g_text_font_animation_tick(revision);
    if ram::g_text_font_animation_enabled(revision).read(context) == 0 {
        if bank.read(context) == 0 {
            return Ok(Some(0));
        }
        tick.write(context, 0);
    }
    let previous = tick.read(context);
    let current = tick.read(context).wrapping_add(1);
    tick.write(context, current);
    if arithmetic::rem_i32(i32::from(previous), i32::from(arguments[0] as i16))? == 0 {
        let new_bank = bank.read(context) ^ 1;
        bank.write(context, new_bank);
        let source = ram::g_text_font_buffers(revision)
            .at(i32::from(bank.read(context)))
            .address();
        call(context, ids::DMA_COPY_WORDS_TO_VRAM, &[source, 0xb680, 944])?;
        return Ok(Some(1));
    }
    Ok(Some(0))
}

/// G/src/ui/inventory.c:694 (`TextWriteStringTiles`).
/// Reads the zero test separately from the emitted character, as the original does.
pub fn text_write_string_tiles(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let mut text = Address::<i8>::new(arguments[0]);
    while text.read(context) != 0 {
        let character = text.read(context);
        text = text.offset(1);
        let glyph = call(
            context,
            ids::TEXT_MAP_CHARACTER_TO_GLYPH,
            &[i32::from(character) as u32],
        )? as i8;
        context.write_vdp_data(i32::from(glyph).wrapping_add(0x85b4) as u16);
    }
    Ok(None)
}

/// G/src/ui/hud.c:24 (`HudLoadLevelNumberSprites`).
pub fn hud_load_level_number_sprites(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let player = arguments[0] as i16;
    let (sprite, mut tile, level) = if player == 0 {
        (11, 0x7e1_i16, ram::g_toejam_level(revision).read(context))
    } else {
        (13, 0x7e9_i16, ram::g_earl_level(revision).read(context))
    };
    let units = i32::from(level) % 10;
    let tens = i32::from(level) / 10;
    context.write_vdp_command(vram_write(i32::from(tile).wrapping_shl(5)));
    let source = rom::image_level_numbers(revision).at(units).address();
    call(context, ids::MOVE_DATA_TO_VRAM, &[source, 64])?;
    let attributes = (u16::from(rom::level_number_palettes(revision).at(units).read(context))
        << 13)
        .wrapping_add(0x8000);
    let sprites = ram::g_hardware_sprite_table(revision);
    sprites.at(sprite).size().write(context, 5);
    sprites
        .at(sprite)
        .tile()
        .write(context, (tile as u16).wrapping_add(attributes));
    tile = tile.wrapping_add(4);
    context.write_vdp_command(vram_write(i32::from(tile).wrapping_shl(5)));
    let source = rom::image_level_numbers(revision).at(tens).address();
    call(context, ids::MOVE_DATA_TO_VRAM, &[source, 64])?;
    let attributes = (u16::from(rom::level_number_palettes(revision).at(tens).read(context)) << 13)
        .wrapping_add(0x8000);
    sprites.at(sprite + 1).size().write(context, 5);
    sprites
        .at(sprite + 1)
        .tile()
        .write(context, (tile as u16).wrapping_add(attributes));
    Ok(None)
}

/// G/src/ui/hud.c:384 (`HudLoadLivesSpriteTiles`).
/// The original keeps uploading beyond four tiles when lives exceeds eight.
pub fn hud_load_lives_sprite_tiles(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let player = i32::from(arguments[0] as i16);
    if ram::g_player_lives(revision).at(player).read(context) < 0 {
        return Ok(None);
    }
    let (mut tile, sprite, icons) = if player == 0 {
        (0x7bc_i32, 19, rom::icon_tiles_a(revision).at(0))
    } else {
        (0x7dc_i32, 20, rom::icon_tiles_b(revision).at(0))
    };
    let lives = i16::from(ram::g_player_lives(revision).at(player).read(context));
    let sprites = ram::g_hardware_sprite_table(revision);
    sprites.at(sprite).size().write(context, 12);
    sprites
        .at(sprite)
        .tile()
        .write(context, (tile as u16).wrapping_add(0xe000));
    let (first_count, first_source, second_count, second_source) = if lives > 4 {
        (
            lives.wrapping_sub(4),
            icons.byte_offset(32),
            8_i16.wrapping_sub(lives),
            icons.cast::<u8>(),
        )
    } else {
        (
            lives,
            icons,
            4_i16.wrapping_sub(lives),
            rom::level_tiles(revision).at(0),
        )
    };
    for (count, source) in [
        (first_count, first_source.cast::<u8>()),
        (second_count, second_source),
    ] {
        for _ in 0..count {
            context.write_vdp_command(vram_write(tile.wrapping_shl(5)));
            call(context, ids::MOVE_DATA_TO_VRAM, &[source.address(), 16])?;
            tile = tile.wrapping_add(1);
        }
    }
    ram::g_hud_displayed_lives(revision)
        .at(player)
        .write(context, lives as i8);
    Ok(None)
}

/// G/src/ui/window_panels.c:135 (`DrawNestedWindowFrame`).
pub fn draw_nested_window_frame(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    context.write_vdp_command(0x79a00000);
    call(
        context,
        ids::MOVE_DATA_TO_VRAM,
        &[rom::inventory_auxiliary_tiles(revision).address(), 80],
    )?;
    context.write_vdp_command(0x7a400000);
    let second = i32::from(
        rom::inventory_auxiliary_tile_counts(revision)
            .at(1)
            .read(context),
    );
    let first = i32::from(
        rom::inventory_auxiliary_tile_counts(revision)
            .at(0)
            .read(context),
    );
    let count = second.wrapping_add(first).wrapping_shl(4) as u32;
    call(
        context,
        ids::MOVE_DATA_TO_VRAM,
        &[
            rom::inventory_auxiliary_tiles_large(revision).address(),
            count,
        ],
    )?;
    for row in 0_i32..28 {
        context.write_vdp_command(0x60000003_u32.wrapping_add((row as u32) << 23));
        for column in 0..40 {
            let tile = if row == 0 || row == 27 || column == 0 || column == 39 {
                0xe1d1
            } else if row == 1 || row == 26 || column == 1 || column == 38 {
                0xe1cd
            } else if (column == 2 || column == 37) && row > 2 && row < 25 {
                if column == 2 { 0xe1ce } else { 0xe9ce }
            } else if (row == 2 || row == 25) && column > 2 && column < 37 {
                if row == 2 { 0xe1cf } else { 0xf1cf }
            } else if row == 2 && column == 2 {
                0xe1d0
            } else if row == 25 && column == 2 {
                0xf1d0
            } else if row == 2 && column == 37 {
                0xe9d0
            } else if row == 25 && column == 37 {
                0xf9d0
            } else {
                0x8000
            };
            context.write_vdp_data(tile);
        }
    }
    Ok(None)
}

/// G/src/ui/window_panels.c:176 (`DrawPresentDisplayPanel`).
pub fn draw_present_display_panel(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    if ram::g_present_display_panel_drawn(revision).read(context) != 0 {
        return Ok(None);
    }
    call(context, ids::DISABLE_BLANKING_GRAPHICS, &[])?;
    for row in 0_i32..5 {
        context.write_vdp_command(0x60140003_u32.wrapping_add(((row + 5) as u32) << 23));
        for column in 0..19 {
            let mut tile = u16::from(
                rom::present_display_tilemap(revision)
                    .at(row)
                    .at(column)
                    .read(context),
            );
            if tile & 0x80 != 0 {
                tile = (tile & 0x7f) | 0x800;
            }
            context.write_vdp_data(0xc1d2_u16.wrapping_add(tile));
        }
    }
    call(context, ids::ENABLE_BLANKING_GRAPHICS, &[])?;
    ram::g_present_display_panel_drawn(revision).write(context, 1);
    Ok(None)
}

/// G/src/ui/window_panels.c:200 (`DrawPauseChoicePanel`).
pub fn draw_pause_choice_panel(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let selection = i32::from(arguments[0] as i16);
    call(context, ids::DISABLE_BLANKING_GRAPHICS, &[])?;
    let tile_base = (i32::from(
        rom::inventory_auxiliary_tile_counts(revision)
            .at(1)
            .read(context),
    ) + 465) as u16;
    for row in 0_i32..6 {
        context.write_vdp_command(0x60200003_u32.wrapping_add(((row + 11) as u32) << 23));
        for column in 0..8 {
            let tile = if selection < 0 {
                0x8000
            } else {
                0x8000_u16
                    .wrapping_add(
                        rom::inventory_vdp_tilemap(revision)
                            .at(selection)
                            .at(row)
                            .at(column)
                            .read(context),
                    )
                    .wrapping_add(tile_base)
            };
            context.write_vdp_data(tile);
        }
    }
    call(context, ids::ENABLE_BLANKING_GRAPHICS, &[])?;
    Ok(None)
}
