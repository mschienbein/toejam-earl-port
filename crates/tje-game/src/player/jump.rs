use crate::{Context, Result, call, ids};
use tje_mem::{Address, ObjectState, ram, rom, rom_symbol};

/// G/src/player/jump_motion.c:117 (`PlayerApplyPhasedJumpInput`).
pub(super) fn player_apply_phased_jump_input(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let state = Address::<ObjectState>::new(arguments[0]);
    let player = i32::from(arguments[1] as i16);
    let phase = arguments[2] as i8;
    let jump = arguments[3] as i8;
    let revision = context.revision();
    let input = if ram::g_player_control_remap_entities(revision)
        .at(player)
        .read(context)
        != 0
    {
        call(
            context,
            ids::PLAYER_REMAP_DIRECTIONAL_INPUT,
            &[player as u32],
        )? as u8
    } else {
        ram::g_player_processed_input(revision)
            .at(player)
            .read(context)
    };
    let mut vx = state.velocity_x().read(context);
    let mut vy = state.velocity_y().read(context);
    let left = input & 4 != 0;
    let right = input & 8 != 0;
    let up = input & 1 != 0;
    let down = input & 2 != 0;
    if phase != -1 {
        ram::g_player_phased_jump_phase(revision)
            .at(player)
            .write(context, phase);
        state.frame().write(context, phase.wrapping_sub(1));
        state.frame_duration().write(context, 1);
        if jump != 0 {
            if state.height().read(context) < 50 {
                state.velocity_height().write(context, 256);
                state.acceleration_height().write(context, -10);
            } else {
                state.velocity_height().write(context, 0);
            }
            if (vx > 0 && right) || (vx < 0 && left) {
                vx = vx.wrapping_shl(1);
            } else if (vx < 0 && right) || (vx > 0 && left) {
                vx = 0;
            }
            if (vy > 0 && down) || (vy < 0 && up) {
                vy = vy.wrapping_shl(1);
            } else if (vy < 0 && down) || (vy > 0 && up) {
                vy = 0;
            }
        }
    }
    if vx > 0 && !right {
        vx = vx.wrapping_sub(10);
        if vx < 0 {
            vx = 0;
        }
    } else if vx < 0 && !left {
        vx = vx.wrapping_add(10);
        if vx > 0 {
            vx = 0;
        }
    }
    if vy > 0 && !down {
        vy = vy.wrapping_sub(10);
        if vy < 0 {
            vy = 0;
        }
    } else if vy < 0 && !up {
        vy = vy.wrapping_add(10);
        if vy > 0 {
            vy = 0;
        }
    }
    let mut maximum = if vy != 0 { 384 } else { 512 };
    if i32::from(vx).abs() > maximum {
        vx = if vx > 0 {
            maximum as i16
        } else {
            -maximum as i16
        };
    }
    maximum -= if vx != 0 { 128 } else { 64 };
    if i32::from(vy).abs() > maximum {
        vy = if vy > 0 {
            maximum as i16
        } else {
            -maximum as i16
        };
    }
    state.velocity_x().write(context, vx);
    state.velocity_y().write(context, vy);
    if ram::g_player_phased_jump_speech_used(revision)
        .at(player)
        .read(context)
        == 0
        && state.height().read(context) > 10
    {
        let text = rom_symbol(revision, "GameText_01201E")
            .expect("source speech symbol")
            .address;
        call(context, ids::GENERATE_SPEECH, &[player as u32, text, 1])?;
        ram::g_player_phased_jump_speech_used(revision)
            .at(player)
            .write(context, 1);
    }
    Ok(None)
}

/// G/src/player/jump_motion.c:21 (`PlayerUpdatePhasedJumpMotion`).
pub(super) fn player_update_phased_jump_motion(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let state = Address::<ObjectState>::new(arguments[0]);
    let terrain = arguments[1] as u16;
    let revision = context.revision();
    let player = i32::from(state.entity_index().read(context));
    if ram::g_player_powerups(revision).at(player).read(context) != 0 {
        return Ok(None);
    }
    if ram::g_player_terrain_sinking(revision)
        .at(player)
        .read(context)
        == 0
        && state.animation_group_mask().read(context) & 0xc != 0
    {
        state.height().write(context, 0);
        state
            .draw_flags()
            .write(context, state.draw_flags().read(context) & !0x40);
        return Ok(None);
    }
    let resting = |context: &dyn Context| {
        state.sprite_state().read(context) == 0
            || state.sprite_state().read(context) == 4
            || state.animation_group_mask().read(context) & 0x2040_0000 != 0
            || state.sprite_state().read(context) == 76
    };
    if ram::g_player_fall_states(revision).at(player).read(context) != 0
        || !(resting(context) || state.animation_group_mask().read(context) & 0x8000 != 0)
        || ram::g_player_elevator_flags(revision)
            .at(player)
            .read(context)
            != 0
        || (ram::g_player_terrain_sinking(revision)
            .at(player)
            .read(context)
            != 0
            && state.height().read(context) < -4)
    {
        return Ok(None);
    }
    let direction = state.sprite_state().read(context) & 3;
    let mut phase = -1_i8;
    let jump = ram::g_player_processed_input(revision)
        .at(player)
        .read(context)
        & 0x40;
    if jump == 0 {
        ram::g_player_phased_jump_armed(revision)
            .at(player)
            .write(context, 1);
        ram::g_player_choice_action_release_latches(revision)
            .at(player)
            .write(context, 0);
    }
    if ram::g_player_choice_action_release_latches(revision)
        .at(player)
        .read(context)
        != 0
    {
        return Ok(None);
    }
    let current_phase = ram::g_player_phased_jump_phase(revision).at(player);
    let ticks = ram::g_player_phased_jump_phase_ticks(revision).at(player);
    if resting(context) {
        if jump == 0
            || ram::g_player_phased_jump_armed(revision)
                .at(player)
                .read(context)
                == 0
        {
            return Ok(None);
        }
        phase = 1;
    } else if current_phase.read(context) == 0 && jump == 0 {
        if ticks.read(context) > 1 {
            phase = 1;
        }
    } else if current_phase.read(context) == 1 && jump != 0 {
        phase = 0;
        ticks.write(context, 0);
        call(context, ids::PLAY_SOUND, &[23, 20])?;
    } else if state.height().read(context) < 4 {
        state.velocity_x().write(context, 0);
        state.acceleration_x().write(context, 0);
        state.velocity_y().write(context, 0);
        state.acceleration_y().write(context, 0);
        if state.velocity_height().read(context) < 0 {
            state.height().write(context, 0);
            state.velocity_height().write(context, 0);
            state.acceleration_height().write(context, 0);
            state
                .draw_flags()
                .write(context, state.draw_flags().read(context) & !0x40);
            if terrain >= 0x80 {
                if terrain == 0x90 {
                    ram::g_player_jump_counters(revision)
                        .at(player)
                        .write(context, 0);
                } else {
                    phase = direction + 64;
                    call(
                        context,
                        ids::PLAY_SOUND_EFFECT,
                        &[rom::sfx_splash(revision).address(), 50, 0],
                    )?;
                }
            } else {
                phase = call(context, ids::GET_ACTIVE_PRESENT_SPRITE, &[state.address()])? as i8;
                state.animation_motion_axes().write(context, 3);
                ram::g_player_phased_jump_armed(revision)
                    .at(player)
                    .write(context, 0);
            }
            if phase != -1 {
                call(context, ids::SET_SPRITE, &[state.address(), phase as u32])?;
                phase = -1;
            }
        } else if terrain == 0x90 {
            let counter = ram::g_player_jump_counters(revision).at(player);
            if counter.read(context) > 10 {
                call(
                    context,
                    ids::PLAYER_START_FALLING,
                    &[state.address(), player as u32],
                )?;
            } else if ticks.read(context) > 1 {
                phase = current_phase.read(context) ^ 1;
                ticks.write(context, 0);
                counter.write(context, counter.read(context).wrapping_add(1));
            }
        }
        if phase == -1 {
            return Ok(None);
        }
    }
    if ticks.read(context) < 64 {
        ticks.write(context, ticks.read(context).wrapping_add(1));
    }
    if state.animation_group_mask().read(context) & 0x8000 == 0 {
        call(context, ids::SET_SPRITE, &[state.address(), 61])?;
        state
            .draw_flags()
            .write(context, state.draw_flags().read(context) | 0x40);
        state.animation_motion_axes().write(context, 0);
    } else if terrain == 0x90 {
        state
            .draw_flags()
            .write(context, state.draw_flags().read(context) & !0x40);
    } else {
        state
            .draw_flags()
            .write(context, state.draw_flags().read(context) | 0x40);
    }
    call(
        context,
        ids::PLAYER_APPLY_PHASED_JUMP_INPUT,
        &[
            state.address(),
            player as u32,
            phase as u32,
            u32::from(jump),
        ],
    )?;
    Ok(None)
}
