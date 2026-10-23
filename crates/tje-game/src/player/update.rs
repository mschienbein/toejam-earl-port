use crate::{Context, Result, call, ids};
use tje_mem::{Address, ObjectState, ram};

/// G/src/player/active_present_falling_and_respawn.c:990 (`PlayerUpdateDeathAnimation`).
/// The final-life branch delegates to its original terminal transition until the
/// shared native reset contract is available; it is not replaced by a return.
pub(super) fn player_update_death_animation(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let player = Address::<ObjectState>::new(arguments[0]);
    let terrain = arguments[1] as i16;
    let revision = context.revision();
    let number = i32::from(player.entity_index().read(context));
    let counter = ram::g_player_jump_counters(revision).at(number);
    if matches!(player.sprite_state().read(context), 5 | 6) {
        let camera = if ram::g_split_screen_enabled(revision).read(context) == 0 || number == 0 {
            ram::g_camera1_pending_y(revision).read(context)
        } else {
            ram::g_camera2_pending_y(revision).read(context)
        };
        if i32::from(player.y().read(context))
            - i32::from(player.height().read(context))
            - i32::from(camera)
            < 0
        {
            player.velocity_height().write(context, 0);
            if player.behavior_flags().read(context) & 0x80 != 0 {
                if ram::g_player_inventory_open(revision)
                    .at(number ^ 1)
                    .read(context)
                    == 0
                {
                    call(
                        context,
                        ids::PLAYER_FINALIZE_LAST_LIFE_EXIT,
                        &[player.address(), number as u32],
                    )?;
                }
                return Ok(None);
            }
            player.height().write(context, 0);
            player
                .behavior_flags()
                .write(context, player.behavior_flags().read(context) | 1);
            if ram::g_player_pre_death_sprite_state(revision)
                .at(number)
                .read(context)
                == 2
            {
                player
                    .behavior_flags()
                    .write(context, player.behavior_flags().read(context) | 0x10);
                ram::g_player_respawn_suppression_ticks(revision)
                    .at(number)
                    .write(context, 20);
            }
            if terrain != 0x90 {
                let sprite = call(context, ids::GET_ACTIVE_PRESENT_SPRITE, &[player.address()])?;
                call(context, ids::SET_SPRITE, &[player.address(), sprite])?;
            } else {
                call(
                    context,
                    ids::PLAYER_START_FALLING,
                    &[player.address(), number as u32],
                )?;
            }
        } else if player.height().read(context) < 0 {
            counter.write(context, counter.read(context).wrapping_sub(1));
            if counter.read(context) == 0 {
                counter.write(context, 6);
            }
            player
                .frame()
                .write(context, if counter.read(context) > 3 { 0 } else { -1 });
            player.sprite_ready().write(context, 0);
        } else if player.behavior_flags().read(context) & 0x80 != 0
            && player.height().read(context) >= 20
        {
            if player.sprite_state().read(context) == 5 {
                call(context, ids::SET_SPRITE, &[player.address(), 6])?;
                player.velocity_height().write(context, 0);
                counter.write(context, 30);
                // The two source literals follow the catalogued code extent;
                // G/build/rev02/asm/c_active_present_falling_and_respawn.s:2466ff.
                let metadata = ids::PLAYER_UPDATE_DEATH_ANIMATION.metadata();
                let text = metadata
                    .address(revision)
                    .wrapping_add(metadata.size(revision))
                    .wrapping_add(if number == 0 { 0 } else { 12 });
                call(
                    context,
                    ids::GENERATE_SPEECH,
                    &[(number ^ 1) as u32, text, 1],
                )?;
            } else if player.velocity_height().read(context) == 0 {
                let (stored, remaining) = super::counters::predecrement_byte(counter.read(context));
                counter.write(context, stored);
                if remaining <= 0 {
                    call(context, ids::SET_SPRITE, &[player.address(), 5])?;
                    player.sprite_state().write(context, 6);
                    player.velocity_height().write(context, 256);
                }
            }
        }
    }
    let suppression = ram::g_player_respawn_suppression_ticks(revision).at(number);
    if suppression.read(context) != 0 {
        suppression.write(context, suppression.read(context).wrapping_sub(1));
        if suppression.read(context) == 0 {
            player
                .behavior_flags()
                .write(context, player.behavior_flags().read(context) & !0x10);
        }
    }
    Ok(None)
}

/// G/src/player/active_present_falling_and_respawn.c:1052 (`ProcessPlayers`).
/// Preserve the exact handler chain and inventory-open continue, including the
/// skipped common promotion/remap/terrain tail for that player.
/// Matching prologue MOVEM D2-D4/A2, no LINK. The final CheckPlayerFalling
/// call is at the restored outgoing-area baseline and returns at +0x1a0 in
/// both revisions; earlier proven value-only handlers keep their contract.
pub(super) fn process_players(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    super::frames::with_frame(
        context,
        crate::call_frame::FrameLayout::movem(0, 0x1c, 0x04),
        |context, _| process_players_body(context),
    )
}

fn process_players_body(context: &mut dyn Context) -> Result<Option<u32>> {
    let revision = context.revision();
    for number in 0..2 {
        let terrain = u32::from(
            (if number == 0 {
                ram::g_player1_terrain_underfoot(revision)
            } else {
                ram::g_player2_terrain_underfoot(revision)
            })
            .read(context)
                & 0xf0,
        );
        let player = ram::g_player_info(revision).at(number);
        if ram::g_player_lives(revision).at(number).read(context) < 0 {
            call(context, ids::PLAYER_PROCESS_LIFE_REQUEST, &[number as u32])?;
        } else {
            if ram::g_player_inventory_open(revision)
                .at(number)
                .read(context)
                != 0
            {
                continue;
            }
            call(context, ids::PLAYER_UPDATE_TEETERING, &[player.address()])?;
            call(
                context,
                ids::PLAYER_UPDATE_HURT_RECOVERY_AND_IDLE,
                &[player.address(), terrain],
            )?;
            call(
                context,
                ids::PLAYER_UPDATE_SPRING_SHOE_JUMP,
                &[player.address(), terrain],
            )?;
            call(
                context,
                ids::PLAYER_RESOLVE_DESCENDING_TERRAIN,
                &[player.address(), terrain],
            )?;
            call(
                context,
                ids::PLAYER_UPDATE_PHASED_JUMP_MOTION,
                &[player.address(), terrain],
            )?;
            call(
                context,
                ids::PLAYER_UPDATE_ROCKET_SKATES_MOTION,
                &[player.address(), terrain],
            )?;
            call(
                context,
                ids::PLAYER_UPDATE_JUMP_LANDING_ANIMATION,
                &[player.address(), terrain],
            )?;
            call(
                context,
                ids::PLAYER_UPDATE_PRESENT_COMPLETION_AND_TERRAIN_POSE,
                &[player.address(), terrain],
            )?;
            call(
                context,
                ids::PLAYER_UPDATE_TERRAIN_SINKING,
                &[player.address(), terrain],
            )?;
            call(
                context,
                ids::PLAYER_UPDATE_DEATH_ANIMATION,
                &[player.address(), terrain],
            )?;
            call(
                context,
                ids::PLAYER_UPDATE_PRESENT_SHOOTING,
                &[player.address(), terrain],
            )?;
            call(context, ids::PROCESS_SLEEP, &[player.address()])?;
            call(
                context,
                ids::PLAYER_UPDATE_WHIRLWIND_AND_RECOVERY,
                &[player.address(), terrain],
            )?;
            call(
                context,
                ids::PLAYER_UPDATE_HEALTH_AND_LIVES,
                &[player.address(), terrain],
            )?;
            if ram::g_player_elevator_flags(revision)
                .at(number)
                .read(context)
                != 0
                || i32::from(
                    ram::g_doorway_player_contact_flags(revision)
                        .at(0)
                        .read(context),
                ) & 3
                    == number + 1
                || i32::from(ram::g_second_doorway_player_contact_flags(revision).read(context)) & 3
                    == number + 1
            {
                player
                    .behavior_flags()
                    .write(context, player.behavior_flags().read(context) | 0x20);
            } else if !matches!(player.sprite_state().read(context), 70 | 120) {
                player
                    .behavior_flags()
                    .write(context, player.behavior_flags().read(context) & !0x20);
            }
        }
        call(
            context,
            ids::BUMMER_LIFE_PROMO,
            &[player.address(), terrain],
        )?;
        call(
            context,
            ids::PLAYER_UPDATE_CONTROL_REMAP_EFFECT,
            &[player.address()],
        )?;
        call(
            context,
            ids::PRESENT_UPDATE_TERRAIN_EFFECTS,
            &[player.address(), terrain],
        )?;
    }
    super::frames::call_from(
        context,
        ids::PROCESS_PLAYERS,
        ids::CHECK_PLAYER_FALLING,
        &[],
        0x1a0,
    )?;
    if ram::g_player_selection(revision).read(context) == 2 {
        call(context, ids::PLAYER_UPDATE_HEALTH_SHARING_CLAP, &[])?;
    }
    Ok(None)
}
