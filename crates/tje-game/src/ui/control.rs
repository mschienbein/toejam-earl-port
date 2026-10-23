use super::draws::literal;
use super::entities::entity;
use crate::call_frame::{CallSite, FrameLayout, LocalFrame};
use crate::{Context, Result, call, call_at, ids};
use tje_mem::{Address, Revision, ram, rom};

fn wait_phase(context: &mut dyn Context) -> Result<()> {
    let revision = context.revision();
    let counter = ram::g_frame_counter(revision);
    let previous = counter.read(context);
    context.wait_vblank_frame(counter.address(), previous)
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
pub fn inventory_redraw_synchronously(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    ram::g_inventory_redraw_player(revision).write(context, arguments[0] as i8);
    ram::g_inventory_redraw_phase(revision).write(context, 4);
    while ram::g_inventory_redraw_phase(revision).read(context) != 0 {
        call(context, ids::INVENTORY_SERVICE_REDRAW_PHASE, &[])?;
    }
    Ok(None)
}

/// G/src/ui/inventory.c:125 (`InventoryServiceRedrawPhase`).
pub fn inventory_service_redraw_phase(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let phase = ram::g_inventory_redraw_phase(revision);
    if phase.read(context) >= 3 {
        let (start, end) = if ram::g_inventory_redraw_player(revision).read(context) == 0 {
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
        for row in start..end {
            context.write_vdp_command(0x50000003_u32.wrapping_add((row as u32) << 23));
            for column in 0..40 {
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
        }
    } else if phase.read(context) == 2 {
        call(context, ids::FORCE_TEXT_BACKGROUND_COLOR_TO_BLACK, &[1])?;
        let player = i32::from(ram::g_inventory_redraw_player(revision).read(context));
        call(context, ids::INVENTORY_DRAW_PLAYER_STATS, &[player as u32])?;
        let player = i32::from(ram::g_inventory_redraw_player(revision).read(context));
        let banner = i32::from(
            ram::g_inventory_banner_mode(revision)
                .at(player)
                .read(context),
        ) as u32;
        call(
            context,
            ids::INVENTORY_DRAW_ACTION_BANNER,
            &[player as u32, banner, 1],
        )?;
        call(context, ids::DRAW_HUD, &[])?;
    } else {
        let player = i32::from(ram::g_inventory_redraw_player(revision).read(context));
        if ram::g_player_inventory_open(revision)
            .at(player)
            .read(context)
            == 2
        {
            call(
                context,
                ids::INVENTORY_DRAW_MAIL_ORDER_OFFERS,
                &[player as u32],
            )?;
        } else {
            call(context, ids::LOAD_PRESENTS_INTO_INVENTORY, &[player as u32])?;
            let player = i32::from(ram::g_inventory_redraw_player(revision).read(context));
            if ram::g_player_inventory_open(revision)
                .at(player)
                .read(context)
                == 3
            {
                call(context, ids::INVENTORY_DRAW_BUCKS, &[player as u32])?;
            }
        }
    }
    let next = phase.read(context).wrapping_sub(1);
    phase.write(context, next);
    Ok(None)
}

/// G/src/ui/inventory.c:197 (`DrawInventory`).
pub fn draw_inventory(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let player = arguments[0] as i16;
    let menu = arguments[1] as i16;
    let index = i32::from(player);
    let viewport = if player == 0 { 0 } else { 1 };
    let object = entity(context, player);
    if ram::g_split_screen_enabled(revision).read(context) != 0 {
        call(context, ids::ENTITY_CLEAR_VIEWPORT_SPRITES, &[viewport])?;
        let flags = object.behavior_flags().read(context) | 0x10;
        object.behavior_flags().write(context, flags);
        object.frame_duration().write(context, 0);
    }
    if menu == 1 {
        ram::g_inventory_banner_mode(revision)
            .at(index)
            .write(context, 0);
    } else if menu == 2 {
        ram::g_inventory_banner_mode(revision)
            .at(index)
            .write(context, 2);
        let level = i32::from(object.level().read(context)) as u32;
        call(
            context,
            ids::SET_MAIL_ORDER_PRESENTS,
            &[index as u32, level],
        )?;
    } else {
        ram::g_inventory_banner_mode(revision)
            .at(index)
            .write(context, 3);
    }
    if revision == Revision::Rev02 {
        ram::g_player_idle_ticks(revision)
            .at(index)
            .write(context, 0);
    }
    call(
        context,
        ids::INVENTORY_INITIALIZE_DISPLAY_ENTITIES,
        &[index as u32],
    )?;
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
        wait_phase(context)?;
    }
    if ram::g_split_screen_enabled(revision).read(context) == 0 {
        ram::g_camera2_pending_x(revision).write(context, 0);
        ram::g_camera2_pending_y(revision).write(context, 112);
    }
    call(context, ids::ENABLE_TEXT_FONT_ANIMATION, &[])?;
    Ok(None)
}

/// G/src/ui/inventory.c:290 (`ToggleInventory`).
pub fn toggle_inventory(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let player = arguments[0] as i16;
    let index = i32::from(player);
    let inventories = ram::g_player_inventory_open(revision);
    if inventories.at(index).read(context) != 0 {
        call(
            context,
            ids::INVENTORY_DEACTIVATE_DISPLAY_ENTITIES,
            &[index as u32],
        )?;
        if ram::g_split_screen_enabled(revision).read(context) != 0 {
            inventories.at(index).write(context, 0);
            if inventories.at(index ^ 1).read(context) == 0
                && call(context, ids::PLAYERS_REQUIRE_SEPARATE_VIEWS, &[])? == 0
            {
                call(context, ids::UPDATE_SCREEN_SPLIT_MODE, &[1])?;
            } else {
                ram::g_hud_redraw_pending(revision).write(context, 1);
            }
            let object = entity(context, player);
            let flags = object.behavior_flags().read(context) & !0x10;
            object.behavior_flags().write(context, flags);
            if object.frame_phase().read(context) != 0 {
                object.frame_duration().write(context, 1);
            }
            call(
                context,
                ids::UPDATE_ENTITY_VIEWPORT_SPRITES,
                &[index as u32],
            )?;
        } else {
            if inventories.at(index ^ 1).read(context) == 0 {
                let x = ram::g_camera1_pending_x(revision).read(context);
                ram::g_camera2_pending_x(revision).write(context, x);
                let y = ram::g_camera1_pending_y(revision).read(context);
                ram::g_camera2_pending_y(revision).write(context, y);
            }
            inventories.at(index).write(context, 0);
            call(context, ids::WAIT_FOR_V_BLANK_WITH_GRAPHICS_PAUSED, &[])?;
            call(context, ids::DISABLE_BLANKING_GRAPHICS, &[])?;
            call(context, ids::DRAW_HUD, &[])?;
            if inventories.at(index ^ 1).read(context) == 0 {
                call(context, ids::FORCE_TEXT_BACKGROUND_COLOR_TO_BLACK, &[0])?;
            }
            call(context, ids::ENABLE_BLANKING_GRAPHICS, &[])?;
        }
        call(
            context,
            ids::DISABLE_TEXT_FONT_ANIMATION_IF_MENUS_CLOSED,
            &[],
        )?;
    } else if ram::g_player_elevator_flags(revision)
        .at(index)
        .read(context)
        == 0
        || ram::g_elevator_departure_in_progress(revision).read(context) == 0
    {
        call(context, ids::DRAW_INVENTORY, &[index as u32, 1])?;
    }
    Ok(None)
}

/// G/src/ui/inventory.c:542 (`OpenPresent`).
/// Preserve the compiled caller's D2-D5/A2 save image and residual words before
/// dispatching present effects, including their original omitted-argument paths.
pub fn open_present(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x3c, 0x04))?;
    let player = arguments[0] as i16;
    context.set_data_register(
        2,
        (context.data_register(2) & 0xffff0000) | u32::from(player as u16),
    );
    context.set_address_register(2, ram::g_player_inventory(context.revision()).address());
    let result = open_present_inner(context, arguments);
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

fn open_present_inner(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let player = i32::from(arguments[0] as i16);
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
        let site = CallSite::relative(ids::OPEN_PRESENT, revision, 0x60);
        present = call_at(context, ids::SELECT_RANDOM_PRESENT, &[], site)? as i8;
        context.set_data_register(
            4,
            (context.data_register(4) & 0xffffff00) | u32::from(present as u8),
        );
    }
    if present == -1
        || call_at(
            context,
            ids::OPEN_SELECTED_PRESENT,
            &[player as u32, i32::from(present) as u32],
            CallSite::relative(ids::OPEN_PRESENT, revision, 0x82),
        )? != 0
    {
        call_at(
            context,
            ids::PLAY_SOUND,
            &[26, 50],
            CallSite::relative(ids::OPEN_PRESENT, revision, 0x96),
        )?;
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
    Ok(Some(i32::from(present).wrapping_add(1) as u32))
}

/// G/src/ui/inventory.c:589 (`InventoryBuySelectedMailOrderPresent`).
pub fn inventory_buy_selected_mail_order_present(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let player = i32::from(arguments[0] as i16);
    let inventory = ram::g_player_inventory(revision).at(player);
    let mut slot = 0_i8;
    while slot < 16 && inventory.at(i32::from(slot)).read(context) != -1 {
        slot = slot.wrapping_add(1);
    }
    let selection = i32::from(
        ram::g_player_inventory_selection_row(revision)
            .at(player)
            .read(context),
    );
    let bucks = ram::g_player_bucks(revision).at(player);
    if slot < 16
        && bucks.read(context)
            >= ram::g_player_mailbox_prices(revision)
                .at(player)
                .at(selection)
                .read(context)
    {
        let price = ram::g_player_mailbox_prices(revision)
            .at(player)
            .at(selection)
            .read(context);
        let next = bucks.read(context).wrapping_sub(price);
        bucks.write(context, next);
        let present = ram::g_player_mailbox_items(revision)
            .at(player)
            .at(selection)
            .read(context);
        inventory.at(i32::from(slot)).write(context, present);
        let text = literal(context, "MailOrderSuccessStr")?;
        call(context, ids::GENERATE_SPEECH, &[player as u32, text, 1])?;
    } else {
        call(context, ids::PLAY_SOUND, &[26, 50])?;
        if slot == 16 {
            call(context, ids::START_DIALOGUE, &[25, player as u32])?;
        } else {
            let text = literal(context, "MailOrderFailureStr")?;
            call(context, ids::GENERATE_SPEECH, &[player as u32, text, 1])?;
        }
    }
    Ok(None)
}

/// G/src/ui/inventory.c:327 (`CheckIfOpenForBoth`).
pub fn check_if_open_for_both(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x0c, 0))?;
    let result = check_if_open_for_both_inner(context, arguments);
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
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let opening = Address::<i16>::new(arguments[0]);
    if opening.read(context) == 0 && opening.offset(1).read(context) == 0 {
        return Ok(None);
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
    let mut present = call_at(
        context,
        ids::OPEN_PRESENT,
        &[player as u32],
        CallSite::relative(ids::CHECK_IF_OPEN_FOR_BOTH, revision, 0x2a),
    )? as i16;
    context.set_data_register(
        2,
        (context.data_register(2) & 0xffff0000) | u32::from(present as u16),
    );
    if present != 0 && ram::g_player_lives(revision).at(player ^ 1).read(context) >= 0 {
        if call_at(
            context,
            ids::PLAYERS_REQUIRE_SEPARATE_VIEWS,
            &[],
            CallSite::relative(ids::CHECK_IF_OPEN_FOR_BOTH, revision, 0x50),
        )? == 0
        {
            present = present.wrapping_sub(1);
            context.set_data_register(
                2,
                (context.data_register(2) & 0xffff0000) | u32::from(present as u16),
            );
            if ram::g_player_inventory_open(revision)
                .at(player ^ 1)
                .read(context)
                != 0
            {
                call_at(
                    context,
                    ids::TOGGLE_INVENTORY,
                    &[(player ^ 1) as u32],
                    CallSite::relative(ids::CHECK_IF_OPEN_FOR_BOTH, revision, 0x7a),
                )?;
            }
            if ![8, 15, 12, 19, 23, 25].contains(&present)
                && call_at(
                    context,
                    ids::OPEN_SELECTED_PRESENT,
                    &[(player ^ 1) as u32, i32::from(present) as u32],
                    CallSite::relative(ids::CHECK_IF_OPEN_FOR_BOTH, revision, 0xc0),
                )? == 0
                && present == 16
            {
                let text = tje_mem::rom_symbol(revision, "ThanksALotStr")
                    .map(|symbol| symbol.address)
                    .ok_or_else(|| {
                        crate::Error::Platform("missing UI ThanksALotStr ROM symbol".into())
                    })?;
                call_at(
                    context,
                    ids::GENERATE_SPEECH,
                    &[(player ^ 1) as u32, text, 1],
                    CallSite::relative(ids::CHECK_IF_OPEN_FOR_BOTH, revision, 0xe6),
                )?;
            }
        } else if ram::g_player_inventory_open(revision)
            .at(player ^ 1)
            .read(context)
            != 0
        {
            call_at(
                context,
                ids::INVENTORY_REFRESH_CONTENTS_AND_WAIT,
                &[(player ^ 1) as u32],
                CallSite::relative(ids::CHECK_IF_OPEN_FOR_BOTH, revision, 0x10e),
            )?;
        }
    }
    Ok(None)
}
