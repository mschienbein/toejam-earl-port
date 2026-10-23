use crate::{Context, Result, call, ids};
use tje_mem::{Address, EarthlingState, ram, rom};

/// G/src/earthlings/spawning_and_banks.c:290 (`SetEarthlingState`).
/// Atomic: install ROM animation fields before scanning sequence controls/freeing.
pub fn set_earthling_state(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let earthling = Address::<EarthlingState>::new(arguments[0]);
    let state = arguments[1] as i16;
    earthling.state().write(context, state as i8);
    if earthling.object().read(context) == 0 {
        return Ok(None);
    }
    let data = rom::earthling_definitions(revision).at(i32::from(earthling.kind().read(context)));
    let animation = data
        .states()
        .read(context)
        .target()
        .offset(i32::from(state));
    let object = ram::g_player_info(revision).at(i32::from(earthling.object().read(context)));
    object
        .sequence()
        .write(context, animation.sequence().read(context));
    object
        .frames()
        .write(context, animation.frames().read(context));
    object
        .frame_phase()
        .write(context, animation.period().read(context));
    object.frame_duration().write(context, 1);
    object.sprite_state().write(context, state as i8);
    let frame = object.frame().read(context);
    if frame >= 0 {
        object.frame().write(context, frame.wrapping_sub(1))
    }
    let mut sequence_index = 0_i16;
    while sequence_index <= i16::from(object.frame().read(context)) {
        if object
            .sequence()
            .read(context)
            .target()
            .offset(i32::from(sequence_index))
            .read(context) as u8
            & 0xf0
            == 0xf0
        {
            object.frame().write(context, -1);
        }
        sequence_index = sequence_index.wrapping_add(1);
    }
    if state != 2 && earthling.second_object().read(context) != 0 {
        let second = earthling.second_object().read(context) as u32;
        call(context, ids::FREE_ENTITY, &[second])?;
        earthling.second_object().write(context, 0);
    }
    Ok(None)
}

fn release_objects(context: &mut dyn Context, earthling: Address<EarthlingState>) -> Result<()> {
    if earthling.object().read(context) != 0 {
        let object = earthling.object().read(context) as u32;
        call(context, ids::FREE_ENTITY, &[object])?;
        earthling.object().write(context, 0);
    }
    if earthling.second_object().read(context) != 0 {
        let object = earthling.second_object().read(context) as u32;
        call(context, ids::FREE_ENTITY, &[object])?;
        earthling.second_object().write(context, 0);
    }
    Ok(())
}

/// G/src/earthlings/spawning_and_banks.c:345 (`EarthlingRestoreSavedPosition`).
/// Atomic: both releases precede coordinate reads/stores (including aliasing).
pub fn earthling_restore_saved_position(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let earthling = Address::<EarthlingState>::new(arguments[0]);
    release_objects(context, earthling)?;
    earthling
        .x()
        .write(context, earthling.saved_x_or_status().read(context));
    earthling
        .y()
        .write(context, earthling.saved_y().read(context));
    Ok(None)
}

/// G/src/earthlings/spawning_and_banks.c:751 (`EarthlingRemoveFromLevel`).
/// Atomic: preserve kind-first removal and every matching bank's high-bit slot mask.
pub fn earthling_remove_from_level(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let earthling = Address::<EarthlingState>::new(arguments[0]);
    release_objects(context, earthling)?;
    earthling.kind().write(context, -1);
    for bank in 0..2 {
        if ram::g_earthling_bank_users(revision).at(bank).read(context) != 0
            && ram::g_earthling_bank_levels(revision)
                .at(bank)
                .read(context)
                == earthling.level().read(context)
        {
            for slot in 0..20 {
                if earthling == ram::g_earthling_banks(revision).at(bank).at(slot) {
                    let level = i32::from(
                        ram::g_earthling_bank_levels(revision)
                            .at(bank)
                            .read(context),
                    );
                    let mask = ram::g_earthling_spawn_failure_masks(revision).at(level);
                    mask.write(context, mask.read(context) | (0x8000_0000_u32 >> slot));
                }
            }
        }
    }
    Ok(None)
}

/// G/src/earthlings/bank_expiry.c:9 (`EarthlingReleaseLevelBank`).
/// Atomic: decrement each loaded matching bank, releasing only on final reference.
pub fn earthling_release_level_bank(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let level = arguments[0] as i16;
    for bank in 0..2 {
        let users = ram::g_earthling_bank_users(revision).at(bank);
        let bank_level = ram::g_earthling_bank_levels(revision).at(bank);
        if users.read(context) != 0 && i16::from(bank_level.read(context)) == level {
            users.write(context, users.read(context).wrapping_sub(1));
            if users.read(context) == 0 {
                bank_level.write(context, -1);
                for slot in 0..20 {
                    let earthling = ram::g_earthling_banks(revision).at(bank).at(slot);
                    let object = earthling.object().read(context);
                    earthling.kind().write(context, -1);
                    if object != 0 {
                        call(context, ids::FREE_ENTITY, &[object as u32])?;
                        earthling.object().write(context, 0);
                    }
                    let second = earthling.second_object().read(context);
                    if second != 0 {
                        call(context, ids::FREE_ENTITY, &[second as u32])?;
                        earthling.second_object().write(context, 0);
                    }
                }
            }
        }
    }
    Ok(None)
}

/// G/src/earthlings/spawning_and_banks.c:644 (`EarthlingApplyNearbySleep`).
/// Atomic: alternating records, RAM position eligibility, ROM sleep descriptors.
pub fn earthling_apply_nearby_sleep(
    context: &mut dyn Context,
    _arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    for source in 0..2 {
        let handle = ram::g_player_boom_box_entities(revision)
            .at(source)
            .read(context);
        if handle == 0 {
            continue;
        }
        let source_object = ram::g_player_info(revision).at(i32::from(handle));
        let bank = if ram::g_earthling_bank_users(revision).at(0).read(context) != 0
            && ram::g_earthling_bank_levels(revision).at(0).read(context)
                == source_object.level().read(context)
        {
            0
        } else {
            1
        };
        let mut slot = i32::from(ram::g_earthling_update_parity(revision).read(context));
        while slot < 20 {
            let earthling = ram::g_earthling_banks(revision).at(bank).at(slot);
            slot += 2;
            let kind = earthling.kind().read(context);
            let state = earthling.state().read(context);
            if kind == -1
                || state == 99
                || earthling.object().read(context) == 0
                || [0, 8, 14, 20, 23, 24].contains(&kind)
                || call(
                    context,
                    ids::EARTHLING_IS_MAIL_ORDER_MAILBOX,
                    &[earthling.address()],
                )? != 0
            {
                continue;
            }
            if ((kind == 7 || kind == 18) && state == 3)
                || (kind == 11 && state < 2)
                || ((kind == 17 || kind == 10) && (i32::from(state) & 0xfc) != 4)
            {
                continue;
            }
            if (i32::from(earthling.x().read(context)) - i32::from(source_object.x().read(context)))
                .wrapping_abs()
                >= 320
                || (i32::from(earthling.y().read(context))
                    - i32::from(source_object.y().read(context)))
                .wrapping_abs()
                    >= 224
            {
                continue;
            }
            let object =
                ram::g_player_info(revision).at(i32::from(earthling.object().read(context)));
            object.velocity_x().write(context, 0);
            object.velocity_y().write(context, 0);
            let animation = rom::sleep_animation_descriptors(revision).at(i32::from(kind));
            object
                .sequence()
                .write(context, animation.sequence().read(context));
            object
                .frames()
                .write(context, animation.frames().read(context));
            object.frame_phase().write(context, 6);
            object.frame_duration().write(context, 1);
            object.frame().write(context, -1);
            object.sprite_state().write(context, 99);
            earthling.state().write(context, 99);
            if earthling.second_object().read(context) != 0 {
                let second = earthling.second_object().read(context) as u32;
                call(context, ids::FREE_ENTITY, &[second])?;
                earthling.second_object().write(context, 0);
            }
        }
    }
    Ok(None)
}

/// G/src/earthlings/spawning_and_banks.c:322 (`EarthlingRelocateRecord`).
/// Atomic: release objects, retry the original spawn/elevator predicates.
pub fn earthling_relocate_record(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let earthling = Address::<EarthlingState>::new(arguments[0]);
    release_objects(context, earthling)?;
    let other_level = u32::from(
        ram::g_earthling_bank_users(revision).at(0).read(context) == 0
            || ram::g_earthling_bank_levels(revision).at(0).read(context)
                != earthling.level().read(context),
    );
    loop {
        let kind = earthling.kind().read(context) as u32;
        let level = earthling.level().read(context) as u32;
        call(
            context,
            ids::FIND_SPAWN_LOCATION_FOR_EARTHLING,
            &[
                kind,
                other_level,
                earthling.x().address(),
                earthling.y().address(),
                level,
            ],
        )?;
        let x = earthling.x().read(context) as u32;
        let y = earthling.y().read(context) as u32;
        let level = earthling.level().read(context) as u32;
        if call(context, ids::ELEVATOR_CONTAINS_POSITION, &[x, y, level])? == 0 {
            break;
        }
    }
    Ok(None)
}

/// G/src/earthlings/spawning_and_banks.c:720 (`EarthlingResetStateTwoByDrawMask`).
/// Atomic: preserve the signed-char mask's promotion against unsigned draw flags.
pub fn earthling_reset_state_two_by_draw_mask(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let mask = arguments[0] as i8 as i32 as u32;
    let mut changed = 0;
    for bank in 0..2 {
        if ram::g_earthling_bank_users(revision).at(bank).read(context) == 0 {
            continue;
        }
        for slot in 0..20 {
            let earthling = ram::g_earthling_banks(revision).at(bank).at(slot);
            if earthling.kind().read(context) != -1
                && earthling.object().read(context) != 0
                && earthling.state().read(context) == 2
            {
                let definition = rom::earthling_definitions(revision)
                    .at(i32::from(earthling.kind().read(context)));
                let object =
                    ram::g_player_info(revision).at(i32::from(earthling.object().read(context)));
                if definition.behavior_flags().read(context) & 1 != 0
                    && u32::from(object.draw_flags().read(context)) & mask != 0
                {
                    call(context, ids::SET_EARTHLING_STATE, &[earthling.address(), 0])?;
                    earthling.timer().write(context, 0);
                    changed = 1;
                }
            }
        }
    }
    Ok(Some(changed))
}

/// G/src/earthlings/spawning_and_banks.c:702 (`EarthlingWakeLevel`).
/// Atomic: bank one remains the fallback even without a matching loaded bank.
pub fn earthling_wake_level(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let level = arguments[0] as i16;
    let bank = if ram::g_earthling_bank_users(revision).at(0).read(context) != 0
        && i16::from(ram::g_earthling_bank_levels(revision).at(0).read(context)) == level
    {
        0
    } else {
        1
    };
    for slot in 0..20 {
        let earthling = ram::g_earthling_banks(revision).at(bank).at(slot);
        if earthling.kind().read(context) != -1 && earthling.state().read(context) == 99 {
            call(context, ids::SET_EARTHLING_STATE, &[earthling.address(), 0])?;
        }
    }
    Ok(None)
}

/// G/src/earthlings/spawning_and_banks.c:781 (`DamageEarthling`).
/// Atomic: health decrement precedes friendly protection; removal precedes effect.
pub fn damage_earthling(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let earthling = Address::<EarthlingState>::new(arguments[0]);
    if earthling.address() == 0 || earthling.kind().read(context) <= 0 {
        return Ok(None);
    }
    let mut killed = false;
    if call(
        context,
        ids::EARTHLING_IS_MAIL_ORDER_MAILBOX,
        &[earthling.address()],
    )? == 0
        && earthling.kind().read(context) != 20
        && earthling.kind().read(context) < 23
    {
        let health = earthling.health_visibility().at(0);
        let (stored_health, value) = super::predecrement_byte(health.read(context));
        health.write(context, stored_health);
        killed = value <= 0;
    }
    if killed {
        if [17, 10, 11].contains(&earthling.kind().read(context))
            && call(context, ids::NICE_EARTHLING_CHECK, &[earthling.address()])? != 0
        {
            earthling.health_visibility().at(0).write(context, 1);
            return Ok(None);
        }
        call(
            context,
            ids::EARTHLING_REMOVE_FROM_LEVEL,
            &[earthling.address()],
        )?;
        let x = earthling.x().read(context) as u32;
        let y = earthling.y().read(context) as u32;
        let level = earthling.level().read(context) as u32;
        call(context, ids::ENTITY_SPAWN_TIMED_EFFECT, &[x, y, level])?;
    } else if earthling.second_object().read(context) != 0 {
        call(context, ids::SET_EARTHLING_STATE, &[earthling.address(), 0])?;
        earthling.timer().write(context, 0);
    }
    Ok(None)
}

/// G/src/earthlings/spawning_and_banks.c:828 (`DoVikingAbility`).
/// Atomic: check effect capacity before damaging the last hit point.
pub fn do_viking_ability(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let mask = arguments[0] as i8 as i32 as u32;
    for bank in 0..2 {
        if ram::g_earthling_bank_users(revision).at(bank).read(context) == 0 {
            continue;
        }
        for slot in 0..20 {
            let earthling = ram::g_earthling_banks(revision).at(bank).at(slot);
            if earthling.kind().read(context) != -1
                && earthling.kind().read(context) != 17
                && earthling.object().read(context) != 0
            {
                let object =
                    ram::g_player_info(revision).at(i32::from(earthling.object().read(context)));
                if u32::from(object.draw_flags().read(context)) & mask != 0
                    && (earthling.health_visibility().at(0).read(context) > 1
                        || call(context, ids::ENTITY_HAS_FREE_TIMED_EFFECT_SLOT, &[])? != 0)
                {
                    call(context, ids::DAMAGE_EARTHLING, &[earthling.address()])?;
                }
            }
        }
    }
    Ok(None)
}

/// G/src/earthlings/spawning_and_banks.c:804 (`EarthlingDamageOnHazardTerrain`).
/// Atomic: retain the full int terrain result view in this translation unit.
pub fn earthling_damage_on_hazard_terrain(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let player = arguments[0] as i16 as u32;
    let level = arguments[1] as i16;
    let bank = if ram::g_earthling_bank_users(revision).at(0).read(context) != 0
        && i16::from(ram::g_earthling_bank_levels(revision).at(0).read(context)) == level
    {
        0
    } else {
        1
    };
    for slot in 0..20 {
        let earthling = ram::g_earthling_banks(revision).at(bank).at(slot);
        let kind = earthling.kind().read(context);
        if kind > 0
            && earthling.object().read(context) != 0
            && ![6, 22, 7, 20, 8, 19].contains(&kind)
        {
            let x = earthling.x().read(context) as u32;
            let y = earthling.y().read(context) as u32;
            if call(
                context,
                ids::GET_MAP_TERRAIN_AT_WORLD_POSITION,
                &[player, x, y],
            )? == 0x90
            {
                earthling.health_visibility().at(0).write(context, 0);
                call(context, ids::DAMAGE_EARTHLING, &[earthling.address()])?;
            }
        }
    }
    Ok(None)
}

/// G/src/earthlings/spawning_and_banks.c:273 (`SetupInitialEarthlings`).
/// Atomic: clear both record banks, retain for living players, then assign pieces.
pub fn setup_initial_earthlings(
    context: &mut dyn Context,
    _arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    for slot in 0..20 {
        ram::g_earthling_banks(revision)
            .at(0)
            .at(slot)
            .kind()
            .write(context, -1);
        ram::g_earthling_bank2_records(revision)
            .at(slot)
            .kind()
            .write(context, -1);
    }
    if ram::g_player_lives(revision).at(0).read(context) >= 0 {
        call(context, ids::EARTHLING_RETAIN_LEVEL_BANK, &[1])?;
    }
    if ram::g_player2_lives(revision).read(context) >= 0 {
        call(context, ids::EARTHLING_RETAIN_LEVEL_BANK, &[1])?;
    }
    call(context, ids::ASSIGN_SHIP_PIECE_LEVELS, &[])?;
    Ok(None)
}

/// G/src/earthlings/spawning_and_banks.c:361 (`EarthlingAllocateEntities`).
/// Atomic: object allocation, ROM definition/animation reads and RAM initialization.
pub fn earthling_allocate_entities(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let earthling = Address::<EarthlingState>::new(arguments[0]);
    let definition =
        rom::earthling_definitions(revision).at(i32::from(earthling.kind().read(context)));
    let allocated = call(context, ids::ALLOCATE_ENTITY, &[])? as i8;
    earthling.object().write(context, allocated);
    if allocated != 0 {
        let object = ram::g_player_info(revision).at(i32::from(earthling.object().read(context)));
        object
            .earthling()
            .write(context, tje_mem::Ptr::new(earthling.address()));
        object
            .process()
            .write(context, definition.update().read(context));
        object
            .animation_motion_axes()
            .write(context, definition.movement_mode().read(context));
        object.x().write(context, earthling.x().read(context));
        object.y().write(context, earthling.y().read(context));
        if earthling.kind().read(context) == 7 {
            object.height().write(context, 40)
        }
        object
            .level()
            .write(context, earthling.level().read(context));
        object
            .width()
            .write(context, definition.width().read(context));
        object
            .height_pixels()
            .write(context, definition.height_pixels().read(context));
        object
            .sprite_count()
            .write(context, definition.sprite_count().read(context) as u8);
        object
            .tile_count()
            .write(context, definition.tile_count().read(context) as u8);
        if call(
            context,
            ids::EARTHLING_IS_MAIL_ORDER_MAILBOX,
            &[earthling.address()],
        )? != 0
        {
            object.sprite_count().write(context, 1);
            object.tile_count().write(context, 12);
            object.animation_motion_axes().write(context, 0);
            object
                .draw_flags()
                .write(context, object.draw_flags().read(context) | 8);
        }
        object.tile().write(context, 0x8000);
        if earthling.kind().read(context) != 14 {
            object
                .behavior_flags()
                .write(context, object.behavior_flags().read(context) | 1)
        }
        let mut companion = false;
        if definition.behavior_flags().read(context) & 1 != 0
            && super::decisions::next_cached_random(context) & 1 != 0
        {
            let second = call(context, ids::ALLOCATE_ENTITY, &[])? as i8;
            earthling.second_object().write(context, second);
            companion = second != 0;
        }
        if companion {
            call(context, ids::SET_EARTHLING_STATE, &[earthling.address(), 2])?;
            let object =
                ram::g_player_info(revision).at(i32::from(earthling.second_object().read(context)));
            object.x().write(context, earthling.x().read(context));
            object
                .y()
                .write(context, earthling.y().read(context).wrapping_add(1));
            let index = i32::from(earthling.kind().read(context)) * 2;
            object.x().write(
                context,
                object.x().read(context).wrapping_add(i16::from(
                    rom::earthling_coordinate_pairs(revision)
                        .at(index)
                        .read(context),
                )),
            );
            object.height().write(
                context,
                i16::from(
                    rom::earthling_coordinate_pairs(revision)
                        .at(index + 1)
                        .read(context),
                ),
            );
            object
                .level()
                .write(context, earthling.level().read(context));
            object.width().write(context, 32);
            object.height_pixels().write(context, 32);
            object.sprite_count().write(context, 1);
            object.tile_count().write(context, 9);
            object.tile().write(context, 0x8000);
            let animation = rom::earthling_companion_animations(revision).at(0);
            object
                .sequence()
                .write(context, animation.sequence().read(context));
            object
                .frames()
                .write(context, animation.frames().read(context));
            object
                .frame_phase()
                .write(context, animation.period().read(context));
            object.frame_duration().write(context, 1);
        } else if earthling.kind().read(context) == 8 {
            object.behavior_flags().write(context, 0x40);
            call(context, ids::SET_EARTHLING_STATE, &[earthling.address(), 3])?;
        } else {
            call(context, ids::SET_EARTHLING_STATE, &[earthling.address(), 0])?;
        }
        earthling.timer().write(context, 0);
    } else if earthling.kind().read(context) == 0 {
        call(context, ids::ENTITY_RELEASE_ONE_BY_PRIORITY, &[])?;
    } else {
        call(
            context,
            ids::EARTHLING_RELOCATE_RECORD,
            &[earthling.address()],
        )?;
    }
    Ok(None)
}
