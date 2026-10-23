use crate::{
    Context, FunctionId, Result, call, call_at,
    call_frame::{CallSite, FrameLayout, LocalFrame, OutgoingArguments},
    ids,
    ui::draws::literal,
};
use tje_mem::{Address, MenuItem, ram, rom};

/// G/src/ui/main_menu.c:31 (`NewGameMainMenu`).
/// c_main_menu.s:4-5 saves A2-A4 and reads the missing incoming short slot.
pub fn new_game_main_menu(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0, 0x1c))?;
    let incoming_selection = frame.argument_word(0).read(context) as u16;
    context.set_data_register(
        0,
        (context.data_register(0) & 0xffff0000) | u32::from(incoming_selection),
    );
    let revision = context.revision();
    context.set_address_register(2, ram::g_player_info(revision).address());
    context.set_address_register(3, ram::g_player_rank(revision).address());
    context.set_address_register(4, ram::g_player_lives(revision).address());
    'select_players: loop {
        let selection = call_at(
            context,
            ids::MENU_WHO_HANDLER,
            &[],
            CallSite::relative(ids::NEW_GAME_MAIN_MENU, revision, 0x1e),
        )? as i16;
        ram::g_controller_ports_swapped(revision).write(context, 0);
        match selection {
            0 => {
                ram::g_player_lives(revision).at(0).write(context, 0);
                ram::g_player_rank(revision).at(0).write(context, 0);
                ram::g_player_info(revision).at(0).level().write(context, 0);
                ram::g_player_lives(revision).at(1).write(context, 0);
                ram::g_player_rank(revision).at(1).write(context, 0);
                ram::g_player_info(revision).at(1).level().write(context, 0);
                ram::g_player_selection(revision).write(context, 2);
                ram::g_controller_ports_swapped(revision).write(context, 0);
            }
            1 => {
                ram::g_player_lives(revision).at(0).write(context, 0);
                ram::g_player_rank(revision).at(0).write(context, 0);
                ram::g_player_info(revision).at(0).level().write(context, 0);
                ram::g_player_lives(revision).at(1).write(context, -1);
                ram::g_player_rank(revision).at(1).write(context, 9);
                ram::g_player_info(revision)
                    .at(1)
                    .level()
                    .write(context, 99);
                ram::g_player_selection(revision).write(context, 0);
                let swapped = call_at(
                    context,
                    ids::GET_CTRL2_INPUT,
                    &[],
                    CallSite::relative(ids::NEW_GAME_MAIN_MENU, revision, 0x84),
                )? != 0;
                ram::g_controller_ports_swapped(revision).write(context, i8::from(swapped));
            }
            2 => {
                ram::g_player_lives(revision).at(0).write(context, -1);
                ram::g_player_rank(revision).at(0).write(context, 9);
                ram::g_player_info(revision)
                    .at(0)
                    .level()
                    .write(context, 99);
                ram::g_player_lives(revision).at(1).write(context, 0);
                ram::g_player_rank(revision).at(1).write(context, 0);
                ram::g_player_info(revision).at(1).level().write(context, 0);
                ram::g_player_selection(revision).write(context, 1);
                let swapped = call_at(
                    context,
                    ids::GET_CTRL1_INPUT,
                    &[],
                    CallSite::relative(ids::NEW_GAME_MAIN_MENU, revision, 0xb4),
                )? != 0;
                ram::g_controller_ports_swapped(revision).write(context, i8::from(swapped));
            }
            _ => {}
        }
        loop {
            match call_at(
                context,
                ids::MENU_WHAT_HANDLER,
                &[],
                CallSite::relative(ids::NEW_GAME_MAIN_MENU, revision, 0xcc),
            )? as i16
            {
                0 => break 'select_players,
                1 => {
                    let selection = call_at(
                        context,
                        ids::MENU_JAM_OUT_HANDLER,
                        &[],
                        CallSite::relative(ids::NEW_GAME_MAIN_MENU, revision, 0xf4),
                    )?;
                    call_at(
                        context,
                        ids::START_JAM_OUT,
                        &[selection],
                        CallSite::relative(ids::NEW_GAME_MAIN_MENU, revision, 0xfe),
                    )?;
                    continue 'select_players;
                }
                2 => {
                    call_at(
                        context,
                        ids::INTRO_VIDEO,
                        &[],
                        CallSite::relative(ids::NEW_GAME_MAIN_MENU, revision, 0x10a),
                    )?;
                    continue 'select_players;
                }
                3 => {
                    let swapped = call_at(
                        context,
                        ids::MENU_CONTROLLER_OPTIONS_HANDLER,
                        &[],
                        CallSite::relative(ids::NEW_GAME_MAIN_MENU, revision, 0x112),
                    )? as i8;
                    ram::g_controller_buttons_ac_swapped(revision).write(context, swapped);
                }
                4 => {
                    ram::g_world_type(revision).write(context, 1);
                    break 'select_players;
                }
                _ => {}
            }
        }
    }
    frame.leave(context)?;
    Ok(None)
}

/// G/src/ui/main_menu.c:101 (`ShowGameOverAndReset`).
/// c_main_menu.s:111-168: saved D2/A2 and retained argument areas remain live
/// through the nonreturning reset; no invented RTS or frame cleanup follows it.
pub fn show_game_over_and_reset(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let _frame = LocalFrame::enter(context, FrameLayout::movem(0, 4, 4))?;
    let revision = context.revision();
    let dispatcher = context.dispatcher();
    let caller = ids::SHOW_GAME_OVER_AND_RESET;
    let mode = ram::g_mode2_reg_softcopy(revision);
    context.set_address_register(2, mode.address());
    call_at(
        context,
        ids::DRAW_MENU_WINDOW_FRAME,
        &[0],
        CallSite::relative(caller, revision, 0x12),
    )?;
    context.write_vdp_command(0x57160003);
    let text = literal(context, "BogusGameOverStr")?;
    let text_argument = OutgoingArguments::push(context, &[text])?;
    text_argument.call(
        context,
        dispatcher,
        ids::TEXT_WRITE_STRING_TILES,
        CallSite::relative(caller, revision, 0x2a),
    )?;
    let background_argument = OutgoingArguments::push(context, &[1])?;
    background_argument.call(
        context,
        dispatcher,
        ids::FORCE_TEXT_BACKGROUND_COLOR_TO_BLACK,
        CallSite::relative(caller, revision, 0x34),
    )?;
    mode.write(context, mode.read(context) | 0x40);
    background_argument.release(context)?;
    text_argument.release(context)?;
    context.write_vdp_control(u16::from(mode.read(context)) | 0x8104);
    call_at(
        context,
        ids::ENABLE_TEXT_FONT_ANIMATION,
        &[],
        CallSite::relative(caller, revision, 0x50),
    )?;
    ram::g_frame_counter(revision).write(context, 0);
    loop {
        loop {
            let wait_argument = OutgoingArguments::push(context, &[1])?;
            wait_argument.call(
                context,
                dispatcher,
                ids::WAIT_FOR_NEXT_V_BLANK,
                CallSite::relative(caller, revision, 0x60),
            )?;
            call_at(
                context,
                ids::TEXT_ANIMATE_FONT_BANKS,
                &[8],
                CallSite::relative(caller, revision, 0x6a),
            )?;
            wait_argument.release(context)?;
            if ram::g_frame_counter(revision).read(context) > 600 {
                break;
            }
            // Matching compiler evaluates the second player's call first.
            let second = call_at(
                context,
                ids::READ_MAPPED_PLAYER_INPUT,
                &[1],
                CallSite::relative(caller, revision, 0x80),
            )?;
            context.set_data_register(2, second);
            let first = call_at(
                context,
                ids::READ_MAPPED_PLAYER_INPUT,
                &[0],
                CallSite::relative(caller, revision, 0x8c),
            )?;
            let input = (first | context.data_register(2)) & 0xf0;
            context.set_data_register(2, input);
            if input != 0 {
                break;
            }
        }
        let noise_arguments = OutgoingArguments::push(context, &[6, 99])?;
        noise_arguments.call(
            context,
            dispatcher,
            ids::PLAY_SOUND,
            CallSite::relative(caller, revision, 0xa6),
        )?;
        let silence_arguments = OutgoingArguments::push(context, &[0, 98])?;
        silence_arguments.call(
            context,
            dispatcher,
            ids::PLAY_SOUND,
            CallSite::relative(caller, revision, 0xb2),
        )?;
        call_at(
            context,
            ids::AUDIO_REQUEST_FM_CHANNEL_RELEASE,
            &[],
            CallSite::relative(caller, revision, 0xb8),
        )?;
        call_at(
            context,
            ids::MUSIC_REQUEST_TRACK_POINTER_RESET,
            &[],
            CallSite::relative(caller, revision, 0xbe),
        )?;
        mode.write(context, mode.read(context) & 0x3c);
        silence_arguments.release(context)?;
        noise_arguments.release(context)?;
        context.write_vdp_control(u16::from(mode.read(context)) | 0x8104);
        call_at(
            context,
            ids::CLEAR_COLOUR_RAM,
            &[],
            CallSite::relative(caller, revision, 0xdc),
        )?;
        call_at(
            context,
            ids::CLEAR_VERTICAL_SCROLL_RAM,
            &[],
            CallSite::relative(caller, revision, 0xe2),
        )?;
        call_at(
            context,
            ids::CLEAR_VIDEO_RAM,
            &[],
            CallSite::relative(caller, revision, 0xe8),
        )?;
        call_at(
            context,
            ids::RESTART_FROM_RESET_ENTRY,
            &[],
            CallSite::relative(caller, revision, 0xee),
        )?;
    }
}

// G/c_main_menu.s:178-195,201-218,224-241,247-264 in both revisions.
// The header argument remains allocated beneath the GeneralMenuHandler slots.
fn menu_handler(
    context: &mut dyn Context,
    caller: FunctionId,
    header: u32,
    items: u32,
    count: Address<i16>,
) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 4, 0))?;
    let dispatcher = context.dispatcher();
    let revision = context.revision();
    let first_return = if caller == ids::MENU_WHO_HANDLER {
        8
    } else {
        10
    };
    let header_argument = OutgoingArguments::push(context, &[header])?;
    header_argument.call(
        context,
        dispatcher,
        ids::ADD_MENU_HEADER,
        CallSite::relative(caller, revision, first_return),
    )?;
    let count = i32::from(count.read(context)) as u32;
    let selection = call_at(
        context,
        ids::GENERAL_MENU_HANDLER,
        &[2, items, count],
        CallSite::relative(caller, revision, first_return + 26),
    )?;
    context.set_data_register(2, selection);
    call_at(
        context,
        ids::REMOVE_MENU_HEADER,
        &[],
        CallSite::relative(caller, revision, first_return + 38),
    )?;
    header_argument.release(context)?;
    let selection = context.data_register(2);
    frame.leave(context)?;
    Ok(Some(selection))
}

/// G/src/ui/main_menu.c:131 (`MenuWhoHandler`).
pub fn menu_who_handler(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    menu_handler(
        context,
        ids::MENU_WHO_HANDLER,
        0,
        rom::menu_who_items(revision).address(),
        rom::menu_who_item_count(revision),
    )
}

/// G/src/ui/main_menu.c:141 (`MenuControllerOptionsHandler`).
pub fn menu_controller_options_handler(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    menu_handler(
        context,
        ids::MENU_CONTROLLER_OPTIONS_HANDLER,
        3,
        rom::menu_controller_options_items(revision).address(),
        rom::menu_controller_options_item_count(revision),
    )
}

/// G/src/ui/main_menu.c:151 (`MenuWhatHandler`).
pub fn menu_what_handler(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    menu_handler(
        context,
        ids::MENU_WHAT_HANDLER,
        1,
        rom::menu_what_items(revision).address(),
        rom::menu_what_item_count(revision),
    )
}

/// G/src/ui/main_menu.c:161 (`MenuJamOutHandler`).
pub fn menu_jam_out_handler(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    menu_handler(
        context,
        ids::MENU_JAM_OUT_HANDLER,
        2,
        rom::menu_jam_out_items(revision).address(),
        rom::menu_jam_out_item_count(revision),
    )
}

fn controller_inputs(context: &mut dyn Context) -> Result<(u32, u32)> {
    let second = call(context, ids::GET_CTRL2_INPUT, &[])?;
    let first = call(context, ids::GET_CTRL1_INPUT, &[])?;
    Ok((first, second))
}

/// G/src/ui/main_menu.c:210 (`GeneralMenuHandler`).
/// Input edge tests and controller calls follow c_main_menu.s:400-429 order.
pub fn general_menu_handler(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let cursor_column = i32::from(arguments[0] as i16) as u32;
    let items = Address::<MenuItem>::new(arguments[1]);
    let item_count = arguments[2] as i16;
    let mut previous_input = 0_u8;
    call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    let mode = ram::g_mode2_reg_softcopy(revision);
    mode.write(context, mode.read(context) & 0x3c);
    context.write_vdp_control(u16::from(mode.read(context)) | 0x8104);
    call(context, ids::DRAW_STARFIELD_PLANE, &[])?;
    call(context, ids::DRAW_MENU_WINDOW_FRAME, &[0])?;
    call(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_game_line1(revision).address(), 0],
    )?;
    call(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_game_line4(revision).address(), 3],
    )?;
    call(context, ids::FORCE_TEXT_BACKGROUND_COLOR_TO_BLACK, &[1])?;
    call(
        context,
        ids::MENU_DRAW_TEXT_ITEMS,
        &[items.address(), i32::from(item_count) as u32],
    )?;
    call(context, ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, &[])?;
    call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    call(context, ids::UPLOAD_QUEUED_SPRITE_GRAPHICS, &[0, 0])?;
    mode.write(context, mode.read(context) | 0x40);
    context.write_vdp_control(u16::from(mode.read(context)) | 0x8104);
    call(context, ids::ENABLE_TEXT_FONT_ANIMATION, &[])?;
    loop {
        let (first, second) = controller_inputs(context)?;
        if (first | second) & 0xf0 == 0 {
            break;
        }
        call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
        call(context, ids::TEXT_ANIMATE_FONT_BANKS, &[8])?;
    }
    call(context, ids::CREATE_MENU_CURSOR_ENTITY, &[])?;
    let mut selected_item = 0_i16;
    let mut selected = items;
    loop {
        ram::g_sprite_graphics_upload_pending(revision).write(context, 0);
        call(context, ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, &[])?;
        call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
        call(context, ids::UPLOAD_QUEUED_SPRITE_GRAPHICS, &[0, 0])?;
        call(context, ids::TEXT_ANIMATE_FONT_BANKS, &[8])?;
        call(context, ids::SELECT_VIEWPORT_SPRITE_CHAIN, &[0])?;
        call(context, ids::GET_RANDOM_NUMBER, &[])?;
        let (first, second) = controller_inputs(context)?;
        let input = (first as u8) | (second as u8);
        if input != previous_input {
            if input & 0xf0 != 0 {
                if items.address() != rom::menu_who_items(revision).address()
                    || selected.selection().read(context) != 0
                    || call(context, ids::ARE_BOTH_CONTROLLERS_CONNECTED, &[])? != 0
                {
                    break;
                }
                call(context, ids::PLAY_SOUND, &[26, 50])?;
                context.write_vdp_command(0x5c040003);
                let text = literal(context, "NeedTwoControllersStr")?;
                call(context, ids::TEXT_WRITE_STRING_TILES, &[text])?;
            } else if input & 1 != 0 {
                selected_item = selected_item.wrapping_sub(1);
                call(context, ids::PLAY_SOUND, &[7, 50])?;
                if selected_item < 0
                    || items
                        .offset(i32::from(selected_item))
                        .selection()
                        .read(context)
                        < 0
                {
                    selected_item = selected_item.wrapping_add(1);
                }
            } else if input & 2 != 0 {
                call(context, ids::PLAY_SOUND, &[7, 50])?;
                selected_item = selected_item.wrapping_add(1);
                if selected_item >= item_count
                    || items
                        .offset(i32::from(selected_item))
                        .selection()
                        .read(context)
                        < 0
                {
                    selected_item = selected_item.wrapping_sub(1);
                }
            }
            previous_input = input;
        }
        selected = items.offset(i32::from(selected_item));
        let row = u32::from(selected.y().read(context));
        call(
            context,
            ids::POSITION_MENU_CURSOR_ENTITY,
            &[cursor_column, row],
        )?;
    }
    call(
        context,
        ids::DISABLE_TEXT_FONT_ANIMATION_IF_MENUS_CLOSED,
        &[],
    )?;
    mode.write(context, mode.read(context) & 0x3c);
    context.write_vdp_control(u16::from(mode.read(context)) | 0x8104);
    call(context, ids::RELEASE_MENU_CURSOR_ENTITY, &[])?;
    call(context, ids::FORCE_TEXT_BACKGROUND_COLOR_TO_BLACK, &[0])?;
    context.write_vdp_control(0x9100);
    context.write_vdp_control(0x9200);
    Ok(Some(i32::from(selected.selection().read(context)) as u32))
}
