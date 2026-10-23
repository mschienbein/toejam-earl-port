use crate::{Context, Result, call, ids};
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
        call(context, ids::BUILD_VIEWPORT_SPRITE_LINKS, &[])?;
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
pub fn elevator_prepare_ride_dialogue(
    context: &mut dyn Context,
    args: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let entity = tje_mem::Address::<ObjectState>::new(args[0]);
    let lines = ram::g_elevator_ride_speech_entities(revision);
    if ram::g_player_lives(revision).at(0).read(context) >= 0
        && ram::g_player_lives(revision).at(1).read(context) >= 0
    {
        let random = call(context, ids::GET_RANDOM_NUMBER, &[])? as u16;
        if u32::from(random) % 100 < 75 {
            let mut speech =
                (u32::from(call(context, ids::GET_RANDOM_NUMBER, &[])? as u16) % 14) as i16;
            let mask = ram::g_elevator_ride_used_dialogue_mask(revision);
            let mut attempt = 0;
            while attempt < 14 {
                if (1_i32.wrapping_shl(speech as u32) & mask.read(context)) == 0 {
                    break;
                }
                speech = ((i32::from(speech) + 1) % 14) as i16;
                attempt += 1;
            }
            if attempt != 14 {
                let used = mask.read(context) | 1_i32.wrapping_shl(speech as u32);
                mask.write(context, used);
                let x = entity.x().read(context).wrapping_sub(24);
                entity.x().write(context, x);
                let descriptor = rom::speech_string_pointers(revision).at(i32::from(speech));
                let text = descriptor.first_line().read(context).address();
                let speaker = i32::from(entity.entity_index().read(context));
                let first = call(context, ids::GENERATE_SPEECH, &[speaker as u32, text, 0])? as i16;
                let mut second = 0_i16;
                if descriptor.second_line().read(context).address() != 0 {
                    let text = descriptor.second_line().read(context).address();
                    let speaker = i32::from(entity.entity_index().read(context));
                    second =
                        call(context, ids::GENERATE_SPEECH, &[speaker as u32, text, 0])? as i16;
                }
                let x = entity.x().read(context).wrapping_add(40);
                entity.x().write(context, x);
                let text = descriptor.reply_first_line().read(context).address();
                let speaker = i32::from(entity.entity_index().read(context));
                let reply_first =
                    call(context, ids::GENERATE_SPEECH, &[speaker as u32, text, 0])? as i16;
                let mut reply_second = 0_i16;
                if descriptor.reply_second_line().read(context).address() != 0 {
                    let text = descriptor.reply_second_line().read(context).address();
                    let speaker = i32::from(entity.entity_index().read(context));
                    reply_second =
                        call(context, ids::GENERATE_SPEECH, &[speaker as u32, text, 0])? as i16;
                }
                let x = entity.x().read(context).wrapping_sub(16);
                entity.x().write(context, x);
                call(context, ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, &[])?;
                for (index, line) in [first, second, reply_first, reply_second]
                    .into_iter()
                    .enumerate()
                {
                    lines.at(index as i32).write(context, line as u8);
                    if line as u8 != 0 {
                        ram::g_player_info(revision)
                            .cast::<ObjectState>()
                            .byte_offset(i32::from(line.wrapping_shl(7)))
                            .sprite_ready()
                            .write(context, 0);
                    }
                }
                call(context, ids::BUILD_VIEWPORT_SPRITE_LINKS, &[])?;
                return Ok(None);
            }
        }
    }
    for index in 0..4 {
        lines.at(index).write(context, 0);
    }
    Ok(None)
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
    call(context, ids::ELEVATOR_ALTERNATE_RIDE_ENTITIES, &[])?;
    call(context, ids::SELECT_PRIMARY_VIEWPORT_SPRITES, &[])?;
    call(context, ids::MUSIC_UPDATE_SEQUENCER, &[])?;
    let counter = ram::g_frame_counter(revision);
    let next = counter.read(context).wrapping_add(1);
    counter.write(context, next);
    Ok(None)
}

/// G/src/elevator/elevator_scene.c:633 (`ElevatorEndRideScene`).
pub fn elevator_end_ride_scene(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let velocity = ram::g_elevator_ride_scroll_velocity(revision);
    while velocity.read(context) < 2048 {
        context.wait_vblank_frame(velocity.address(), velocity.read(context) as u16)?;
    }
    ram::g_elevator_ride_scroll_acceleration(revision).write(context, -2048);
    while velocity.read(context) > 0 {
        context.wait_vblank_frame(velocity.address(), velocity.read(context) as u16)?;
    }
    ram::g_v_blank_func(revision).write(context, tje_mem::Ptr::new(0));
    call(context, ids::CLEAR_SPEECH_BUBBLES_AND_REPLIES, &[])?;
    call(context, ids::DISABLE_BLANKING_GRAPHICS, &[])?;
    let mode = ram::g_mode2_reg_softcopy(revision);
    let value = mode.read(context) & 0x3c;
    mode.write(context, value);
    context.write_vdp_control(0x8104 | u16::from(mode.read(context)));
    call(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_game_line2(revision).address(), 1],
    )?;
    context.write_vdp_command(0x40000000);
    call(
        context,
        ids::MOVE_DATA_TO_VRAM,
        &[rom::level_tiles(revision).address(), 1040],
    )?;
    Ok(None)
}

/// G/src/elevator/elevator_scene.c:579 (`ElevatorBeginRideScene`).
/// The pending-byte spin suspends until a real IRQ can change its containing
/// RAM word; every wake rechecks the byte, never fabricating upload completion.
pub fn elevator_begin_ride_scene(context: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let elevator = tje_mem::Address::<tje_mem::ElevatorRecord>::new(args[0]);
    let first = ram::g_player_info(revision).at(i32::from(elevator.entity().read(context)));
    ram::g_elevator_ride_entity(revision).write(context, tje_mem::Ptr::new(first.address()));
    let first = ram::g_elevator_ride_entity(revision).read(context).target();
    let second = first.offset(2);
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
    first.sprite_ready().write(context, 0);
    let level = ram::g_player1_map_level(revision).read(context);
    second.level().write(context, level);
    second.sprite_ready().write(context, 0);
    call(context, ids::ELEVATOR_SET_STATE, &[elevator.address(), 12])?;
    call(context, ids::CLEAR_TRANSIENT_ENTITIES, &[])?;
    loop {
        call(context, ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, &[])?;
        let pending = ram::g_sprite_graphics_upload_pending(revision);
        while pending.read(context) != 0 {
            let address = pending.address() & !1;
            let previous = context.read_u16(address);
            context.wait_vblank_frame(address, previous)?;
        }
        if first.sprite_ready().read(context) != 0 && second.sprite_ready().read(context) != 0 {
            break;
        }
    }
    call(context, ids::DISABLE_BLANKING_GRAPHICS, &[])?;
    call(
        context,
        ids::ELEVATOR_PREPARE_RIDE_DIALOGUE,
        &[first.address()],
    )?;
    call(context, ids::WAIT_FOR_V_BLANK_WITH_GRAPHICS_PAUSED, &[])?;
    for band in 0..4 {
        for row in 0..8 {
            context.write_vdp_command(
                (((band << 3) + row) as u32)
                    .wrapping_shl(23)
                    .wrapping_add(0x60000002),
            );
            for _ in 0..6 {
                for column in 0..8 {
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
            }
        }
    }
    context.write_vdp_control(0x9100);
    context.write_vdp_control(0x9200);
    call(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_title_screen_bank1(revision).address(), 1],
    )?;
    call(context, ids::ELEVATOR_LOAD_RIDE_BACKGROUND_TILES, &[])?;
    call(context, ids::HUD_HIDE_PLAYER_SPRITES, &[2])?;
    ram::g_elevator_ride_scroll_y_fixed(revision).write(context, 0);
    context.write_vdp_command(0x40000010);
    context.write_vdp_data(ram::g_elevator_ride_scroll_y_low_word(revision).read(context) as u16);
    ram::g_elevator_ride_scroll_velocity(revision).write(context, 128);
    ram::g_elevator_ride_scroll_acceleration(revision).write(context, 1024);
    context.write_vdp_command(0x7c000003);
    context.write_vdp_data(0xffff);
    ram::g_frame_counter(revision).write(context, 0);
    ram::g_v_blank_func(revision).write(
        context,
        tje_mem::Ptr::new(ids::ELEVATOR_RIDE_V_BLANK.metadata().address(revision)),
    );
    Ok(None)
}

/// G/src/elevator/elevator_scene.c:652 (`ElevatorAdvanceRidersToNextLevel`).
/// On the original invalid/no-rider paths, destination level and the elevator
/// pointer retain entry D3/A3. Preserve those values rather than initializing
/// the C locals or replacing unchecked original accesses with validation.
pub fn elevator_advance_riders_to_next_level(
    context: &mut dyn Context,
    args: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let mut handle = args[0] as i8;
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
            call(context, ids::EARTHLING_RELEASE_LEVEL_BANK, &[level as u32])?;
            let level = i32::from(entity.level().read(context));
            call(context, ids::MAP_RELEASE_LEVEL_ITEM_BANK, &[level as u32])?;
            let level = entity.level().read(context).wrapping_add(1);
            entity.level().write(context, level);
            destination_level = entity.level().read(context);
            rides[player as usize] = true;
        }
    }
    if (ram::g_player_lives(revision).at(0).read(context) < 0 || rides[0])
        && (ram::g_player_lives(revision).at(1).read(context) < 0 || rides[1])
    {
        let requested = ram::g_requested_music_selection(revision);
        let current = ram::g_current_music_selection(revision);
        requested.write(context, 6);
        while current.read(context) != requested.read(context) {
            let address = current.address() & !1;
            let previous = context.read_u16(address);
            context.wait_vblank_frame(address, previous)?;
        }
        let bank = (i32::from(handle) - 1) >> 1;
        elevator = ram::g_elevator_records(revision).at(bank).at(0);
        call(
            context,
            ids::ELEVATOR_BEGIN_RIDE_SCENE,
            &[elevator.address()],
        )?;
    }
    for player in 0..2 {
        if rides[player as usize] {
            let entity = ram::g_player_info(revision).at(player);
            let level = i32::from(entity.level().read(context));
            call(
                context,
                ids::MAP_LOAD_PLAYER_LEVEL,
                &[player as u32, level as u32],
            )?;
            let level = i32::from(entity.level().read(context));
            call(context, ids::EARTHLING_RETAIN_LEVEL_BANK, &[level as u32])?;
            let level = i32::from(entity.level().read(context));
            call(context, ids::LOAD_LEVEL_ITEM_BANK, &[level as u32])?;
        }
    }
    if ram::g_v_blank_func(revision).read(context).address() != 0 {
        call(context, ids::ELEVATOR_END_RIDE_SCENE, &[])?;
        call(context, ids::ELEVATOR_SET_STATE, &[elevator.address(), 11])?;
    }
    call(context, ids::DISABLE_BLANKING_GRAPHICS, &[])?;
    let mode = ram::g_mode2_reg_softcopy(revision);
    let value = mode.read(context) & 0x3c;
    mode.write(context, value);
    context.write_vdp_control(0x8104 | u16::from(mode.read(context)));
    if destination_level < 25 {
        call(
            context,
            ids::SETUP_ELEVATOR,
            &[i32::from(destination_level) as u32, 0],
        )?;
    }
    handle = call(
        context,
        ids::SETUP_ELEVATOR,
        &[i32::from(destination_level) as u32, 1],
    )? as i8;
    elevator = ram::g_elevator_records(revision)
        .at((i32::from(handle) - 1) >> 1)
        .at(1);
    let destination_y = elevator.y().read(context).wrapping_add(5);
    let destination_x = elevator.x().read(context);
    for player in [1, 0] {
        if rides[player as usize] {
            let entity = ram::g_player_info(revision).at(player);
            entity.y().write(context, destination_y);
            entity.x().write(context, destination_x);
            call(
                context,
                ids::HUD_LOAD_LEVEL_NUMBER_SPRITES,
                &[player as u32],
            )?;
            ram::g_player_elevator_flags(revision)
                .at(player)
                .write(context, handle);
            if destination_level < 26 {
                call(
                    context,
                    ids::TELEPHONE_PLACE_OUTSIDE_PLAYER_VIEWS,
                    &[player as u32],
                )?;
            }
        }
    }
    if destination_level == 26 {
        call(context, ids::LOAD_FINAL_LEVEL_MAP, &[])?;
    }
    ram::g_elevator_departure_in_progress(revision).write(context, 0);
    call(context, ids::UPDATE_SCREEN_SPLIT_MODE, &[1])?;
    let value = mode.read(context) | 0x40;
    mode.write(context, value);
    context.write_vdp_control(0x8104 | u16::from(mode.read(context)));
    call(context, ids::ENABLE_BLANKING_GRAPHICS, &[])?;
    if ram::g_current_music_selection(revision).read(context) == 6 {
        call(context, ids::AUDIO_REQUEST_FM_CHANNEL_RELEASE, &[])?;
        call(context, ids::MUSIC_REQUEST_TRACK_POINTER_RESET, &[])?;
    }
    Ok(None)
}
