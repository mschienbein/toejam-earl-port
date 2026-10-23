use crate::call_frame::{FrameLayout, LocalFrame};
use crate::{Context, Result, call, ids};
use tje_mem::{ram, rom};

/// G/src/player/active_present_falling_and_respawn.c:250 (`CheckPlayerFalling`).
/// Matching compiler prologue LINK -4, MOVEM D2-D5/A2-A5. The two falling
/// bytes are A6-4/A6-3; the remaining local bytes are never initialized.
/// Value-only proven pose/query calls retain their readable-layer contract.
/// MakePlayerFall receives the real local address and true return PC +0x2fa
/// (same offset in both matched revisions), not a host array or scratch global.
pub(super) fn check_player_falling(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    super::frames::with_frame(context, FrameLayout::link(4, 0x3c, 0x3c), check)
}

fn check(context: &mut dyn Context, stack: &LocalFrame) -> Result<Option<u32>> {
    let revision = context.revision();
    let falling = stack.local::<[i8; 2]>(-4);
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
                player.velocity_height().write(context, 0);
                player.acceleration_height().write(context, 0);
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
                if player.animation_group_mask().read(context) & 0x20 == 0 {
                    call(
                        context,
                        ids::SET_SPRITE,
                        &[player.address(), (direction + 20) as u32],
                    )?;
                    player.velocity_height().write(context, 0);
                    player.acceleration_height().write(context, -128);
                }
                state.write(context, 0);
                continue;
            }
            if counter.read(context) <= 0 {
                player.height().write(context, 0);
                player.velocity_height().write(context, 0);
                player.acceleration_height().write(context, 0);
                state.write(context, 0);
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
                player.height().write(context, 0);
                player.velocity_height().write(context, 0);
                player.acceleration_height().write(context, 0);
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
            let eligible = matches!(other.sprite_state().read(context), 0 | 4 | 76 | 1)
                || other.animation_group_mask().read(context) & 0x2040_0000 != 0
                || (other.sprite_state().read(context) == 57
                    && other.velocity_y().read(context) == 0);
            let overlap = if eligible {
                let args = [
                    other.address(),
                    player.x().read(context) as u32,
                    player.y().read(context) as u32,
                    player.level().read(context) as u32,
                ];
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
                    && other.animation_group_mask().read(context) & 0x2000_0000 == 0
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
                other.velocity_x().write(context, 0);
                other.velocity_y().write(context, 0);
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
