use crate::call_frame::{CallSite, FrameLayout, LocalFrame, OutgoingArguments};
use crate::{Context, Result, execution::WaitRequest, ids};
use tje_mem::{Address, DialogueDescriptor, ObjectState, Ptr, Revision, ram, rom};

fn leave_frame(
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
    let leave = frame.leave(context);
    match (result, leave) {
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(value), Ok(())) => Ok(value),
    }
}

fn decrement_bubble_lifetime(value: i8) -> (i8, bool) {
    // SUBQ.B/BLT consumes the original subtraction overflow at -128.
    let difference = i16::from(value) - 1;
    (difference as i8, difference < 0)
}

fn object(revision: Revision, index: i16) -> Address<ObjectState> {
    // The matching compiler uses ASL.W #7 followed by ADDA.W, not a long
    // multiply. Preserve the wrapped signed word even for an invalid speaker.
    ram::g_player_info(revision)
        .cast::<ObjectState>()
        .byte_offset(i32::from(index.wrapping_shl(7)))
}

fn dialogue_record(revision: Revision, index: i16) -> Address<DialogueDescriptor> {
    // The compiler's ASL.W/ADD.W sequence multiplies by twelve before ADDA.W.
    rom::dialogue_descriptors(revision)
        .cast::<DialogueDescriptor>()
        .byte_offset(i32::from(index.wrapping_mul(12)))
}

/// G/src/speech/speech_and_dialogue.c:71 (`GenerateSpeech`).
/// Preserve slot eviction, byte result narrowing, upload IRQ waits, and the
/// low-byte lifetime override alias. No ready value is synthesized.
pub fn generate_speech(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::link(12, 0x7c, 0x1c))?;
    let result = (|| {
        let revision = context.revision();
        let speaker = arguments[0] as i16;
        let text = arguments[1];
        let replace_attached = arguments[2] as i8;
        let dispatcher = context.dispatcher();
        let site = |offset| CallSite::relative(ids::GENERATE_SPEECH, revision, offset);
        let source = object(revision, speaker);
        if source.draw_flags().read(context) & 6 == 0 {
            return Ok(Some(0));
        }
        if speaker <= 1 && source.sprite_state().read(context) == 117 {
            return Ok(Some(0));
        }
        let entities = ram::g_speech_bubble_entities(revision);
        let lifetimes = ram::g_speech_bubble_lifetimes(revision);
        if replace_attached != 0 {
            for slot in 0..4 {
                let number = i16::from(entities.at(slot).read(context));
                if number != 0 {
                    let bubble = object(revision, number);
                    if bubble.behavior_flags().read(context) & 8 != 0
                        && i16::from(bubble.owner().read(context)) == speaker
                    {
                        dispatcher.call_at_void(
                            context,
                            ids::FREE_ENTITY,
                            &[i32::from(number) as u32],
                            site(0xa0),
                        )?;
                        context.discard_u8(entities.at(slot).address())?;
                        entities.at(slot).write(context, 0);
                    }
                }
            }
        }
        let mut slot = 0;
        while slot < 4 && entities.at(slot).read(context) != 0 {
            slot += 1;
        }
        if slot == 4 {
            slot = 0;
            for index in 1..4 {
                // CMP.B evaluates the incumbent before the candidate in both ROMs.
                if lifetimes.at(slot).read(context) > lifetimes.at(index).read(context) {
                    slot = index;
                }
            }
            let number = i32::from(entities.at(slot).read(context));
            dispatcher.call_at_void(context, ids::FREE_ENTITY, &[number as u32], site(0xf6))?;
        }
        let mut length = 0_i16;
        while context.read_u8(text.wrapping_add(i32::from(length) as u32)) != 0 {
            length = length.wrapping_add(1);
        }
        if length > 12 {
            length = 12;
        }
        let allocated = dispatcher.call_at(context, ids::ALLOCATE_ENTITY, &[], site(0x128))? as i8;
        entities.at(slot).write(context, allocated);
        if allocated == 0 {
            return Ok(Some(0));
        }
        let bubble = object(revision, i16::from(entities.at(slot).read(context)));
        let x = source.x().read(context);
        bubble.x().write(context, x);
        let y = (i32::from(source.y().read(context)) + 15) as i16;
        bubble.y().write(context, y);
        let height = (i32::from(source.height().read(context)) + 15) as i16;
        bubble.height().write(context, height);
        let level = source.level().read(context);
        bubble.level().write(context, level);
        bubble
            .width()
            .write(context, (i32::from(length) << 3) as u8);
        bubble.height_pixels().write(context, 16);
        bubble.sprite_count().write(context, 4);
        bubble.tile_count().write(context, 13);
        if ram::g_player1_map_level(revision).read(context) < 26 {
            bubble.tile().write(context, 0x8000);
        }
        let frame = rom::speech_bubble_text_frame(revision);
        bubble
            .current_frame()
            .write(context, Ptr::new(frame.address()));
        let viewport = if source.draw_flags().read(context) & 2 != 0 {
            0
        } else {
            1
        };
        // The compiled caller holds slot in D4.W here. Preserve that residue for
        // AllocateEntityViewportSprites' original uninitialized-tile bug path,
        // then restore this C function's externally preserved incoming D4.
        let saved_d4 = context.data_register(4);
        context.set_data_register(4, (saved_d4 & 0xffff_0000) | u32::from(slot as u16));
        let allocation = dispatcher.call_at_void(
            context,
            ids::ALLOCATE_ENTITY_VIEWPORT_SPRITES,
            &[bubble.address(), viewport],
            site(if viewport == 0 { 0x1c2 } else { 0x1d4 }),
        );
        context.set_data_register(4, saved_d4);
        allocation?;
        if bubble.draw_flags().read(context) & 6 == 0 {
            let number = i32::from(entities.at(slot).read(context));
            dispatcher.call_at_void(context, ids::FREE_ENTITY, &[number as u32], site(0x1f8))?;
            context.discard_u8(entities.at(slot).address())?;
            entities.at(slot).write(context, 0);
            return Ok(Some(0));
        }
        let flags = bubble.draw_flags().read(context) | 8;
        bubble.draw_flags().write(context, flags);
        // TST.B/BNE at REV02 01E228 / REV00 01E1E6: the wait owns
        // every poll, including the satisfying zero byte.
        context.wait_until(WaitRequest::ram_byte_equal(
            ram::g_sprite_graphics_upload_pending(revision).address(),
            0xff,
            0,
        ))?;
        let tile = u32::from(bubble.tile().read(context) & 0x7ff);
        let queue = OutgoingArguments::push(context, &[text, i32::from(length) as u32, tile])?;
        queue.call_void(
            context,
            dispatcher,
            ids::QUEUE_SPEECH_TEXT_TILES,
            site(0x22e),
        )?;
        let prepare =
            OutgoingArguments::push(context, &[bubble.address(), frame.address(), 1, viewport])?;
        prepare.call_void(
            context,
            dispatcher,
            ids::PREPARE_ENTITY_SPRITE_FRAME,
            site(0x246),
        )?;
        prepare.release(context)?;
        queue.release(context)?;
        bubble.sprite_ready().write(context, 1);
        if replace_attached != 0 {
            let flags = bubble.behavior_flags().read(context) | 8;
            bubble.behavior_flags().write(context, flags);
            bubble.owner().write(context, speaker as i8);
            bubble.attachment_y_or_latch().write(context, 15);
            bubble.attachment_height().write(context, 15);
        }
        if ram::g_next_speech_lifetime_override(revision).read(context) != 0 {
            let lifetime = ram::g_next_speech_lifetime_override_low_byte(revision).read(context);
            lifetimes.at(slot).write(context, lifetime);
            context.discard_u16(ram::g_next_speech_lifetime_override(revision).address())?;
            ram::g_next_speech_lifetime_override(revision).write(context, 0);
        } else {
            lifetimes.at(slot).write(context, 40);
        }
        Ok(Some(i32::from(entities.at(slot).read(context)) as u32))
    })();
    leave_frame(context, frame, result)
}

/// G/src/speech/speech_and_dialogue.c:157 (`ClearSpeechBubblesAndReplies`).
pub fn clear_speech_bubbles_and_replies(
    context: &mut dyn Context,
    _arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let entities = ram::g_speech_bubble_entities(revision);
    for slot in 0..4 {
        if entities.at(slot).read(context) != 0 {
            let number = i32::from(entities.at(slot).read(context));
            crate::call_void(context, ids::FREE_ENTITY, &[number as u32])?;
            entities.at(slot).write(context, 0);
        }
    }
    ram::g_delayed_speech_reply_timers(revision)
        .at(0)
        .write(context, 0);
    ram::g_second_speech_reply_timer(revision).write(context, 0);
    ram::g_reunion_dialogue_pending(revision).write(context, 0);
    Ok(None)
}

/// G/src/speech/speech_and_dialogue.c:200 (`StartDialogue`).
/// The speaker is read as the incoming word even for original callers that omit
/// it; those callers must supply their recovered saved-D2 slot, not a fixed player.
pub fn start_dialogue(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x3c, 0x0c))?;
    let result = (|| {
        let revision = context.revision();
        let dialogue_number = arguments[0] as i16;
        let mut speaker = arguments[1] as i16;
        let dispatcher = context.dispatcher();
        let site = |offset| CallSite::relative(ids::START_DIALOGUE, revision, offset);
        let timers = ram::g_delayed_speech_reply_timers(revision);
        let speakers = ram::g_delayed_speech_reply_speakers(revision);
        let slot = if timers.at(0).read(context) != 0 {
            if timers.at(1).read(context) != 0 {
                return Ok(Some(u32::MAX));
            }
            1
        } else {
            0
        };
        let dialogue = dialogue_record(revision, dialogue_number);
        if dialogue.first_speaker().read(context) <= 1 {
            speaker = i16::from(dialogue.first_speaker().read(context));
        } else if timers.at(slot ^ 1).read(context) != 0
            && i16::from(speakers.at(slot ^ 1).read(context)) == speaker
        {
            return Ok(Some(u32::MAX));
        }
        let reply = if dialogue.reply_speaker().read(context) <= 1 {
            dialogue.reply_speaker().read(context)
        } else if dialogue.reply_speaker().read(context) == 2 {
            speaker as i8
        } else {
            (speaker ^ 1) as i8
        };
        speakers.at(slot).write(context, reply);
        let reply_speaker = i16::from(speakers.at(slot).read(context));
        if speaker <= 1
            && (ram::g_player_lives(revision)
                .at(i32::from(speaker))
                .read(context)
                < 0
                || ram::g_player_inventory_open(revision)
                    .at(i32::from(speaker))
                    .read(context)
                    != 0)
        {
            return Ok(Some(u32::MAX));
        }
        if dialogue.first_line().read(context).address() != 0 {
            let replace = OutgoingArguments::push(context, &[1])?;
            let first_line = dialogue.first_line().read(context).address();
            let line = OutgoingArguments::push(context, &[first_line])?;
            let speaker_argument = OutgoingArguments::push(context, &[i32::from(speaker) as u32])?;
            speaker_argument.call(context, dispatcher, ids::GENERATE_SPEECH, site(0xf2))?;
            speaker_argument.release(context)?;
            line.release(context)?;
            replace.release(context)?;
        }
        if dialogue.reply().read(context).address() != 0
            && (ram::g_split_screen_enabled(revision).read(context) == 0
                || speaker == reply_speaker)
            && (reply_speaker > 1
                || (ram::g_player_lives(revision)
                    .at(i32::from(reply_speaker))
                    .read(context)
                    >= 0
                    && ram::g_player_inventory_open(revision)
                        .at(i32::from(reply_speaker))
                        .read(context)
                        == 0))
        {
            let delay = dialogue.delay().read(context);
            timers.at(slot).write(context, delay);
            let text = dialogue.reply().read(context);
            ram::g_delayed_speech_reply_texts(revision)
                .at(slot)
                .write(context, text);
        }
        Ok(Some(slot as u32))
    })();
    leave_frame(context, frame, result)
}

/// G/src/speech/speech_and_dialogue.c:264 (`UpdateSpeechAndDialogue`).
/// Each omitted StartDialogue speaker occupies the saved incoming D2 slot in
/// this original's MOVEM frame (matching listings, lines 575 and 727-780).
/// Recover that actual register value rather than supplying a corrected speaker.
pub fn update_speech_and_dialogue(
    context: &mut dyn Context,
    _arguments: &[u32],
) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x1c, 0x1c))?;
    let result: Result<Option<u32>> = (|| {
        let revision = context.revision();
        let dispatcher = context.dispatcher();
        let site = |offset| CallSite::relative(ids::UPDATE_SPEECH_AND_DIALOGUE, revision, offset);
        context.set_address_register(2, ids::START_DIALOGUE.metadata().address(revision));
        context.set_address_register(3, ram::g_player_info(revision).address());
        context.set_address_register(4, ids::GET_RANDOM_NUMBER.metadata().address(revision));
        let entities = ram::g_speech_bubble_entities(revision);
        let lifetimes = ram::g_speech_bubble_lifetimes(revision);
        for slot in 0..4 {
            let number = i16::from(entities.at(slot).read(context));
            if number != 0 {
                let bubble = object(revision, number);
                let (lifetime, expired) =
                    decrement_bubble_lifetime(lifetimes.at(slot).read(context));
                lifetimes.at(slot).write(context, lifetime);
                if expired || bubble.draw_flags().read(context) & 6 == 0 {
                    dispatcher.call_at_void(
                        context,
                        ids::FREE_ENTITY,
                        &[i32::from(number) as u32],
                        site(0x58),
                    )?;
                    context.discard_u8(entities.at(slot).address())?;
                    entities.at(slot).write(context, 0);
                }
            }
        }
        let timers = ram::g_delayed_speech_reply_timers(revision);
        for slot in 0..2 {
            if timers.at(slot).read(context) != 0 {
                let next = timers.at(slot).read(context).wrapping_sub(1);
                timers.at(slot).write(context, next);
                if next == 0 {
                    let replace = OutgoingArguments::push(context, &[1])?;
                    let text = ram::g_delayed_speech_reply_texts(revision)
                        .at(slot)
                        .read(context)
                        .address();
                    let line = OutgoingArguments::push(context, &[text])?;
                    let speaker = i32::from(
                        ram::g_delayed_speech_reply_speakers(revision)
                            .at(slot)
                            .read(context),
                    );
                    let speaker_argument = OutgoingArguments::push(context, &[speaker as u32])?;
                    speaker_argument.call(context, dispatcher, ids::GENERATE_SPEECH, site(0xc0))?;
                    speaker_argument.release(context)?;
                    line.release(context)?;
                    replace.release(context)?;
                }
            }
        }
        if dispatcher.call_at(context, ids::GET_RANDOM_NUMBER, &[], site(0xce))? & 0x1fff == 1 {
            let player =
                (dispatcher.call_at(context, ids::GET_RANDOM_NUMBER, &[], site(0xdc))? & 1) as i8;
            let mask = u32::from(
                object(revision, i16::from(player))
                    .draw_flags()
                    .read(context)
                    & 6,
            );
            if dispatcher.call_at(
                context,
                ids::EARTHLING_RESET_STATE_TWO_BY_DRAW_MASK,
                &[mask],
                site(0xfa),
            )? != 0
            {
                let slot = dispatcher.call_at(
                    context,
                    ids::START_DIALOGUE,
                    &[0, i32::from(player) as u32],
                    site(0x10c),
                )? as i16;
                if slot != -1 {
                    let sound = OutgoingArguments::push(
                        context,
                        &[rom::sfx_achoo(revision).address(), 50, 0],
                    )?;
                    sound.call_void(context, dispatcher, ids::PLAY_SOUND_EFFECT, site(0x128))?;
                    let split = ram::g_split_screen_enabled(revision).read(context);
                    sound.release(context)?;
                    if split != 0 {
                        context.discard_u16(timers.at(i32::from(slot)).address())?;
                        timers.at(i32::from(slot)).write(context, 0);
                    }
                }
            }
        }
        let first = object(revision, 0);
        let second = object(revision, 1);
        if second.level().read(context) == first.level().read(context) {
            let dx = i32::from(first.x().read(context)) - i32::from(second.x().read(context));
            let distance_x =
                dispatcher.call_at(context, ids::ABS, &[dx as u32], site(0x164))? as i16;
            let dy = i32::from(first.y().read(context)) - i32::from(second.y().read(context));
            let distance_y =
                dispatcher.call_at(context, ids::ABS, &[dy as u32], site(0x17e))? as i16;
            if ram::g_reunion_dialogue_pending(revision).read(context) != 0
                && ram::g_split_screen_enabled(revision).read(context) == 0
                && distance_x < 260
                && distance_y < 174
            {
                let dialogue = if dispatcher.call_at(
                    context,
                    ids::PLAYER_HAS_ODD_ELEVATOR_STATE,
                    &[0],
                    site(0x1a6),
                )? != 0
                    && dispatcher.call_at(context, ids::GET_RANDOM_NUMBER, &[], site(0x1ae))? & 3
                        == 0
                {
                    3
                } else if dispatcher.call_at(
                    context,
                    ids::PLAYER_HAS_ODD_ELEVATOR_STATE,
                    &[1],
                    site(0x1c4),
                )? != 0
                    && dispatcher.call_at(context, ids::GET_RANDOM_NUMBER, &[], site(0x1cc))? & 3
                        == 0
                {
                    4
                } else {
                    1
                };
                crate::call_at(
                    context,
                    ids::START_DIALOGUE,
                    &[dialogue],
                    CallSite::relative(ids::UPDATE_SPEECH_AND_DIALOGUE, revision, 0x1de),
                )?;
                context.discard_u8(ram::g_reunion_dialogue_pending(revision).address())?;
                ram::g_reunion_dialogue_pending(revision).write(context, 0);
            } else if distance_x > 640 || distance_y > 448 {
                ram::g_reunion_dialogue_pending(revision).write(context, 1);
            }
            // ORIGINAL BUG: the bound visits only player zero; the dead else arm's
            // alternate dialogue is never substituted for this one.
            if ram::g_player_control_remap_entities(revision)
                .at(0)
                .read(context)
                != 0
                && distance_x < 30
                && distance_y < 20
                && dispatcher.call_at(context, ids::GET_RANDOM_NUMBER, &[], site(0x21c))? & 0xff
                    == 2
            {
                crate::call_at(
                    context,
                    ids::START_DIALOGUE,
                    &[22],
                    CallSite::relative(ids::UPDATE_SPEECH_AND_DIALOGUE, revision, 0x238),
                )?;
            }
        } else {
            ram::g_reunion_dialogue_pending(revision).write(context, 1);
        }
        Ok(None)
    })();
    leave_frame(context, frame, result)
}

#[cfg(test)]
mod tests {
    use super::{decrement_bubble_lifetime, dialogue_record, object};
    use tje_mem::{Revision, ram, rom};

    #[test]
    fn every_bubble_lifetime_byte_preserves_subq_blt_overflow() {
        for bits in 0..=255_u8 {
            let (stored, expired) = decrement_bubble_lifetime(bits as i8);
            let result = bits.wrapping_sub(1);
            let negative = result & 0x80 != 0;
            let overflow = bits == 0x80;
            assert_eq!(stored as u8, result);
            assert_eq!(expired, negative != overflow, "lifetime {bits:#04x}");
        }
    }

    #[test]
    fn every_word_index_keeps_object_and_dialogue_effective_address_wrap() {
        for revision in [Revision::Rev02, Revision::Rev00] {
            let objects = ram::g_player_info(revision).address();
            let dialogues = rom::dialogue_descriptors(revision).address();
            for bits in 0..=65535_u32 {
                let index = bits as u16 as i16;
                let object_displacement = ((bits * 128) & 0xffff) as u16 as i16;
                let dialogue_displacement = ((bits * 12) & 0xffff) as u16 as i16;
                assert_eq!(
                    object(revision, index).address(),
                    objects.wrapping_add(i32::from(object_displacement) as u32)
                );
                assert_eq!(
                    dialogue_record(revision, index).address(),
                    dialogues.wrapping_add(i32::from(dialogue_displacement) as u32)
                );
            }
            assert_eq!(object(revision, 512).address(), objects);
            assert_eq!(object(revision, -1).address(), objects - 128);
            assert_eq!(dialogue_record(revision, 5461).address(), dialogues - 4);
        }
    }
}
