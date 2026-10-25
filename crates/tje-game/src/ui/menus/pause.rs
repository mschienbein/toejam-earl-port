use crate::ui::hud_all::frame as stack;
use crate::{
    Context, FunctionId, Result, call_at_void,
    call_frame::{CallSite, FrameLayout, LocalFrame},
    ids,
    ui::entities::entity,
};
use tje_mem::{ram, rom};

fn set_word(context: &mut dyn Context, index: usize, value: u16) {
    let residual = context.data_register(index) & 0xffff0000;
    context.set_data_register(index, residual | u32::from(value));
}

fn read_input(
    context: &mut dyn Context,
    caller: FunctionId,
    player: u32,
    offset: u32,
) -> Result<u32> {
    if caller == ids::GAME_PAUSED {
        if player == 0 {
            stack::push_clear(context)?;
        } else {
            stack::push_pea(context, player)?;
        }
    } else {
        stack::push_move(context, player)?;
    }
    let input = stack::call(context, caller, ids::READ_MAPPED_PLAYER_INPUT, offset)?;
    stack::release(context, 4)?;
    Ok(input)
}

// G/build/{rev02,rev00}/asm/c_pause.s:276-323. The comparison re-reads
// the candidate after the minimum; neither its TST value nor the map byte
// may be cached across those distinct source transactions.
fn apply_ship_piece_hint(context: &mut dyn Context) -> Result<i16> {
    let revision = context.revision();
    let assignments = ram::g_ship_piece_level_assignments(revision);
    let mut entry_number = 0_i16;
    loop {
        if assignments.at(i32::from(entry_number)).read(context) != 0 {
            break;
        }
        entry_number += 1;
        if entry_number >= 10 {
            break;
        }
    }
    let mut minimum_entry = entry_number;
    entry_number += 1;
    while entry_number < 10 {
        if assignments.at(i32::from(entry_number)).read(context) != 0 {
            let minimum = assignments.at(i32::from(minimum_entry)).read(context);
            let candidate = assignments.at(i32::from(entry_number)).read(context);
            if candidate < minimum {
                minimum_entry = entry_number;
            }
        }
        entry_number += 1;
    }
    for entry in 0..10 {
        if entry != i32::from(minimum_entry) {
            context.discard_u8(assignments.at(entry).address())?;
            assignments.at(entry).write(context, 0);
        }
    }
    let level = ram::g_player1_map_level(revision).read(context);
    if level != assignments.at(i32::from(minimum_entry)).read(context) {
        let next_level = ram::g_player1_map_level(revision)
            .read(context)
            .wrapping_add(1);
        assignments
            .at(i32::from(minimum_entry))
            .write(context, next_level);
    }
    Ok(minimum_entry)
}

/// G/src/ui/pause.c:23 (`RunFinalLevelPauseMenu`).
/// Busy input loops intentionally retain their call/read order. Approval requires
/// the reviewed input-poll-loop contract, including reset's nonreturning exit.
pub fn run_final_level_pause_menu(context: &mut dyn Context) -> Result<Option<u32>> {
    // c_pause.s:4-8, identical in both revisions.
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x0c, 0x0c))?;
    let revision = context.revision();
    let player = i32::from(frame.argument_word(0).read(context)) as u32;
    let caller = ids::RUN_FINAL_LEVEL_PAUSE_MENU;
    set_word(context, 2, player as u16);
    set_word(context, 3, 0);
    context.set_address_register(
        2,
        ids::READ_MAPPED_PLAYER_INPUT.metadata().address(revision),
    );
    context.set_address_register(3, ids::DRAW_PAUSE_CHOICE_PANEL.metadata().address(revision));
    let mut reset_selected = false;
    // c_pause.s:9-23 keeps seven outgoing slots until the music request.
    stack::push_clear(context)?;
    stack::call_void(context, caller, ids::DRAW_PAUSE_CHOICE_PANEL, 0x1a)?;
    stack::push_pea(context, 99)?;
    stack::push_pea(context, 6)?;
    stack::call_void(context, caller, ids::PLAY_SOUND, 0x28)?;
    stack::push_pea(context, 1)?;
    stack::call_void(context, caller, ids::WAIT_FOR_NEXT_V_BLANK, 0x32)?;
    stack::push_pea(context, 99)?;
    stack::push_pea(context, 20)?;
    stack::call_void(context, caller, ids::PLAY_SOUND, 0x40)?;
    stack::push_clear(context)?;
    stack::call_void(
        context,
        caller,
        ids::AUDIO_REQUEST_CHANNEL_TEMPLATE_EFFECT,
        0x48,
    )?;
    stack::call_void(context, caller, ids::AUDIO_REQUEST_FM_CHANNEL_RELEASE, 0x4e)?;
    stack::call_void(
        context,
        caller,
        ids::MUSIC_REQUEST_TRACK_POINTER_RESET,
        0x54,
    )?;
    stack::release(context, 28)?;
    // c_pause.s:24-31: keep the mapped-input call, not a substitute RAM poll.
    while read_input(context, caller, player, 0x60)? & 0x80 != 0 {
        context.poll_yield()?;
    }
    loop {
        if !reset_selected && read_input(context, caller, player, 0x74)? & 2 != 0 {
            call_at_void(
                context,
                ids::DRAW_PAUSE_CHOICE_PANEL,
                &[1],
                CallSite::relative(caller, revision, 0x82),
            )?;
            reset_selected = true;
            context.set_data_register(3, 1);
            context.poll_yield()?;
            continue;
        }
        if reset_selected && read_input(context, caller, player, 0x94)? & 1 != 0 {
            stack::push_clear(context)?;
            stack::call_void(context, caller, ids::DRAW_PAUSE_CHOICE_PANEL, 0xa0)?;
            stack::release(context, 4)?;
            reset_selected = false;
            set_word(context, 3, 0);
            context.poll_yield()?;
            continue;
        }
        if read_input(context, caller, player, 0xae)? & 0xc0 != 0 {
            break;
        }
        context.poll_yield()?;
    }
    call_at_void(
        context,
        ids::DRAW_PAUSE_CHOICE_PANEL,
        &[u32::MAX],
        CallSite::relative(caller, revision, 0xbe),
    )?;
    if reset_selected {
        stack::call_void(context, caller, ids::RESTART_FROM_RESET_ENTRY, 0xca)?;
    }
    ram::g_current_music_selection(revision).write(context, -1);
    while read_input(context, caller, player, 0xda)? & 0xc0 != 0 {
        context.poll_yield()?;
    }
    frame.leave(context)?;
    Ok(None)
}

fn both_inputs(context: &mut dyn Context, first_return: u32, second_return: u32) -> Result<u32> {
    let second = read_input(context, ids::GAME_PAUSED, 1, first_return)?;
    context.set_data_register(2, second);
    let first = read_input(context, ids::GAME_PAUSED, 0, second_return)?;
    let input = context.data_register(2) | first;
    context.set_data_register(2, input);
    Ok(input)
}

/// G/src/ui/pause.c:62 (`GamePaused`).
/// c_pause.s:92-94,119: LINK -4/MOVEM D2-D6,A2-A4; object_number is A6-2.
/// The cheat's all-zero ship table deliberately indexes element ten, as original.
pub fn game_paused(context: &mut dyn Context) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::link(4, 0x7c, 0x1c))?;
    let revision = context.revision();
    let player = i32::from(frame.argument_word(0).read(context));
    let caller = ids::GAME_PAUSED;
    let mut input_stage = 0_u16;
    set_word(context, 2, player as u16);
    set_word(context, 5, 0);
    context.set_address_register(2, ram::g_ship_piece_level_assignments(revision).address());
    context.set_address_register(
        4,
        ids::READ_MAPPED_PLAYER_INPUT.metadata().address(revision),
    );
    context.set_data_register(4, ram::g_player1_map_level(revision).address());
    if ram::g_player1_map_level(revision).read(context) == 26 {
        stack::push_move(context, player as u32)?;
        stack::call_void(context, caller, ids::RUN_FINAL_LEVEL_PAUSE_MENU, 0x32)?;
        stack::release(context, 4)?;
        frame.leave(context)?;
        return Ok(None);
    }
    let inventory = ram::g_player_inventory_open(revision)
        .at(player)
        .read(context);
    if inventory != 0 {
        frame.leave(context)?;
        return Ok(None);
    }
    stack::push_clear(context)?;
    let object_number =
        stack::call(context, caller, ids::ALLOCATE_ENTITY_WITH_EVICTION, 0x54)? as u16;
    stack::release(context, 4)?;
    frame.local::<u16>(-2).write(context, object_number);
    if object_number == 0 {
        frame.leave(context)?;
        return Ok(None);
    }
    let object = entity(context, frame.local::<u16>(-2).read(context) as i16);
    context.set_address_register(3, object.address());
    object.x().write(
        context,
        ram::g_camera1_pending_x(revision)
            .read(context)
            .wrapping_add(160),
    );
    object.y().write(
        context,
        ram::g_camera1_pending_y(revision)
            .read(context)
            .wrapping_add(225),
    );
    object
        .level()
        .write(context, ram::g_player1_map_level(revision).read(context));
    let mut viewport = 0_u32;
    let mut viewport_mask = 2;
    if ram::g_split_screen_enabled(revision).read(context) != 0 {
        object.height().write(context, 168);
        if player != 0 {
            object
                .level()
                .write(context, ram::g_player2_map_level(revision).read(context));
            object.x().write(
                context,
                ram::g_camera2_pending_x(revision)
                    .read(context)
                    .wrapping_add(160),
            );
            object.y().write(
                context,
                ram::g_camera2_pending_y(revision)
                    .read(context)
                    .wrapping_add(225),
            );
            viewport = 1;
            viewport_mask = 4;
        }
    } else {
        object.height().write(context, 112);
    }
    if viewport == 0 {
        set_word(context, 3, 0);
    } else {
        context.set_data_register(3, 1);
    }
    context.set_data_register(6, u32::from(viewport_mask));
    object.sprite_count().write(context, 4);
    object.tile_count().write(context, 48);
    object
        .tile()
        .write(context, object.tile().read(context) | 0x8000);
    let animation = rom::pause_display_animation(revision);
    object
        .sequence()
        .write(context, animation.sequence().read(context));
    object
        .frames()
        .write(context, animation.frames().read(context));
    object.frame_duration().write(context, 1);
    object.frame().write(context, -1);
    stack::push_move(context, viewport)?;
    stack::push_move(context, object.address())?;
    stack::call_void(
        context,
        caller,
        ids::ALLOCATE_ENTITY_VIEWPORT_SPRITES,
        0x12e,
    )?;
    stack::release(context, 8)?;
    while object.draw_flags().read(context) & viewport_mask == 0 {
        context.poll_yield()?;
        stack::call_void(context, caller, ids::ENTITY_RELEASE_ONE_BY_PRIORITY, 0x120)?;
        stack::push_move(context, viewport)?;
        stack::push_move(context, object.address())?;
        stack::call_void(
            context,
            caller,
            ids::ALLOCATE_ENTITY_VIEWPORT_SPRITES,
            0x12e,
        )?;
        stack::release(context, 8)?;
    }
    loop {
        stack::call_void(context, caller, ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, 0x144)?;
        if object.sprite_ready().read(context) != 0 {
            break;
        }
        context.poll_yield()?;
    }
    // c_pause.s:189-201 keeps six outgoing slots across the sound requests.
    stack::push_pea(context, 99)?;
    stack::push_pea(context, 6)?;
    stack::call_void(context, caller, ids::PLAY_SOUND, 0x158)?;
    stack::push_pea(context, 1)?;
    stack::call_void(context, caller, ids::WAIT_FOR_NEXT_V_BLANK, 0x162)?;
    stack::push_pea(context, 99)?;
    stack::push_pea(context, 20)?;
    stack::call_void(context, caller, ids::PLAY_SOUND, 0x170)?;
    stack::push_clear(context)?;
    stack::call_void(
        context,
        caller,
        ids::AUDIO_REQUEST_CHANNEL_TEMPLATE_EFFECT,
        0x178,
    )?;
    stack::call_void(
        context,
        caller,
        ids::AUDIO_REQUEST_FM_CHANNEL_RELEASE,
        0x17e,
    )?;
    stack::call_void(
        context,
        caller,
        ids::MUSIC_REQUEST_TRACK_POINTER_RESET,
        0x184,
    )?;
    stack::release(context, 24)?;
    // c_pause.s:202-212 and338-348: both calls are bitwise-OR operands;
    // observing Start on player2 must not skip player1's source call.
    while both_inputs(context, 0x18e, 0x196)? & 0x80 != 0 {
        context.poll_yield()?;
    }
    loop {
        let first_input = read_input(context, ids::GAME_PAUSED, 0, 0x1a4)? as u8;
        context.set_data_register(
            2,
            (context.data_register(2) & 0xffffff00) | u32::from(first_input),
        );
        let second_input = read_input(context, ids::GAME_PAUSED, 1, 0x1ae)? as u8;
        context.set_data_register(
            3,
            (context.data_register(3) & 0xffffff00) | u32::from(second_input),
        );
        if (first_input | second_input) & 0x80 != 0 {
            break;
        }
        match input_stage {
            0 => {
                if first_input == 0x71 || second_input == 0x71 {
                    input_stage = input_stage.wrapping_add(1);
                }
            }
            1 => {
                if first_input == 0x48 || second_input == 0x48 {
                    input_stage = input_stage.wrapping_add(1);
                }
            }
            2 => {
                if first_input == 0x12 || second_input == 0x12 {
                    input_stage = input_stage.wrapping_add(1);
                }
            }
            3 if first_input == 0x24 || second_input == 0x24 => {
                let minimum_entry = apply_ship_piece_hint(context)?;
                set_word(context, 2, 10);
                set_word(context, 3, minimum_entry as u16);
                call_at_void(
                    context,
                    ids::PLAY_SOUND,
                    &[20, 99],
                    CallSite::relative(caller, revision, 0x28c),
                )?;
                input_stage = input_stage.wrapping_add(1);
            }
            _ => {}
        }
        set_word(context, 5, input_stage);
        context.poll_yield()?;
    }
    ram::g_current_music_selection(revision).write(context, -1);
    let object_number = u32::from(frame.local::<u16>(-2).read(context));
    stack::push_move(context, object_number)?;
    stack::call_void(context, caller, ids::FREE_ENTITY, 0x2aa)?;
    stack::release(context, 4)?;
    loop {
        let input = both_inputs(context, 0x2b2, 0x2ba)?;
        if input & 0x80 == 0 {
            break;
        }
        context.poll_yield()?;
    }
    frame.leave(context)?;
    Ok(None)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod poll_tests;
