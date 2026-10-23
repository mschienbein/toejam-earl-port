use crate::{Context, Result, call, ids};
use tje_mem::{Address, ObjectState, ram};

/// G/src/player/open_rocket_skates.c:11 (`OpenRocketSkates`).
pub(super) fn open_rocket_skates(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let state = Address::<ObjectState>::new(arguments[0]);
    let player = i32::from(arguments[1] as i16);
    let terrain = arguments[2] as u16;
    let revision = context.revision();
    let mut sprite = 1_i8;
    call(context, ids::STOP_PLAYER, &[state.address()])?;
    if state.height().read(context) > 0 {
        state.acceleration_height().write(context, -32);
    } else if state.animation_group_mask().read(context) & 0x800 != 0
        || matches!(state.sprite_state().read(context), 3 | 7)
    {
        state.height().write(context, -16);
        state.velocity_height().write(context, 256);
        state.acceleration_height().write(context, 0);
    }
    if state.animation_group_mask().read(context) & 0x07b5_fffc != 0 {
        sprite = state.sprite_state().read(context) & 3;
    }
    sprite += if terrain == 0x90 || state.height().read(context) > 0 {
        104
    } else if terrain == 0x80 {
        100
    } else {
        92
    };
    call(context, ids::SET_SPRITE, &[state.address(), sprite as u32])?;
    state.animation_motion_axes().write(context, 0);
    let previous = ram::g_player_previous_processed_input(revision).at(player);
    previous.write(context, previous.read(context) & 0xf0);
    if state.height().read(context) >= 0 {
        state
            .draw_flags()
            .write(context, state.draw_flags().read(context) | 0x40);
    }
    Ok(None)
}

/// G/src/player/rocket_motion.c:16 (`PlayerUpdateRocketSkatesSprite`).
pub(super) fn player_update_rocket_skates_sprite(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let state = Address::<ObjectState>::new(arguments[0]);
    let player = i32::from(arguments[1] as i16);
    let terrain = arguments[2] as u16;
    let mut sprite = -1_i8;
    let velocity_x = state.velocity_x().read(context);
    let velocity_y = state.velocity_y().read(context);
    let mut direction = state.sprite_state().read(context) & 3;
    if velocity_x > 0 && direction != 3 {
        direction = 3;
    } else if velocity_x < 0 && direction != 2 {
        direction = 2;
    } else if velocity_y > 0 && direction != 1 {
        direction = 1;
    } else if velocity_y < 0 && direction != 0 {
        direction = 0;
    }
    let jump = ram::g_player_processed_input(context.revision())
        .at(player)
        .read(context)
        & 0x40;
    if jump != 0 && terrain <= 0x50 && state.animation_group_mask().read(context) & 0x80_0000 != 0 {
        sprite = direction + 96;
    } else if jump == 0 && state.animation_group_mask().read(context) & 0x100_0000 != 0 {
        state.velocity_height().write(context, 0x400);
        state.acceleration_height().write(context, -64);
        sprite = direction + 104;
    } else if (matches!(terrain, 0x80 | 0x90)
        && state.animation_group_mask().read(context) & 0x600_0000 == 0)
        || (terrain != 0x80 && state.animation_group_mask().read(context) & 0x200_0000 != 0)
    {
        state.velocity_height().write(context, 0x200);
        state.acceleration_height().write(context, -64);
        sprite = direction + 104;
    } else if state.animation_group_mask().read(context) & 0x400_0000 != 0
        && state.height().read(context) <= 0
    {
        if terrain <= 0x50 {
            sprite = direction + 92;
        } else if terrain == 0x80 {
            sprite = direction + 100;
        } else {
            call(context, ids::STOP_PLAYER, &[state.address()])?;
            state.acceleration_height().write(context, -32);
            sprite = (state.sprite_state().read(context) & 3) + 20;
            ram::g_player_landing_counter_seed(context.revision())
                .at(player)
                .write(context, 2);
        }
    } else if direction as u32 != (state.sprite_state().read(context) as u32 & 3) {
        sprite =
            ((i32::from(state.sprite_state().read(context)) & 0xfc) + i32::from(direction)) as i8;
    }
    if sprite != -1 {
        call(context, ids::SET_SPRITE, &[state.address(), sprite as u32])?;
    }
    Ok(None)
}

/// G/src/player/rocket_motion.c:70 (`PlayerApplyRocketSkatesInput`).
/// ORIGINAL BUG: the X clamp excludes the Y clamp, even when both exceed their limits.
pub(super) fn player_apply_rocket_skates_input(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let state = Address::<ObjectState>::new(arguments[0]);
    let player = i32::from(arguments[1] as i16);
    let changed = arguments[2] as i8;
    let input = arguments[3] as u8;
    let mut velocity_x = state.velocity_x().read(context);
    let mut velocity_y = state.velocity_y().read(context);
    if state.animation_group_mask().read(context) & 0x400_0000 == 0
        || state.draw_flags().read(context) & 0x30 != 0
    {
        let mut up = input & 1 != 0;
        let mut down = input & 2 != 0;
        let mut left = input & 4 != 0;
        let mut right = input & 8 != 0;
        let direction = state.sprite_state().read(context) & 3;
        let diagonal = (up || down) && (left || right);
        if !diagonal || changed != 0 {
            if diagonal {
                if up && direction == 0 {
                    up = false;
                } else if down && direction == 1 {
                    down = false;
                } else if left && direction == 2 {
                    left = false;
                } else if right && direction == 3 {
                    right = false;
                }
            }
            let delay = ram::g_player_rocket_turn_delay(context.revision()).at(player);
            if input & 0xf == 0 {
                match direction {
                    0 => up = true,
                    1 => {}
                    2 => left = true,
                    3 => right = true,
                    _ => unreachable!(),
                }
            } else if up && direction == 1 {
                up = false;
                delay.write(context, 3);
                if !(left || right) {
                    right = true;
                }
            } else if down && direction == 0 {
                delay.write(context, 3);
                if !(left || right) {
                    left = true;
                }
            } else if left && direction == 3 {
                left = false;
                delay.write(context, 3);
                if !(up || down) {
                    up = true;
                }
            } else if right && direction == 2 {
                // The original sets the local `down` here; the later branch
                // chooses downward acceleration whenever `up` is false.
                right = false;
                delay.write(context, 3);
            }
            if left || right {
                velocity_y = 0;
                state.acceleration_y().write(context, 0);
                if left {
                    state.acceleration_x().write(context, -128);
                    if velocity_x > -0x300 {
                        velocity_x = -0x300;
                    }
                } else {
                    state.acceleration_x().write(context, 128);
                    if velocity_x < 0x300 {
                        velocity_x = 0x300;
                    }
                }
            } else {
                velocity_x = 0;
                state.acceleration_x().write(context, 0);
                if up {
                    state.acceleration_y().write(context, -128);
                    if velocity_y > -0x240 {
                        velocity_y = -0x240;
                    }
                } else {
                    state.acceleration_y().write(context, 128);
                    if velocity_y < 0x240 {
                        velocity_y = 0x240;
                    }
                }
            }
        }
    }
    if i32::from(velocity_x).abs() > 0x600 {
        velocity_x = if velocity_x > 0 { 0x600 } else { -0x600 };
    } else if i32::from(velocity_y).abs() > 0x480 {
        velocity_y = if velocity_y > 0 { 0x480 } else { -0x480 };
    }
    state.velocity_x().write(context, velocity_x);
    state.velocity_y().write(context, velocity_y);
    Ok(None)
}

/// G/src/player/rocket_motion.c:176 (`PlayerUpdateRocketSkatesMotion`).
pub(super) fn player_update_rocket_skates_motion(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let state = Address::<ObjectState>::new(arguments[0]);
    let terrain = arguments[1] as u16;
    let revision = context.revision();
    let player = i32::from(state.entity_index().read(context));
    if ram::g_player_fall_states(revision).at(player).read(context) != 0
        || state.animation_group_mask().read(context) & 0x780_0000 == 0
        || ram::g_player_elevator_flags(revision)
            .at(player)
            .read(context)
            != 0
    {
        return Ok(None);
    }
    if (state.velocity_height().read(context) < 0 && state.height().read(context) <= 0)
        || (state.velocity_height().read(context) > 0
            && state.acceleration_height().read(context) == 0
            && state.height().read(context) >= 0)
    {
        state.height().write(context, 0);
        state.velocity_height().write(context, 0);
        state.acceleration_height().write(context, 0);
        state
            .draw_flags()
            .write(context, state.draw_flags().read(context) | 0x40);
    }
    let mut changed = 0;
    let delay = ram::g_player_rocket_turn_delay(revision).at(player);
    if ram::g_player_processed_input(revision)
        .at(player)
        .read(context)
        & 0xf
        != ram::g_player_previous_processed_input(revision)
            .at(player)
            .read(context)
            & 0xf
    {
        changed = 1;
        delay.write(context, 0);
    } else if delay.read(context) != 0 {
        let previous = delay.read(context);
        delay.write(context, previous.wrapping_sub(1));
        if previous != 0 {
            return Ok(None);
        }
    }
    let mut input = if ram::g_player_control_remap_entities(revision)
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
    if state.velocity_y().read(context) != 0 && state.draw_flags().read(context) & 0x20 != 0 {
        input &= 0xfc;
        changed = 1;
        if input & 0xc == 0 {
            input |= if super::poses::next_random(context) & 1 != 0 {
                1
            } else {
                2
            };
        }
    }
    if state.velocity_x().read(context) != 0 && state.draw_flags().read(context) & 0x10 != 0 {
        input &= 0xf3;
        changed = 1;
        if input & 3 == 0 {
            input |= if super::poses::next_random(context) & 1 != 0 {
                4
            } else {
                8
            };
        }
    }
    call(
        context,
        ids::PLAYER_APPLY_ROCKET_SKATES_INPUT,
        &[state.address(), player as u32, changed, u32::from(input)],
    )?;
    call(
        context,
        ids::PLAYER_UPDATE_ROCKET_SKATES_SPRITE,
        &[state.address(), player as u32, u32::from(terrain)],
    )?;
    Ok(None)
}
