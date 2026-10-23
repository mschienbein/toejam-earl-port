use crate::{Context, Result, call, function_at, ids};
use tje_mem::{Address, EarthlingState, ram, rom};

fn callback(context: &mut dyn Context, address: u32, arguments: &[u32]) -> Result<()> {
    let id = function_at(context.revision(), address).ok_or_else(|| {
        crate::Error::Platform(format!("unknown Earthling callback {address:08x}"))
    })?;
    call(context, id, arguments)?;
    Ok(())
}

/// G/src/earthlings/spawning_and_banks.c:426 (`LoadInEarthlings`).
/// Atomic: alternating records, wrapped short camera deltas, object allocation/free.
pub fn load_in_earthlings(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let records = Address::<EarthlingState>::new(arguments[0]);
    let mut index = ram::g_earthling_update_parity(revision).read(context);
    while index < 20 {
        let earthling = records.offset(i32::from(index));
        if earthling.object().read(context) != 0 {
            let object =
                ram::g_player_info(revision).at(i32::from(earthling.object().read(context)));
            earthling.x().write(context, object.x().read(context));
            earthling.y().write(context, object.y().read(context));
        }
        let tx = earthling
            .x()
            .read(context)
            .wrapping_sub(ram::g_camera1_pending_x(revision).read(context));
        let ty = earthling
            .y()
            .read(context)
            .wrapping_sub(ram::g_camera1_pending_y(revision).read(context));
        let ex = earthling
            .x()
            .read(context)
            .wrapping_sub(ram::g_camera2_pending_x(revision).read(context));
        let ey = earthling
            .y()
            .read(context)
            .wrapping_sub(ram::g_camera2_pending_y(revision).read(context));
        let visibility = earthling.health_visibility().at(1);
        if earthling.kind().read(context) != -1 {
            let primary = earthling.level().read(context)
                == ram::g_player1_map_level(revision).read(context)
                && i32::from(tx) + 160 > 0
                && tx < 480
                && i32::from(ty) + 112 > 0
                && ty < 336;
            visibility.write(
                context,
                if primary {
                    visibility.read(context) | 2
                } else {
                    visibility.read(context) & !2
                },
            );
            let secondary = earthling.level().read(context)
                == ram::g_player2_map_level(revision).read(context)
                && ram::g_split_screen_enabled(revision).read(context) != 0
                && i32::from(ex) + 160 > 0
                && ex < 480
                && i32::from(ey) + 112 > 0
                && ey < 336;
            visibility.write(
                context,
                if secondary {
                    visibility.read(context) | 4
                } else {
                    visibility.read(context) & !4
                },
            );
        } else {
            visibility.write(context, 0)
        }
        if visibility.read(context) & 6 != 0 {
            if earthling.object().read(context) == 0 {
                call(
                    context,
                    ids::EARTHLING_ALLOCATE_ENTITIES,
                    &[earthling.address()],
                )?;
            }
        } else {
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
        }
        index = index.wrapping_add(2);
    }
    Ok(None)
}

/// G/src/earthlings/spawning_and_banks.c:579 (`EarthlingDispatchAbilities`).
/// TimeAware: ROM ability callbacks include GenericEnemyThink's speech wait.
pub fn earthling_dispatch_abilities(
    context: &mut dyn Context,
    _arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    for bank in 0..2 {
        if ram::g_earthling_bank_users(revision).at(bank).read(context) == 0 {
            continue;
        }
        let mut index = ram::g_earthling_update_parity(revision).read(context);
        while index < 20 {
            let earthling = ram::g_earthling_banks(revision)
                .at(bank)
                .at(i32::from(index));
            if earthling.kind().read(context) != -1
                && earthling.object().read(context) != 0
                && earthling.state().read(context) != 99
                && call(
                    context,
                    ids::EARTHLING_IS_MAIL_ORDER_MAILBOX,
                    &[earthling.address()],
                )? == 0
            {
                let definition = rom::earthling_definitions(revision)
                    .at(i32::from(earthling.kind().read(context)));
                if definition.ability().read(context).address() != 0
                    && call(
                        context,
                        ids::EARTHLING_IS_UPDATE_BLOCKED,
                        &[earthling.address()],
                    )? == 0
                {
                    let ability = definition.ability().read(context).address();
                    callback(context, ability, &[earthling.address()])?;
                }
            }
            index = index.wrapping_add(2);
        }
    }
    Ok(None)
}

/// G/src/earthlings/spawning_and_banks.c:602 (`EarthlingDispatchPlayerContacts`).
/// TimeAware: ROM contact callbacks include speech, inventory and ship scenes.
/// Keep the sampled first-player order and the cached object/definition pointers.
pub fn earthling_dispatch_player_contacts(
    context: &mut dyn Context,
    _arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    for bank in 0..2 {
        if ram::g_earthling_bank_users(revision).at(bank).read(context) == 0 {
            continue;
        }
        let mut slot = ram::g_earthling_update_parity(revision).read(context);
        while slot < 20 {
            let earthling = ram::g_earthling_banks(revision)
                .at(bank)
                .at(i32::from(slot));
            slot = slot.wrapping_add(2);
            if earthling.kind().read(context) == -1
                || earthling.object().read(context) == 0
                || earthling.state().read(context) == 99
            {
                continue;
            }
            let definition =
                rom::earthling_definitions(revision).at(i32::from(earthling.kind().read(context)));
            let object =
                ram::g_player_info(revision).at(i32::from(earthling.object().read(context)));
            let mut number = i32::from(super::decisions::next_cached_random(context) & 1);
            for _ in 0..2 {
                let player = ram::g_player_info(revision).at(number);
                if ram::g_player_inventory_open(revision)
                    .at(number)
                    .read(context)
                    == 0
                    && ram::g_player_elevator_flags(revision)
                        .at(number)
                        .read(context)
                        == 0
                    && player.behavior_flags().read(context) & 0x10 == 0
                {
                    let x = u32::from(definition.contact_distance_x().read(context));
                    let y = u32::from(definition.contact_distance_y().read(context));
                    if call(
                        context,
                        ids::ENTITY_IS_WITHIN_RANGE,
                        &[object.address(), player.address(), x, y],
                    )? != 0
                    {
                        if definition.behavior_flags().read(context) & 1 != 0
                            && earthling.state().read(context) == 2
                        {
                            call(context, ids::SET_EARTHLING_STATE, &[earthling.address(), 0])?;
                            earthling.timer().write(context, 0);
                        }
                        let touch = definition.touch().read(context).address();
                        callback(context, touch, &[number as u32, earthling.address()])?;
                    }
                }
                number ^= 1;
            }
            if earthling.kind().read(context) == 7 {
                call(
                    context,
                    ids::EARTHLING_CHECK_DECOY_ATTACK_RANGE,
                    &[earthling.address(), definition.address()],
                )?;
            }
        }
    }
    Ok(None)
}

/// G/src/earthlings/spawning_and_banks.c:871 (`ProcessEarthlings`).
/// TimeAware: callbacks may wait. Preserve bank order, all passes and final parity.
pub fn process_earthlings(context: &mut dyn Context, _arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    if ram::g_player_inventory_open(revision).at(0).read(context) != 0
        && ram::g_player2_inventory_open(revision).read(context) != 0
    {
        return Ok(None);
    }
    if ram::g_earthling_bank_users(revision).at(0).read(context) != 0 {
        call(
            context,
            ids::LOAD_IN_EARTHLINGS,
            &[ram::g_earthling_banks(revision).at(0).address()],
        )?;
    }
    if ram::g_earthling_bank2_users(revision).read(context) != 0 {
        call(
            context,
            ids::LOAD_IN_EARTHLINGS,
            &[ram::g_earthling_bank2_records(revision).address()],
        )?;
    }
    call(context, ids::EARTHLING_DISPATCH_ABILITIES, &[])?;
    call(context, ids::EARTHLING_DISPATCH_PLAYER_CONTACTS, &[])?;
    call(context, ids::EARTHLING_APPLY_NEARBY_SLEEP, &[])?;
    call(context, ids::SHIP_UPDATE_PENDING_PIECE_INDICATOR, &[])?;
    call(
        context,
        ids::RELEASE_FINISHED_MOLE_EFFECTS_AND_STOLEN_PRESENTS,
        &[],
    )?;
    let parity = ram::g_earthling_update_parity(revision);
    parity.write(context, parity.read(context) ^ 1);
    Ok(None)
}
