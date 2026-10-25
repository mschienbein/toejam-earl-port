use crate::call_frame::{FrameLayout, LocalFrame};
use crate::{Context, Result, call, ids};
use tje_mem::{ram, rom};

/// G/src/player/active_present_falling_and_respawn.c:250 (`CheckPlayerFalling`).
/// Matching compiler prologue LINK -4, MOVEM D2-D5/A2-A5. The two falling
/// bytes are A6-2/A6-1 (compiler lines477-478), and PEA -2(A6) passes them on;
/// the other local bytes remain untouched.
/// MakePlayerFall receives the real local address and true return PC +0x2fa
/// (same offset in both matched revisions), not a host array or scratch global.
pub(super) fn check_player_falling(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    super::frames::with_frame(context, FrameLayout::link(4, 0x3c, 0x3c), check)
}

fn check(context: &mut dyn Context, stack: &LocalFrame) -> Result<Option<u32>> {
    let revision = context.revision();
    let falling = stack.local::<[i8; 2]>(-2);
    falling.at(0).write(context, 0);
    falling.at(1).write(context, 0);
    for number in 0..2 {
        if ram::g_player_lives(revision).at(number).read(context) < 0 {
            continue;
        }
        let player = ram::g_player_info(revision).at(number);
        let other = ram::g_player_info(revision).at(number ^ 1);
        let state = ram::g_player_fall_states(revision).at(number);
        let counter = ram::g_player_jump_counters(revision).at(number);
        if state.read(context) == 1 {
            if player.height().read(context) < -224 {
                super::frames::clear_word(context, player.velocity_height().address())?;
                super::frames::clear_word(context, player.acceleration_height().address())?;
                falling.at(number).write(context, 1);
                counter.write(
                    context,
                    if ram::g_player_powerups(revision).at(number).read(context) == 5 {
                        1
                    } else {
                        3
                    },
                );
            } else {
                player.acceleration_height().write(context, -128);
            }
        } else if state.read(context) == 2 {
            let land = u16::from(
                (if number == 0 {
                    ram::g_player1_terrain_underfoot(revision)
                } else {
                    ram::g_player2_terrain_underfoot(revision)
                })
                .read(context),
            );
            let direction = player.sprite_state().read(context) & 3;
            if land >= 0x80 {
                if player
                    .animation_group_mask()
                    .byte_offset(3)
                    .cast::<u8>()
                    .read(context)
                    & 0x20
                    == 0
                {
                    call(
                        context,
                        ids::SET_SPRITE,
                        &[player.address(), (direction + 20) as u32],
                    )?;
                    super::frames::clear_word(context, player.velocity_height().address())?;
                    player.acceleration_height().write(context, -128);
                }
                super::frames::clear_byte(context, state.address())?;
                continue;
            }
            if counter.read(context) <= 0 {
                super::frames::clear_word(context, player.height().address())?;
                super::frames::clear_word(context, player.velocity_height().address())?;
                super::frames::clear_word(context, player.acceleration_height().address())?;
                super::frames::clear_byte(context, state.address())?;
                let selected =
                    call(context, ids::GET_ACTIVE_PRESENT_SPRITE, &[player.address()])? as i8;
                call(
                    context,
                    ids::SET_SPRITE,
                    &[player.address(), selected as u32],
                )?;
            } else if player.velocity_height().read(context) < 0
                && player.height().read(context) < 4
            {
                call(
                    context,
                    ids::SET_SPRITE,
                    &[player.address(), (direction + 24) as u32],
                )?;
                super::frames::clear_word(context, player.height().address())?;
                super::frames::clear_word(context, player.velocity_height().address())?;
                super::frames::clear_word(context, player.acceleration_height().address())?;
            } else if player.frame_duration().read(context) == 0 {
                call(
                    context,
                    ids::SET_SPRITE,
                    &[player.address(), (direction + 20) as u32],
                )?;
                player.velocity_height().write(context, 512);
                player.acceleration_height().write(context, -64);
                let jump_count = i16::from(counter.read(context));
                player.velocity_height().write(
                    context,
                    player
                        .velocity_height()
                        .read(context)
                        .wrapping_add((i32::from(jump_count) << 8) as i16),
                );
                counter.write(context, counter.read(context).wrapping_sub(1));
            }
            let mut selected = -1_i8;
            // Compiler lines638-650 interleave repeated state loads with the mask.
            let eligible = other.sprite_state().read(context) == 0
                || other.sprite_state().read(context) == 4
                || super::frames::read_long(context, other.animation_group_mask().address())
                    & 0x2040_0000
                    != 0
                || other.sprite_state().read(context) == 76
                || other.sprite_state().read(context) == 1
                || (other.sprite_state().read(context) == 57
                    && other.velocity_y().read(context) == 0);
            let overlap = if eligible {
                // Right-to-left argument loads: level, Y, then X (lines654-663).
                let level = player.level().read(context) as u32;
                let y = player.y().read(context) as u32;
                let x = player.x().read(context) as u32;
                let args = [other.address(), x, y, level];
                call(context, ids::ENTITY_IS_NEAR_POSITION, &args)? != 0
            } else {
                false
            };
            if overlap {
                if player.height().read(context) < 20 {
                    selected = 2;
                    other
                        .draw_flags()
                        .write(context, other.draw_flags().read(context) | 0x80);
                    call(
                        context,
                        ids::PLAY_SOUND_EFFECT,
                        &[rom::sfx_squish(revision).address(), 50, 0],
                    )?;
                } else if other.sprite_state().read(context) != 1
                    && super::frames::read_long(context, other.animation_group_mask().address())
                        & 0x2000_0000
                        == 0
                    && player.height().read(context) < 300
                {
                    selected = 1;
                }
            } else if other.sprite_state().read(context) == 1 {
                selected = call(context, ids::GET_ACTIVE_PRESENT_SPRITE, &[other.address()])? as i8;
            }
            if selected != -1 {
                call(
                    context,
                    ids::SET_SPRITE,
                    &[other.address(), selected as u32],
                )?;
                super::frames::clear_word(context, other.velocity_x().address())?;
                super::frames::clear_word(context, other.velocity_y().address())?;
            }
        }
        if player.sprite_state().read(context) == 2 && player.frame_duration().read(context) == 0 {
            let selected =
                call(context, ids::GET_ACTIVE_PRESENT_SPRITE, &[player.address()])? as i8;
            call(
                context,
                ids::SET_SPRITE,
                &[player.address(), selected as u32],
            )?;
            player
                .draw_flags()
                .write(context, player.draw_flags().read(context) & !0x80);
            player
                .behavior_flags()
                .write(context, player.behavior_flags().read(context) | 0x10);
            ram::g_player_respawn_suppression_ticks(revision)
                .at(number)
                .write(context, 20);
        }
    }
    super::frames::call_from(
        context,
        ids::CHECK_PLAYER_FALLING,
        ids::MAKE_PLAYER_FALL,
        &[falling.address()],
        0x2fa,
    )?;
    Ok(None)
}
