use crate::{Context, Result, call, ids};
use tje_mem::{Address, LevelItem, ram, rom};

/// G/src/presents/item_updates.c:116 (`PresentReleaseDroppedEntities`).
/// Atomic: bounded bank scan and storage-only FreeEntity calls.
pub fn present_release_dropped_entities(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let items = ram::g_dropped_present_table(context.revision());
    for index in 0..32 {
        let item = items.at(index);
        let number = item.object().read(context);
        if number != 0 {
            call(context, ids::FREE_ENTITY, &[number as u32])?;
            item.object().write(context, 0);
        }
    }
    Ok(None)
}

/// G/src/presents/item_updates.c:259 (`PresentTryEndActivePowerup`).
/// TimeAware: HudLoadActionIconSprite may require the VBlank upload interrupt.
pub fn present_try_end_active_powerup(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let index = i32::from(arguments[0] as i16);
    let player = super::word_indexed_player(revision, index);
    let terrain = if index == 0 {
        ram::g_player1_terrain_underfoot(revision).read(context)
    } else {
        ram::g_player2_terrain_underfoot(revision).read(context)
    } & 0xf0;
    let mut sprite = -1_i8;
    if ram::g_player_powerups(revision).at(index).read(context) == 5 {
        call(context, ids::PLAY_SOUND, &[6, 99])?;
    }
    if player.height().read(context) > 0
        && player.animation_group_mask().read(context) & (0x8000 | 0x4000000) != 0
    {
        call(context, ids::STOP_PLAYER, &[player.address()])?;
        player.acceleration_height().write(context, -32);
        sprite = (player.sprite_state().read(context) & 3).wrapping_add(20);
        ram::g_player_landing_counter_seed(revision)
            .at(index)
            .write(context, 2);
    } else if player.animation_group_mask().read(context) & 0x20 == 0
        && player.height().read(context) <= 0
    {
        call(context, ids::STOP_PLAYER, &[player.address()])?;
        ram::g_player_powerups(revision)
            .at(index)
            .write(context, -1);
        call(context, ids::HUD_LOAD_ACTION_ICON_SPRITE, &[index as u32])?;
        call(
            context,
            ids::HUD_POSITION_ACTION_ICON_SPRITE,
            &[index as u32],
        )?;
        let groups = player.animation_group_mask().read(context);
        if groups & 0x40000 != 0
            || player.sprite_state().read(context) == 77
            || (groups & (0x7800000 | 0x8000) != 0 && terrain == 0x80)
        {
            sprite = (player.sprite_state().read(context) & 3).wrapping_add(64);
            player.animation_motion_axes().write(context, 0);
            player
                .draw_flags()
                .write(context, player.draw_flags().read(context) & !0x40);
        } else if (matches!(player.sprite_state().read(context), 0 | 4 | 76 | 3 | 7)
            || groups & (0x400000 | 0x20000000 | 0x7800000 | 0x0830c000) != 0)
            && groups & 0x20000000 == 0
        {
            sprite = call(context, ids::GET_ACTIVE_PRESENT_SPRITE, &[player.address()])? as i8;
            player
                .draw_flags()
                .write(context, player.draw_flags().read(context) & !0x40);
        }
    }
    if sprite != -1 {
        call(context, ids::SET_SPRITE, &[player.address(), sprite as u32])?;
    }
    Ok(None)
}

/// G/src/presents/item_updates.c:131 (`EatYummyFood`).
/// TimeAware: GenerateSpeech can wait for the sprite-upload interrupt.
pub fn eat_yummy_food(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let index = i32::from(arguments[0] as i16);
    let health = ram::g_player_health(revision).at(index);
    health.write(context, health.read(context).wrapping_add(8));
    let text = tje_mem::rom_symbol(revision, "GameText_0152F0")
        .expect("catalogued food text")
        .address;
    call(context, ids::GENERATE_SPEECH, &[index as u32, text, 1])?;
    Ok(None)
}

/// G/src/presents/item_updates.c:139 (`EatYuckyFood`).
/// TimeAware: GenerateSpeech can wait for the sprite-upload interrupt.
pub fn eat_yucky_food(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let index = i32::from(arguments[0] as i16);
    let health = ram::g_player_health(revision).at(index);
    health.write(context, health.read(context).wrapping_sub(8));
    let text = tje_mem::rom_symbol(revision, "GameText_015324")
        .expect("catalogued food text")
        .address;
    call(context, ids::GENERATE_SPEECH, &[index as u32, text, 1])?;
    Ok(None)
}

/// G/src/presents/item_updates.c:305 (`ReducePresentTimer`).
/// TimeAware: expiration can call the HUD's interrupt-dependent upload path.
pub fn reduce_present_timer(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    for index in 0..2 {
        let player = ram::g_player_info(revision).at(index);
        if ram::g_player_powerups(revision).at(index).read(context) < 0
            || ram::g_player_lives(revision).at(index).read(context) < 0
            || matches!(player.sprite_state().read(context), 5 | 6)
            || ram::g_player_inventory_open(revision)
                .at(index)
                .read(context)
                != 0
        {
            continue;
        }
        let icon = ram::g_hardware_sprite_table(revision).at(21 + index);
        if ram::g_player_elevator_flags(revision)
            .at(index)
            .read(context)
            != 0
        {
            if icon.y().read(context) == 1 {
                call(
                    context,
                    ids::HUD_POSITION_ACTION_ICON_SPRITE,
                    &[index as u32],
                )?;
            }
        } else {
            let timer = ram::g_player_powerup_timers(revision).at(index);
            let show = if timer.read(context) == 0 {
                call(
                    context,
                    ids::PRESENT_TRY_END_ACTIVE_POWERUP,
                    &[index as u32],
                )?;
                ram::g_player_powerups(revision).at(index).read(context) < 0
            } else {
                let remaining = timer.read(context).wrapping_sub(1);
                timer.write(context, remaining);
                if remaining > 300 {
                    true
                } else if timer.read(context) > 100 {
                    timer.read(context) & 12 != 0
                } else {
                    timer.read(context) & 2 != 0
                }
            };
            if show {
                call(
                    context,
                    ids::HUD_POSITION_ACTION_ICON_SPRITE,
                    &[index as u32],
                )?;
            } else {
                icon.y().write(context, 1);
            }
        }
    }
    Ok(None)
}

/// G/src/presents/item_updates.c:147 (`EatFudgeSundae`).
/// TimeAware: dialogue can wait for an interrupt-owned graphics upload.
pub fn eat_fudge_sundae(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let index = i32::from(arguments[0] as i16);
    let health = ram::g_player_health(context.revision()).at(index);
    health.write(context, health.read(context).wrapping_add(12));
    call(context, ids::START_DIALOGUE, &[6, index as u32])?;
    Ok(None)
}

/// G/src/presents/item_updates.c:84 (`EntityUpdateLevelItemVisibility`).
/// Atomic: RAM camera bounds and entity allocation/release. EntityCreateLevelItem's
/// missing fourth argument consumes its saved entry A2, so this caller supplies
/// the item pointer residue observed in G/build/{rev02,rev00}/asm/c_item_updates.s.
pub fn entity_update_level_item_visibility(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let items = Address::<LevelItem>::new(arguments[0]);
    let count = i32::from(arguments[1] as i16);
    for index in 0..count {
        let item = items.offset(index);
        let first_x = item
            .x()
            .read(context)
            .wrapping_sub(ram::g_camera1_pending_x(revision).read(context));
        let first_y = item
            .y()
            .read(context)
            .wrapping_sub(ram::g_camera1_pending_y(revision).read(context));
        let second_x = item
            .x()
            .read(context)
            .wrapping_sub(ram::g_camera2_pending_x(revision).read(context));
        let second_y = item
            .y()
            .read(context)
            .wrapping_sub(ram::g_camera2_pending_y(revision).read(context));
        if item.kind().read(context) != -1
            && ((item.level().read(context) == ram::g_player1_map_level(revision).read(context)
                && i32::from(first_x) + 20 > 0
                && first_x < 340
                && i32::from(first_y) + 20 > 0
                && ((item.kind().read(context) < 81 && first_y < 244)
                    || (item.kind().read(context) >= 81 && first_y < 288)))
                || (item.level().read(context) == ram::g_player2_map_level(revision).read(context)
                    && ram::g_split_screen_enabled(revision).read(context) != 0
                    && i32::from(second_x) + 20 > 0
                    && second_x < 340
                    && i32::from(second_y) + 20 > 0
                    && ((item.kind().read(context) < 81 && second_y < 132)
                        || (item.kind().read(context) >= 81 && second_y < 176))))
        {
            if item.object().read(context) == 0 {
                let saved_a2 = context.address_register(2);
                context.set_address_register(2, item.address());
                let result = call(context, ids::ENTITY_CREATE_LEVEL_ITEM, &[item.address()]);
                context.set_address_register(2, saved_a2);
                result?;
            }
        } else {
            let number = item.object().read(context);
            if number != 0 {
                call(context, ids::FREE_ENTITY, &[number as u32])?;
                item.object().write(context, 0);
            }
        }
    }
    Ok(None)
}

/// G/src/presents/item_updates.c:156 (`PlayerCollectNearbyItems`).
/// TimeAware: pickup speech/dialogue may wait for graphics IRQ progress.
/// Full-health food is still removed, and collected bits use the original
/// register-count shift, including its modulo-64 behavior.
pub fn player_collect_nearby_items(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let index = i32::from(arguments[0] as i16);
    let items = Address::<LevelItem>::new(arguments[1]);
    let count = i32::from(arguments[2] as i16);
    let record = arguments[3] as i8;
    let player = super::word_indexed_player(revision, index);
    if ram::g_player_lives(revision).at(index).read(context) < 0
        || player.height().read(context) >= 4
        || matches!(player.sprite_state().read(context), 3 | 7 | 77)
        || player.animation_group_mask().read(context) & (0x800 | 0x1000 | 0x40000) != 0
    {
        return Ok(None);
    }
    for slot in 0..count {
        let item = items.offset(slot);
        if item.object().read(context) == 0 {
            continue;
        }
        let mut collected = false;
        if (i32::from(item.x().read(context)) - i32::from(player.x().read(context))).wrapping_abs()
            < 16
            && (i32::from(item.y().read(context)) - i32::from(player.y().read(context)))
                .wrapping_abs()
                < 8
        {
            if item.kind().read(context) < 64 {
                let inventory = ram::g_player_inventory(revision).at(index);
                for inventory_slot in 0..16 {
                    if inventory.at(inventory_slot).read(context) == -1 {
                        inventory
                            .at(inventory_slot)
                            .write(context, item.kind().read(context));
                        collected = true;
                        call(context, ids::PLAY_SOUND, &[1, 50])?;
                        call(
                            context,
                            ids::EARTHLING_CLEAR_MOLE_PLAYER_CONTACT_FLAGS,
                            &[player.address()],
                        )?;
                        break;
                    }
                }
                if !collected && call(context, ids::HAS_SPEAKER_SPEECH, &[index as u32])? == 0 {
                    call(context, ids::START_DIALOGUE, &[25, index as u32])?;
                }
            } else {
                collected = true;
                let food = i16::from(item.kind().read(context)).wrapping_sub(64);
                let mut stuffed = false;
                if item.kind().read(context) == 80 {
                    let bucks = ram::g_player_bucks(revision).at(index);
                    bucks.write(context, bucks.read(context).wrapping_add(1));
                    call(
                        context,
                        ids::PLAY_SOUND_EFFECT,
                        &[rom::sfx_money(revision).address(), 50, 0],
                    )?;
                } else {
                    let amount = i16::from(
                        rom::present_auxiliary_signed_lookup(revision)
                            .at(i32::from(food))
                            .read(context),
                    );
                    let health = ram::g_player_health(revision).at(index);
                    if amount > 0
                        && i32::from(health.read(context))
                            >= call(context, ids::PLAYER_GET_MAXIMUM_HEALTH, &[index as u32])?
                                as i32
                    {
                        let text = tje_mem::rom_symbol(revision, "FoodIsFullStr")
                            .expect("catalogued food text")
                            .address;
                        call(context, ids::GENERATE_SPEECH, &[index as u32, text, 1])?;
                        call(
                            context,
                            ids::PLAY_SOUND_EFFECT,
                            &[rom::sfx_chow_down(revision).address(), 50, 0],
                        )?;
                        stuffed = true;
                    } else {
                        health.write(context, health.read(context).wrapping_add(amount as i8));
                        if amount > 0 {
                            call(
                                context,
                                ids::PLAY_SOUND_EFFECT,
                                &[rom::sfx_chow_down(revision).address(), 50, 0],
                            )?;
                        } else {
                            call(context, ids::PLAY_SOUND, &[18, 40])?;
                        }
                    }
                }
                if !stuffed {
                    call(
                        context,
                        ids::START_DIALOGUE,
                        &[(i32::from(food) + 5) as u32, index as u32],
                    )?;
                }
            }
        }
        if collected {
            item.kind().write(context, -1);
            if record != 0 {
                let mask = ram::g_collected_level_item_bits(revision)
                    .at(i32::from(item.level().read(context)));
                mask.write(
                    context,
                    mask.read(context) | tje_mem::arithmetic::lsr_u32(0x80000000, slot as u32),
                );
            }
        }
    }
    Ok(None)
}

/// G/src/presents/item_updates.c:366 (`MapUpdateItemsAndPresents`).
/// TimeAware: pickup speech, powerup HUD expiration, doorway and telephone waits.
pub fn map_update_items_and_presents(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    if ram::g_player_inventory_open(revision).at(0).read(context) != 0
        && ram::g_player_inventory_open(revision).at(1).read(context) != 0
    {
        return Ok(None);
    }
    for bank in 0..2 {
        if ram::g_item_bank_users(revision).at(bank).read(context) != 0 {
            call(
                context,
                ids::ENTITY_UPDATE_LEVEL_ITEM_VISIBILITY,
                &[ram::g_object_table(revision).at(bank).address(), 32],
            )?;
        }
    }
    call(
        context,
        ids::ENTITY_UPDATE_LEVEL_ITEM_VISIBILITY,
        &[ram::g_dropped_present_table(revision).address(), 32],
    )?;
    for index in 0..2 {
        if ram::g_player_lives(revision).at(index).read(context) >= 0
            && ram::g_player_inventory_open(revision)
                .at(index)
                .read(context)
                == 0
        {
            for bank in 0..2 {
                if ram::g_item_bank_users(revision).at(bank).read(context) != 0
                    && ram::g_player_info(revision).at(index).level().read(context)
                        == ram::g_item_bank_levels(revision).at(bank).read(context)
                {
                    let items = ram::g_object_table(revision).at(bank);
                    call(
                        context,
                        ids::PLAYER_COLLECT_NEARBY_ITEMS,
                        &[index as u32, items.address(), 28, 1],
                    )?;
                    call(
                        context,
                        ids::PLAYER_CHECK_LEVEL_ITEM_HAZARDS,
                        &[index as u32, items.at(28).address()],
                    )?;
                }
            }
            call(
                context,
                ids::PLAYER_COLLECT_NEARBY_ITEMS,
                &[
                    index as u32,
                    ram::g_dropped_present_table(revision).address(),
                    32,
                    0,
                ],
            )?;
        }
    }
    call(context, ids::REDUCE_PRESENT_TIMER, &[])?;
    call(context, ids::PROCESS_PRESENTS, &[])?;
    if ram::g_player1_map_level(revision).read(context) == 26 {
        call(context, ids::FUNKATRON_UPDATE_SCENE_ENTITIES, &[])?;
    }
    Ok(None)
}
