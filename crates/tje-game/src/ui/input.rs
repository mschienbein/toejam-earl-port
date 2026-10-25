use super::control::finish_inventory_frame;
use super::entities::entity;
use super::hud_all::frame;
use crate::call_frame::{CallSite, FrameLayout, LocalFrame, OutgoingArguments};
use crate::{Context, Result, ids};
use tje_mem::{Address, ram, rom_symbol};

/// G/src/ui/inventory.c:368 (`RandomizePresents`).
/// c_inventory.s:819-908 saves D2-D4/A2 and retains the two refresh
/// arguments, the VBlank argument and sound arguments until each loop ends.
pub fn randomize_presents(context: &mut dyn Context) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x1c, 0x04))?;
    let player = frame.argument_slot(0).read(context) as i32;
    context.set_data_register(2, player as u32);
    context.set_data_register(4, context.data_register(4) & 0xffff0000);
    context.set_address_register(
        2,
        ids::INVENTORY_REFRESH_CONTENTS_AND_WAIT
            .metadata()
            .address(context.revision()),
    );
    let result = randomize_presents_inner(context, player);
    finish_inventory_frame(context, frame, result)
}

fn randomize_presents_inner(context: &mut dyn Context, player: i32) -> Result<Option<u32>> {
    let revision = context.revision();
    let caller = ids::RANDOMIZE_PRESENTS;
    let dispatcher = context.dispatcher();
    let mut close_other = false;
    frame::call_void(context, caller, ids::SETUP_PRESENT_IDENTITIES, 0x16)?;
    let initial_refresh = {
        let argument = player as u32;
        frame::push_move(context, argument)?;
        OutgoingArguments::existing(context, 1)?
    };
    initial_refresh.call_void(
        context,
        dispatcher,
        ids::INVENTORY_REFRESH_CONTENTS_AND_WAIT,
        CallSite::relative(caller, revision, 0x1a),
    )?;
    let other_alive = ram::g_player_lives(revision).at(player ^ 1).read(context) >= 0;
    initial_refresh.release(context)?;
    if other_alive {
        if ram::g_player_inventory_open(revision)
            .at(player ^ 1)
            .read(context)
            == 0
        {
            let draw = {
                frame::push_pea(context, 1)?;
                frame::push_move(context, (player ^ 1) as u32)?;
                OutgoingArguments::existing(context, 2)?
            };
            draw.call_void(
                context,
                dispatcher,
                ids::DRAW_INVENTORY,
                CallSite::relative(caller, revision, 0x50),
            )?;
            {
                let argument = (player ^ 1) as u32;
                frame::push_move(context, argument)?;
                frame::call_void(context, caller, ids::INVENTORY_UPDATE_SELECTOR_ENTITY, 0x5c)?;
                frame::release(context, 4)?;
            };
            draw.release(context)?;
            close_other = true;
            context.set_data_register(4, 1);
        } else {
            {
                let argument = (player ^ 1) as u32;
                frame::push_move(context, argument)?;
                frame::call_void(
                    context,
                    caller,
                    ids::INVENTORY_REFRESH_CONTENTS_AND_WAIT,
                    0x70,
                )?;
                frame::release(context, 4)?;
            };
        }
    }
    let first = ram::g_player1_inventory_selector_flags(revision);
    let flags = first.read(context) as u8 | 0x40;
    first.write(context, flags as i8);
    let second = ram::g_player2_inventory_selector_flags(revision);
    let flags = second.read(context) as u8 | 0x40;
    second.write(context, flags as i8);
    ram::g_inventory_labels_suppressed(revision).write(context, 1);
    let sound = rom_symbol(revision, "Sfx_Randomize")
        .map(|symbol| symbol.address)
        .ok_or_else(|| crate::Error::Platform("missing Sfx_Randomize ROM symbol".into()))?;
    {
        let third = 1;
        frame::push_pea(context, third)?;
        let second = 96;
        frame::push_pea(context, second)?;
        let first = sound;
        frame::push_move(context, first)?;
        frame::call_void(context, caller, ids::PLAY_SOUND_EFFECT, 0x9e)?;
        frame::release(context, 12)?;
    };
    context.set_data_register(3, context.data_register(3) & 0xffff0000);
    for frame in 0..60 {
        frame::call_void(context, caller, ids::RANDOMIZE_INVENTORY, 0xa8)?;
        let own = {
            let argument = player as u32;
            frame::push_move(context, argument)?;
            OutgoingArguments::existing(context, 1)?
        };
        own.call_void(
            context,
            dispatcher,
            ids::INVENTORY_REFRESH_CONTENTS_AND_WAIT,
            CallSite::relative(caller, revision, 0xac),
        )?;
        OutgoingArguments::existing(context, 0)?.call_void(
            context,
            dispatcher,
            ids::UPDATE_AND_QUEUE_ENTITY_SPRITES,
            CallSite::relative(caller, revision, 0xb2),
        )?;
        let other = {
            let argument = (player ^ 1) as u32;
            frame::push_move(context, argument)?;
            OutgoingArguments::existing(context, 1)?
        };
        other.call_void(
            context,
            dispatcher,
            ids::INVENTORY_REFRESH_CONTENTS_AND_WAIT,
            CallSite::relative(caller, revision, 0xbc),
        )?;
        OutgoingArguments::existing(context, 0)?.call_void(
            context,
            dispatcher,
            ids::UPDATE_AND_QUEUE_ENTITY_SPRITES,
            CallSite::relative(caller, revision, 0xc2),
        )?;
        let wait = {
            let argument = (frame / 5) as u32;
            frame::push_move(context, argument)?;
            OutgoingArguments::existing(context, 1)?
        };
        wait.call_void(
            context,
            dispatcher,
            ids::WAIT_FOR_NEXT_V_BLANK,
            CallSite::relative(caller, revision, 0xd4),
        )?;
        let sound = {
            frame::push_pea(context, 50)?;
            frame::push_pea(context, 7)?;
            OutgoingArguments::existing(context, 2)?
        };
        sound.call_void(
            context,
            dispatcher,
            ids::PLAY_SOUND,
            CallSite::relative(caller, revision, 0xe2),
        )?;
        context.set_data_register(
            3,
            (context.data_register(3) & 0xffff0000) | (frame + 1) as u32,
        );
        sound.release(context)?;
        wait.release(context)?;
        other.release(context)?;
        own.release(context)?;
    }
    context.discard_u8(ram::g_inventory_labels_suppressed(revision).address())?;
    ram::g_inventory_labels_suppressed(revision).write(context, 0);
    frame::call_void(
        context,
        caller,
        ids::PRESENT_CLEAR_WORLD_ENTITY_READINESS,
        0xfa,
    )?;

    if close_other {
        {
            let argument = (player ^ 1) as u32;
            frame::push_move(context, argument)?;
            frame::call_void(context, caller, ids::TOGGLE_INVENTORY, 0x10a)?;
            frame::release(context, 4)?;
        };
    }
    Ok(None)
}

/// G/src/ui/inventory.c:403 (`ProcessInventory`).
/// The actual LINK A6,#-4/MOVEM D2/A2-A4 frame owns opening[2]. The outgoing
/// pointer for CheckIfOpenForBoth uses its real compiled JSR return address.
pub fn process_inventory(context: &mut dyn Context) -> Result<Option<u32>> {
    let revision = context.revision();
    let caller = ids::PROCESS_INVENTORY;
    let dispatcher = context.dispatcher();
    let frame = LocalFrame::enter(context, FrameLayout::link(4, 0x04, 0x1c))?;
    let result = (|| -> Result<Option<u32>> {
        context.set_address_register(2, ram::g_player_inventory_open(revision).address());
        context.set_address_register(3, ram::g_player_processed_input(revision).address());
        context.set_address_register(4, ram::g_player_inventory_scroll_row(revision).address());
        let opening = frame.local::<i16>(-4);
        opening.write(context, 0);
        opening.offset(1).write(context, 0);
        let mut loop_exit_player = 2_i16;
        for player_number in 0_i16..2 {
            let player = i32::from(player_number);
            // CheckBButton and other original-bug descendants can consume D2.
            let residue = (context.data_register(2) & 0xffff0000) | u32::from(player_number as u16);
            context.set_data_register(2, residue);
            let object = entity(context, player_number);
            if ram::g_player_lives(revision).at(player).read(context) < 0
                || object.sprite_state().read(context) == 5
                || object.sprite_state().read(context) == 6
            {
                continue;
            }
            let previous = ram::g_player_previous_processed_input(revision).at(player);
            let input = ram::g_player_processed_input(revision).at(player);
            let inventory = ram::g_player_inventory_open(revision).at(player);
            if previous.read(context) & 0x10 == 0 && input.read(context) & 0x10 != 0 {
                {
                    let argument = player as u32;
                    frame::push_move(context, argument)?;
                    frame::call_void(context, caller, ids::TOGGLE_INVENTORY, 0x80)?;
                    frame::release(context, 4)?;
                };
                return Ok(None);
            }
            if inventory.read(context) != 0 {
                if inventory.read(context) == 1
                    && previous.read(context) & 0x20 == 0
                    && input.read(context) & 0x20 != 0
                {
                    let banner = ram::g_inventory_banner_mode(revision).at(player);
                    let next = banner.read(context) ^ 1;
                    banner.write(context, next);
                    let blanked = {
                        frame::push_clear(context)?;
                        OutgoingArguments::existing(context, 1)?
                    };
                    let menu = i32::from(banner.read(context)) as u32;
                    let banner_arguments = {
                        frame::push_move(context, menu)?;
                        frame::push_move(context, player as u32)?;
                        OutgoingArguments::existing(context, 2)?
                    };
                    banner_arguments.call_void(
                        context,
                        dispatcher,
                        ids::INVENTORY_DRAW_ACTION_BANNER,
                        CallSite::relative(caller, revision, 0xec),
                    )?;
                    {
                        let second = 50;
                        frame::push_pea(context, second)?;
                        let first = 7;
                        frame::push_pea(context, first)?;
                        frame::call_void(context, caller, ids::PLAY_SOUND, 0xfc)?;
                        frame::release(context, 8)?;
                    };
                    banner_arguments.release(context)?;
                    blanked.release(context)?;
                } else if previous.read(context) & 0x40 == 0 && input.read(context) & 0x40 != 0 {
                    if inventory.read(context) == 2 {
                        let toggle = {
                            let argument = player as u32;
                            frame::push_move(context, argument)?;
                            OutgoingArguments::existing(context, 1)?
                        };
                        toggle.call_void(
                            context,
                            dispatcher,
                            ids::TOGGLE_INVENTORY,
                            CallSite::relative(caller, revision, 0x13c),
                        )?;
                        {
                            let argument = player as u32;
                            frame::push_move(context, argument)?;
                            frame::call_void(
                                context,
                                caller,
                                ids::INVENTORY_BUY_SELECTED_MAIL_ORDER_PRESENT,
                                0x146,
                            )?;
                            frame::release(context, 4)?;
                        };
                        toggle.release(context)?;
                    } else if inventory.read(context) == 3 {
                        if {
                            let argument = player as u32;
                            frame::push_move(context, argument)?;
                            let returned = frame::call(
                                context,
                                caller,
                                ids::INVENTORY_IDENTIFY_SELECTED_PRESENT,
                                0x162,
                            )?;
                            frame::release(context, 4)?;
                            returned
                        } != 0
                        {
                            let refresh = {
                                let argument = player as u32;
                                frame::push_move(context, argument)?;
                                OutgoingArguments::existing(context, 1)?
                            };
                            refresh.call_void(
                                context,
                                dispatcher,
                                ids::INVENTORY_REFRESH_CONTENTS_AND_WAIT,
                                CallSite::relative(caller, revision, 0x176),
                            )?;
                            let other_open = ram::g_player_inventory_open(revision)
                                .at(player ^ 1)
                                .read(context)
                                != 0;
                            refresh.release(context)?;
                            if other_open {
                                {
                                    let argument = (player ^ 1) as u32;
                                    frame::push_move(context, argument)?;
                                    frame::call_void(
                                        context,
                                        caller,
                                        ids::INVENTORY_REFRESH_CONTENTS_AND_WAIT,
                                        0x196,
                                    )?;
                                    frame::release(context, 4)?;
                                };
                            }
                        }
                    } else if ram::g_inventory_banner_mode(revision)
                        .at(player)
                        .read(context)
                        == 1
                    {
                        let drop = {
                            let argument = player as u32;
                            frame::push_move(context, argument)?;
                            OutgoingArguments::existing(context, 1)?
                        };
                        drop.call_void(
                            context,
                            dispatcher,
                            ids::INVENTORY_DROP_SELECTED_PRESENT,
                            CallSite::relative(caller, revision, 0x1b4),
                        )?;
                        {
                            let argument = player as u32;
                            frame::push_move(context, argument)?;
                            frame::call_void(
                                context,
                                caller,
                                ids::INVENTORY_REFRESH_CONTENTS_AND_WAIT,
                                0x1c0,
                            )?;
                            frame::release(context, 4)?;
                        };
                        drop.release(context)?;
                    } else {
                        let next = previous.read(context) | 0x40;
                        previous.write(context, next);
                        if {
                            let argument = player as u32;
                            frame::push_move(context, argument)?;
                            let returned = frame::call(
                                context,
                                caller,
                                ids::INVENTORY_GET_SELECTED_PRESENT,
                                0x1de,
                            )?;
                            frame::release(context, 4)?;
                            returned
                        } as u8
                            == 18
                        {
                            {
                                let argument = player as u32;
                                frame::push_move(context, argument)?;
                                frame::call_void(context, caller, ids::RANDOMIZE_PRESENTS, 0x1f4)?;
                                frame::release(context, 4)?;
                            };
                        } else {
                            {
                                let argument = player as u32;
                                frame::push_move(context, argument)?;
                                frame::call_void(context, caller, ids::TOGGLE_INVENTORY, 0x204)?;
                                frame::release(context, 4)?;
                            };
                            Address::<i16>::new(context.address_register(6).wrapping_sub(4))
                                .offset(player)
                                .write(context, 1);
                        }
                    }
                } else if previous.read(context) & 0x0f == 0 && input.read(context) & 0x0f != 0 {
                    let row = ram::g_player_inventory_selection_row(revision).at(player);
                    let scroll = ram::g_player_inventory_scroll_row(revision).at(player);
                    if input.read(context) & 1 != 0 {
                        if row.read(context) > 0 {
                            let next = row.read(context).wrapping_sub(1);
                            row.write(context, next);
                        } else if inventory.read(context) != 2 && scroll.read(context) > 0 {
                            let next = scroll.read(context).wrapping_sub(1);
                            scroll.write(context, next);
                            {
                                let argument = player as u32;
                                frame::push_move(context, argument)?;
                                frame::call_void(
                                    context,
                                    caller,
                                    ids::INVENTORY_REFRESH_CONTENTS_AND_WAIT,
                                    0x29a,
                                )?;
                                frame::release(context, 4)?;
                            };
                        }
                    } else if input.read(context) & 2 != 0 {
                        if row.read(context) < 2 {
                            let next = row.read(context).wrapping_add(1);
                            row.write(context, next);
                        } else if inventory.read(context) != 2 && scroll.read(context) < 5 {
                            let slot = (i32::from(scroll.read(context)) + 3) * 2;
                            if ram::g_player_inventory(revision)
                                .at(player)
                                .at(slot)
                                .read(context)
                                != -1
                            {
                                let next = scroll.read(context).wrapping_add(1);
                                scroll.write(context, next);
                                {
                                    let argument = player as u32;
                                    frame::push_move(context, argument)?;
                                    frame::call_void(
                                        context,
                                        caller,
                                        ids::INVENTORY_REFRESH_CONTENTS_AND_WAIT,
                                        0x29a,
                                    )?;
                                    frame::release(context, 4)?;
                                };
                            }
                        }
                    } else if inventory.read(context) != 2 {
                        let column = ram::g_player_inventory_selection_column(revision).at(player);
                        let next = column.read(context) ^ 1;
                        column.write(context, next);
                    }
                    {
                        let second = 50;
                        frame::push_pea(context, second)?;
                        let first = 7;
                        frame::push_pea(context, first)?;
                        frame::call_void(context, caller, ids::PLAY_SOUND, 0x344)?;
                        frame::release(context, 8)?;
                    };
                }
                if inventory.read(context) != 0 {
                    {
                        let argument = player as u32;
                        frame::push_move(context, argument)?;
                        frame::call_void(
                            context,
                            caller,
                            ids::INVENTORY_UPDATE_SELECTOR_ENTITY,
                            0x35a,
                        )?;
                        frame::release(context, 4)?;
                    };
                }
                if Address::<i16>::new(context.address_register(6).wrapping_sub(4))
                    .offset(player)
                    .read(context)
                    != 0
                {
                    loop_exit_player = player_number;
                    break;
                }
            }
        }
        let residue = (context.data_register(2) & 0xffff0000) | u32::from(loop_exit_player as u16);
        context.set_data_register(2, residue);
        let opening_pointer = context.address_register(6).wrapping_sub(4);
        frame::push_pea(context, opening_pointer)?;
        frame::call_void(context, caller, ids::CHECK_IF_OPEN_FOR_BOTH, 0x37c)?;
        frame::release(context, 4)?;
        if ram::g_pending_unfall_player(revision).read(context) != 0 {
            let player =
                i32::from(ram::g_pending_unfall_player(revision).read(context)).wrapping_sub(1);
            let residue = (context.data_register(2) & 0xffff0000) | u32::from(player as u16);
            context.set_data_register(2, residue);
            if ram::g_player_inventory_open(revision)
                .at(player ^ 1)
                .read(context)
                != 0
            {
                {
                    let argument = (player ^ 1) as u32;
                    frame::push_move(context, argument)?;
                    frame::call_void(context, caller, ids::TOGGLE_INVENTORY, 0x3ac)?;
                    frame::release(context, 4)?;
                };
            }
            {
                let argument = player as u32;
                frame::push_move(context, argument)?;
                frame::call_void(
                    context,
                    caller,
                    ids::PLAYER_RELOCATE_TO_PARTNER_OR_NEXT_LEVEL,
                    0x3ba,
                )?;
                frame::release(context, 4)?;
            };
            context.discard_u8(ram::g_pending_unfall_player(revision).address())?;
            ram::g_pending_unfall_player(revision).write(context, 0);
        }
        Ok(None)
    })();
    finish_inventory_frame(context, frame, result)
}

/// G/src/ui/inventory.c:491 (`CheckBButton`).
/// ORIGINAL BUG: frame begins with incoming D2.low, not an invented zero.
pub fn check_b_button(context: &mut dyn Context) -> Result<Option<u32>> {
    let saved = LocalFrame::enter(context, FrameLayout::movem(0, 0x04, 0x04))?;
    let revision = context.revision();
    let caller = ids::CHECK_B_BUTTON;
    context.set_address_register(2, ram::g_player_inventory_open(revision).address());
    let result = (|| -> Result<Option<u32>> {
        frame::call_void(context, caller, ids::PROCESS_INVENTORY, 0xe)?;
        let inventories = ram::g_player_inventory_open(revision);
        if (inventories.at(0).read(context) != 0 || inventories.at(1).read(context) != 0)
            && ram::g_split_screen_enabled(revision).read(context) == 0
        {
            while inventories.at(0).read(context) != 0 || inventories.at(1).read(context) != 0 {
                let current = context.data_register(2);
                let frame = (current as i16).wrapping_add(1);
                context.set_data_register(2, (current & 0xffff0000) | u32::from(frame as u16));

                let remainder = i32::from(frame) % 3;
                context.set_data_register(0, remainder as u32);
                context.set_data_register(1, 3);

                if remainder == 0 {
                    frame::call_void(context, caller, ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, 0x46)?;
                } else {
                    {
                        let argument = 1;
                        frame::push_pea(context, argument)?;
                        frame::call_void(context, caller, ids::WAIT_FOR_NEXT_V_BLANK, 0x52)?;
                        frame::release(context, 4)?;
                    };
                }
                frame::call_void(context, caller, ids::GET_PLAYER_INPUT, 0x5a)?;
                frame::call_void(context, caller, ids::PROCESS_INVENTORY, 0x5e)?;
            }
        }
        Ok(None)
    })();
    finish_inventory_frame(context, saved, result)
}

/// G/src/ui/inventory.c:510 (`InventoryOpenMailOrderMenu`).
pub fn inventory_open_mail_order_menu(context: &mut dyn Context) -> Result<Option<u32>> {
    open_special_inventory(context, 2)
}

/// G/src/ui/inventory.c:521 (`InventoryOpenIdentificationMenu`).
pub fn inventory_open_identification_menu(context: &mut dyn Context) -> Result<Option<u32>> {
    open_special_inventory(context, 3)
}

fn open_special_inventory(context: &mut dyn Context, menu: u32) -> Result<Option<u32>> {
    let revision = context.revision();
    // Restore the actual saved slot, including source-visible alias mutations.
    let saved_d2 = context.data_register(2);
    let saved = {
        let argument = saved_d2;
        frame::push_move(context, argument)?;
        OutgoingArguments::existing(context, 1)?
    };
    let player = context
        .frame_state()
        .ok_or(crate::call_frame::FrameError::Unavailable)?
        .argument_word(0)
        .read(context);
    let caller = if menu == 2 {
        ids::INVENTORY_OPEN_MAIL_ORDER_MENU
    } else {
        ids::INVENTORY_OPEN_IDENTIFICATION_MENU
    };
    context.set_data_register(2, (saved_d2 & 0xffff0000) | u32::from(player as u16));
    let result = (|| -> Result<Option<u32>> {
        {
            let second = menu;
            frame::push_pea(context, second)?;
            let first = i32::from(player) as u32;
            frame::push_move(context, first)?;
            frame::call_void(context, caller, ids::DRAW_INVENTORY, 0x14)?;
            frame::release(context, 8)?;
        };
        ram::g_player_processed_input(revision)
            .at(i32::from(player))
            .write(context, 0x40);
        ram::g_player_previous_processed_input(revision)
            .at(i32::from(player))
            .write(context, 0x40);
        frame::call_void(context, caller, ids::CHECK_B_BUTTON, 0x3a)?;
        Ok(None)
    })();
    let result = result.map_err(|error| context.protect_timing_failure(error));
    if result
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        return result;
    }
    let restored = saved.argument_slot(0).read(context);
    context.set_data_register(2, restored);

    match (result, saved.release(context)) {
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(value), Ok(())) => Ok(value),
    }
}
