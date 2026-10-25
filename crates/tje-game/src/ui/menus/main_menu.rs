use crate::ui::hud_all::frame as stack;
use crate::{
    Context, FunctionId, Result, call_at, call_at_void,
    call_frame::{CallSite, FrameLayout, LocalFrame, OutgoingArguments},
    ids,
    ui::draws::literal,
};
use tje_mem::{Address, MenuItem, ram, rom};

/// G/src/ui/main_menu.c:31 (`NewGameMainMenu`).
/// c_main_menu.s:4-5 saves A2-A4 and reads the missing incoming short slot.
pub fn new_game_main_menu(context: &mut dyn Context) -> Result<Option<u32>> {
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
        clear_byte(context, ram::g_controller_ports_swapped(revision))?;
        match selection {
            0 => {
                clear_byte(context, ram::g_player_lives(revision).at(0))?;
                clear_byte(context, ram::g_player_rank(revision).at(0))?;
                clear_byte(context, ram::g_player_info(revision).at(0).level())?;
                clear_byte(context, ram::g_player_lives(revision).at(1))?;
                clear_byte(context, ram::g_player_rank(revision).at(1))?;
                clear_byte(context, ram::g_player_info(revision).at(1).level())?;
                ram::g_player_selection(revision).write(context, 2);
                clear_byte(context, ram::g_controller_ports_swapped(revision))?;
            }
            1 => {
                clear_byte(context, ram::g_player_lives(revision).at(0))?;
                clear_byte(context, ram::g_player_rank(revision).at(0))?;
                clear_byte(context, ram::g_player_info(revision).at(0).level())?;
                ram::g_player_lives(revision).at(1).write(context, -1);
                ram::g_player_rank(revision).at(1).write(context, 9);
                ram::g_player_info(revision)
                    .at(1)
                    .level()
                    .write(context, 99);
                clear_byte(context, ram::g_player_selection(revision))?;
                let swapped = call_at(
                    context,
                    ids::GET_CTRL2_INPUT,
                    &[],
                    CallSite::relative(ids::NEW_GAME_MAIN_MENU, revision, 0x84),
                )? != 0;
                if swapped {
                    ram::g_controller_ports_swapped(revision).write(context, 1);
                } else {
                    clear_byte(context, ram::g_controller_ports_swapped(revision))?;
                }
            }
            2 => {
                ram::g_player_lives(revision).at(0).write(context, -1);
                ram::g_player_rank(revision).at(0).write(context, 9);
                ram::g_player_info(revision)
                    .at(0)
                    .level()
                    .write(context, 99);
                clear_byte(context, ram::g_player_lives(revision).at(1))?;
                clear_byte(context, ram::g_player_rank(revision).at(1))?;
                clear_byte(context, ram::g_player_info(revision).at(1).level())?;
                ram::g_player_selection(revision).write(context, 1);
                let swapped = call_at(
                    context,
                    ids::GET_CTRL1_INPUT,
                    &[],
                    CallSite::relative(ids::NEW_GAME_MAIN_MENU, revision, 0xb4),
                )? != 0;
                if swapped {
                    ram::g_controller_ports_swapped(revision).write(context, 1);
                } else {
                    clear_byte(context, ram::g_controller_ports_swapped(revision))?;
                }
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
                    stack::push_move(context, selection)?;
                    stack::call_void(context, ids::NEW_GAME_MAIN_MENU, ids::START_JAM_OUT, 0xfe)?;
                    stack::release(context, 4)?;
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
pub fn show_game_over_and_reset(context: &mut dyn Context) -> Result<Option<u32>> {
    let _frame = LocalFrame::enter(context, FrameLayout::movem(0, 4, 4))?;
    let revision = context.revision();
    let dispatcher = context.dispatcher();
    let caller = ids::SHOW_GAME_OVER_AND_RESET;
    let mode = ram::g_mode2_reg_softcopy(revision);
    context.set_address_register(2, mode.address());
    let window_argument = cleared_arguments(context, 1)?;
    window_argument.call_void(
        context,
        dispatcher,
        ids::DRAW_MENU_WINDOW_FRAME,
        CallSite::relative(caller, revision, 0x12),
    )?;
    window_argument.release(context)?;
    context.write_vdp_command(0x57160003);
    let text = literal(context, "BogusGameOverStr")?;
    let text_argument = OutgoingArguments::push(context, &[text])?;
    text_argument.call_void(
        context,
        dispatcher,
        ids::TEXT_WRITE_STRING_TILES,
        CallSite::relative(caller, revision, 0x2a),
    )?;
    let background_argument = OutgoingArguments::push(context, &[1])?;
    background_argument.call_void(
        context,
        dispatcher,
        ids::FORCE_TEXT_BACKGROUND_COLOR_TO_BLACK,
        CallSite::relative(caller, revision, 0x34),
    )?;
    mode.write(context, mode.read(context) | 0x40);
    background_argument.release(context)?;
    text_argument.release(context)?;
    context.write_vdp_control(u16::from(mode.read(context)) | 0x8104);
    call_at_void(
        context,
        ids::ENABLE_TEXT_FONT_ANIMATION,
        &[],
        CallSite::relative(caller, revision, 0x50),
    )?;
    context.discard_u16(ram::g_frame_counter(revision).address())?;
    ram::g_frame_counter(revision).write(context, 0);
    loop {
        loop {
            let wait_argument = OutgoingArguments::push(context, &[1])?;
            wait_argument.call_void(
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
        noise_arguments.call_void(
            context,
            dispatcher,
            ids::PLAY_SOUND,
            CallSite::relative(caller, revision, 0xa6),
        )?;
        stack::push_pea(context, 98)?;
        stack::push_clear(context)?;
        let silence_arguments = OutgoingArguments::existing(context, 2)?;
        silence_arguments.call_void(
            context,
            dispatcher,
            ids::PLAY_SOUND,
            CallSite::relative(caller, revision, 0xb2),
        )?;
        call_at_void(
            context,
            ids::AUDIO_REQUEST_FM_CHANNEL_RELEASE,
            &[],
            CallSite::relative(caller, revision, 0xb8),
        )?;
        call_at_void(
            context,
            ids::MUSIC_REQUEST_TRACK_POINTER_RESET,
            &[],
            CallSite::relative(caller, revision, 0xbe),
        )?;
        mode.write(context, mode.read(context) & 0x3c);
        silence_arguments.release(context)?;
        noise_arguments.release(context)?;
        context.write_vdp_control(u16::from(mode.read(context)) | 0x8104);
        call_at_void(
            context,
            ids::CLEAR_COLOUR_RAM,
            &[],
            CallSite::relative(caller, revision, 0xdc),
        )?;
        call_at_void(
            context,
            ids::CLEAR_VERTICAL_SCROLL_RAM,
            &[],
            CallSite::relative(caller, revision, 0xe2),
        )?;
        call_at_void(
            context,
            ids::CLEAR_VIDEO_RAM,
            &[],
            CallSite::relative(caller, revision, 0xe8),
        )?;
        call_at_void(
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
    if header == 0 {
        stack::push_clear(context)?;
    } else {
        stack::push_pea(context, header)?;
    }
    let header_argument = OutgoingArguments::existing(context, 1)?;
    header_argument.call_void(
        context,
        dispatcher,
        ids::ADD_MENU_HEADER,
        CallSite::relative(caller, revision, first_return),
    )?;
    let count = i32::from(count.read(context)) as u32;
    context.set_data_register(0, count);
    stack::push_move(context, count)?;
    stack::push_pea(context, items)?;
    stack::push_pea(context, 2)?;
    let general_arguments = OutgoingArguments::existing(context, 3)?;
    let selection = general_arguments.call(
        context,
        dispatcher,
        ids::GENERAL_MENU_HANDLER,
        CallSite::relative(caller, revision, first_return + 26),
    )?;
    general_arguments.release(context)?;
    context.set_data_register(2, selection);
    call_at_void(
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
pub fn menu_who_handler(context: &mut dyn Context) -> Result<Option<u32>> {
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
pub fn menu_controller_options_handler(context: &mut dyn Context) -> Result<Option<u32>> {
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
pub fn menu_what_handler(context: &mut dyn Context) -> Result<Option<u32>> {
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
pub fn menu_jam_out_handler(context: &mut dyn Context) -> Result<Option<u32>> {
    let revision = context.revision();
    menu_handler(
        context,
        ids::MENU_JAM_OUT_HANDLER,
        2,
        rom::menu_jam_out_items(revision).address(),
        rom::menu_jam_out_item_count(revision),
    )
}

fn set_word(context: &mut dyn Context, index: usize, value: u16) {
    let high = context.data_register(index) & 0xffff0000;
    context.set_data_register(index, high | u32::from(value));
}

fn set_byte(context: &mut dyn Context, index: usize, value: u8) {
    let high = context.data_register(index) & 0xffffff00;
    context.set_data_register(index, high | u32::from(value));
}

fn cleared_arguments(context: &mut dyn Context, slots: u32) -> Result<OutgoingArguments> {
    for _ in 0..slots {
        stack::push_clear(context)?;
    }
    OutgoingArguments::existing(context, slots)
}

fn clear_byte(context: &mut dyn Context, address: Address<i8>) -> Result<()> {
    context.discard_u8(address.address())?;
    address.write(context, 0);
    Ok(())
}

fn controller_inputs(
    context: &mut dyn Context,
    second_return: u32,
    first_return: u32,
    byte: bool,
) -> Result<u32> {
    let revision = context.revision();
    let second = call_at(
        context,
        ids::GET_CTRL2_INPUT,
        &[],
        CallSite::relative(ids::GENERAL_MENU_HANDLER, revision, second_return),
    )?;
    if byte {
        set_byte(context, 2, second as u8);
    } else {
        context.set_data_register(2, second);
    }
    let first = call_at(
        context,
        ids::GET_CTRL1_INPUT,
        &[],
        CallSite::relative(ids::GENERAL_MENU_HANDLER, revision, first_return),
    )?;
    let input = context.data_register(2) | first;
    if byte {
        set_byte(context, 2, input as u8);
    } else {
        context.set_data_register(2, input);
    }
    Ok(if byte { u32::from(input as u8) } else { input })
}

/// G/src/ui/main_menu.c:210 (`GeneralMenuHandler`).
/// Input edge tests and controller calls follow c_main_menu.s:400-429 order.
pub fn general_menu_handler(context: &mut dyn Context) -> Result<Option<u32>> {
    // c_main_menu.s:341-348: lazy argument loads and the causal A6-4 pointer.
    let frame = LocalFrame::enter(context, FrameLayout::link(4, 0x7c, 0x1c))?;
    let cursor_column = i32::from(frame.argument_word(0).read(context)) as u32;
    set_word(context, 4, cursor_column as u16);
    let items = Address::<MenuItem>::new(frame.argument_slot(1).read(context));
    context.set_address_register(2, items.address());
    let item_count = frame.argument_word(2).read(context);
    set_word(context, 3, item_count as u16);
    set_byte(context, 5, 0);
    let revision = context.revision();
    let caller = ids::GENERAL_MENU_HANDLER;
    let dispatcher = context.dispatcher();
    let site = |offset| CallSite::relative(caller, revision, offset);
    let mode = ram::g_mode2_reg_softcopy(revision);
    frame.local::<u32>(-4).write(context, mode.address());
    context.set_address_register(3, ids::WAIT_FOR_NEXT_V_BLANK.metadata().address(revision));

    let first_wait = OutgoingArguments::push(context, &[1])?;
    first_wait.call_void(context, dispatcher, ids::WAIT_FOR_NEXT_V_BLANK, site(0x2a))?;
    mode.write(context, mode.read(context) & 0x3c);
    let command = u16::from(mode.read(context)) | 0x8104;
    context.set_data_register(6, u32::from(command));
    first_wait.release(context)?;
    context.write_vdp_control(command);
    call_at_void(context, ids::DRAW_STARFIELD_PLANE, &[], site(0x54))?;

    // c_main_menu.s:361-390 retains all eleven outgoing slots until both
    // palette uploads, text/sprite work and the display-enable read complete.
    let window = cleared_arguments(context, 1)?;
    window.call_void(context, dispatcher, ids::DRAW_MENU_WINDOW_FRAME, site(0x5c))?;
    stack::push_clear(context)?;
    stack::push_pea(context, rom::palette_game_line1(revision).address())?;
    let palette1 = OutgoingArguments::existing(context, 2)?;
    palette1.call_void(
        context,
        dispatcher,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        site(0x6a),
    )?;
    let palette4 =
        OutgoingArguments::push(context, &[rom::palette_game_line4(revision).address(), 3])?;
    palette4.call_void(
        context,
        dispatcher,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        site(0x7a),
    )?;
    let background = OutgoingArguments::push(context, &[1])?;
    background.call_void(
        context,
        dispatcher,
        ids::FORCE_TEXT_BACKGROUND_COLOR_TO_BLACK,
        site(0x84),
    )?;
    let count = i32::from(item_count) as u32;
    context.set_data_register(2, count);
    stack::push_move(context, count)?;
    stack::push_move(context, items.address())?;
    let text = OutgoingArguments::existing(context, 2)?;
    text.call_void(context, dispatcher, ids::MENU_DRAW_TEXT_ITEMS, site(0x90))?;
    call_at_void(
        context,
        ids::UPDATE_AND_QUEUE_ENTITY_SPRITES,
        &[],
        site(0x98),
    )?;
    let wait = OutgoingArguments::push(context, &[1])?;
    wait.call_void(context, dispatcher, ids::WAIT_FOR_NEXT_V_BLANK, site(0x9e))?;
    let upload = cleared_arguments(context, 2)?;
    upload.call_void(
        context,
        dispatcher,
        ids::UPLOAD_QUEUED_SPRITE_GRAPHICS,
        site(0xa8),
    )?;
    mode.write(context, mode.read(context) | 0x40);
    let command = u16::from(mode.read(context)) | 0x8104;
    context.set_data_register(6, u32::from(command));
    upload.release(context)?;
    wait.release(context)?;
    text.release(context)?;
    background.release(context)?;
    palette4.release(context)?;
    palette1.release(context)?;
    window.release(context)?;
    context.write_vdp_control(command);
    call_at_void(context, ids::ENABLE_TEXT_FONT_ANIMATION, &[], site(0xd4))?;
    loop {
        let input = controller_inputs(context, 0xee, 0xf6, false)? & 0xf0;
        context.set_data_register(2, input);
        if input == 0 {
            break;
        }
        let wait = OutgoingArguments::push(context, &[1])?;
        wait.call_void(context, dispatcher, ids::WAIT_FOR_NEXT_V_BLANK, site(0xdc))?;
        let font = OutgoingArguments::push(context, &[8])?;
        font.call(
            context,
            dispatcher,
            ids::TEXT_ANIMATE_FONT_BANKS,
            site(0xe6),
        )?;
        font.release(context)?;
        wait.release(context)?;
    }
    call_at_void(context, ids::CREATE_MENU_CURSOR_ENTITY, &[], site(0x104))?;
    let mut previous_input = 0_u8;
    let mut selected_item = 0_i16;
    set_word(context, 6, 0);
    let mut selected = items;
    context.set_address_register(4, selected.address());
    loop {
        clear_byte(
            context,
            ram::g_sprite_graphics_upload_pending(revision).cast(),
        )?;
        call_at_void(
            context,
            ids::UPDATE_AND_QUEUE_ENTITY_SPRITES,
            &[],
            site(0x118),
        )?;
        let wait = OutgoingArguments::push(context, &[1])?;
        wait.call_void(context, dispatcher, ids::WAIT_FOR_NEXT_V_BLANK, site(0x11e))?;
        let upload = cleared_arguments(context, 2)?;
        upload.call_void(
            context,
            dispatcher,
            ids::UPLOAD_QUEUED_SPRITE_GRAPHICS,
            site(0x128),
        )?;
        let font = OutgoingArguments::push(context, &[8])?;
        font.call(
            context,
            dispatcher,
            ids::TEXT_ANIMATE_FONT_BANKS,
            site(0x132),
        )?;
        let viewport = cleared_arguments(context, 1)?;
        viewport.call_void(
            context,
            dispatcher,
            ids::SELECT_VIEWPORT_SPRITE_CHAIN,
            site(0x13a),
        )?;
        call_at(context, ids::GET_RANDOM_NUMBER, &[], site(0x140))?;
        let input = controller_inputs(context, 0x146, 0x14e, true)? as u8;
        viewport.release(context)?;
        font.release(context)?;
        upload.release(context)?;
        wait.release(context)?;
        if input != previous_input {
            if input & 0xf0 != 0 {
                if items.address() != rom::menu_who_items(revision).address()
                    || selected.selection().read(context) != 0
                    || call_at(
                        context,
                        ids::ARE_BOTH_CONTROLLERS_CONNECTED,
                        &[],
                        site(0x184),
                    )? != 0
                {
                    break;
                }
                call_at_void(context, ids::PLAY_SOUND, &[26, 50], site(0x198))?;
                context.write_vdp_command(0x5c040003);
                let text = literal(context, "NeedTwoControllersStr")?;
                call_at_void(context, ids::TEXT_WRITE_STRING_TILES, &[text], site(0x1b0))?;
            } else if input & 1 != 0 {
                selected_item = selected_item.wrapping_sub(1);
                set_word(context, 6, selected_item as u16);
                call_at_void(context, ids::PLAY_SOUND, &[7, 50], site(0x1ca))?;
                if selected_item < 0
                    || items
                        .offset(i32::from(selected_item))
                        .selection()
                        .read(context)
                        < 0
                {
                    selected_item = selected_item.wrapping_add(1);
                    set_word(context, 6, selected_item as u16);
                }
            } else if input & 2 != 0 {
                call_at_void(context, ids::PLAY_SOUND, &[7, 50], site(0x1f8))?;
                selected_item = selected_item.wrapping_add(1);
                set_word(context, 6, selected_item as u16);
                if selected_item >= item_count
                    || items
                        .offset(i32::from(selected_item))
                        .selection()
                        .read(context)
                        < 0
                {
                    selected_item = selected_item.wrapping_sub(1);
                    set_word(context, 6, selected_item as u16);
                }
            }
            previous_input = input;
            set_byte(context, 5, input);
        }
        selected = items.offset(i32::from(selected_item));
        context.set_data_register(2, (i32::from(selected_item) as u32).wrapping_mul(8));
        context.set_address_register(4, selected.address());
        let row = u32::from(selected.y().read(context));
        stack::push_move(context, row)?;
        stack::push_move(context, cursor_column)?;
        stack::call_void(context, caller, ids::POSITION_MENU_CURSOR_ENTITY, 0x234)?;
        stack::release(context, 8)?;
    }
    call_at_void(
        context,
        ids::DISABLE_TEXT_FONT_ANIMATION_IF_MENUS_CLOSED,
        &[],
        site(0x242),
    )?;
    mode.write(context, mode.read(context) & 0x3c);
    let command = u16::from(mode.read(context)) | 0x8104;
    context.set_data_register(2, u32::from(command));
    context.write_vdp_control(command);
    call_at_void(context, ids::RELEASE_MENU_CURSOR_ENTITY, &[], site(0x268))?;
    let background = cleared_arguments(context, 1)?;
    background.call_void(
        context,
        dispatcher,
        ids::FORCE_TEXT_BACKGROUND_COLOR_TO_BLACK,
        site(0x272),
    )?;
    background.release(context)?;
    context.write_vdp_control(0x9100);
    context.write_vdp_control(0x9200);
    let selection = i32::from(selected.selection().read(context)) as u32;
    context.set_address_register(0, selection);
    frame.leave(context)?;
    Ok(Some(selection))
}

#[cfg(test)]
mod tests;
