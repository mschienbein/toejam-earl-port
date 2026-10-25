use super::draws::literal;
use super::entities::entity;
use super::hud_all::frame;
use crate::call_frame::{CallSite, FrameLayout, LocalFrame, OutgoingArguments};
use crate::{Context, Result, call, call_at, ids};
use tje_mem::{Address, Revision, ram, rom};

pub(super) fn finish_inventory_frame(
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
    match (result, frame.leave(context)) {
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(value), Ok(())) => Ok(value),
    }
}

/// G/src/ui/inventory.c:821 (`InventoryInitializeDisplayEntities`).
/// Original MOVEM D2-D6/A2 is materialized because the allocator's original bug
/// consumes entry D4.low: this caller leaves its current object number there.
pub fn inventory_initialize_display_entities(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let player = arguments[0] as i16;
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x7c, 0x04))?;
    let result = (|| -> Result<Option<u32>> {
        let index = i32::from(player);
        ram::g_player_inventory_selection_row(revision)
            .at(index)
            .write(context, 0);
        ram::g_player_inventory_selection_column(revision)
            .at(index)
            .write(context, 0);
        ram::g_player_inventory_scroll_row(revision)
            .at(index)
            .write(context, 0);
        let (first, viewport, ready) = if player == 0 {
            if ram::g_split_screen_enabled(revision).read(context) != 0 {
                (26_i16, 0_u32, 2_u8)
            } else {
                (26, 1, 4)
            }
        } else {
            (33, 1, 4)
        };
        let mut object = entity(context, first);
        for number in first..first + 7 {
            object.draw_flags().write(context, 1);
            loop {
                let residual = (context.data_register(4) & 0xffff0000) | u32::from(number as u16);
                context.set_data_register(4, residual);
                let site =
                    CallSite::relative(ids::INVENTORY_INITIALIZE_DISPLAY_ENTITIES, revision, 0x7a);
                call_at(
                    context,
                    ids::ALLOCATE_ENTITY_VIEWPORT_SPRITES,
                    &[object.address(), viewport],
                    site,
                )?;
                if object.draw_flags().read(context) & ready != 0 {
                    break;
                }
                call(context, ids::ENTITY_RELEASE_ONE_BY_PRIORITY, &[])?;
            }
            object.sprite_ready().write(context, 0);
            let flags = object.behavior_flags().read(context) | 0x40;
            object.behavior_flags().write(context, flags);
            object = object.offset(1);
        }
        let object = entity(context, player);
        let flags = object.draw_flags().read(context) | 8;
        object.draw_flags().write(context, flags);
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

/// G/src/ui/inventory.c:268 (`InventoryRedrawSynchronously`).
pub fn inventory_redraw_synchronously(context: &mut dyn Context) -> Result<Option<u32>> {
    let revision = context.revision();
    let player = context
        .frame_state()
        .ok_or(crate::call_frame::FrameError::Unavailable)?
        .argument_slot(0)
        .read(context);
    ram::g_inventory_redraw_player(revision).write(context, player as i8);
    ram::g_inventory_redraw_phase(revision).write(context, 4);
    loop {
        let phase = ram::g_inventory_redraw_phase(revision).read(context);

        if phase == 0 {
            break;
        }
        frame::call(
            context,
            ids::INVENTORY_REDRAW_SYNCHRONOUSLY,
            ids::INVENTORY_SERVICE_REDRAW_PHASE,
            0x18,
        )?;
    }
    Ok(None)
}

/// G/src/ui/inventory.c:125 (`InventoryServiceRedrawPhase`).
/// Matching c_inventory.s:159-296 saves D2-D4/A2-A3. Argument areas
/// survive the stats/banner/HUD chain exactly as in the compiled caller.
pub fn inventory_service_redraw_phase(context: &mut dyn Context) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x1c, 0x0c))?;
    let revision = context.revision();
    context.set_address_register(2, ram::g_inventory_redraw_player(revision).address());
    context.set_address_register(3, ram::g_inventory_redraw_phase(revision).address());
    let result = inventory_service_redraw_phase_inner(context);
    finish_inventory_frame(context, frame, result)
}

fn inventory_service_redraw_phase_inner(context: &mut dyn Context) -> Result<Option<u32>> {
    let revision = context.revision();
    let caller = ids::INVENTORY_SERVICE_REDRAW_PHASE;
    let dispatcher = context.dispatcher();
    let phase = ram::g_inventory_redraw_phase(revision);
    let player_slot = ram::g_inventory_redraw_player(revision);
    if phase.read(context) >= 3 {
        let (start, end) = if player_slot.read(context) == 0 {
            if phase.read(context) == 4 {
                (0, 7)
            } else {
                (7, 14)
            }
        } else if phase.read(context) == 4 {
            (14, 21)
        } else {
            (21, 28)
        };
        context.set_data_register(4, end as u32);
        context.set_data_register(
            3,
            if start == 0 {
                context.data_register(3) & 0xffff0000
            } else {
                start as u32
            },
        );
        for row in start..end {
            context.write_vdp_command(0x50000003_u32.wrapping_add((row as u32) << 23));
            for column in 0..40 {
                context
                    .set_data_register(2, (context.data_register(2) & 0xffff0000) | column as u32);
                let mut tile = u16::from(
                    rom::font_background_byte_map(revision)
                        .at(row)
                        .at(column)
                        .read(context),
                );
                if tile & 0x40 != 0 {
                    tile |= 0x800;
                }
                if tile & 0x80 != 0 {
                    tile |= 0x1000;
                }
                tile &= 0xff3f;
                context.write_vdp_data(tile.wrapping_add(0x8580));
            }
            context.set_data_register(2, (context.data_register(2) & 0xffff0000) | 40);
            context.set_data_register(
                3,
                (context.data_register(3) & 0xffff0000) | (row + 1) as u32,
            );
        }
    } else if phase.read(context) == 2 {
        let background = {
            frame::push_pea(context, 1)?;
            OutgoingArguments::existing(context, 1)?
        };
        background.call(
            context,
            dispatcher,
            ids::FORCE_TEXT_BACKGROUND_COLOR_TO_BLACK,
            CallSite::relative(caller, revision, 0xba),
        )?;
        let player = i32::from(player_slot.read(context));
        let stats = {
            let argument = player as u32;
            frame::push_move(context, argument)?;
            OutgoingArguments::existing(context, 1)?
        };
        stats.call(
            context,
            dispatcher,
            ids::INVENTORY_DRAW_PLAYER_STATS,
            CallSite::relative(caller, revision, 0xc6),
        )?;
        // PEA 1 precedes the banner-mode read; the player argument is loaded
        // again afterwards, not cached across the two source byte accesses.
        let blanked = {
            frame::push_pea(context, 1)?;
            OutgoingArguments::existing(context, 1)?
        };
        let player = i32::from(player_slot.read(context));
        let banner = i32::from(
            ram::g_inventory_banner_mode(revision)
                .at(player)
                .read(context),
        );
        let banner_argument = {
            let argument = banner as u32;
            frame::push_move(context, argument)?;
            OutgoingArguments::existing(context, 1)?
        };
        let player = i32::from(player_slot.read(context));
        let banner_player = {
            let argument = player as u32;
            frame::push_move(context, argument)?;
            OutgoingArguments::existing(context, 1)?
        };
        banner_player.call(
            context,
            dispatcher,
            ids::INVENTORY_DRAW_ACTION_BANNER,
            CallSite::relative(caller, revision, 0xee),
        )?;
        OutgoingArguments::existing(context, 0)?.call(
            context,
            dispatcher,
            ids::DRAW_HUD,
            CallSite::relative(caller, revision, 0xf6),
        )?;
        banner_player.release(context)?;
        banner_argument.release(context)?;
        blanked.release(context)?;
        stats.release(context)?;
        background.release(context)?;
    } else {
        let player = i32::from(player_slot.read(context));
        if ram::g_player_inventory_open(revision)
            .at(player)
            .read(context)
            == 2
        {
            let player = i32::from(player_slot.read(context));
            {
                let argument = player as u32;
                frame::push_move(context, argument)?;
                let returned = frame::call(
                    context,
                    caller,
                    ids::INVENTORY_DRAW_MAIL_ORDER_OFFERS,
                    0x11c,
                )?;
                frame::release(context, 4)?;
                returned
            };
        } else {
            let player = i32::from(player_slot.read(context));
            {
                let argument = player as u32;
                frame::push_move(context, argument)?;
                let returned =
                    frame::call(context, caller, ids::LOAD_PRESENTS_INTO_INVENTORY, 0x12c)?;
                frame::release(context, 4)?;
                returned
            };
            let player = i32::from(player_slot.read(context));
            if ram::g_player_inventory_open(revision)
                .at(player)
                .read(context)
                == 3
            {
                let player = i32::from(player_slot.read(context));
                {
                    let argument = player as u32;
                    frame::push_move(context, argument)?;
                    let returned = frame::call(context, caller, ids::INVENTORY_DRAW_BUCKS, 0x150)?;
                    frame::release(context, 4)?;
                    returned
                };
            }
        }
    }
    let next = phase.read(context).wrapping_sub(1);
    phase.write(context, next);
    Ok(None)
}

/// G/src/ui/inventory.c:197 (`DrawInventory`).
/// Matching c_inventory.s:395-486 saves D2/D3/A2-A4. The loop observes
/// phase/open/phase in that order; gFrameCounter is not a source operand.
pub fn draw_inventory(context: &mut dyn Context) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x0c, 0x1c))?;
    let revision = context.revision();
    let player = frame.argument_word(0).read(context);
    let menu = frame.argument_word(1).read(context);
    context.set_data_register(
        3,
        (context.data_register(3) & 0xffff0000) | u32::from(player as u16),
    );
    context.set_data_register(
        2,
        (context.data_register(2) & 0xffff0000) | u32::from(menu as u16),
    );
    context.set_address_register(4, ram::g_inventory_banner_mode(revision).address());
    context.set_address_register(2, ram::g_inventory_redraw_phase(revision).address());
    context.set_address_register(3, entity(context, player).address());
    let result = draw_inventory_inner(context, player, menu);
    finish_inventory_frame(context, frame, result)
}

fn draw_inventory_inner(context: &mut dyn Context, player: i16, menu: i16) -> Result<Option<u32>> {
    let revision = context.revision();
    let caller = ids::DRAW_INVENTORY;
    let index = i32::from(player);
    let viewport = if player == 0 { 0 } else { 1 };
    let object = entity(context, player);
    if ram::g_split_screen_enabled(revision).read(context) != 0 {
        {
            let argument = viewport;
            frame::push_move(context, argument)?;
            let returned = frame::call(context, caller, ids::ENTITY_CLEAR_VIEWPORT_SPRITES, 0x42)?;
            frame::release(context, 4)?;
            returned
        };
        let flags = object.behavior_flags().read(context) | 0x10;
        object.behavior_flags().write(context, flags);
        context.discard_u8(object.frame_duration().address())?;
        object.frame_duration().write(context, 0);
    }
    if menu == 1 {
        context.discard_u8(ram::g_inventory_banner_mode(revision).at(index).address())?;
        ram::g_inventory_banner_mode(revision)
            .at(index)
            .write(context, 0);
    } else if menu == 2 {
        ram::g_inventory_banner_mode(revision)
            .at(index)
            .write(context, 2);
        let level = i32::from(object.level().read(context)) as u32;
        {
            let second = level;
            frame::push_move(context, second)?;
            let first = index as u32;
            frame::push_move(context, first)?;
            let returned = frame::call(context, caller, ids::SET_MAIL_ORDER_PRESENTS, 0x82)?;
            frame::release(context, 8)?;
            returned
        };
    } else {
        ram::g_inventory_banner_mode(revision)
            .at(index)
            .write(context, 3);
    }
    if revision == Revision::Rev02 {
        context.discard_u16(ram::g_player_idle_ticks(revision).at(index).address())?;
        ram::g_player_idle_ticks(revision)
            .at(index)
            .write(context, 0);
    }
    {
        let argument = index as u32;
        frame::push_move(context, argument)?;
        let returned = frame::call(
            context,
            caller,
            ids::INVENTORY_INITIALIZE_DISPLAY_ENTITIES,
            if revision == Revision::Rev02 {
                0xaa
            } else {
                0x9a
            },
        )?;
        frame::release(context, 4)?;
        returned
    };
    ram::g_inventory_redraw_player(revision).write(context, player as i8);
    ram::g_inventory_redraw_phase(revision).write(context, 4);
    while ram::g_inventory_redraw_phase(revision).read(context) != 0 {
        if ram::g_player_inventory_open(revision)
            .at(index)
            .read(context)
            == 0
            && ram::g_inventory_redraw_phase(revision).read(context) <= 2
        {
            ram::g_player_inventory_open(revision)
                .at(index)
                .write(context, menu as i8);
        }
        context.wait_for_interrupt()?;
    }
    if ram::g_split_screen_enabled(revision).read(context) == 0 {
        context.discard_u16(ram::g_camera2_pending_x(revision).address())?;
        ram::g_camera2_pending_x(revision).write(context, 0);
        ram::g_camera2_pending_y(revision).write(context, 112);
    }
    frame::call(
        context,
        caller,
        ids::ENABLE_TEXT_FONT_ANIMATION,
        if revision == Revision::Rev02 {
            0xfc
        } else {
            0xec
        },
    )?;
    Ok(None)
}

/// G/src/ui/inventory.c:290 (`ToggleInventory`).
pub fn toggle_inventory(context: &mut dyn Context) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x04, 0x04))?;
    let revision = context.revision();
    let player = frame.argument_word(0).read(context);
    context.set_data_register(
        2,
        (context.data_register(2) & 0xffff0000) | u32::from(player as u16),
    );
    context.set_address_register(2, ram::g_player_inventory_open(revision).address());
    let result = toggle_inventory_inner(context, player);
    finish_inventory_frame(context, frame, result)
}

fn toggle_inventory_inner(context: &mut dyn Context, player: i16) -> Result<Option<u32>> {
    let revision = context.revision();
    let caller = ids::TOGGLE_INVENTORY;
    let index = i32::from(player);
    let inventories = ram::g_player_inventory_open(revision);
    if inventories.at(index).read(context) != 0 {
        {
            let argument = index as u32;
            frame::push_move(context, argument)?;
            let returned = frame::call(
                context,
                caller,
                ids::INVENTORY_DEACTIVATE_DISPLAY_ENTITIES,
                0x24,
            )?;
            frame::release(context, 4)?;
            returned
        };
        if ram::g_split_screen_enabled(revision).read(context) != 0 {
            context.discard_u8(inventories.at(index).address())?;
            inventories.at(index).write(context, 0);
            if inventories.at(index ^ 1).read(context) == 0
                && frame::call(context, caller, ids::PLAYERS_REQUIRE_SEPARATE_VIEWS, 0x4c)? == 0
            {
                {
                    let argument = 1;
                    frame::push_pea(context, argument)?;
                    let returned =
                        frame::call(context, caller, ids::UPDATE_SCREEN_SPLIT_MODE, 0x5a)?;
                    frame::release(context, 4)?;
                    returned
                };
            } else {
                ram::g_hud_redraw_pending(revision).write(context, 1);
            }
            let object = entity(context, player);
            let flags = object.behavior_flags().read(context) & !0x10;
            object.behavior_flags().write(context, flags);
            if object.frame_phase().read(context) != 0 {
                object.frame_duration().write(context, 1);
            }
            {
                let argument = index as u32;
                frame::push_move(context, argument)?;
                let returned =
                    frame::call(context, caller, ids::UPDATE_ENTITY_VIEWPORT_SPRITES, 0x90)?;
                frame::release(context, 4)?;
                returned
            };
        } else {
            if inventories.at(index ^ 1).read(context) == 0 {
                let x = ram::g_camera1_pending_x(revision).read(context);
                ram::g_camera2_pending_x(revision).write(context, x);
                let y = ram::g_camera1_pending_y(revision).read(context);
                ram::g_camera2_pending_y(revision).write(context, y);
            }
            context.discard_u8(inventories.at(index).address())?;
            inventories.at(index).write(context, 0);
            frame::call(
                context,
                caller,
                ids::WAIT_FOR_V_BLANK_WITH_GRAPHICS_PAUSED,
                0xc2,
            )?;
            frame::call(context, caller, ids::DISABLE_BLANKING_GRAPHICS, 0xc8)?;
            frame::call(context, caller, ids::DRAW_HUD, 0xce)?;
            if inventories.at(index ^ 1).read(context) == 0 {
                frame::push_clear(context)?;
                frame::call(
                    context,
                    caller,
                    ids::FORCE_TEXT_BACKGROUND_COLOR_TO_BLACK,
                    0xe2,
                )?;
                frame::release(context, 4)?;
            }
            frame::call(context, caller, ids::ENABLE_BLANKING_GRAPHICS, 0xea)?;
        }
        frame::call(
            context,
            caller,
            ids::DISABLE_TEXT_FONT_ANIMATION_IF_MENUS_CLOSED,
            0xee,
        )?;
    } else {
        let elevator = ram::g_player_elevator_flags(revision)
            .at(index)
            .read(context);

        let can_open = if elevator == 0 {
            true
        } else {
            let departing = ram::g_elevator_departure_in_progress(revision).read(context);

            departing == 0
        };
        if can_open {
            {
                let second = 1;
                frame::push_pea(context, second)?;
                let first = index as u32;
                frame::push_move(context, first)?;
                let returned = frame::call(context, caller, ids::DRAW_INVENTORY, 0x116)?;
                frame::release(context, 8)?;
                returned
            };
        }
    }
    Ok(None)
}

/// G/src/ui/inventory.c:542 (`OpenPresent`).
/// Preserve the compiled caller's D2-D5/A2 save image and residual words before
/// dispatching present effects, including their original omitted-argument paths.
pub fn open_present(context: &mut dyn Context) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x3c, 0x04))?;
    let player = frame.argument_word(0).read(context);
    context.set_data_register(
        2,
        (context.data_register(2) & 0xffff0000) | u32::from(player as u16),
    );
    context.set_address_register(2, ram::g_player_inventory(context.revision()).address());
    let result = open_present_inner(context, i32::from(player));
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

fn open_present_inner(context: &mut dyn Context, player: i32) -> Result<Option<u32>> {
    let revision = context.revision();
    let row = i32::from(
        ram::g_player_inventory_selection_row(revision)
            .at(player)
            .read(context),
    );
    let scroll = i32::from(
        ram::g_player_inventory_scroll_row(revision)
            .at(player)
            .read(context),
    );
    let column = i32::from(
        ram::g_player_inventory_selection_column(revision)
            .at(player)
            .read(context),
    );
    let mut slot = (row
        .wrapping_add(scroll)
        .wrapping_mul(2)
        .wrapping_add(column)) as i8;
    let inventory = ram::g_player_inventory(revision).at(player);
    let mut present = inventory.at(i32::from(slot)).read(context);
    context.set_data_register(
        3,
        (context.data_register(3) & 0xffffff00) | u32::from(slot as u8),
    );
    context.set_data_register(
        4,
        (context.data_register(4) & 0xffffff00) | u32::from(present as u8),
    );
    while present == 26 {
        present = frame::call(context, ids::OPEN_PRESENT, ids::SELECT_RANDOM_PRESENT, 0x60)? as i8;
        context.set_data_register(
            4,
            (context.data_register(4) & 0xffffff00) | u32::from(present as u8),
        );
    }
    if present == -1 || {
        let second = i32::from(present) as u32;
        frame::push_move(context, second)?;
        let first = player as u32;
        frame::push_move(context, first)?;
        let returned = frame::call(context, ids::OPEN_PRESENT, ids::OPEN_SELECTED_PRESENT, 0x82)?;
        frame::release(context, 8)?;
        returned
    } != 0
    {
        {
            let second = 50;
            frame::push_pea(context, second)?;
            let first = 26;
            frame::push_pea(context, first)?;
            let returned = frame::call(context, ids::OPEN_PRESENT, ids::PLAY_SOUND, 0x96)?;
            frame::release(context, 8)?;
            returned
        };

        return Ok(Some(0));
    }
    if inventory.at(i32::from(slot)).read(context) != 26 {
        ram::g_present_info(revision)
            .at(i32::from(present))
            .identified()
            .write(context, 1);
    }
    while slot < 15 && inventory.at(i32::from(slot) + 1).read(context) != -1 {
        let value = inventory.at(i32::from(slot) + 1).read(context);
        inventory.at(i32::from(slot)).write(context, value);
        slot = slot.wrapping_add(1);
    }
    inventory.at(i32::from(slot)).write(context, -1);
    let returned = i32::from(present).wrapping_add(1);
    Ok(Some(returned as u32))
}

/// G/src/ui/inventory.c:589 (`InventoryBuySelectedMailOrderPresent`).
pub fn inventory_buy_selected_mail_order_present(context: &mut dyn Context) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x3c, 0x04))?;
    let player = frame.argument_word(0).read(context);
    context.set_data_register(
        3,
        (context.data_register(3) & 0xffff0000) | u32::from(player as u16),
    );
    context.set_data_register(4, context.data_register(4) & 0xffffff00);
    let result = inventory_buy_selected_mail_order_present_inner(context, i32::from(player));
    finish_inventory_frame(context, frame, result)
}

fn inventory_buy_selected_mail_order_present_inner(
    context: &mut dyn Context,
    player: i32,
) -> Result<Option<u32>> {
    let revision = context.revision();
    let caller = ids::INVENTORY_BUY_SELECTED_MAIL_ORDER_PRESENT;
    let inventory = ram::g_player_inventory(revision).at(player);
    let mut slot = 0_i8;
    while slot < 16 {
        context.set_data_register(2, (context.data_register(2) & 0xffff0000) | slot as u32);
        if inventory.at(i32::from(slot)).read(context) == -1 {
            break;
        }
        slot = slot.wrapping_add(1);
        context.set_data_register(
            4,
            (context.data_register(4) & 0xffffff00) | u32::from(slot as u8),
        );
    }
    let selection = ram::g_player_inventory_selection_row(revision)
        .at(player)
        .read(context);
    context.set_data_register(
        5,
        (context.data_register(5) & 0xffffff00) | u32::from(selection as u8),
    );
    let bucks = ram::g_player_bucks(revision).at(player);
    let prices = ram::g_player_mailbox_prices(revision)
        .at(player)
        .at(i32::from(selection));
    // MOVE.B price,D0 precedes CMP.B bucks,D0. Reversing these two
    // semantic reads is observable when an interrupt changes their state.
    let affordable = if slot < 16 {
        let price = prices.read(context);
        context.set_address_register(2, ram::g_player_bucks(revision).address());
        price <= bucks.read(context)
    } else {
        false
    };
    if affordable {
        context.set_data_register(
            2,
            (context.data_register(2) & 0xffff0000) | selection as i16 as u16 as u32,
        );
        context.set_address_register(
            2,
            ram::g_player_mailbox_prices(revision).at(player).address(),
        );
        let price = prices.read(context);
        let next = bucks.read(context).wrapping_sub(price);
        bucks.write(context, next);
        context.set_data_register(2, (context.data_register(2) & 0xffff0000) | slot as u32);
        context.set_address_register(2, ram::g_player_mailbox_items(revision).address());
        let present = ram::g_player_mailbox_items(revision)
            .at(player)
            .at(i32::from(selection))
            .read(context);
        inventory.at(i32::from(slot)).write(context, present);
        let text = literal(context, "MailOrderSuccessStr")?;
        {
            let third = 1;
            frame::push_pea(context, third)?;
            let second = text;
            frame::push_pea(context, second)?;
            let first = player as u32;
            frame::push_move(context, first)?;
            let returned = frame::call(context, caller, ids::GENERATE_SPEECH, 0x11a)?;
            frame::release(context, 12)?;
            returned
        };
    } else {
        {
            let second = 50;
            frame::push_pea(context, second)?;
            let first = 26;
            frame::push_pea(context, first)?;
            let returned = frame::call(context, caller, ids::PLAY_SOUND, 0xe8)?;
            frame::release(context, 8)?;
            returned
        };
        if slot == 16 {
            {
                let second = player as u32;
                frame::push_move(context, second)?;
                let first = 25;
                frame::push_pea(context, first)?;
                let returned = frame::call(context, caller, ids::START_DIALOGUE, 0x100)?;
                frame::release(context, 8)?;
                returned
            };
        } else {
            let text = literal(context, "MailOrderFailureStr")?;
            {
                let third = 1;
                frame::push_pea(context, third)?;
                let second = text;
                frame::push_pea(context, second)?;
                let first = player as u32;
                frame::push_move(context, first)?;
                let returned = frame::call(context, caller, ids::GENERATE_SPEECH, 0x11a)?;
                frame::release(context, 12)?;
                returned
            };
        }
    }
    Ok(None)
}

/// G/src/ui/inventory.c:327 (`CheckIfOpenForBoth`).
pub fn check_if_open_for_both(context: &mut dyn Context) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x0c, 0))?;
    let opening = frame.argument_slot(0).read(context);
    let result = check_if_open_for_both_inner(context, Address::new(opening));
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

fn check_if_open_for_both_inner(
    context: &mut dyn Context,
    opening: Address<i16>,
) -> Result<Option<u32>> {
    let revision = context.revision();
    let caller = ids::CHECK_IF_OPEN_FOR_BOTH;
    let first = opening.read(context);

    if first == 0 {
        let second = opening.offset(1).read(context);

        if second == 0 {
            return Ok(None);
        }
    }
    let player = if opening.read(context) != 0 {
        0_i32
    } else {
        1_i32
    };
    context.set_data_register(
        3,
        if player == 0 {
            context.data_register(3) & 0xffff0000
        } else {
            1
        },
    );
    let mut present = {
        let argument = player as u32;
        frame::push_move(context, argument)?;
        let returned = frame::call(context, caller, ids::OPEN_PRESENT, 0x2a)?;
        frame::release(context, 4)?;
        returned
    } as i16;
    context.set_data_register(
        2,
        (context.data_register(2) & 0xffff0000) | u32::from(present as u16),
    );

    if present == 0 {
        return Ok(None);
    }
    let lives = ram::g_player_lives(revision).at(player ^ 1).read(context);

    if lives < 0 {
        return Ok(None);
    }
    let separate = frame::call(context, caller, ids::PLAYERS_REQUIRE_SEPARATE_VIEWS, 0x50)?;

    if separate == 0 {
        present = present.wrapping_sub(1);
        context.set_data_register(
            2,
            (context.data_register(2) & 0xffff0000) | u32::from(present as u16),
        );
        let other_open = ram::g_player_inventory_open(revision)
            .at(player ^ 1)
            .read(context);

        if other_open != 0 {
            {
                let argument = (player ^ 1) as u32;
                frame::push_move(context, argument)?;
                let returned = frame::call(context, caller, ids::TOGGLE_INVENTORY, 0x7a)?;
                frame::release(context, 4)?;
                returned
            };
        }
        for excluded in [8, 15, 12, 19, 23, 25] {
            if present as u16 == excluded {
                return Ok(None);
            }
        }
        let failed = {
            let second = i32::from(present) as u32;
            frame::push_move(context, second)?;
            let first = (player ^ 1) as u32;
            frame::push_move(context, first)?;
            let returned = frame::call(context, caller, ids::OPEN_SELECTED_PRESENT, 0xc0)?;
            frame::release(context, 8)?;
            returned
        };

        if failed == 0 && present as u16 == 16 {
            let text = literal(context, "ThanksALotStr")?;
            {
                let third = 1;
                frame::push_pea(context, third)?;
                let second = text;
                frame::push_pea(context, second)?;
                let first = (player ^ 1) as u32;
                frame::push_move(context, first)?;
                let returned = frame::call(context, caller, ids::GENERATE_SPEECH, 0xe6)?;
                frame::release(context, 12)?;
                returned
            };
        }
    } else {
        let other_open = ram::g_player_inventory_open(revision)
            .at(player ^ 1)
            .read(context);

        if other_open != 0 {
            {
                let argument = (player ^ 1) as u32;
                frame::push_move(context, argument)?;
                let returned = frame::call(
                    context,
                    caller,
                    ids::INVENTORY_REFRESH_CONTENTS_AND_WAIT,
                    0x10e,
                )?;
                frame::release(context, 4)?;
                returned
            };
        }
    }
    Ok(None)
}

#[cfg(test)]
#[path = "inventory_replay_tests.rs"]
mod inventory_replay_tests;
