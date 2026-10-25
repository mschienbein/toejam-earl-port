use crate::call_frame::{CallSite, FrameError};
use crate::{Context, Result, call, call_at_void, call_void, ids};
use tje_mem::{Address, TelephoneState, ram, rom};

/// G/src/presents/telephone.c:52 (`TelephoneStartRing`).
/// Atomic: queued PCM request and RAM animation/timer stores, no ringing wait.
pub fn telephone_start_ring(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let index = i32::from(arguments[0] as i16);
    let telephone = Address::<TelephoneState>::new(arguments[1]);
    if ram::g_elevator_departure_in_progress(revision).read(context) == 0 {
        call_void(
            context,
            ids::PLAY_SOUND_EFFECT,
            &[rom::sfx_telephone(revision).address(), 50, 0],
        )?;
    }
    ram::g_telephone_time_to_ring(revision)
        .at(index)
        .write(context, 60);
    if telephone.object().read(context) != 0 {
        let object = ram::g_player_info(revision).at(i32::from(telephone.object().read(context)));
        let animation = rom::telephone_animations(revision).at(0);
        object
            .sequence()
            .write(context, animation.sequence().read(context));
        object.frame().write(context, -1);
        object.frame_duration().write(context, 1);
        object.frame_phase().write(context, 3);
    }
    Ok(None)
}

/// G/src/presents/telephone_open.c:39 (`OpenTelephone`).
/// Atomic: two RAM telephone records, coordinate adjustment and a transient effect.
pub fn open_telephone(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let index = i32::from(arguments[0] as i16);
    let mut player = super::word_indexed_player(revision, index);
    let states = ram::g_telephone_states(revision);
    let activated = ram::g_telephone_activated(revision);
    let number = if states.at(index).level().read(context) == 0 {
        index
    } else if states.at(index ^ 1).level().read(context) == 0 {
        index ^ 1
    } else if activated.at(index).read(context) == 0 {
        index
    } else if activated.at(index ^ 1).read(context) == 0 {
        index ^ 1
    } else {
        index
    };
    let telephone = states.at(number);
    telephone
        .level()
        .write(context, player.level().read(context));
    telephone.x().write(context, player.x().read(context));
    telephone
        .y()
        .write(context, player.y().read(context).wrapping_add(10));
    if ram::g_player_lives(revision).at(index ^ 1).read(context) >= 0 {
        // The XOR expression is promoted first: this separate ASL.L path must
        // retain full-width scaling (G's c_telephone_open.s:192-198).
        player = ram::g_player_info(revision).at(index ^ 1);
        if player.level().read(context) == telephone.level().read(context)
            && 16
                > (i32::from(telephone.x().read(context)) - i32::from(player.x().read(context)))
                    .wrapping_abs()
        {
            while 8
                > (i32::from(telephone.y().read(context)) - i32::from(player.y().read(context)))
                    .wrapping_abs()
            {
                telephone
                    .y()
                    .write(context, telephone.y().read(context).wrapping_add(1));
            }
        }
    }
    activated.at(number).write(context, 0);
    let effect_arguments = [
        index as u32,
        telephone.x().read(context) as u32,
        (i32::from(telephone.y().read(context)) + 1) as u32,
        0,
        telephone.level().read(context) as u32,
    ];
    call(
        context,
        ids::PRESENT_START_TRANSIENT_EFFECT,
        &effect_arguments,
    )?;
    Ok(None)
}

/// G/src/presents/telephone_clear.c:7 (`TelephoneClearAll`).
/// Atomic: two RAM records and storage-only entity releases.
pub fn telephone_clear_all(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    for index in 0..2 {
        let telephone = ram::g_telephone_states(revision).at(index);
        if telephone.level().read(context) != 0 {
            let number = telephone.object().read(context);
            if number != 0 {
                call_void(context, ids::FREE_ENTITY, &[number as u32])?;
                telephone.object().write(context, 0);
            }
            telephone.level().write(context, 0);
            ram::g_telephone_activated(revision)
                .at(index)
                .write(context, 0);
            ram::g_telephone_rings_left(revision)
                .at(index)
                .write(context, 0);
            ram::g_telephone_time_to_ring(revision)
                .at(index)
                .write(context, 0);
        }
    }
    Ok(None)
}

/// G/src/presents/telephone.c:25 (`TelephoneCreateDisplayEntity`).
/// Atomic: RAM allocation and ROM animation pointers; no port transactions.
pub fn telephone_create_display_entity(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let telephone = Address::<TelephoneState>::new(arguments[0]);
    let number = call(context, ids::ALLOCATE_ENTITY_WITH_EVICTION, &[0])? as i8;
    telephone.object().write(context, number);
    if number != 0 {
        let object = ram::g_player_info(revision).at(i32::from(number));
        let animation = rom::telephone_animations(revision).at(0);
        object
            .frames()
            .write(context, animation.frames().read(context));
        object.sequence().write(
            context,
            tje_mem::Ptr::new(rom::single_frame_hold_animation(revision).address()),
        );
        object.frame_phase().write(context, 0);
        object.frame_duration().write(context, 1);
        object.frame().write(context, -1);
        object.x().write(context, telephone.x().read(context));
        object.y().write(context, telephone.y().read(context));
        object
            .level()
            .write(context, telephone.level().read(context));
        object.width().write(context, 24);
        object.height_pixels().write(context, 16);
        object.sprite_count().write(context, 1);
        object.tile_count().write(context, 6);
        object.tile().write(context, 0x8000);
        object
            .draw_flags()
            .write(context, object.draw_flags().read(context) | 8);
    }
    Ok(None)
}

/// G/src/presents/telephone.c:72 (`CheckForTelephone`).
/// TimeAware: answering enters DrawGameMap's VBlank and controller-input loops.
pub fn check_for_telephone(context: &mut dyn Context) -> Result<Option<u32>> {
    let frame = context.frame_state().ok_or(FrameError::Unavailable)?;
    let revision = context.revision();
    let index = i32::from(frame.argument_word(0).read(context));
    let telephone = ram::g_telephone_states(revision).at(index);
    if telephone.level().read(context) == 0 {
        return Ok(None);
    }
    let first_x = telephone
        .x()
        .read(context)
        .wrapping_sub(ram::g_camera1_pending_x(revision).read(context));
    let first_y = telephone
        .y()
        .read(context)
        .wrapping_sub(ram::g_camera1_pending_y(revision).read(context));
    let second_x = telephone
        .x()
        .read(context)
        .wrapping_sub(ram::g_camera2_pending_x(revision).read(context));
    let second_y = telephone
        .y()
        .read(context)
        .wrapping_sub(ram::g_camera2_pending_y(revision).read(context));
    if (ram::g_player1_map_level(revision).read(context) == telephone.level().read(context)
        && i32::from(first_x) + 20 > 0
        && first_x < 340
        && i32::from(first_y) + 20 > 0
        && first_y < 244)
        || (ram::g_player2_map_level(revision).read(context) == telephone.level().read(context)
            && ram::g_split_screen_enabled(revision).read(context) != 0
            && i32::from(second_x) + 20 > 0
            && second_x < 340
            && i32::from(second_y) + 20 > 0
            && second_y < 132)
    {
        if telephone.object().read(context) == 0 {
            call_void(
                context,
                ids::TELEPHONE_CREATE_DISPLAY_ENTITY,
                &[telephone.address()],
            )?;
        }
    } else if telephone.object().read(context) != 0 {
        let number = telephone.object().read(context);
        call_void(context, ids::FREE_ENTITY, &[number as u32])?;
        context.discard_u8(telephone.object().address())?; // CLR.B.
        telephone.object().write(context, 0);
    }
    let activated = ram::g_telephone_activated(revision).at(index);
    if activated.read(context) == 0 {
        for player_number in 0..2 {
            let player = ram::g_player_info(revision).at(player_number);
            if ram::g_player_lives(revision)
                .at(player_number)
                .read(context)
                >= 0
                && telephone.level().read(context) == player.level().read(context)
                && ram::g_pcm_effect_cooldown_ticks(revision).read(context) == 0
                && {
                    let dx = i32::from(telephone.x().read(context))
                        - i32::from(player.x().read(context));
                    (call(context, ids::ABS, &[dx as u32])? as i32) < 320
                }
                && {
                    let dy = i32::from(telephone.y().read(context))
                        - i32::from(player.y().read(context));
                    (call(context, ids::ABS, &[dy as u32])? as i32) < 224
                }
            {
                activated.write(context, 1);
                ram::g_telephone_rings_left(revision)
                    .at(index)
                    .write(context, 6);
                call_void(
                    context,
                    ids::TELEPHONE_START_RING,
                    &[index as u32, telephone.address()],
                )?;
                break;
            }
        }
        if activated.read(context) == 0 {
            return Ok(None);
        }
    }
    let delay = ram::g_telephone_time_to_ring(revision).at(index);
    let remaining = super::predecrement_i8(context, delay);
    if remaining <= 0 {
        let rings = ram::g_telephone_rings_left(revision).at(index);
        let remaining = super::predecrement_i8(context, rings);
        if remaining > 0
            && (ram::g_player_info(revision).at(0).level().read(context)
                == telephone.level().read(context)
                || ram::g_player_info(revision).at(1).level().read(context)
                    == telephone.level().read(context))
        {
            if ram::g_pcm_effect_cooldown_ticks(revision).read(context) == 0 {
                call_void(
                    context,
                    ids::TELEPHONE_START_RING,
                    &[index as u32, telephone.address()],
                )?;
            } else {
                rings.write(context, rings.read(context).wrapping_add(1));
                delay.write(context, 1);
            }
        } else {
            if telephone.object().read(context) != 0 {
                let number = telephone.object().read(context);
                call_void(context, ids::FREE_ENTITY, &[number as u32])?;
                context.discard_u8(telephone.object().address())?; // CLR.B.
                telephone.object().write(context, 0);
                let level = telephone.level().read(context);
                let y = i32::from(telephone.y().read(context)) + 1;
                let x = telephone.x().read(context);
                let args = [index as u32, x as u32, y as u32, 0, level as u32];
                call(context, ids::PRESENT_START_TRANSIENT_EFFECT, &args)?;
            }
            context.discard_u8(telephone.level().address())?; // CLR.B.
            telephone.level().write(context, 0);
        }
    }
    if telephone.object().read(context) != 0 {
        for player_number in 0..2 {
            let player = ram::g_player_info(revision).at(player_number);
            if ram::g_player_lives(revision)
                .at(player_number)
                .read(context)
                >= 0
                && telephone.level().read(context) == player.level().read(context)
                && {
                    let dx = i32::from(telephone.x().read(context))
                        - i32::from(player.x().read(context));
                    (call(context, ids::ABS, &[dx as u32])? as i32) < 16
                }
                && {
                    let dy = i32::from(telephone.y().read(context))
                        - i32::from(player.y().read(context));
                    (call(context, ids::ABS, &[dy as u32])? as i32) < 8
                }
            {
                let number = telephone.object().read(context);
                call_void(context, ids::FREE_ENTITY, &[number as u32])?;
                context.discard_u8(telephone.object().address())?; // CLR.B.
                telephone.object().write(context, 0);
                ram::g_map_reveal_mode(revision).write(context, 1);
                call_at_void(
                    context,
                    ids::DRAW_GAME_MAP,
                    &[player_number as u32],
                    CallSite::relative(ids::CHECK_FOR_TELEPHONE, revision, 0x304),
                )?;
                context.discard_u16(ram::g_map_reveal_mode(revision).address())?; // CLR.W.
                ram::g_map_reveal_mode(revision).write(context, 0);
                context.discard_u8(telephone.level().address())?; // CLR.B.
                telephone.level().write(context, 0);
                break;
            }
        }
    }
    Ok(None)
}
