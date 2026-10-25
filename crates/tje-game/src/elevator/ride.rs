use crate::{
    Context, Result, call, call_at,
    call_frame::{CallSite, FrameLayout, LocalFrame, OutgoingArguments},
    execution::WaitRequest,
    ids,
};
use tje_mem::{ObjectState, ram, rom};

/// G/src/elevator/elevator_scene.c:437 (`ElevatorLoadRideBackgroundTiles`).
/// Read/decode the four ROM pieces; queue-index post-increments precede each
/// run-word store, retaining aliases if the original queue has overflowed.
pub fn elevator_load_ride_background_tiles(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let level = ram::g_player1_map_level(revision).read(context);
    let frame = rom::spr_transition_background(revision)
        .at(i32::from(level) * 2)
        .read(context)
        .target();
    for part_number in 0..4 {
        let part = frame.piece(part_number);
        let id = if part.tile_format().read(context) < 0 {
            ids::SPRITE_UNPACK_INTERLEAVED_RLE
        } else {
            ids::SPRITE_UNPACK_RLE
        };
        call(context, id, &[part.address(), 16])?;
        let offset = ram::g_sprite_tile_buffer_offset(revision);
        let next = offset.read(context).wrapping_add(512);
        offset.write(context, next);
    }
    let count = ram::g_sprite_tile_upload_queue_word_count(revision);
    for word in [1, 64] {
        let index = count.read(context);
        let next = count.read(context).wrapping_add(1);
        count.write(context, next);
        ram::g_sprite_tile_upload_runs(revision)
            .at(i32::from(index))
            .write(context, word);
    }
    call(context, ids::UPLOAD_QUEUED_SPRITE_GRAPHICS, &[1, 1])?;
    Ok(None)
}

/// G/src/elevator/elevator_scene.c:463 (`ElevatorAlternateRideEntities`).
pub fn elevator_alternate_ride_entities(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let first = ram::g_elevator_ride_entity(revision).read(context).target();
    let second = first.offset(2);
    if ram::g_frame_counter(revision).read(context) & 15 == 0 {
        if first.sprite_ready().read(context) != 0 {
            first.sprite_ready().write(context, 0);
            second.sprite_ready().write(context, 1);
        } else {
            first.sprite_ready().write(context, 1);
            second.sprite_ready().write(context, 0);
        }
        // No caller prologue. Absolute JSR returns at 0x13248/0x13218.
        call_at(
            context,
            ids::BUILD_VIEWPORT_SPRITE_LINKS,
            &[],
            CallSite::relative(ids::ELEVATOR_ALTERNATE_RIDE_ENTITIES, revision, 0x3a),
        )?;
        call(
            context,
            ids::DMA_COPY_WORDS_TO_VRAM,
            &[
                ram::g_hardware_sprite_table(revision).address(),
                0xf800,
                320,
            ],
        )?;
        let link = ram::g_primary_viewport_sentinel_link(revision).read(context);
        ram::g_primary_viewport_uploaded_link(revision).write(context, link);
    }
    Ok(None)
}

/// G/src/elevator/elevator_scene.c:530 (`ElevatorUpdateRideDialogue`).
/// Missing second lines advance the shared frame counter to 180 or 419.
pub fn elevator_update_ride_dialogue(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let counter = ram::g_frame_counter(revision);
    let lines = ram::g_elevator_ride_speech_entities(revision);
    let set_ready = |context: &mut dyn Context, index: i32, ready| {
        let line = lines.at(index).read(context);
        if line != 0 {
            let entity = ram::g_player_info(revision)
                .cast::<ObjectState>()
                .byte_offset(i32::from(i16::from(line).wrapping_shl(7)));
            entity.sprite_ready().write(context, ready);
        }
        line
    };
    if counter.read(context) == 60 {
        set_ready(context, 0, 1);
    } else if counter.read(context) == 120 {
        set_ready(context, 0, 0);
        if set_ready(context, 1, 1) == 0 {
            counter.write(context, 180);
        }
    } else if counter.read(context) == 180 {
        set_ready(context, 1, 0);
    } else if counter.read(context) == 300 {
        set_ready(context, 2, 1);
    } else if counter.read(context) == 360 {
        set_ready(context, 2, 0);
        if set_ready(context, 3, 1) == 0 {
            counter.write(context, 419);
        }
    } else if counter.read(context) == 420 {
        call(context, ids::CLEAR_SPEECH_BUBBLES_AND_REPLIES, &[])?;
    }
    Ok(None)
}

/// G/src/elevator/elevator_scene.c:484 (`ElevatorPrepareRideDialogue`).
/// LINK A6,#-4 and MOVEM D2-D6/A2-A4 are retained because sprite linking may
/// address the saved caller frame. Reply line one's word really lives at A6-2.
pub fn elevator_prepare_ride_dialogue(context: &mut dyn Context) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::link(4, 0x7c, 0x1c))?;
    let entity_address = frame.argument_slot(0).read(context);
    let result = prepare_dialogue_body(context, entity_address);
    let result = result.map_err(|error| context.protect_timing_failure(error));
    if result
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        return result.map(|()| None);
    }
    let leave = frame.leave(context);
    result?;
    leave?;
    Ok(None)
}

fn prepare_dialogue_body(context: &mut dyn Context, entity_address: u32) -> Result<()> {
    let revision = context.revision();
    let entity = tje_mem::Address::<ObjectState>::new(entity_address);
    let lines = ram::g_elevator_ride_speech_entities(revision);
    context.set_address_register(2, entity.address());
    context.set_address_register(3, lines.address());
    context.set_address_register(4, rom::speech_string_pointers(revision).address());
    context.set_data_register(4, ids::GENERATE_SPEECH.metadata().address(revision));
    set_low_word(context, 5, 0);
    set_low_word(context, 6, 0);
    if ram::g_player_lives(revision).at(0).read(context) >= 0
        && ram::g_player_lives(revision).at(1).read(context) >= 0
    {
        let random = call_at(
            context,
            ids::GET_RANDOM_NUMBER,
            &[],
            CallSite::relative(ids::ELEVATOR_PREPARE_RIDE_DIALOGUE, revision, 0x3e),
        )? as u16;
        set_low_word(context, 2, random as i16);
        if u32::from(random) % 100 < 75 {
            let mut speech = (u32::from(call_at(
                context,
                ids::GET_RANDOM_NUMBER,
                &[],
                CallSite::relative(ids::ELEVATOR_PREPARE_RIDE_DIALOGUE, revision, 0x66),
            )? as u16)
                % 14) as i16;
            let mask = ram::g_elevator_ride_used_dialogue_mask(revision);
            let mut attempt = 0;
            // MOVEQ #0,D2 precedes MOVE.W of the unsigned RNG result.
            context.set_data_register(2, speech as u32);
            set_low_word(context, 3, attempt);
            while attempt < 14 {
                if (1_i32.wrapping_shl(speech as u32) & mask.read(context)) == 0 {
                    break;
                }
                speech = ((i32::from(speech) + 1) % 14) as i16;
                attempt += 1;
                set_low_word(context, 2, speech);
                set_low_word(context, 3, attempt);
            }
            if attempt != 14 {
                let used = mask.read(context) | 1_i32.wrapping_shl(speech as u32);
                mask.write(context, used);
                let x = entity.x().read(context).wrapping_sub(24);
                entity.x().write(context, x);
                let descriptor = rom::speech_string_pointers(revision).at(i32::from(speech));
                let first = generate_ride_line(context, entity, descriptor.first_line(), 0xe6)?;
                set_low_word(context, 3, first);
                if descriptor.second_line().read(context).address() != 0 {
                    let second =
                        generate_ride_line(context, entity, descriptor.second_line(), 0x110)?;
                    set_low_word(context, 5, second);
                }
                let x = entity.x().read(context).wrapping_add(40);
                entity.x().write(context, x);
                let reply_first =
                    generate_ride_line(context, entity, descriptor.reply_first_line(), 0x134)?;
                let reply_slot =
                    tje_mem::Address::<i16>::new(context.address_register(6).wrapping_sub(2));
                reply_slot.write(context, reply_first);
                if descriptor.reply_second_line().read(context).address() != 0 {
                    let reply_second =
                        generate_ride_line(context, entity, descriptor.reply_second_line(), 0x160)?;
                    set_low_word(context, 6, reply_second);
                }
                let x = entity.x().read(context).wrapping_sub(16);
                entity.x().write(context, x);
                call_at(
                    context,
                    ids::UPDATE_AND_QUEUE_ENTITY_SPRITES,
                    &[],
                    CallSite::relative(ids::ELEVATOR_PREPARE_RIDE_DIALOGUE, revision, 0x170),
                )?;
                for index in 0..4 {
                    // Resolve the live A6 at the original use point: Queue's
                    // Build callee can overwrite its saved caller A6 at 55/56
                    // links. A cached LocalFrame base would hide that bug.
                    let byte = match index {
                        0 => context.data_register(3) as u8,
                        1 => context.data_register(5) as u8,
                        2 => {
                            tje_mem::Address::<u8>::new(context.address_register(6).wrapping_sub(1))
                                .read(context)
                        }
                        _ => context.data_register(6) as u8,
                    };
                    lines.at(index).write(context, byte);
                    let line = if byte != 0 {
                        match index {
                            0 => context.data_register(3) as i16,
                            1 => context.data_register(5) as i16,
                            2 => tje_mem::Address::<i16>::new(
                                context.address_register(6).wrapping_sub(2),
                            )
                            .read(context),
                            _ => context.data_register(6) as i16,
                        }
                    } else {
                        0
                    };
                    if byte != 0 {
                        let ready = ram::g_player_info(revision)
                            .cast::<ObjectState>()
                            .byte_offset(i32::from(line.wrapping_shl(7)))
                            .sprite_ready();
                        context.discard_u8(ready.address())?;
                        ready.write(context, 0);
                    }
                }
                call_at(
                    context,
                    ids::BUILD_VIEWPORT_SPRITE_LINKS,
                    &[],
                    CallSite::relative(ids::ELEVATOR_PREPARE_RIDE_DIALOGUE, revision, 0x1c8),
                )?;
                return Ok(());
            }
        }
    }
    for index in 0..4 {
        context.discard_u8(lines.at(index).address())?;
        lines.at(index).write(context, 0);
    }
    Ok(())
}

// The compiler pushes the third argument before reading the text pointer, and
// pushes that pointer before reading the speaker byte. Keep that interleaving:
// the emulated stack and entity/descriptor storage may alias.
fn generate_ride_line(
    context: &mut dyn Context,
    entity: tje_mem::Address<ObjectState>,
    text: tje_mem::Address<tje_mem::Ptr<i8>>,
    return_offset: u32,
) -> Result<i16> {
    let revision = context.revision();
    let dispatcher = context.dispatcher();
    let final_argument = OutgoingArguments::push(context, &[0])?;
    let pointer = text.read(context).address();
    let text_argument = OutgoingArguments::push(context, &[pointer])?;
    let speaker = i32::from(entity.entity_index().read(context)) as u32;
    let speaker_argument = OutgoingArguments::push(context, &[speaker])?;
    let line = speaker_argument.call(
        context,
        dispatcher,
        ids::GENERATE_SPEECH,
        CallSite::relative(ids::ELEVATOR_PREPARE_RIDE_DIALOGUE, revision, return_offset),
    )? as i16;
    speaker_argument.release(context)?;
    text_argument.release(context)?;
    final_argument.release(context)?;
    Ok(line)
}

fn set_low_word(context: &mut dyn Context, register: usize, value: i16) {
    let high = context.data_register(register) & 0xffff0000;
    context.set_data_register(register, high | u32::from(value as u16));
}

/// G/src/elevator/elevator_scene.c:562 (`ElevatorRideVBlank`).
pub fn elevator_ride_v_blank(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let acceleration = ram::g_elevator_ride_scroll_acceleration(revision);
    let velocity = ram::g_elevator_ride_scroll_velocity(revision);
    if acceleration.read(context) < 4096 {
        let next = acceleration.read(context).wrapping_add(1);
        acceleration.write(context, next);
    }
    if acceleration.read(context) < 0 || velocity.read(context) < 2048 {
        let next = velocity
            .read(context)
            .wrapping_add(acceleration.read(context) >> 8);
        velocity.write(context, next);
    }
    let fixed = ram::g_elevator_ride_scroll_y_fixed(revision);
    let next = fixed
        .read(context)
        .wrapping_sub(i32::from(velocity.read(context)));
    fixed.write(context, next);
    context.write_vdp_command(0x40000010);
    context.write_vdp_data((fixed.read(context) >> 8) as u16);
    call(context, ids::ELEVATOR_UPDATE_RIDE_DIALOGUE, &[])?;
    // PC-relative JSR at 0x13558/0x13528, with no RideVBlank prologue.
    call_at(
        context,
        ids::ELEVATOR_ALTERNATE_RIDE_ENTITIES,
        &[],
        CallSite::relative(ids::ELEVATOR_RIDE_V_BLANK, revision, 0x4e),
    )?;
    call(context, ids::SELECT_PRIMARY_VIEWPORT_SPRITES, &[])?;
    call(context, ids::MUSIC_UPDATE_SEQUENCER, &[])?;
    let counter = ram::g_frame_counter(revision);
    let next = counter.read(context).wrapping_add(1);
    counter.write(context, next);
    Ok(None)
}

/// G/src/elevator/elevator_scene.c:633 (`ElevatorEndRideScene`).
/// The source polls signed <2048 and >0; neither is a masked equality test.
/// Preserve each literal word read and yield only on a taken spin backedge.
pub fn elevator_end_ride_scene(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let velocity = ram::g_elevator_ride_scroll_velocity(revision);
    loop {
        let polled = velocity.read(context);
        if polled >= 2048 {
            break;
        }
        context.poll_yield()?;
    }
    ram::g_elevator_ride_scroll_acceleration(revision).write(context, -2048);
    loop {
        let polled = velocity.read(context);
        if polled <= 0 {
            break;
        }
        context.poll_yield()?;
    }
    let callback = ram::g_v_blank_func(revision).address();
    context.discard_u16(callback)?;
    context.discard_u16(callback.wrapping_add(2))?;
    ram::g_v_blank_func(revision).write(context, tje_mem::Ptr::new(0));
    call_at(
        context,
        ids::CLEAR_SPEECH_BUBBLES_AND_REPLIES,
        &[],
        CallSite::relative(ids::ELEVATOR_END_RIDE_SCENE, revision, 0x26),
    )?;
    call_at(
        context,
        ids::DISABLE_BLANKING_GRAPHICS,
        &[],
        CallSite::relative(ids::ELEVATOR_END_RIDE_SCENE, revision, 0x2c),
    )?;
    let mode = ram::g_mode2_reg_softcopy(revision);
    let value = mode.read(context) & 0x3c;
    mode.write(context, value);
    context.write_vdp_control(0x8104 | u16::from(mode.read(context)));
    call_at(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_game_line2(revision).address(), 1],
        CallSite::relative(ids::ELEVATOR_END_RIDE_SCENE, revision, 0x58),
    )?;
    context.write_vdp_command(0x40000000);
    call_at(
        context,
        ids::MOVE_DATA_TO_VRAM,
        &[rom::level_tiles(revision).address(), 1040],
        CallSite::relative(ids::ELEVATOR_END_RIDE_SCENE, revision, 0x74),
    )?;
    Ok(None)
}

/// G/src/elevator/elevator_scene.c:579 (`ElevatorBeginRideScene`).
/// The exact-byte wait owns every pending poll, including its terminal zero.
/// The compound sprite-ready loop retains short-circuit reads and yields only
/// when another sprite update is required.
/// Preserve MOVEM D2-D5/A2-A4 and the retained state/dialogue/palette argument
/// areas; callees that inspect the original caller frame see those real slots.
pub fn elevator_begin_ride_scene(context: &mut dyn Context) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x3c, 0x1c))?;
    let elevator_address = frame.argument_slot(0).read(context);
    let result = begin_ride_body(context, elevator_address);
    let result = result.map_err(|error| context.protect_timing_failure(error));
    if result
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        return result.map(|()| None);
    }
    let leave = frame.leave(context);
    result?;
    leave?;
    Ok(None)
}

fn begin_ride_body(context: &mut dyn Context, elevator_address: u32) -> Result<()> {
    let revision = context.revision();
    let elevator = tje_mem::Address::<tje_mem::ElevatorRecord>::new(elevator_address);
    let dispatcher = context.dispatcher();
    context.set_address_register(1, elevator.address());
    context.set_address_register(2, ram::g_player1_map_level(revision).address());
    let first = ram::g_player_info(revision).at(i32::from(elevator.entity().read(context)));
    ram::g_elevator_ride_entity(revision).write(context, tje_mem::Ptr::new(first.address()));
    let first = ram::g_elevator_ride_entity(revision).read(context).target();
    let second = first.offset(2);
    context.set_address_register(3, first.address());
    context.set_address_register(4, second.address());
    let x = ram::g_camera1_pending_x(revision)
        .read(context)
        .wrapping_add(160);
    first.x().write(context, x);
    let y = ram::g_camera1_pending_y(revision)
        .read(context)
        .wrapping_add(144);
    first.y().write(context, y);
    let level = ram::g_player1_map_level(revision).read(context);
    first.level().write(context, level);
    context.discard_u8(first.sprite_ready().address())?;
    first.sprite_ready().write(context, 0);
    let level = ram::g_player1_map_level(revision).read(context);
    second.level().write(context, level);
    context.discard_u8(second.sprite_ready().address())?;
    second.sprite_ready().write(context, 0);
    let state_arguments = OutgoingArguments::push(context, &[elevator.address(), 12])?;
    state_arguments.call(
        context,
        dispatcher,
        ids::ELEVATOR_SET_STATE,
        CallSite::relative(ids::ELEVATOR_BEGIN_RIDE_SCENE, revision, 0x6a),
    )?;
    call_at(
        context,
        ids::CLEAR_TRANSIENT_ENTITIES,
        &[],
        CallSite::relative(ids::ELEVATOR_BEGIN_RIDE_SCENE, revision, 0x70),
    )?;
    state_arguments.release(context)?;
    loop {
        call_at(
            context,
            ids::UPDATE_AND_QUEUE_ENTITY_SPRITES,
            &[],
            CallSite::relative(ids::ELEVATOR_BEGIN_RIDE_SCENE, revision, 0x78),
        )?;
        let pending = ram::g_sprite_graphics_upload_pending(revision);
        context.wait_until(WaitRequest::ram_byte_equal(pending.address(), u8::MAX, 0))?;
        if first.sprite_ready().read(context) != 0 && second.sprite_ready().read(context) != 0 {
            break;
        }
        context.poll_yield()?;
    }
    call_at(
        context,
        ids::DISABLE_BLANKING_GRAPHICS,
        &[],
        CallSite::relative(ids::ELEVATOR_BEGIN_RIDE_SCENE, revision, 0x92),
    )?;
    let dialogue_arguments = OutgoingArguments::push(context, &[first.address()])?;
    dialogue_arguments.call(
        context,
        dispatcher,
        ids::ELEVATOR_PREPARE_RIDE_DIALOGUE,
        CallSite::relative(ids::ELEVATOR_BEGIN_RIDE_SCENE, revision, 0x98),
    )?;
    call_at(
        context,
        ids::WAIT_FOR_V_BLANK_WITH_GRAPHICS_PAUSED,
        &[],
        CallSite::relative(ids::ELEVATOR_BEGIN_RIDE_SCENE, revision, 0x9e),
    )?;
    dialogue_arguments.release(context)?;
    for band in 0..4 {
        set_low_word(context, 5, band as i16);
        for row in 0..8 {
            set_low_word(context, 4, row as i16);
            context.write_vdp_command(
                (((band << 3) + row) as u32)
                    .wrapping_shl(23)
                    .wrapping_add(0x60000002),
            );
            for repeat in 0..6 {
                set_low_word(context, 3, repeat);
                for column in 0..8 {
                    set_low_word(context, 2, column as i16);
                    let mut tile = u16::from(
                        rom::transition_background_tile_offsets(revision)
                            .at(row)
                            .at(column)
                            .read(context),
                    )
                    .wrapping_add(0x2001);
                    let level = ram::g_player1_map_level(revision).read(context);
                    if rom::sprite_priority_lookup(revision)
                        .at(i32::from(level))
                        .read(context)
                        != 0
                    {
                        tile |= 0x8000;
                    }
                    context.write_vdp_data(tile);
                }
                set_low_word(context, 2, 8);
            }
            set_low_word(context, 3, 6);
        }
        set_low_word(context, 4, 8);
    }
    set_low_word(context, 5, 4);
    context.write_vdp_control(0x9100);
    context.write_vdp_control(0x9200);
    let palette_arguments = OutgoingArguments::push(
        context,
        &[rom::palette_title_screen_bank1(revision).address(), 1],
    )?;
    palette_arguments.call(
        context,
        dispatcher,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        CallSite::relative(ids::ELEVATOR_BEGIN_RIDE_SCENE, revision, 0x138),
    )?;
    call_at(
        context,
        ids::ELEVATOR_LOAD_RIDE_BACKGROUND_TILES,
        &[],
        CallSite::relative(ids::ELEVATOR_BEGIN_RIDE_SCENE, revision, 0x13c),
    )?;
    let hud_arguments = OutgoingArguments::push(context, &[2])?;
    hud_arguments.call(
        context,
        dispatcher,
        ids::HUD_HIDE_PLAYER_SPRITES,
        CallSite::relative(ids::ELEVATOR_BEGIN_RIDE_SCENE, revision, 0x146),
    )?;
    hud_arguments.release(context)?;
    palette_arguments.release(context)?;
    let fixed = ram::g_elevator_ride_scroll_y_fixed(revision).address();
    context.discard_u16(fixed)?;
    context.discard_u16(fixed.wrapping_add(2))?;
    ram::g_elevator_ride_scroll_y_fixed(revision).write(context, 0);
    context.write_vdp_command(0x40000010);
    context.write_vdp_data(ram::g_elevator_ride_scroll_y_low_word(revision).read(context) as u16);
    ram::g_elevator_ride_scroll_velocity(revision).write(context, 128);
    ram::g_elevator_ride_scroll_acceleration(revision).write(context, 1024);
    context.write_vdp_command(0x7c000003);
    context.write_vdp_data(0xffff);
    context.discard_u16(ram::g_frame_counter(revision).address())?;
    ram::g_frame_counter(revision).write(context, 0);
    ram::g_v_blank_func(revision).write(
        context,
        tje_mem::Ptr::new(ids::ELEVATOR_RIDE_V_BLANK.metadata().address(revision)),
    );
    Ok(())
}

/// G/src/elevator/elevator_scene.c:652 (`ElevatorAdvanceRidersToNextLevel`).
/// On the original invalid/no-rider paths, destination level and the elevator
/// pointer retain entry D3/A3. Preserve those values rather than initializing
/// the C locals or replacing unchecked original accesses with validation.
pub fn elevator_advance_riders_to_next_level(context: &mut dyn Context) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::link(4, 0x7c, 0x0c))?;
    let handle = frame.argument_byte(0).read(context);
    let result = advance_riders_body(context, handle);
    let result = result.map_err(|error| context.protect_timing_failure(error));
    if result
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        return result.map(|()| None);
    }
    let leave = frame.leave(context);
    result?;
    leave?;
    Ok(None)
}

fn advance_riders_body(context: &mut dyn Context, mut handle: i8) -> Result<()> {
    let revision = context.revision();
    let dispatcher = context.dispatcher();
    let caller = ids::ELEVATOR_ADVANCE_RIDERS_TO_NEXT_LEVEL;
    set_low_byte(context, 2, handle);
    set_low_byte(context, 4, 0);
    set_low_byte(context, 5, 0);
    context.set_address_register(2, ram::g_player_info(revision).address());
    let mut destination_level = context.data_register(3) as i8;
    let mut elevator =
        tje_mem::Address::<tje_mem::ElevatorRecord>::new(context.address_register(3));
    let mut rides = [false; 2];
    for player in 0..2 {
        if handle
            == ram::g_player_elevator_flags(revision)
                .at(player)
                .read(context)
        {
            let entity = ram::g_player_info(revision).at(player);
            let level = i32::from(entity.level().read(context));
            let release_earthlings = OutgoingArguments::push(context, &[level as u32])?;
            release_earthlings.call(
                context,
                dispatcher,
                ids::EARTHLING_RELEASE_LEVEL_BANK,
                CallSite::relative(caller, revision, if player == 0 { 0x2e } else { 0x62 }),
            )?;
            let level = i32::from(entity.level().read(context));
            let release_items = OutgoingArguments::push(context, &[level as u32])?;
            release_items.call(
                context,
                dispatcher,
                ids::MAP_RELEASE_LEVEL_ITEM_BANK,
                CallSite::relative(caller, revision, if player == 0 { 0x3e } else { 0x72 }),
            )?;
            let level = entity.level().read(context).wrapping_add(1);
            entity.level().write(context, level);
            destination_level = entity.level().read(context);
            set_low_byte(context, 3, destination_level);
            release_items.release(context)?;
            release_earthlings.release(context)?;
            context.set_data_register(if player == 0 { 4 } else { 5 }, 1);
            rides[player as usize] = true;
        }
    }
    if (ram::g_player_lives(revision).at(0).read(context) < 0 || rides[0])
        && (ram::g_player_lives(revision).at(1).read(context) < 0 || rides[1])
    {
        let requested = ram::g_requested_music_selection(revision);
        let current = ram::g_current_music_selection(revision);
        requested.write(context, 6);
        // MOVE.B requested,D0 precedes CMP.B current,D0 on every poll.
        while requested.read(context) != current.read(context) {
            context.poll_yield()?;
        }
        let bank = (i32::from(handle) - 1) >> 1;
        elevator = ram::g_elevator_records(revision).at(bank).at(0);
        context.set_address_register(3, elevator.address());
        call_at(
            context,
            ids::ELEVATOR_BEGIN_RIDE_SCENE,
            &[elevator.address()],
            CallSite::relative(caller, revision, 0xc8),
        )?;
    }
    for player in 0..2 {
        if rides[player as usize] {
            let entity = ram::g_player_info(revision).at(player);
            let level = i32::from(entity.level().read(context));
            let map_arguments = OutgoingArguments::push(context, &[player as u32, level as u32])?;
            map_arguments.call(
                context,
                dispatcher,
                ids::MAP_LOAD_PLAYER_LEVEL,
                CallSite::relative(caller, revision, if player == 0 { 0xe0 } else { 0x11c }),
            )?;
            let level = i32::from(entity.level().read(context));
            let retain_arguments = OutgoingArguments::push(context, &[level as u32])?;
            retain_arguments.call(
                context,
                dispatcher,
                ids::EARTHLING_RETAIN_LEVEL_BANK,
                CallSite::relative(caller, revision, if player == 0 { 0xf0 } else { 0x12c }),
            )?;
            let level = i32::from(entity.level().read(context));
            let item_arguments = OutgoingArguments::push(context, &[level as u32])?;
            item_arguments.call(
                context,
                dispatcher,
                ids::LOAD_LEVEL_ITEM_BANK,
                CallSite::relative(caller, revision, if player == 0 { 0x100 } else { 0x13c }),
            )?;
            item_arguments.release(context)?;
            retain_arguments.release(context)?;
            map_arguments.release(context)?;
        }
    }
    if ram::g_v_blank_func(revision).read(context).address() != 0 {
        call_at(
            context,
            ids::ELEVATOR_END_RIDE_SCENE,
            &[],
            CallSite::relative(caller, revision, 0x14c),
        )?;
        call_at(
            context,
            ids::ELEVATOR_SET_STATE,
            &[elevator.address(), 11],
            CallSite::relative(caller, revision, 0x156),
        )?;
    }
    call_at(
        context,
        ids::DISABLE_BLANKING_GRAPHICS,
        &[],
        CallSite::relative(caller, revision, 0x15e),
    )?;
    let mode = ram::g_mode2_reg_softcopy(revision);
    let value = mode.read(context) & 0x3c;
    mode.write(context, value);
    context.write_vdp_control(0x8104 | u16::from(mode.read(context)));
    if destination_level < 25 {
        call_at(
            context,
            ids::SETUP_ELEVATOR,
            &[i32::from(destination_level) as u32, 0],
            CallSite::relative(caller, revision, 0x18e),
        )?;
    }
    handle = call_at(
        context,
        ids::SETUP_ELEVATOR,
        &[i32::from(destination_level) as u32, 1],
        CallSite::relative(caller, revision, 0x1a0),
    )? as i8;
    set_low_byte(context, 2, handle);
    elevator = ram::g_elevator_records(revision)
        .at((i32::from(handle) - 1) >> 1)
        .at(1);
    context.set_address_register(3, elevator.address());
    let destination_y = elevator.y().read(context).wrapping_add(5);
    set_low_word(context, 6, destination_y);
    let destination_x = elevator.x().read(context);
    let destination_slot =
        tje_mem::Address::<i16>::new(context.address_register(6).wrapping_sub(2));
    destination_slot.write(context, destination_x);
    for player in [1, 0] {
        if rides[player as usize] {
            let entity = ram::g_player_info(revision).at(player);
            entity.y().write(context, destination_y);
            let destination_x =
                tje_mem::Address::<i16>::new(context.address_register(6).wrapping_sub(2))
                    .read(context);
            entity.x().write(context, destination_x);
            call_at(
                context,
                ids::HUD_LOAD_LEVEL_NUMBER_SPRITES,
                &[player as u32],
                CallSite::relative(caller, revision, if player == 1 { 0x1da } else { 0x208 }),
            )?;
            ram::g_player_elevator_flags(revision)
                .at(player)
                .write(context, handle);
            if destination_level < 26 {
                call_at(
                    context,
                    ids::TELEPHONE_PLACE_OUTSIDE_PLAYER_VIEWS,
                    &[player as u32],
                    CallSite::relative(caller, revision, if player == 1 { 0x1f2 } else { 0x21e }),
                )?;
            }
        }
    }
    if destination_level == 26 {
        call_at(
            context,
            ids::LOAD_FINAL_LEVEL_MAP,
            &[],
            CallSite::relative(caller, revision, 0x22a),
        )?;
    }
    context.discard_u8(ram::g_elevator_departure_in_progress(revision).address())?;
    ram::g_elevator_departure_in_progress(revision).write(context, 0);
    let split_arguments = OutgoingArguments::push(context, &[1])?;
    split_arguments.call(
        context,
        dispatcher,
        ids::UPDATE_SCREEN_SPLIT_MODE,
        CallSite::relative(caller, revision, 0x23a),
    )?;
    let value = mode.read(context) | 0x40;
    mode.write(context, value);
    let hardware_mode = 0x8104 | u16::from(mode.read(context));
    split_arguments.release(context)?;
    context.write_vdp_control(hardware_mode);
    call_at(
        context,
        ids::ENABLE_BLANKING_GRAPHICS,
        &[],
        CallSite::relative(caller, revision, 0x25e),
    )?;
    if ram::g_current_music_selection(revision).read(context) == 6 {
        call_at(
            context,
            ids::AUDIO_REQUEST_FM_CHANNEL_RELEASE,
            &[],
            CallSite::relative(caller, revision, 0x26e),
        )?;
        call_at(
            context,
            ids::MUSIC_REQUEST_TRACK_POINTER_RESET,
            &[],
            CallSite::relative(caller, revision, 0x274),
        )?;
    }
    Ok(())
}

fn set_low_byte(context: &mut dyn Context, register: usize, value: i8) {
    let high = context.data_register(register) & 0xffffff00;
    context.set_data_register(register, high | u32::from(value as u8));
}
