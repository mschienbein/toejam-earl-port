use crate::{Context, Result, call, ids};
use tje_mem::{ram, rom};

/// G/src/earthlings/nice_earthling_touch.c:65 (`CheckAButton`).
/// Atomic: ordered input/result/latch stores and bounded object free calls.
pub fn check_a_button(context: &mut dyn Context, _arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    for player in 0..2 {
        let active = ram::g_player_interaction_choice_active(revision).at(player);
        if active.read(context) == 0 {
            continue;
        }
        let handle = ram::g_player_interaction_choice_entities(revision).at(player);
        let display = ram::g_player_info(revision).at(i32::from(handle.read(context)));
        let mut choice = display.sprite_state().read(context);
        let state = ram::g_player_info(revision)
            .at(player)
            .sprite_state()
            .read(context);
        let cancel = state == 5 || state == 6;
        let input = ram::g_player_processed_input(revision).at(player);
        let previous = ram::g_player_previous_processed_input(revision).at(player);
        if cancel || (input.read(context) & 0x40 != 0 && previous.read(context) & 0x40 == 0) {
            ram::g_player_interaction_responses(revision)
                .at(player)
                .write(
                    context,
                    if cancel {
                        0
                    } else if display.sprite_state().read(context) == 1 {
                        1
                    } else {
                        2
                    },
                );
            let object_number = handle.read(context) as u32;
            call(context, ids::FREE_ENTITY, &[object_number])?;
            handle.write(context, 0);
            active.write(context, 0);
            previous.write(context, 0x40);
            input.write(context, 0x40);
            ram::g_player_choice_action_release_latches(revision)
                .at(player)
                .write(context, 1);
        } else {
            if input.read(context) & 1 != 0 {
                choice = 0
            } else if input.read(context) & 2 != 0 {
                choice = 1
            }
            if choice != display.sprite_state().read(context) {
                display.frame().write(context, choice.wrapping_sub(1));
                display.frame_duration().write(context, 1);
                display.sprite_state().write(context, choice);
            }
            input.write(context, 0);
        }
    }
    Ok(None)
}

/// G/src/earthlings/nice_earthling_touch.c:390 (`PlayerCreateLifeRequestDisplay`).
/// Atomic: object/viewport allocation and animation RAM, with no VDP access.
pub fn player_create_life_request_display(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let handle = ram::g_life_request_display_entity(revision);
    let allocated = call(context, ids::ALLOCATE_ENTITY, &[])? as i16;
    handle.write(context, allocated);
    if allocated != 0 {
        let object = super::word_object(revision, handle.read(context));
        object.x().write(
            context,
            ram::g_camera1_pending_x(revision)
                .read(context)
                .wrapping_add(80),
        );
        object.y().write(
            context,
            ram::g_camera1_pending_y(revision)
                .read(context)
                .wrapping_add(224),
        );
        object.height().write(context, 112);
        object
            .level()
            .write(context, ram::g_player1_map_level(revision).read(context));
        object.width().write(context, 32);
        object.height_pixels().write(context, 48);
        object.sprite_count().write(context, 4);
        object.tile_count().write(context, 40);
        object
            .tile()
            .write(context, object.tile().read(context) | 0x8000);
        let animation = if arguments[0] as i16 == 0 {
            rom::player_animation_descriptors(revision).at(5)
        } else {
            rom::player2_animation_descriptors(revision).at(5)
        };
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
        object.frame().write(context, -1);
        let object_number = handle.read(context) as u32;
        call(
            context,
            ids::UPDATE_ENTITY_VIEWPORT_SPRITES,
            &[object_number],
        )?;
        if object.draw_flags().read(context) & 2 != 0 {
            return Ok(Some(0));
        }
        let object_number = handle.read(context) as u32;
        call(context, ids::FREE_ENTITY, &[object_number])?;
    }
    Ok(Some(1))
}

/// G/src/earthlings/nice_earthling_touch.c:36 (`PlayerCreateInteractionChoiceDisplay`).
/// Atomic: preserve response reset before allocation and active flag as final store.
pub fn player_create_interaction_choice_display(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let player = i32::from(arguments[0] as i16);
    ram::g_player_interaction_responses(revision)
        .at(player)
        .write(context, 0);
    let handle = ram::g_player_interaction_choice_entities(revision).at(player);
    let allocated = call(context, ids::ALLOCATE_ENTITY_WITH_EVICTION, &[0])? as i8;
    handle.write(context, allocated);
    if allocated != 0 {
        let display = ram::g_player_info(revision).at(i32::from(handle.read(context)));
        display.level().write(
            context,
            super::word_object(revision, player as i16)
                .level()
                .read(context),
        );
        display
            .behavior_flags()
            .write(context, display.behavior_flags().read(context) | 8);
        display.owner().write(context, player as i8);
        display.attachment_y_or_latch().write(context, 10);
        display.attachment_height().write(context, 10);
        display
            .tile()
            .write(context, display.tile().read(context) | 0x8000);
        display.width().write(context, 32);
        display.height_pixels().write(context, 32);
        display.sprite_count().write(context, 1);
        display.tile_count().write(context, 16);
        let animation = rom::player_interaction_choice_animations(revision).at(0);
        display
            .sequence()
            .write(context, animation.sequence().read(context));
        display
            .frames()
            .write(context, animation.frames().read(context));
        display.frame().write(context, -1);
        display.frame_duration().write(context, 1);
        ram::g_player_interaction_choice_active(revision)
            .at(player)
            .write(context, 1);
    }
    Ok(None)
}

/// G/src/earthlings/nice_earthling_touch.c:425 (`PlayerProcessLifeRequest`).
/// TimeAware: the exact-width upload-byte RAM predicate can await ISR writes;
/// speech and relocation retain their own reviewed time-progressing contracts.
/// Both inline literals follow the inventoried executable extent (RTS).
pub fn player_process_life_request(context: &mut dyn Context) -> Result<Option<u32>> {
    let frame = crate::call_frame::LocalFrame::enter(
        context,
        crate::call_frame::FrameLayout::movem(0, 0x1c, 0x3c),
    )?;
    let number = frame.argument_word(0).read(context);
    let result = life_request(context, number);
    super::framed::finish(context, frame, result)
}

fn life_request(context: &mut dyn Context, number: i16) -> Result<Option<u32>> {
    use super::framed::{invoke, invoke_on};
    use crate::call_frame::OutgoingArguments;
    let revision = context.revision();
    let caller = ids::PLAYER_PROCESS_LIFE_REQUEST;
    let level = ram::g_player1_map_level(revision).read(context);

    if level == 26 {
        return Ok(None);
    }
    let other = i32::from(number ^ 1);
    let mut player = super::word_object(revision, other as i16);

    let handle = ram::g_life_request_display_entity(revision);
    if handle.read(context) == 0
        && ram::g_player_lives(revision).at(other).read(context) > 0
        && ram::g_player_interaction_choice_active(revision)
            .at(other)
            .read(context)
            == 0
        && ram::g_player_elevator_flags(revision)
            .at(other)
            .read(context)
            == 0
        && (player.sprite_state().read(context) == 0
            || player.sprite_state().read(context) == 4
            || player.animation_group_mask().read(context) & 0x2040_0000 != 0
            || player.sprite_state().read(context) == 76
            || player.animation_group_mask().read(context) & 0x400c != 0)
        && player.animation_group_mask().read(context) & 0x2000_0000 == 0
        && invoke(
            context,
            caller,
            ids::READ_MAPPED_PLAYER_INPUT,
            &[number as u32],
            0x021df2,
        )? & 0x70
            != 0
        && invoke(
            context,
            caller,
            ids::PLAYER_CREATE_LIFE_REQUEST_DISPLAY,
            &[number as u32],
            0x021e06,
        )? == 0
    {
        let sprite = invoke(
            context,
            caller,
            ids::GET_ACTIVE_PRESENT_SPRITE,
            &[player.address()],
            0x021e16,
        )?;
        // c_nice_earthling_touch.s:1259-1271: SetSprite's two slots stay
        // allocated through the later one-slot SetNextSpeechLifetime call.
        let sprite_args = OutgoingArguments::push(context, &[player.address(), sprite])?;
        invoke_on(context, caller, ids::SET_SPRITE, &sprite_args, 0x021e22)?;
        let display = handle.read(context);
        player = super::word_object(revision, display);

        let height = player.height().read(context);
        let height = height.wrapping_add(30);
        player.height().write(context, height);
        invoke(
            context,
            caller,
            ids::SET_NEXT_SPEECH_LIFETIME,
            &[64],
            0x021e3e,
        )?;
        sprite_args.release(context)?;
        let metadata = caller.metadata();
        let text = metadata.address(revision)
            + metadata.size(revision)
            + if ram::g_player_selection(revision).read(context) != 2 {
                0
            } else {
                12
            };
        let speech_tail = OutgoingArguments::push(context, &[text, 0])?;
        let speaker = handle.read(context) as u32;
        let speech_head = OutgoingArguments::push(context, &[speaker])?;
        invoke_on(
            context,
            caller,
            ids::GENERATE_SPEECH,
            &speech_head,
            0x021e66,
        )?;
        speech_head.release(context)?;
        speech_tail.release(context)?;
        let height = player.height().read(context);
        let height = height.wrapping_sub(30);
        player.height().write(context, height);
        invoke(
            context,
            caller,
            ids::PLAYER_CREATE_INTERACTION_CHOICE_DISPLAY,
            &[other as u32],
            0x021e7a,
        )?;
        ram::g_life_request_pending(revision).write(context, 1);
    }
    let pending = ram::g_life_request_pending(revision).read(context);

    if pending != 0 {
        let response = ram::g_player_interaction_responses(revision).at(other);
        let response_value = response.read(context);

        if response_value == 0 {
            return Ok(None);
        }
        let object = handle.read(context) as u32;
        invoke(context, caller, ids::FREE_ENTITY, &[object], 0x021eae)?;
        context.discard_u16(handle.address())?;
        handle.write(context, 0);
        if response.read(context) == 2 {
            // c_nice_earthling_touch.s:1317-1318: TST.B / BNE. The
            // predicate owns every byte poll, including the satisfying zero.
            context.wait_until(crate::execution::WaitRequest::ram_byte_equal(
                ram::g_sprite_graphics_upload_pending(revision).address(),
                u8::MAX,
                0,
            ))?;
            invoke(
                context,
                caller,
                ids::DISABLE_BLANKING_GRAPHICS,
                &[],
                0x021ed4,
            )?;
            let mode = ram::g_mode2_reg_softcopy(revision);
            mode.write(context, mode.read(context) & 0x3c);
            context.write_device_u16(0xc00004, 0x8104 | u16::from(mode.read(context)));
            if ram::g_player_selection(revision).read(context) == 2 {
                let lives = ram::g_player_lives(revision).at(other);
                let old = lives.read(context);
                let next = old.wrapping_sub(1);
                lives.write(context, next);
                invoke(
                    context,
                    caller,
                    ids::HUD_LOAD_LIVES_SPRITE_TILES,
                    &[other as u32],
                    0x021f10,
                )?;
            }
            invoke(
                context,
                caller,
                ids::COPY_LEVEL_BETWEEN_PLAYERS,
                &[0],
                0x021f1a,
            )?;
            ram::g_player2_map_level(revision)
                .write(context, ram::g_player1_map_level(revision).read(context));
            player = super::word_object(revision, number);

            player.draw_flags().write(context, 1);
            player.behavior_flags().write(context, 1);
            context.discard_u8(player.sprite_state().address())?;
            player.sprite_state().write(context, 0);
            let reset_args = OutgoingArguments::push(context, &[number as u32])?;
            invoke_on(
                context,
                caller,
                ids::PLAYER_RESET_RANK_AND_RESOURCES,
                &reset_args,
                0x021f4e,
            )?;
            invoke(
                context,
                caller,
                ids::STOP_PLAYER,
                &[player.address()],
                0x021f56,
            )?;
            reset_args.release(context)?;
            context.discard_u8(player.animation_motion_axes().address())?;
            player.animation_motion_axes().write(context, 0);
            ram::g_player_selection(revision).write(context, 2);
            ram::g_split_screen_enabled(revision).write(context, 1);
            invoke(
                context,
                caller,
                ids::PLAYER_RELOCATE_TO_PARTNER_OR_NEXT_LEVEL,
                &[number as u32],
                0x021f74,
            )?;
        }
        context.discard_u16(ram::g_life_request_pending(revision).address())?;
        ram::g_life_request_pending(revision).write(context, 0);
        context.discard_u8(response.address())?;
        response.write(context, 0);
    }
    Ok(None)
}
