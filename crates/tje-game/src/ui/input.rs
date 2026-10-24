use super::entities::entity;
use crate::call_frame::{CallSite, FrameLayout, LocalFrame};
use crate::{Context, Result, call, call_at, ids};
use tje_mem::{ram, rom_symbol};

/// G/src/ui/inventory.c:368 (`RandomizePresents`).
pub fn randomize_presents(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let player = arguments[0] as i32;
    let mut close_other = false;
    call(context, ids::SETUP_PRESENT_IDENTITIES, &[])?;
    call(
        context,
        ids::INVENTORY_REFRESH_CONTENTS_AND_WAIT,
        &[player as u32],
    )?;
    if ram::g_player_lives(revision).at(player ^ 1).read(context) >= 0 {
        if ram::g_player_inventory_open(revision)
            .at(player ^ 1)
            .read(context)
            == 0
        {
            call(context, ids::DRAW_INVENTORY, &[(player ^ 1) as u32, 1])?;
            call(
                context,
                ids::INVENTORY_UPDATE_SELECTOR_ENTITY,
                &[(player ^ 1) as u32],
            )?;
            close_other = true;
        } else {
            call(
                context,
                ids::INVENTORY_REFRESH_CONTENTS_AND_WAIT,
                &[(player ^ 1) as u32],
            )?;
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
    call(context, ids::PLAY_SOUND_EFFECT, &[sound, 96, 1])?;
    for frame in 0..60 {
        call(context, ids::RANDOMIZE_INVENTORY, &[])?;
        call(
            context,
            ids::INVENTORY_REFRESH_CONTENTS_AND_WAIT,
            &[player as u32],
        )?;
        call(context, ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, &[])?;
        call(
            context,
            ids::INVENTORY_REFRESH_CONTENTS_AND_WAIT,
            &[(player ^ 1) as u32],
        )?;
        call(context, ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, &[])?;
        call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[(frame / 5) as u32])?;
        call(context, ids::PLAY_SOUND, &[7, 50])?;
    }
    ram::g_inventory_labels_suppressed(revision).write(context, 0);
    call(context, ids::PRESENT_CLEAR_WORLD_ENTITY_READINESS, &[])?;
    if close_other {
        call(context, ids::TOGGLE_INVENTORY, &[(player ^ 1) as u32])?;
    }
    Ok(None)
}

/// G/src/ui/inventory.c:403 (`ProcessInventory`).
/// The actual LINK A6,#-4/MOVEM D2/A2-A4 frame owns opening[2]. The outgoing
/// pointer for CheckIfOpenForBoth uses its real compiled JSR return address.
pub fn process_inventory(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
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
                call(context, ids::TOGGLE_INVENTORY, &[player as u32])?;
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
                    let menu = i32::from(banner.read(context)) as u32;
                    call(
                        context,
                        ids::INVENTORY_DRAW_ACTION_BANNER,
                        &[player as u32, menu, 0],
                    )?;
                    call(context, ids::PLAY_SOUND, &[7, 50])?;
                } else if previous.read(context) & 0x40 == 0 && input.read(context) & 0x40 != 0 {
                    if inventory.read(context) == 2 {
                        call(context, ids::TOGGLE_INVENTORY, &[player as u32])?;
                        call(
                            context,
                            ids::INVENTORY_BUY_SELECTED_MAIL_ORDER_PRESENT,
                            &[player as u32],
                        )?;
                    } else if inventory.read(context) == 3 {
                        if call(
                            context,
                            ids::INVENTORY_IDENTIFY_SELECTED_PRESENT,
                            &[player as u32],
                        )? != 0
                        {
                            call(
                                context,
                                ids::INVENTORY_REFRESH_CONTENTS_AND_WAIT,
                                &[player as u32],
                            )?;
                            if ram::g_player_inventory_open(revision)
                                .at(player ^ 1)
                                .read(context)
                                != 0
                            {
                                call(
                                    context,
                                    ids::INVENTORY_REFRESH_CONTENTS_AND_WAIT,
                                    &[(player ^ 1) as u32],
                                )?;
                            }
                        }
                    } else if ram::g_inventory_banner_mode(revision)
                        .at(player)
                        .read(context)
                        == 1
                    {
                        call(
                            context,
                            ids::INVENTORY_DROP_SELECTED_PRESENT,
                            &[player as u32],
                        )?;
                        call(
                            context,
                            ids::INVENTORY_REFRESH_CONTENTS_AND_WAIT,
                            &[player as u32],
                        )?;
                    } else {
                        let next = previous.read(context) | 0x40;
                        previous.write(context, next);
                        if call(
                            context,
                            ids::INVENTORY_GET_SELECTED_PRESENT,
                            &[player as u32],
                        )? == 18
                        {
                            call(context, ids::RANDOMIZE_PRESENTS, &[player as u32])?;
                        } else {
                            call(context, ids::TOGGLE_INVENTORY, &[player as u32])?;
                            opening.offset(player).write(context, 1);
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
                            call(
                                context,
                                ids::INVENTORY_REFRESH_CONTENTS_AND_WAIT,
                                &[player as u32],
                            )?;
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
                                call(
                                    context,
                                    ids::INVENTORY_REFRESH_CONTENTS_AND_WAIT,
                                    &[player as u32],
                                )?;
                            }
                        }
                    } else if inventory.read(context) != 2 {
                        let column = ram::g_player_inventory_selection_column(revision).at(player);
                        let next = column.read(context) ^ 1;
                        column.write(context, next);
                    }
                    call(context, ids::PLAY_SOUND, &[7, 50])?;
                }
                if inventory.read(context) != 0 {
                    call(
                        context,
                        ids::INVENTORY_UPDATE_SELECTOR_ENTITY,
                        &[player as u32],
                    )?;
                }
                if opening.offset(player).read(context) != 0 {
                    loop_exit_player = player_number;
                    break;
                }
            }
        }
        let residue = (context.data_register(2) & 0xffff0000) | u32::from(loop_exit_player as u16);
        context.set_data_register(2, residue);
        let site = CallSite::relative(ids::PROCESS_INVENTORY, revision, 0x37c);
        call_at(
            context,
            ids::CHECK_IF_OPEN_FOR_BOTH,
            &[opening.address()],
            site,
        )?;
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
                call(context, ids::TOGGLE_INVENTORY, &[(player ^ 1) as u32])?;
            }
            call(
                context,
                ids::PLAYER_RELOCATE_TO_PARTNER_OR_NEXT_LEVEL,
                &[player as u32],
            )?;
            ram::g_pending_unfall_player(revision).write(context, 0);
        }
        Ok(None)
    })();
    let result = result.map_err(|error| context.protect_timing_failure(error));
    if result
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        return result;
    }
    let leave = frame.leave(context);
    match (result, leave) {
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(value), Ok(())) => Ok(value),
    }
}

/// G/src/ui/inventory.c:491 (`CheckBButton`).
/// ORIGINAL BUG: frame begins with incoming D2.low, not an invented zero.
pub fn check_b_button(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let entry_d2 = context.data_register(2);
    let mut frame = entry_d2 as i16;
    let result = (|| -> Result<Option<u32>> {
        call(context, ids::PROCESS_INVENTORY, &[])?;
        let inventories = ram::g_player_inventory_open(revision);
        if (inventories.at(0).read(context) != 0 || inventories.at(1).read(context) != 0)
            && ram::g_split_screen_enabled(revision).read(context) == 0
        {
            while inventories.at(0).read(context) != 0 || inventories.at(1).read(context) != 0 {
                frame = frame.wrapping_add(1);
                context.set_data_register(2, (entry_d2 & 0xffff0000) | u32::from(frame as u16));
                if i32::from(frame) % 3 == 0 {
                    call(context, ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, &[])?;
                } else {
                    call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
                }
                call(context, ids::GET_PLAYER_INPUT, &[])?;
                call(context, ids::PROCESS_INVENTORY, &[])?;
            }
        }
        Ok(None)
    })();
    let result = result.map_err(|error| context.protect_timing_failure(error));
    if !result
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        context.set_data_register(2, entry_d2);
    }
    result
}

/// G/src/ui/inventory.c:510 (`InventoryOpenMailOrderMenu`).
pub fn inventory_open_mail_order_menu(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    open_special_inventory(context, arguments[0] as i16, 2)
}

/// G/src/ui/inventory.c:521 (`InventoryOpenIdentificationMenu`).
pub fn inventory_open_identification_menu(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    open_special_inventory(context, arguments[0] as i16, 3)
}

fn open_special_inventory(
    context: &mut dyn Context,
    player: i16,
    menu: u32,
) -> Result<Option<u32>> {
    let revision = context.revision();
    let saved_d2 = context.data_register(2);
    context.set_data_register(2, (saved_d2 & 0xffff0000) | u32::from(player as u16));
    let result = (|| -> Result<Option<u32>> {
        call(
            context,
            ids::DRAW_INVENTORY,
            &[i32::from(player) as u32, menu],
        )?;
        ram::g_player_processed_input(revision)
            .at(i32::from(player))
            .write(context, 0x40);
        ram::g_player_previous_processed_input(revision)
            .at(i32::from(player))
            .write(context, 0x40);
        call(context, ids::CHECK_B_BUTTON, &[])?;
        Ok(None)
    })();
    let result = result.map_err(|error| context.protect_timing_failure(error));
    if !result
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        context.set_data_register(2, saved_d2);
    }
    result
}
