use crate::{Context, FunctionId, Result, call_void, function_at, ids};
use tje_mem::{Address, EarthlingState, ram, rom};

fn callback(
    context: &mut dyn Context,
    caller: FunctionId,
    address: u32,
    arguments: &[u32],
    return_rev02: u32,
) -> Result<()> {
    let id = function_at(context.revision(), address).ok_or_else(|| {
        crate::Error::Platform(format!("unknown Earthling callback {address:08x}"))
    })?;
    context.set_address_register(0, address);
    super::framed::invoke_void(context, caller, id, arguments, return_rev02)?;
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
                call_void(
                    context,
                    ids::EARTHLING_ALLOCATE_ENTITIES,
                    &[earthling.address()],
                )?;
            }
        } else {
            if earthling.object().read(context) != 0 {
                let object = earthling.object().read(context) as u32;
                call_void(context, ids::FREE_ENTITY, &[object])?;
                earthling.object().write(context, 0);
            }
            if earthling.second_object().read(context) != 0 {
                let object = earthling.second_object().read(context) as u32;
                call_void(context, ids::FREE_ENTITY, &[object])?;
                earthling.second_object().write(context, 0);
            }
        }
        index = index.wrapping_add(2);
    }
    Ok(None)
}

/// G/src/earthlings/spawning_and_banks.c:579 (`EarthlingDispatchAbilities`).
/// TimeAware: ROM ability callbacks include GenericEnemyThink's speech wait.
pub fn earthling_dispatch_abilities(context: &mut dyn Context) -> Result<Option<u32>> {
    let frame = crate::call_frame::LocalFrame::enter(
        context,
        crate::call_frame::FrameLayout::movem(0, 0x1c, 0x0c),
    )?;
    let result = dispatch_abilities(context);
    super::framed::finish(context, frame, result)
}

fn dispatch_abilities(context: &mut dyn Context) -> Result<Option<u32>> {
    let revision = context.revision();
    let caller = ids::EARTHLING_DISPATCH_ABILITIES;
    for bank in 0..2 {
        if ram::g_earthling_bank_users(revision).at(bank).read(context) == 0 {
            continue;
        }
        let mut index = ram::g_earthling_update_parity(revision).read(context);
        while index < 20 {
            let earthling =
                super::word_record(ram::g_earthling_banks(revision).at(bank).at(0), index);
            if earthling.kind().read(context) != -1
                && earthling.object().read(context) != 0
                && earthling.state().read(context) != 99
                && super::framed::invoke(
                    context,
                    caller,
                    ids::EARTHLING_IS_MAIL_ORDER_MAILBOX,
                    &[earthling.address()],
                    0x01889e,
                )? == 0
            {
                let definition = rom::earthling_definitions(revision)
                    .at(i32::from(earthling.kind().read(context)));
                if definition.ability().read(context).address() != 0
                    && super::framed::invoke(
                        context,
                        caller,
                        ids::EARTHLING_IS_UPDATE_BLOCKED,
                        &[earthling.address()],
                        0x0188c4,
                    )? == 0
                {
                    let ability = definition.ability().read(context).address();
                    callback(context, caller, ability, &[earthling.address()], 0x0188d2)?;
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
pub fn earthling_dispatch_player_contacts(context: &mut dyn Context) -> Result<Option<u32>> {
    let frame = crate::call_frame::LocalFrame::enter(
        context,
        crate::call_frame::FrameLayout::movem(0, 0x7c, 0x1c),
    )?;
    let result = dispatch_player_contacts(context);
    super::framed::finish(context, frame, result)
}

fn dispatch_player_contacts(context: &mut dyn Context) -> Result<Option<u32>> {
    let revision = context.revision();
    let caller = ids::EARTHLING_DISPATCH_PLAYER_CONTACTS;
    for bank in 0..2 {
        if ram::g_earthling_bank_users(revision).at(bank).read(context) == 0 {
            continue;
        }
        let mut slot = ram::g_earthling_update_parity(revision).read(context);
        while slot < 20 {
            let earthling =
                super::word_record(ram::g_earthling_banks(revision).at(bank).at(0), slot);
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
            let index = super::pending_access::next_cached_index(context);
            let mut number = i32::from(
                ram::g_cached_random_values(revision)
                    .at(i32::from(index))
                    .read(context)
                    & 1,
            );
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
                    // c_spawning_and_banks.s:1949-1954 pushes right-to-left.
                    let y = u32::from(definition.contact_distance_y().read(context));
                    let x = u32::from(definition.contact_distance_x().read(context));
                    if super::framed::invoke(
                        context,
                        caller,
                        ids::ENTITY_IS_WITHIN_RANGE,
                        &[object.address(), player.address(), x, y],
                        0x0189fc,
                    )? != 0
                    {
                        if definition.behavior_flags().read(context) & 1 != 0
                            && earthling.state().read(context) == 2
                        {
                            super::framed::invoke_void(
                                context,
                                caller,
                                ids::SET_EARTHLING_STATE,
                                &[earthling.address(), 0],
                                0x018a1c,
                            )?;
                            context.discard_u16(earthling.timer().address())?;
                            earthling.timer().write(context, 0);
                        }
                        let touch = definition.touch().read(context).address();
                        callback(
                            context,
                            caller,
                            touch,
                            &[number as u32, earthling.address()],
                            0x018a30,
                        )?;
                    }
                }
                number ^= 1;
            }
            if earthling.kind().read(context) == 7 {
                super::framed::invoke_void(
                    context,
                    caller,
                    ids::EARTHLING_CHECK_DECOY_ATTACK_RANGE,
                    &[earthling.address(), definition.address()],
                    0x018a50,
                )?;
            }
        }
    }
    Ok(None)
}

/// G/src/earthlings/spawning_and_banks.c:871 (`ProcessEarthlings`).
/// TimeAware: callbacks may wait. Preserve bank order, all passes and final parity.
pub fn process_earthlings(context: &mut dyn Context) -> Result<Option<u32>> {
    let revision = context.revision();
    let caller = ids::PROCESS_EARTHLINGS;
    if ram::g_player_inventory_open(revision).at(0).read(context) != 0 {
        let second = ram::g_player2_inventory_open(revision).read(context);
        if second != 0 {
            return Ok(None);
        }
    }
    if ram::g_earthling_bank_users(revision).at(0).read(context) != 0 {
        super::framed::invoke_void(
            context,
            caller,
            ids::LOAD_IN_EARTHLINGS,
            &[ram::g_earthling_banks(revision).at(0).address()],
            0x019134,
        )?;
    }
    if ram::g_earthling_bank2_users(revision).read(context) != 0 {
        super::framed::invoke_void(
            context,
            caller,
            ids::LOAD_IN_EARTHLINGS,
            &[ram::g_earthling_bank2_records(revision).address()],
            0x019148,
        )?;
    }
    super::framed::invoke_void(
        context,
        caller,
        ids::EARTHLING_DISPATCH_ABILITIES,
        &[],
        0x01914e,
    )?;
    super::framed::invoke_void(
        context,
        caller,
        ids::EARTHLING_DISPATCH_PLAYER_CONTACTS,
        &[],
        0x019152,
    )?;
    super::framed::invoke_void(
        context,
        caller,
        ids::EARTHLING_APPLY_NEARBY_SLEEP,
        &[],
        0x019156,
    )?;
    super::framed::invoke_void(
        context,
        caller,
        ids::SHIP_UPDATE_PENDING_PIECE_INDICATOR,
        &[],
        0x01915c,
    )?;
    super::framed::invoke_void(
        context,
        caller,
        ids::RELEASE_FINISHED_MOLE_EFFECTS_AND_STOLEN_PRESENTS,
        &[],
        0x019162,
    )?;
    let parity = ram::g_earthling_update_parity(revision);
    let value = parity.read(context) ^ 1;
    parity.write(context, value);

    Ok(None)
}
