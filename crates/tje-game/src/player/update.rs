use crate::{Context, Result, call, call_void, ids};
use tje_mem::{Address, ObjectState, ram};

/// G/src/player/active_present_falling_and_respawn.c:990 (`PlayerUpdateDeathAnimation`).
/// The final-life branch delegates to its original terminal transition until the
/// shared native reset contract is available; it is not replaced by a return.
pub(super) fn player_update_death_animation(context: &mut dyn Context) -> Result<Option<u32>> {
    super::frames::with_frame(
        context,
        crate::call_frame::FrameLayout::movem(0, 0x1c, 0x1c),
        |context, stack| {
            let player = Address::<ObjectState>::new(super::frames::read_long(
                context,
                stack.argument_slot(0).address(),
            ));
            let terrain = stack.argument_word(1).read(context);
            let revision = context.revision();
            let number = i32::from(player.entity_index().read(context));
            let counter = ram::g_player_jump_counters(revision).at(number);
            // Matching compiler lines2310-2312 issue two separate CMPI.B loads.
            if player.sprite_state().read(context) == 5 || player.sprite_state().read(context) == 6
            {
                let camera =
                    if ram::g_split_screen_enabled(revision).read(context) == 0 || number == 0 {
                        ram::g_camera1_pending_y(revision).read(context)
                    } else {
                        ram::g_camera2_pending_y(revision).read(context)
                    };
                if i32::from(player.y().read(context))
                    - i32::from(player.height().read(context))
                    - i32::from(camera)
                    < 0
                {
                    super::frames::clear_word(context, player.velocity_height().address())?;
                    if player.behavior_flags().read(context) & 0x80 != 0 {
                        if ram::g_player_inventory_open(revision)
                            .at(number ^ 1)
                            .read(context)
                            == 0
                        {
                            super::frames::call_from_void(
                                context,
                                ids::PLAYER_UPDATE_DEATH_ANIMATION,
                                ids::PLAYER_FINALIZE_LAST_LIFE_EXIT,
                                &[player.address(), number as u32],
                                0x8e,
                            )?;
                        }
                        return Ok(None);
                    }
                    super::frames::clear_word(context, player.height().address())?;
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
                        let sprite =
                            call(context, ids::GET_ACTIVE_PRESENT_SPRITE, &[player.address()])?;
                        call_void(context, ids::SET_SPRITE, &[player.address(), sprite])?;
                    } else {
                        super::frames::call_from_void(
                            context,
                            ids::PLAYER_UPDATE_DEATH_ANIMATION,
                            ids::PLAYER_START_FALLING,
                            &[player.address(), number as u32],
                            0xec,
                        )?;
                    }
                } else if player.height().read(context) < 0 {
                    let remaining = counter.read(context).wrapping_sub(1);
                    counter.write(context, remaining);
                    // BNE consumes SUBQ.B's flags; it does not reload the counter.
                    if remaining == 0 {
                        counter.write(context, 6);
                    }
                    if counter.read(context) > 3 {
                        super::frames::clear_byte(context, player.frame().address())?;
                    } else {
                        player.frame().write(context, -1);
                    }
                    super::frames::clear_byte(context, player.sprite_ready().address())?;
                } else if player.behavior_flags().read(context) & 0x80 != 0
                    && player.height().read(context) >= 20
                {
                    if player.sprite_state().read(context) == 5 {
                        call_void(context, ids::SET_SPRITE, &[player.address(), 6])?;
                        super::frames::clear_word(context, player.velocity_height().address())?;
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
                        let (stored, remaining) =
                            super::counters::predecrement_byte(counter.read(context));
                        counter.write(context, stored);
                        if remaining <= 0 {
                            call_void(context, ids::SET_SPRITE, &[player.address(), 5])?;
                            player.sprite_state().write(context, 6);
                            player.velocity_height().write(context, 256);
                        }
                    }
                }
            }
            let suppression = ram::g_player_respawn_suppression_ticks(revision).at(number);
            if suppression.read(context) != 0 {
                let remaining = suppression.read(context).wrapping_sub(1);
                suppression.write(context, remaining);
                // Matching compiler lines2457-2458 branch on the RMW result.
                if remaining == 0 {
                    player
                        .behavior_flags()
                        .write(context, player.behavior_flags().read(context) & !0x10);
                }
            }
            Ok(None)
        },
    )
}

/// G/src/player/active_present_falling_and_respawn.c:1052 (`ProcessPlayers`).
/// Preserve the exact handler chain and inventory-open continue, including the
/// skipped common promotion/remap/terrain tail for that player.
/// Matching prologue MOVEM D2-D4/A2, no LINK. Catalogue calls prepare real
/// argument slots and the matching return PC in both revisions; the final
/// CheckPlayerFalling call returns at +0x1a0.
pub(super) fn process_players(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    super::frames::with_frame(
        context,
        crate::call_frame::FrameLayout::movem(0, 0x1c, 0x04),
        |context, _| process_players_body(context),
    )
}

fn process_players_body(context: &mut dyn Context) -> Result<Option<u32>> {
    let revision = context.revision();
    // The compiler loads player one's terrain once, then reads player two's
    // terrain at the end of EACH iteration, including the unused final read.
    let mut terrain = u32::from(ram::g_player1_terrain_underfoot(revision).read(context) & 0xf0);
    for number in 0..2 {
        let player = ram::g_player_info(revision).at(number);
        if ram::g_player_lives(revision).at(number).read(context) < 0 {
            super::frames::call_from_void(
                context,
                ids::PROCESS_PLAYERS,
                ids::PLAYER_PROCESS_LIFE_REQUEST,
                &[number as u32],
                0x36,
            )?;
        } else {
            if ram::g_player_inventory_open(revision)
                .at(number)
                .read(context)
                != 0
            {
                terrain =
                    u32::from(ram::g_player2_terrain_underfoot(revision).read(context) & 0xf0);
                continue;
            }
            super::frames::call_from_void(
                context,
                ids::PROCESS_PLAYERS,
                ids::PLAYER_UPDATE_TEETERING,
                &[player.address()],
                0x54,
            )?;
            super::frames::call_from_void(
                context,
                ids::PROCESS_PLAYERS,
                ids::PLAYER_UPDATE_HURT_RECOVERY_AND_IDLE,
                &[player.address(), terrain],
                0x60,
            )?;
            super::frames::call_from_void(
                context,
                ids::PROCESS_PLAYERS,
                ids::PLAYER_UPDATE_SPRING_SHOE_JUMP,
                &[player.address(), terrain],
                0x6c,
            )?;
            super::frames::call_from_void(
                context,
                ids::PROCESS_PLAYERS,
                ids::PLAYER_RESOLVE_DESCENDING_TERRAIN,
                &[player.address(), terrain],
                0x78,
            )?;
            super::frames::call_from_void(
                context,
                ids::PROCESS_PLAYERS,
                ids::PLAYER_UPDATE_PHASED_JUMP_MOTION,
                &[player.address(), terrain],
                0x86,
            )?;
            super::frames::call_from_void(
                context,
                ids::PROCESS_PLAYERS,
                ids::PLAYER_UPDATE_ROCKET_SKATES_MOTION,
                &[player.address(), terrain],
                0x94,
            )?;
            super::frames::call_from_void(
                context,
                ids::PROCESS_PLAYERS,
                ids::PLAYER_UPDATE_JUMP_LANDING_ANIMATION,
                &[player.address(), terrain],
                0xa0,
            )?;
            super::frames::call_from_void(
                context,
                ids::PROCESS_PLAYERS,
                ids::PLAYER_UPDATE_PRESENT_COMPLETION_AND_TERRAIN_POSE,
                &[player.address(), terrain],
                0xac,
            )?;
            super::frames::call_from_void(
                context,
                ids::PROCESS_PLAYERS,
                ids::PLAYER_UPDATE_TERRAIN_SINKING,
                &[player.address(), terrain],
                0xb8,
            )?;
            super::frames::call_from_void(
                context,
                ids::PROCESS_PLAYERS,
                ids::PLAYER_UPDATE_DEATH_ANIMATION,
                &[player.address(), terrain],
                0xc4,
            )?;
            super::frames::call_from_void(
                context,
                ids::PROCESS_PLAYERS,
                ids::PLAYER_UPDATE_PRESENT_SHOOTING,
                &[player.address(), terrain],
                0xd2,
            )?;
            super::frames::call_from_void(
                context,
                ids::PROCESS_PLAYERS,
                ids::PROCESS_SLEEP,
                &[player.address()],
                0xda,
            )?;
            super::frames::call_from_void(
                context,
                ids::PROCESS_PLAYERS,
                ids::PLAYER_UPDATE_WHIRLWIND_AND_RECOVERY,
                &[player.address(), terrain],
                0xe8,
            )?;
            super::frames::call_from_void(
                context,
                ids::PROCESS_PLAYERS,
                ids::PLAYER_UPDATE_HEALTH_AND_LIVES,
                &[player.address(), terrain],
                0xf6,
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
            } else if player.sprite_state().read(context) != 70
                && player.sprite_state().read(context) != 120
            {
                player
                    .behavior_flags()
                    .write(context, player.behavior_flags().read(context) & !0x20);
            }
        }
        super::frames::call_from_void(
            context,
            ids::PROCESS_PLAYERS,
            ids::BUMMER_LIFE_PROMO,
            &[player.address(), terrain],
            0x166,
        )?;
        super::frames::call_from_void(
            context,
            ids::PROCESS_PLAYERS,
            ids::PLAYER_UPDATE_CONTROL_REMAP_EFFECT,
            &[player.address()],
            0x16e,
        )?;
        super::frames::call_from_void(
            context,
            ids::PROCESS_PLAYERS,
            ids::PRESENT_UPDATE_TERRAIN_EFFECTS,
            &[player.address(), terrain],
            0x17c,
        )?;
        terrain = u32::from(ram::g_player2_terrain_underfoot(revision).read(context) & 0xf0);
    }
    super::frames::call_from_void(
        context,
        ids::PROCESS_PLAYERS,
        ids::CHECK_PLAYER_FALLING,
        &[],
        0x1a0,
    )?;
    if ram::g_player_selection(revision).read(context) == 2 {
        super::frames::call_from_void(
            context,
            ids::PROCESS_PLAYERS,
            ids::PLAYER_UPDATE_HEALTH_SHARING_CLAP,
            &[],
            0x1b0,
        )?;
    }
    Ok(None)
}
