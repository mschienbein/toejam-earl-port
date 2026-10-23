use crate::{Context, Result, call, ids};
use tje_mem::{ElevatorRecord, ram, rom};

/// G/src/elevator/elevator_scene.c:327 (`LoadFinalLevelMap`).
/// Functional: the original stable-ROM tile/palette stream and ordered HUD stores.
pub fn load_final_level_map(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    context.write_vdp_command(0x40000000);
    let count = i32::from(rom::level_tiles_size(revision).read(context)).wrapping_shl(4);
    call(
        context,
        ids::MOVE_DATA_TO_VRAM,
        &[rom::level_tiles(revision).address(), count as u32],
    )?;
    call(context, ids::TEXT_COPY_FONT_BANKS_TO_RAM, &[])?;
    for (command, offset, count) in [
        (0x42800000, 0, 80),
        (0x48200000, 160, 16),
        (0x52400000, 192, 48),
        (0x6a800000, 288, 896),
    ] {
        context.write_vdp_command(command);
        call(
            context,
            ids::MOVE_DATA_TO_VRAM,
            &[
                rom::funkatron_extra_tiles(revision)
                    .cast::<u8>()
                    .offset(offset)
                    .address(),
                count,
            ],
        )?;
    }
    for (palette, bank) in [
        (rom::palette_ship_screen_bank3(revision), 0),
        (rom::funkatron_palette_a(revision), 1),
        (rom::funkatron_palette_b(revision), 2),
        (rom::palette_game_line4(revision), 3),
    ] {
        call(
            context,
            ids::COPY_PALETTE_TO_CRAM_SLOT,
            &[palette.address(), bank],
        )?;
    }
    context.write_vdp_control(0x9200);
    call(context, ids::DRAW_NESTED_WINDOW_FRAME, &[])?;
    context.write_vdp_command(0x40020010);
    context.write_vdp_data(0);
    context.write_vdp_command(0x7c020003);
    context.write_vdp_data(0);
    call(context, ids::HUD_HIDE_PLAYER_SPRITES, &[2])?;
    call(context, ids::SET_CAMERA_TILE_PRIORITY_ENABLED, &[0])?;
    Ok(None)
}

/// G/src/elevator/elevator_scene.c:358 (`MapClearActiveLevelObjects`).
pub fn map_clear_active_level_objects(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    for player in 0..2 {
        if ram::g_player_lives(revision).at(player).read(context) >= 0 {
            let level = ram::g_player_info(revision)
                .at(player)
                .level()
                .read(context);
            call(
                context,
                ids::EARTHLING_RELEASE_LEVEL_BANK,
                &[i32::from(level) as u32],
            )?;
            let level = ram::g_player_info(revision)
                .at(player)
                .level()
                .read(context);
            call(
                context,
                ids::MAP_RELEASE_LEVEL_ITEM_BANK,
                &[i32::from(level) as u32],
            )?;
            ram::g_player_elevator_flags(revision)
                .at(player)
                .write(context, 0);
        }
    }
    for player in 0..2 {
        let entity = ram::g_player_info(revision).at(player);
        if entity.draw_flags().read(context) & 1 != 0 {
            if entity.draw_flags().read(context) & 2 != 0 {
                call(
                    context,
                    ids::RELEASE_ENTITY_VIEWPORT_SPRITES,
                    &[entity.address(), 0],
                )?;
            }
            if entity.draw_flags().read(context) & 4 != 0 {
                call(
                    context,
                    ids::RELEASE_ENTITY_VIEWPORT_SPRITES,
                    &[entity.address(), 1],
                )?;
            }
        }
    }
    for number in 0..4 {
        let elevator = ram::g_elevator_records(revision)
            .cast::<ElevatorRecord>()
            .offset(number);
        if elevator.entity().read(context) != 0 {
            let entity = i32::from(elevator.entity().read(context));
            call(context, ids::FREE_ENTITY, &[entity as u32])?;
            let entity = i32::from(elevator.entity().read(context)) + 2;
            call(context, ids::FREE_ENTITY, &[entity as u32])?;
            elevator.entity().write(context, 0);
        }
        call(context, ids::ELEVATOR_SET_STATE, &[elevator.address(), 11])?;
    }
    call(context, ids::CLEAR_TRANSIENT_ENTITIES, &[])?;
    Ok(None)
}

/// G/src/elevator/elevator_scene.c:396 (`MapEnterFinalLevel`).
pub fn map_enter_final_level(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    for player in 0..2 {
        if ram::g_player_lives(revision).at(player).read(context) >= 0 {
            let entity = ram::g_player_info(revision).at(player);
            entity.level().write(context, 26);
            let level = i32::from(entity.level().read(context));
            call(
                context,
                ids::MAP_LOAD_PLAYER_LEVEL,
                &[player as u32, level as u32],
            )?;
            entity.height().write(context, 0);
            call(context, ids::STOP_PLAYER, &[entity.address()])?;
            ram::g_player_fall_states(revision)
                .at(player)
                .write(context, 0);
            call(context, ids::SET_SPRITE, &[entity.address(), 0])?;
            let behavior = entity.behavior_flags().read(context) & !(0x10 | 0x40);
            entity.behavior_flags().write(context, behavior);
            let tile = entity.tile().read(context) & !0x8000;
            entity.tile().write(context, tile);
        }
    }
    let mode = ram::g_mode2_reg_softcopy(revision);
    let value = mode.read(context) & 0x3c;
    mode.write(context, value);
    context.write_vdp_control(0x8104 | u16::from(mode.read(context)));
    call(context, ids::LOAD_FINAL_LEVEL_MAP, &[])?;
    call(context, ids::CLEAR_SPEECH_BUBBLES_AND_REPLIES, &[])?;
    let first = ram::g_player_info(revision).at(0);
    let second = ram::g_player_info(revision).at(1);
    first.x().write(context, 496);
    first.y().write(context, 1467);
    let x = first.x().read(context).wrapping_add(32);
    second.x().write(context, x);
    let y = first.y().read(context);
    second.y().write(context, y);
    call(context, ids::UPDATE_SCREEN_SPLIT_MODE, &[1])?;
    let value = mode.read(context) | 0x40;
    mode.write(context, value);
    context.write_vdp_control(0x8104 | u16::from(mode.read(context)));
    call(context, ids::ENABLE_BLANKING_GRAPHICS, &[])?;
    Ok(None)
}
