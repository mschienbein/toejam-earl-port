use super::frames::{call_from, with_frame};
use crate::call_frame::FrameLayout;
use crate::{Context, Result, call, ids};
use tje_mem::{Address, ObjectState, ram, rom};

pub(super) fn next_random(context: &mut dyn Context) -> u16 {
    let revision = context.revision();
    let index = ram::g_random_cache_index(revision);
    let next = if index.read(context) == 58 {
        0
    } else {
        index.read(context).wrapping_add(1)
    };
    index.write(context, next);
    ram::g_cached_random_values(revision)
        .at(i32::from(next))
        .read(context)
}

/// G/src/player/active_present_falling_and_respawn.c:14 (`GetActivePresentSprite`).
/// c_active_present_falling_and_respawn.s:4-107 (both revisions): preserve the
/// real MOVEM frame, word-only D1 update, and the callee's volatile residues.
pub(super) fn get_active_present_sprite(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    with_frame(context, FrameLayout::movem(0, 4, 0x0c), |context, _| {
        let player = Address::<ObjectState>::new(arguments[0]);
        let revision = context.revision();
        let number = i32::from(player.entity_index().read(context));
        context.set_address_register(1, player.address());
        context.set_address_register(2, ram::g_player_powerups(revision).address());
        context.set_address_register(3, ram::g_player_elevator_flags(revision).address());
        context.set_data_register(
            2,
            (context.data_register(2) & 0xffff_0000) | u32::from(number as u16),
        );
        let land = (if number == 0 {
            ram::g_player1_terrain_underfoot(revision)
        } else {
            ram::g_player2_terrain_underfoot(revision)
        })
        .read(context)
            & 0xf0;
        context.set_data_register(
            1,
            (context.data_register(1) & 0xffff_0000) | u32::from(land),
        );
        let powerup = ram::g_player_powerups(revision).at(number);
        let elevator = ram::g_player_elevator_flags(revision).at(number);
        let sprite;
        if powerup.read(context) == 2 {
            sprite = if land == 0x80 {
                77
            } else if elevator.read(context) == 0 {
                76
            } else {
                player.frame().write(context, -1);
                0
            };
        } else if powerup.read(context) == 5 {
            context.set_data_register(0, u32::from(land));
            context.set_data_register(2, number as u32);
            call_from(
                context,
                ids::GET_ACTIVE_PRESENT_SPRITE,
                ids::OPEN_ROCKET_SKATES,
                &[player.address(), number as u32, u32::from(land)],
                0x80,
            )?;
            return finish_present_sprite(context, -1);
        } else if land == 0x80 && player.height().read(context) == 0 {
            sprite = 3;
        } else if powerup.read(context) == 1 && elevator.read(context) == 0 {
            sprite = 57;
        } else if powerup.read(context) == 0 && elevator.read(context) == 0 {
            if player.animation_group_mask().read(context) & 0x8000 != 0
                && player.height().read(context) > 0
            {
                return finish_present_sprite(context, -1);
            }
            player
                .draw_flags()
                .write(context, player.draw_flags().read(context) | 0x40);
            sprite = 4;
        } else {
            sprite = 0;
            player.frame().write(context, -1);
        }
        player.velocity_x().write(context, 0);
        player.velocity_y().write(context, 0);
        if sprite != 4 {
            player
                .draw_flags()
                .write(context, player.draw_flags().read(context) & !0x40);
        }
        finish_present_sprite(context, sprite)
    })
}

/// The common MOVE.B/EXT.W/EXT.L return tail sets N/Z from the signed byte,
/// clears V/C, and preserves the current X (including a real callee's result).
fn finish_present_sprite(context: &mut dyn Context, sprite: i8) -> Result<Option<u32>> {
    let result = i32::from(sprite) as u32;
    context.set_data_register(0, result);
    let flags = if sprite == 0 {
        4
    } else if sprite < 0 {
        8
    } else {
        0
    };
    context.set_status_register((context.status_register() & !0x000f) | flags);
    Ok(Some(result))
}

/// G/src/player/active_present_falling_and_respawn.c:193 (`PlayerUpdateTeetering`).
pub(super) fn player_update_teetering(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let player = Address::<ObjectState>::new(arguments[0]);
    let revision = context.revision();
    let number = i32::from(player.entity_index().read(context));
    if player.animation_group_mask().read(context) & 0x10 != 0 {
        let counter = ram::g_player_jump_counters(revision).at(number);
        counter.write(context, counter.read(context).wrapping_add(1));
        if counter.read(context) > 30 {
            if ram::g_elevator_departure_in_progress(revision).read(context) != 0 {
                counter.write(context, counter.read(context).wrapping_sub(1));
            } else {
                call(
                    context,
                    ids::PLAYER_START_TEETERING_FALL,
                    &[player.address()],
                )?;
            }
        } else if counter.read(context) == 1 {
            let sample = if next_random(context) & 1 != 0 {
                rom::sfx_teetering1(revision).address()
            } else {
                rom::sfx_teetering2(revision).address()
            };
            call(context, ids::PLAY_SOUND_EFFECT, &[sample, 86, 0])?;
        } else if ram::g_pcm_effect_cooldown_ticks(revision).read(context) == 0 {
            let choice = next_random(context) & 15;
            let sample = match choice {
                0 => Some(rom::sfx_teetering1(revision).address()),
                1 => Some(rom::sfx_teetering2(revision).address()),
                2 => Some(rom::sfx_teetering3(revision).address()),
                _ => None,
            };
            if let Some(sample) = sample {
                call(context, ids::PLAY_SOUND_EFFECT, &[sample, 20, 0])?;
            }
        }
    }
    Ok(None)
}

/// G/src/player/active_present_falling_and_respawn.c:499 (`PlayerUpdateJumpLandingAnimation`).
pub(super) fn player_update_jump_landing_animation(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let player = Address::<ObjectState>::new(arguments[0]);
    let land = arguments[1] as u16;
    let revision = context.revision();
    let number = i32::from(player.entity_index().read(context));
    if player.level().read(context) == 0
        || ram::g_player_powerups(revision).at(number).read(context) == 1
    {
        return Ok(None);
    }
    let mut sprite = -1_i8;
    if player.animation_group_mask().read(context) & 0x80 != 0 {
        let counter = ram::g_player_jump_counters(revision).at(number);
        counter.write(context, counter.read(context).wrapping_add(1));
        if counter.read(context) > 9 {
            sprite = player.sprite_state().read(context).wrapping_add(4);
            player.height().write(context, 1);
        }
    } else if player.animation_group_mask().read(context) & 0x100 != 0 {
        if player.frame_duration().read(context) == 0 {
            call(context, ids::STOP_PLAYER, &[player.address()])?;
            player.height().write(context, 35);
            player
                .draw_flags()
                .write(context, player.draw_flags().read(context) | 0x40);
            sprite = player.sprite_state().read(context).wrapping_add(4);
            player.acceleration_height().write(context, -64);
        }
    } else if player.animation_group_mask().read(context) & 0x200 != 0
        && player.height().read(context) < 4
    {
        if land == 0x80 {
            call(context, ids::STOP_PLAYER, &[player.address()])?;
            player.height().write(context, 0);
            player
                .draw_flags()
                .write(context, player.draw_flags().read(context) & !0x40);
            sprite = player.sprite_state().read(context).wrapping_add(4);
            call(
                context,
                ids::PLAY_SOUND_EFFECT,
                &[rom::sfx_splash(revision).address(), 50, 0],
            )?;
        } else {
            ram::g_player_jump_counters(revision)
                .at(number)
                .write(context, 1);
            ram::g_player_fall_states(revision)
                .at(number)
                .write(context, 2);
            sprite = player
                .sprite_state()
                .read(context)
                .wrapping_add(0xf0_u8 as i8);
        }
    }
    if sprite != -1 {
        call(context, ids::SET_SPRITE, &[player.address(), sprite as u32])?;
    }
    Ok(None)
}

/// G/src/player/active_present_falling_and_respawn.c:699 (`PlayerUpdateSpringShoeJump`).
pub(super) fn player_update_spring_shoe_jump(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let player = Address::<ObjectState>::new(arguments[0]);
    let revision = context.revision();
    let number = i32::from(player.entity_index().read(context));
    if ram::g_player_powerups(revision).at(number).read(context) != 1
        || ram::g_player_fall_states(revision).at(number).read(context) != 0
        || ram::g_player_elevator_flags(revision)
            .at(number)
            .read(context)
            != 0
    {
        return Ok(None);
    }
    let mut sprite = -1_i8;
    let direction = player.sprite_state().read(context) & 3;
    if player.animation_group_mask().read(context) & 0x4000 != 0 {
        if ram::g_player_processed_input(revision)
            .at(number)
            .read(context)
            & 0x40
            != 0
            && (player.velocity_x().read(context) != 0 || player.velocity_y().read(context) != 0)
        {
            sprite = direction + 28;
            ram::g_player_jump_counters(revision)
                .at(number)
                .write(context, 0);
        }
    } else if player.animation_group_mask().read(context) & 0x80 != 0 {
        let counter = ram::g_player_jump_counters(revision).at(number);
        counter.write(context, counter.read(context).wrapping_add(1));
        if counter.read(context) > 4 {
            sprite = if player.velocity_x().read(context) > 0 {
                35
            } else if player.velocity_x().read(context) < 0 {
                34
            } else {
                player.sprite_state().read(context).wrapping_add(4)
            };
            player.height().write(context, 1);
            call(
                context,
                ids::PLAY_SOUND_EFFECT,
                &[rom::sfx_springing(revision).address(), 50, 0],
            )?;
        }
    } else if player.animation_group_mask().read(context) & 0x100 != 0 {
        if player.frame_duration().read(context) == 0 {
            player.height().write(context, 35);
            player.velocity_height().write(context, 256);
            player.acceleration_height().write(context, -4);
            player
                .velocity_x()
                .write(context, player.velocity_x().read(context).wrapping_shl(1));
            player.animation_motion_axes().write(context, 0);
            player
                .draw_flags()
                .write(context, player.draw_flags().read(context) | 0x40);
            sprite = direction + 20;
            if sprite & 2 != 0 {
                sprite ^= 1;
            }
            ram::g_player_landing_counter_seed(revision)
                .at(number)
                .write(context, 2);
            ram::g_player_terrain_sinking(revision)
                .at(number)
                .write(context, 0);
        }
    } else if player.animation_group_mask().read(context) & 4 != 0 {
        sprite = direction + 56;
    }
    if sprite != -1 {
        call(context, ids::SET_SPRITE, &[player.address(), sprite as u32])?;
    }
    Ok(None)
}

/// G/src/player/active_present_falling_and_respawn.c:541 (`PlayerUpdatePresentCompletionAndTerrainPose`).
pub(super) fn player_update_present_completion_and_terrain_pose(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let player = Address::<ObjectState>::new(arguments[0]);
    let revision = context.revision();
    let number = i32::from(player.entity_index().read(context));
    let mut sprite = -1_i8;
    if player.animation_group_mask().read(context) & 0x1000 != 0 {
        if player.frame_duration().read(context) == 0 {
            sprite = call(context, ids::GET_ACTIVE_PRESENT_SPRITE, &[player.address()])? as i8;
        }
    } else if player.animation_group_mask().read(context) & 0x10400 != 0
        && player.level().read(context) != 0
        && player.frame_duration().read(context) == 0
    {
        sprite = if ram::g_player_powerups(revision).at(number).read(context) == 2 {
            77
        } else {
            3
        };
        player.velocity_x().write(context, 0);
        player.velocity_y().write(context, 0);
        player.animation_motion_axes().write(context, 0);
    }
    if player.sprite_state().read(context) == 3 {
        let terrain = if number == 0 {
            ram::g_player1_nearby_terrain(revision)
        } else {
            ram::g_player2_nearby_terrain(revision)
        }
        .cast::<u8>();
        for index in 0..25 {
            if terrain.offset(index).read(context) != 0x80 {
                sprite = 7;
                break;
            }
        }
    }
    if sprite != -1 {
        call(context, ids::SET_SPRITE, &[player.address(), sprite as u32])?;
    }
    Ok(None)
}
#[cfg(test)]
mod producer_tests {
    use super::super::producer_test_context::TestContext;
    use super::*;
    use tje_mem::Revision;

    #[test]
    fn return_tail_sign_extends_every_byte_and_preserves_each_entry_extend() {
        for revision in [Revision::Rev02, Revision::Rev00] {
            let mut context = TestContext::new(revision);
            for raw in 0..=255_u8 {
                for ccr in 0..32 {
                    context.set_data_register(0, 0xa55a1234);
                    context.set_status_register(0x2700 | ccr);
                    let result = finish_present_sprite(&mut context, raw as i8).unwrap();
                    let expected = (raw as i8 as i32) as u32;
                    assert_eq!(result, Some(expected));
                    assert_eq!(context.data_register(0), expected);
                    let flags = if expected == 0 {
                        4
                    } else if expected & 0x80000000 != 0 {
                        8
                    } else {
                        0
                    };
                    assert_eq!(context.status_register(), 0x2700 | (ccr & 0x10) | flags);
                }
            }
        }
    }
}
