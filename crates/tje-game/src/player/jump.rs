use super::frames::{call_from, clear_byte, clear_word, read_long, with_frame};
use crate::call_frame::FrameLayout;
use crate::{Context, Result, ids};
use tje_mem::{Address, ObjectState, ram, rom, rom_symbol};

/// G/src/player/jump_motion.c:117 (`PlayerApplyPhasedJumpInput`).
pub(super) fn player_apply_phased_jump_input(context: &mut dyn Context) -> Result<Option<u32>> {
    // G/build/{rev00,rev02}/asm/c_jump_motion.s:276-300,321:
    // LINK -4, D2-D6/A2; the jump byte is not read on phase == -1.
    with_frame(
        context,
        FrameLayout::link(4, 0x7c, 0x04),
        |context, stack| {
            let state = Address::<ObjectState>::new(stack.argument_slot(0).read(context));
            let player = i32::from(stack.argument_word(1).read(context));
            let phase = stack.argument_byte(2).read(context);
            let revision = context.revision();
            let input = if ram::g_player_control_remap_entities(revision)
                .at(player)
                .read(context)
                != 0
            {
                call_from(
                    context,
                    ids::PLAYER_APPLY_PHASED_JUMP_INPUT,
                    ids::PLAYER_REMAP_DIRECTIONAL_INPUT,
                    &[player as u32],
                    0x30,
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
                if stack.argument_byte(3).read(context) != 0 {
                    if state.height().read(context) < 50 {
                        state.velocity_height().write(context, 256);
                        state.acceleration_height().write(context, -10);
                    } else {
                        clear_word(context, state.velocity_height().address())?;
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
            let absolute_x = call_from(
                context,
                ids::PLAYER_APPLY_PHASED_JUMP_INPUT,
                ids::ABS,
                &[i32::from(vx) as u32],
                0x194,
            )? as i32;
            if absolute_x > maximum {
                vx = if vx > 0 {
                    maximum as i16
                } else {
                    -maximum as i16
                };
            }
            maximum -= if vx != 0 { 128 } else { 64 };
            let absolute_y = call_from(
                context,
                ids::PLAYER_APPLY_PHASED_JUMP_INPUT,
                ids::ABS,
                &[i32::from(vy) as u32],
                0x1d2,
            )? as i32;
            if absolute_y > maximum {
                vy = if vy > 0 {
                    maximum as i16
                } else {
                    -maximum as i16
                };
            }
            state.velocity_x().write(context, vx);
            state.velocity_y().write(context, vy);
            let used = ram::g_player_phased_jump_speech_used(revision).at(player);
            let previous = used.read(context);
            if previous == 0 {
                let height = state.height().read(context);
                if height > 10 {
                    let text = rom_symbol(revision, "GameText_01201E")
                        .expect("source speech symbol")
                        .address;
                    call_from(
                        context,
                        ids::PLAYER_APPLY_PHASED_JUMP_INPUT,
                        ids::GENERATE_SPEECH,
                        &[player as u32, text, 1],
                        0x228,
                    )?;
                    used.write(context, 1);
                }
            }
            Ok(None)
        },
    )
}

/// G/src/player/jump_motion.c:21 (`PlayerUpdatePhasedJumpMotion`).
pub(super) fn player_update_phased_jump_motion(context: &mut dyn Context) -> Result<Option<u32>> {
    // c_jump_motion.s:4-6,269-271: MOVEM D2-D5/A2-A4, no LINK.
    with_frame(
        context,
        FrameLayout::movem(0, 0x3c, 0x1c),
        |context, stack| {
            let state = Address::<ObjectState>::new(stack.argument_slot(0).read(context));
            let terrain = stack.argument_word(1).cast::<u16>().read(context);
            let revision = context.revision();
            let player = i32::from(state.entity_index().read(context));
            let powerup = ram::g_player_powerups(revision).at(player).read(context);
            if powerup != 0 {
                return Ok(None);
            }
            if ram::g_player_terrain_sinking(revision)
                .at(player)
                .read(context)
                == 0
                && read_long(context, state.animation_group_mask().address()) & 0xc != 0
            {
                clear_word(context, state.height().address())?;
                let draw = state.draw_flags().read(context) & !0x40;
                state.draw_flags().write(context, draw);
                return Ok(None);
            }
            let resting = |context: &mut dyn Context| {
                state.sprite_state().read(context) == 0
                    || state.sprite_state().read(context) == 4
                    || read_long(context, state.animation_group_mask().address()) & 0x2040_0000 != 0
                    || state.sprite_state().read(context) == 76
            };
            let falling = ram::g_player_fall_states(revision).at(player).read(context);
            if falling != 0 {
                return Ok(None);
            }
            if !resting(context) {
                let flying = read_long(context, state.animation_group_mask().address()) & 0x8000;
                if flying == 0 {
                    return Ok(None);
                }
            }
            let elevator = ram::g_player_elevator_flags(revision)
                .at(player)
                .read(context);
            if elevator != 0 {
                return Ok(None);
            }
            if ram::g_player_terrain_sinking(revision)
                .at(player)
                .read(context)
                != 0
            {
                let height = state.height().read(context);
                if height < -4 {
                    return Ok(None);
                }
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
                clear_byte(
                    context,
                    ram::g_player_choice_action_release_latches(revision)
                        .at(player)
                        .address(),
                )?;
            }
            let latch = ram::g_player_choice_action_release_latches(revision)
                .at(player)
                .read(context);
            if latch != 0 {
                return Ok(None);
            }
            let current_phase = ram::g_player_phased_jump_phase(revision).at(player);
            let ticks = ram::g_player_phased_jump_phase_ticks(revision).at(player);
            if resting(context) {
                if jump == 0 {
                    return Ok(None);
                }
                let armed = ram::g_player_phased_jump_armed(revision)
                    .at(player)
                    .read(context);
                if armed == 0 {
                    return Ok(None);
                }
                phase = 1;
            } else if current_phase.read(context) == 0 && jump == 0 {
                if ticks.read(context) > 1 {
                    phase = 1;
                }
            } else if current_phase.read(context) == 1 && jump != 0 {
                phase = 0;
                clear_byte(context, ticks.address())?;
                call_from(
                    context,
                    ids::PLAYER_UPDATE_PHASED_JUMP_MOTION,
                    ids::PLAY_SOUND,
                    &[23, 20],
                    0x1a0,
                )?;
            } else if state.height().read(context) < 4 {
                clear_word(context, state.velocity_x().address())?;
                clear_word(context, state.acceleration_x().address())?;
                clear_word(context, state.velocity_y().address())?;
                clear_word(context, state.acceleration_y().address())?;
                if state.velocity_height().read(context) < 0 {
                    clear_word(context, state.height().address())?;
                    clear_word(context, state.velocity_height().address())?;
                    clear_word(context, state.acceleration_height().address())?;
                    state
                        .draw_flags()
                        .write(context, state.draw_flags().read(context) & !0x40);
                    if terrain >= 0x80 {
                        if terrain == 0x90 {
                            clear_byte(
                                context,
                                ram::g_player_jump_counters(revision).at(player).address(),
                            )?;
                        } else {
                            phase = direction + 64;
                            call_from(
                                context,
                                ids::PLAYER_UPDATE_PHASED_JUMP_MOTION,
                                ids::PLAY_SOUND_EFFECT,
                                &[rom::sfx_splash(revision).address(), 50, 0],
                                0x20a,
                            )?;
                        }
                    } else {
                        phase = call_from(
                            context,
                            ids::PLAYER_UPDATE_PHASED_JUMP_MOTION,
                            ids::GET_ACTIVE_PRESENT_SPRITE,
                            &[state.address()],
                            0x218,
                        )? as i8;
                        state.animation_motion_axes().write(context, 3);
                        clear_byte(
                            context,
                            ram::g_player_phased_jump_armed(revision)
                                .at(player)
                                .address(),
                        )?;
                    }
                    if phase != -1 {
                        call_from(
                            context,
                            ids::PLAYER_UPDATE_PHASED_JUMP_MOTION,
                            ids::SET_SPRITE,
                            &[state.address(), phase as u32],
                            0x246,
                        )?;
                        phase = -1;
                    }
                } else if terrain == 0x90 {
                    let counter = ram::g_player_jump_counters(revision).at(player);
                    if counter.read(context) > 10 {
                        call_from(
                            context,
                            ids::PLAYER_UPDATE_PHASED_JUMP_MOTION,
                            ids::PLAYER_START_FALLING,
                            &[state.address(), player as u32],
                            0x26c,
                        )?;
                    } else if ticks.read(context) > 1 {
                        phase = current_phase.read(context) ^ 1;
                        clear_byte(context, ticks.address())?;
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
            if read_long(context, state.animation_group_mask().address()) & 0x8000 == 0 {
                call_from(
                    context,
                    ids::PLAYER_UPDATE_PHASED_JUMP_MOTION,
                    ids::SET_SPRITE,
                    &[state.address(), 61],
                    0x2ce,
                )?;
                state
                    .draw_flags()
                    .write(context, state.draw_flags().read(context) | 0x40);
                clear_byte(context, state.animation_motion_axes().address())?;
            } else if terrain == 0x90 {
                state
                    .draw_flags()
                    .write(context, state.draw_flags().read(context) & !0x40);
            } else {
                state
                    .draw_flags()
                    .write(context, state.draw_flags().read(context) | 0x40);
            }
            call_from(
                context,
                ids::PLAYER_UPDATE_PHASED_JUMP_MOTION,
                ids::PLAYER_APPLY_PHASED_JUMP_INPUT,
                &[
                    state.address(),
                    player as u32,
                    phase as u32,
                    u32::from(jump),
                ],
                0x30c,
            )?;
            Ok(None)
        },
    )
}
