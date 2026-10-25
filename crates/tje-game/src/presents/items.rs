use crate::call_frame::{CallSite, FrameLayout, LocalFrame, OutgoingArguments};
use crate::{Context, FunctionId, Result, call_at, call_at_void, call_void, ids};
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
            call_void(context, ids::FREE_ENTITY, &[number as u32])?;
            item.object().write(context, 0);
        }
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
    item_call(
        context,
        ids::EAT_YUMMY_FOOD,
        ids::GENERATE_SPEECH,
        &[index as u32, text, 1],
        0x28,
    )?;
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
    item_call(
        context,
        ids::EAT_YUCKY_FOOD,
        ids::GENERATE_SPEECH,
        &[index as u32, text, 1],
        0x28,
    )?;
    Ok(None)
}

/// G/src/presents/item_updates.c:305 (`ReducePresentTimer`).
/// TimeAware: expiration can call the HUD's interrupt-dependent upload path.
pub fn reduce_present_timer(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    // c_item_updates.s:729: this preserves the physical outgoing call depth.
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x0c, 0x0c))?;
    let result = reduce_timer_body(context);
    leave_item_frame(context, frame, result)
}

fn reduce_timer_body(context: &mut dyn Context) -> Result<Option<u32>> {
    let revision = context.revision();
    let caller = ids::REDUCE_PRESENT_TIMER;
    for index in 0..2 {
        let player = ram::g_player_info(revision).at(index);
        if !timer_player_eligible(context, revision, index, player) {
            continue;
        }
        let icon = ram::g_hardware_sprite_table(revision).at(21 + index);
        if ram::g_player_elevator_flags(revision)
            .at(index)
            .read(context)
            != 0
        {
            if icon.y().read(context) == 1 {
                item_call_void(
                    context,
                    caller,
                    ids::HUD_POSITION_ACTION_ICON_SPRITE,
                    &[index as u32],
                    0xae,
                )?;
            }
        } else {
            let timer = ram::g_player_powerup_timers(revision).at(index);
            let show = if timer.read(context) == 0 {
                // The powerup re-read precedes ADDQ #4,SP (s:794-798).
                let outgoing = item_retained_call(
                    context,
                    caller,
                    ids::PRESENT_TRY_END_ACTIVE_POWERUP,
                    &[index as u32],
                    0xcc,
                )?;
                let ended = ram::g_player_powerups(revision).at(index).read(context) < 0;
                outgoing.release(context)?;
                ended
            } else {
                timer_show_nonzero(context, timer)
            };
            if show {
                item_call_void(
                    context,
                    caller,
                    ids::HUD_POSITION_ACTION_ICON_SPRITE,
                    &[index as u32],
                    0xae,
                )?;
            } else {
                icon.y().write(context, 1);
            }
        }
    }
    // ADDQ.W 1 -> 2 clears X; final CMPI.W #1,D2 yields positive nonzero.
    context.set_status_register(context.status_register() & !0x1f);
    Ok(None)
}

/// G/src/presents/item_updates.c:147 (`EatFudgeSundae`).
/// TimeAware: dialogue can wait for an interrupt-owned graphics upload.
pub fn eat_fudge_sundae(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let index = i32::from(arguments[0] as i16);
    let health = ram::g_player_health(context.revision()).at(index);
    health.write(context, health.read(context).wrapping_add(12));
    item_call(
        context,
        ids::EAT_FUDGE_SUNDAE,
        ids::START_DIALOGUE,
        &[6, index as u32],
        0x24,
    )?;
    Ok(None)
}

/// G/src/presents/item_updates.c:156 (`PlayerCollectNearbyItems`).
/// TimeAware: pickup speech/dialogue may wait for graphics IRQ progress.
/// c_item_updates.s:298-515: retain LINK -8, the food byte/word local, delayed
/// record_pickup load, actual abs calls and both paired outgoing-argument areas.
/// Full-health food is still removed (the original bug).
pub fn player_collect_nearby_items(context: &mut dyn Context) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::link(8, 0x7c, 0x0c))?;
    let result = collect_items_body(context, &frame);
    leave_item_frame(context, frame, result)
}

fn collect_items_body(context: &mut dyn Context, frame: &LocalFrame) -> Result<Option<u32>> {
    let revision = context.revision();
    let caller = ids::PLAYER_COLLECT_NEARBY_ITEMS;
    let index = i32::from(frame.argument_word(0).read(context));
    let items = Address::<LevelItem>::new(frame.argument_slot(1).read(context));
    let count = frame.argument_word(2).read(context);
    let player = super::word_indexed_player(revision, index);
    // ASL.W #7 supplies X before the eligibility TST/CMP instructions.
    context.set_status_register(
        (context.status_register() & !0x10) | (u16::from(index as u16 & 0x0200 != 0) << 4),
    );
    if !collection_player_eligible(context, revision, index, player) {
        return Ok(None);
    }
    let cursor = frame.local::<i16>(-4);
    let collected = frame.local::<i8>(-1);
    let food = frame.local::<i16>(-6);
    cursor.write(context, 0);
    loop {
        let slot = cursor.read(context);
        super::expiry::compare(context, u32::from(slot as u16), u32::from(count as u16), 16);
        if slot >= count {
            break;
        }
        let item = items.offset(i32::from(slot));
        if item.object().read(context) != 0 {
            collected.write(context, 0);
            let dx = i32::from(item.x().read(context)) - i32::from(player.x().read(context));
            let near_x = (item_call(context, caller, ids::ABS, &[dx as u32], 0x9e)? as i32) < 16;
            let nearby = if near_x {
                let dy = i32::from(item.y().read(context)) - i32::from(player.y().read(context));
                (item_call(context, caller, ids::ABS, &[dy as u32], 0xbe)? as i32) < 8
            } else {
                false
            };
            if nearby {
                if item.kind().read(context) < 64 {
                    let inventory = ram::g_player_inventory(revision).at(index);
                    for inventory_slot in 0..16 {
                        if inventory.at(inventory_slot).read(context) == -1 {
                            inventory
                                .at(inventory_slot)
                                .write(context, item.kind().read(context));
                            collected.write(context, 1);
                            let sound = item_retained_call(
                                context,
                                caller,
                                ids::PLAY_SOUND,
                                &[1, 50],
                                0x112,
                            )?;
                            let clear = item_call_void(
                                context,
                                caller,
                                ids::EARTHLING_CLEAR_MOLE_PLAYER_CONTACT_FLAGS,
                                &[player.address()],
                                0x11a,
                            );
                            if clear
                                .as_ref()
                                .is_err_and(|error| error.preserves_call_frame())
                            {
                                return clear.map(|_| None);
                            }
                            sound.release(context)?;
                            clear?;
                            break;
                        }
                    }
                    if collected.read(context) == 0
                        && item_call(
                            context,
                            caller,
                            ids::HAS_SPEAKER_SPEECH,
                            &[index as u32],
                            0x13c,
                        )? == 0
                    {
                        item_call(
                            context,
                            caller,
                            ids::START_DIALOGUE,
                            &[25, index as u32],
                            0x248,
                        )?;
                    }
                } else {
                    collected.write(context, 1);
                    // MOVE.B kind,-5(A6); MOVE.W -6(A6),D0; EXT.W D0.
                    // The other byte is deliberately not initialized by the C.
                    food.byte_offset(1)
                        .cast::<i8>()
                        .write(context, item.kind().read(context));
                    let widened = i16::from(food.read(context) as i8);
                    food.write(context, widened);
                    food.write(context, food.read(context).wrapping_sub(64));
                    let mut stuffed = false;
                    if item.kind().read(context) == 80 {
                        let bucks = ram::g_player_bucks(revision).at(index);
                        bucks.write(context, bucks.read(context).wrapping_add(1));
                        item_call_void(
                            context,
                            caller,
                            ids::PLAY_SOUND_EFFECT,
                            &[rom::sfx_money(revision).address(), 50, 0],
                            0x192,
                        )?;
                    } else {
                        let amount = i16::from(
                            rom::present_auxiliary_signed_lookup(revision)
                                .at(i32::from(food.read(context)))
                                .read(context),
                        );
                        let health = ram::g_player_health(revision).at(index);
                        // The compiled health load precedes the maximum-health call.
                        if amount > 0
                            && i32::from(health.read(context))
                                >= item_call(
                                    context,
                                    caller,
                                    ids::PLAYER_GET_MAXIMUM_HEALTH,
                                    &[index as u32],
                                    0x1ca,
                                )? as i32
                        {
                            let text = tje_mem::rom_symbol(revision, "FoodIsFullStr")
                                .expect("catalogued food text")
                                .address;
                            let speech = item_retained_call(
                                context,
                                caller,
                                ids::GENERATE_SPEECH,
                                &[index as u32, text, 1],
                                0x1e6,
                            )?;
                            let sound = item_call_void(
                                context,
                                caller,
                                ids::PLAY_SOUND_EFFECT,
                                &[rom::sfx_chow_down(revision).address(), 50, 0],
                                0x1f8,
                            );
                            if sound
                                .as_ref()
                                .is_err_and(|error| error.preserves_call_frame())
                            {
                                return sound.map(|_| None);
                            }
                            speech.release(context)?;
                            sound?;
                            stuffed = true;
                        } else {
                            health.write(context, health.read(context).wrapping_add(amount as i8));
                            if amount > 0 {
                                item_call_void(
                                    context,
                                    caller,
                                    ids::PLAY_SOUND_EFFECT,
                                    &[rom::sfx_chow_down(revision).address(), 50, 0],
                                    0x192,
                                )?;
                            } else {
                                item_call_void(context, caller, ids::PLAY_SOUND, &[18, 40], 0x230)?;
                            }
                        }
                    }
                    if !stuffed {
                        let dialogue = (i32::from(food.read(context)) + 5) as u32;
                        item_call(
                            context,
                            caller,
                            ids::START_DIALOGUE,
                            &[dialogue, index as u32],
                            0x248,
                        )?;
                    }
                }
            }
            if collected.read(context) != 0 {
                item.kind().write(context, -1);
                if frame.argument_byte(3).read(context) != 0 {
                    let bits = ram::g_collected_level_item_bits(revision)
                        .at(i32::from(item.level().read(context)));
                    let shift = cursor.byte_offset(1).cast::<u8>().read(context);
                    let value = bits.read(context)
                        | tje_mem::arithmetic::lsr_u32(0x8000_0000, u32::from(shift));
                    bits.write(context, value);
                }
            }
        }
        cursor.write(context, cursor.read(context).wrapping_add(1));
        // The loop's ADDQ.W cannot carry in this bounded signed-short domain.
        context.set_status_register(context.status_register() & !0x10);
    }
    Ok(None)
}

/// G/src/presents/item_updates.c:366 (`MapUpdateItemsAndPresents`).
/// TimeAware: pickup speech, powerup HUD expiration, doorway and telephone waits.
pub fn map_update_items_and_presents(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x0c, 0x1c))?;
    let result = map_items_body(context);
    leave_item_frame(context, frame, result)
}

fn map_items_body(context: &mut dyn Context) -> Result<Option<u32>> {
    let revision = context.revision();
    let caller = ids::MAP_UPDATE_ITEMS_AND_PRESENTS;
    let inventory = ram::g_player_inventory_open(revision);
    let first_open = inventory.at(0).read(context);
    super::expiry::logical(context, u32::from(first_open as u8), 8);
    if first_open != 0 {
        let second_open = inventory.at(1).read(context);
        super::expiry::logical(context, u32::from(second_open as u8), 8);
        if second_open != 0 {
            return Ok(None);
        }
    }
    for (bank, site) in [(0, 0x30), (1, 0x46)] {
        if ram::g_item_bank_users(revision).at(bank).read(context) != 0 {
            item_call_void(
                context,
                caller,
                ids::ENTITY_UPDATE_LEVEL_ITEM_VISIBILITY,
                &[ram::g_object_table(revision).at(bank).address(), 32],
                site,
            )?;
        }
    }
    item_call_void(
        context,
        caller,
        ids::ENTITY_UPDATE_LEVEL_ITEM_VISIBILITY,
        &[ram::g_dropped_present_table(revision).address(), 32],
        0x56,
    )?;
    for index in 0..2 {
        if ram::g_player_lives(revision).at(index).read(context) >= 0
            && inventory.at(index).read(context) == 0
        {
            for bank in 0..2 {
                // c_item_updates.s:959-968 reads bank level before player level.
                if !bank_matches_player(context, revision, bank, index) {
                    continue;
                }
                let items = ram::g_object_table(revision).at(bank);
                let pickup = item_retained_call(
                    context,
                    caller,
                    ids::PLAYER_COLLECT_NEARBY_ITEMS,
                    &[index as u32, items.address(), 28, 1],
                    0xc4,
                )?;
                // The pickup's four argument slots remain below the hazard's
                // two slots until LEA 24(SP),SP after both calls (s:979-991).
                let hurt = item_call_void(
                    context,
                    caller,
                    ids::PLAYER_CHECK_LEVEL_ITEM_HAZARDS,
                    &[index as u32, items.at(28).address()],
                    0xde,
                );
                if hurt
                    .as_ref()
                    .is_err_and(|error| error.preserves_call_frame())
                {
                    return hurt.map(|_| None);
                }
                pickup.release(context)?;
                hurt?;
            }
            item_call_void(
                context,
                caller,
                ids::PLAYER_COLLECT_NEARBY_ITEMS,
                &[
                    index as u32,
                    ram::g_dropped_present_table(revision).address(),
                    32,
                    0,
                ],
                0x100,
            )?;
        }
    }
    item_call_void(context, caller, ids::REDUCE_PRESENT_TIMER, &[], 0x112)?;
    item_call_void(context, caller, ids::PROCESS_PRESENTS, &[], 0x118)?;
    let level = ram::g_player1_map_level(revision).read(context);
    super::expiry::compare(context, u32::from(level as u8), 26, 8);
    if level == 26 {
        item_call_void(
            context,
            caller,
            ids::FUNKATRON_UPDATE_SCENE_ENTITIES,
            &[],
            0x128,
        )?;
    }
    Ok(None)
}

/// Compiled c_item_updates.s:802-830: SUBQ.W's store is reloaded for the
/// unsigned 300 comparison; the final memory BTST reads only the low byte.
fn timer_show_nonzero(bus: &mut (impl tje_mem::Bus + ?Sized), timer: Address<u16>) -> bool {
    timer.write(bus, timer.read(bus).wrapping_sub(1));
    if timer.read(bus) > 300 {
        true
    } else if timer.read(bus) > 100 {
        timer.read(bus) & 12 != 0
    } else {
        timer.byte_offset(1).cast::<u8>().read(bus) & 2 != 0
    }
}

fn timer_player_eligible(
    bus: &(impl tje_mem::Bus + ?Sized),
    revision: tje_mem::Revision,
    index: i32,
    player: Address<tje_mem::ObjectState>,
) -> bool {
    ram::g_player_powerups(revision).at(index).read(bus) >= 0
        && ram::g_player_lives(revision).at(index).read(bus) >= 0
        && player.sprite_state().read(bus) != 5
        && player.sprite_state().read(bus) != 6
        && ram::g_player_inventory_open(revision).at(index).read(bus) == 0
}

fn bank_matches_player(
    bus: &(impl tje_mem::Bus + ?Sized),
    revision: tje_mem::Revision,
    bank: i32,
    player: i32,
) -> bool {
    ram::g_item_bank_users(revision).at(bank).read(bus) != 0
        && ram::g_item_bank_levels(revision).at(bank).read(bus)
            == ram::g_player_info(revision).at(player).level().read(bus)
}

/// c_item_updates.s:311-325: the three sprite comparisons are separate loads.
fn collection_player_eligible(
    context: &mut dyn Context,
    revision: tje_mem::Revision,
    index: i32,
    player: Address<tje_mem::ObjectState>,
) -> bool {
    let lives = ram::g_player_lives(revision).at(index).read(context);
    super::expiry::logical(context, u32::from(lives as u8), 8);
    if lives < 0 {
        return false;
    }
    let height = player.height().read(context);
    super::expiry::compare(context, u32::from(height as u16), 4, 16);
    if height >= 4 {
        return false;
    }
    for excluded in [3, 7, 77] {
        let state = player.sprite_state().read(context);
        super::expiry::compare(context, u32::from(state as u8), excluded, 8);
        if u32::from(state as u8) == excluded {
            return false;
        }
    }
    let blocked = player.animation_group_mask().read(context) & 0x0004_1800;
    super::expiry::logical(context, blocked, 32);
    blocked == 0
}

fn leave_item_frame(
    context: &mut dyn Context,
    frame: LocalFrame,
    result: Result<Option<u32>>,
) -> Result<Option<u32>> {
    let result = result.map_err(|error| context.protect_timing_failure(error));
    if result
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        return result;
    }
    frame.leave(context)?;
    result
}

fn item_retained_call(
    context: &mut dyn Context,
    caller: FunctionId,
    callee: FunctionId,
    arguments: &[u32],
    return_offset: u32,
) -> Result<OutgoingArguments> {
    let outgoing = OutgoingArguments::push(context, arguments)?;
    let dispatcher = context.dispatcher();
    let site = CallSite::relative(caller, context.revision(), return_offset);
    let result = if callee.metadata().abi.returns_value {
        outgoing.call(context, dispatcher, callee, site).map(|_| ())
    } else {
        outgoing.call_void(context, dispatcher, callee, site)
    };
    if let Err(error) = result {
        if !error.preserves_call_frame() {
            outgoing.release(context)?;
        }
        return Err(error);
    }
    Ok(outgoing)
}

fn item_call(
    context: &mut dyn Context,
    caller: FunctionId,
    callee: FunctionId,
    arguments: &[u32],
    return_offset: u32,
) -> Result<u32> {
    let site = CallSite::relative(caller, context.revision(), return_offset);
    call_at(context, callee, arguments, site)
}

fn item_call_void(
    context: &mut dyn Context,
    caller: FunctionId,
    callee: FunctionId,
    arguments: &[u32],
    return_offset: u32,
) -> Result<()> {
    let site = CallSite::relative(caller, context.revision(), return_offset);
    call_at_void(context, callee, arguments, site)
}

#[cfg(test)]
#[path = "item_replay_tests.rs"]
mod replay_tests;
