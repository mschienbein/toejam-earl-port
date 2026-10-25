//! Extra labelled live-state cases for the ten PresentsB roots.
use super::time_fixtures::{common, labelled};
use std::sync::Arc;
use tje_game::{Context, FunctionId, ids};
use tje_host::{
    Host,
    input::{Button, InputHold, InputScript},
};
use tje_mem::ram;

fn reset(host: &mut Host) {
    common(host);
    let r = host.revision();
    let mut c = host.context();
    ram::g_player1_terrain_underfoot(r).write(&mut c, 0x10);
    ram::g_player2_terrain_underfoot(r).write(&mut c, 0x10);
    for index in 0..2 {
        ram::g_player_active_terrain_effect_entities(r)
            .at(index)
            .write(&mut c, 0);
        ram::g_player_one_shot_terrain_effect_entities(r)
            .at(index)
            .write(&mut c, 0);
        ram::g_doorway_player_contact_flags(r)
            .at(index)
            .write(&mut c, 0);
        ram::g_telephone_states(r)
            .at(index)
            .object()
            .write(&mut c, 0);
        ram::g_telephone_activated(r).at(index).write(&mut c, 0);
        ram::g_telephone_time_to_ring(r).at(index).write(&mut c, 0);
        ram::g_telephone_rings_left(r).at(index).write(&mut c, 0);
        ram::g_player_previous_processed_input(r)
            .at(index)
            .write(&mut c, 0);
        ram::g_player_processed_input(r).at(index).write(&mut c, 0);
    }
}

pub(super) fn run(host: &mut Host, id: FunctionId) {
    let r = host.revision();
    match id {
        ids::OPEN_SELECTED_PRESENT => {
            for index in 0..2 {
                for (label, present, elevator, state, terrain, height, level) in [
                    ("elevator-reject", 8, 1, 0, 0x10, 0, 1),
                    ("ghost-reject", 8, 0, 5, 0x10, 0, 1),
                    ("last-life-ghost-reject", 8, 0, 6, 0x10, 0, 1),
                    ("wet-decoy-reject", 15, 0, 0, 0x80, 0, 1),
                    ("wet-telephone-reject", 19, 0, 0, 0x80, 0, 1),
                    ("wet-schoolbook-reject", 24, 0, 0, 0x80, 0, 1),
                    ("airborne-schoolbook-reject", 24, 0, 0, 0x10, 1, 1),
                    ("wet-boombox-reject", 25, 0, 0, 0x80, 0, 1),
                    ("level-zero-springs-reject", 1, 0, 0, 0x10, 0, 0),
                ] {
                    reset(host);
                    let p = ram::g_player_info(r).at(index);
                    p.sprite_state().write(&mut host.context(), state);
                    p.height().write(&mut host.context(), height);
                    p.level().write(&mut host.context(), level);
                    ram::g_player_elevator_flags(r)
                        .at(index)
                        .write(&mut host.context(), elevator);
                    ram::g_player1_terrain_underfoot(r).write(&mut host.context(), terrain);
                    ram::g_player2_terrain_underfoot(r).write(&mut host.context(), terrain);
                    labelled(host, id, &[index as u32, present], label);
                }
                reset(host);
                ram::g_pending_unfall_player(r).write(&mut host.context(), 1);
                labelled(
                    host,
                    id,
                    &[index as u32, 12],
                    "togetherness-already-pending",
                );
                for (state, groups) in [
                    (0, 0),
                    (4, 0),
                    (76, 0),
                    (8, 0x400000),
                    (8, 0x20000000),
                    (8, 0x8000000),
                    (8, 0),
                ] {
                    reset(host);
                    let p = ram::g_player_info(r).at(index);
                    p.sprite_state().write(&mut host.context(), state);
                    p.animation_group_mask().write(&mut host.context(), groups);
                    labelled(
                        host,
                        id,
                        &[index as u32, 22],
                        &format!("tomato-rain-state-{state}-groups-{groups:08x}"),
                    );
                }
            }
        }
        ids::SETUP_SELECTED_PRESENT => {
            for index in 0..2 {
                for (label, present, height, powerup, fall, groups) in [
                    ("airborne-icarus-expiry", 2, 1, 0, 0, 0x8000),
                    ("airborne-spring-stop", 2, 1, 1, 0, 0x20),
                    ("falling-spring-no-stop", 2, 1, 1, 1, 0x20),
                    ("rocket-fall-revision-guard", 5, 0, -1, 1, 1),
                    ("hitops-refresh-revision-mask", 2, 0, -1, 0, 0x8000000),
                    ("stationary-no-refresh", 2, 0, -1, 0, 0x20000000),
                ] {
                    reset(host);
                    let p = ram::g_player_info(r).at(index);
                    p.height().write(&mut host.context(), height);
                    p.sprite_state().write(&mut host.context(), 8);
                    p.animation_group_mask().write(&mut host.context(), groups);
                    ram::g_player_powerups(r)
                        .at(index)
                        .write(&mut host.context(), powerup);
                    ram::g_player_fall_states(r)
                        .at(index)
                        .write(&mut host.context(), fall);
                    labelled(host, id, &[p.address(), index as u32, present, 0x10], label);
                }
            }
        }
        ids::PERFORM_PROMOTION => {
            for index in 0..2 {
                for (label, promotion, lives, rank, health) in [
                    ("fallback-life-cap", 17, 8, 2, 23),
                    ("fallback-life-increment", 17, 7, 2, 23),
                    ("fallback-rank-life", 11, 7, 2, 23),
                    ("fallback-rank-no-life", 11, 7, 3, 23),
                    ("fallback-bummer-health-clear", 16, 3, 2, -128),
                ] {
                    reset(host);
                    ram::g_player_promotion_display_entities(r)
                        .at(index)
                        .write(&mut host.context(), 6);
                    ram::g_player_lives(r)
                        .at(index)
                        .write(&mut host.context(), lives);
                    ram::g_player_rank(r)
                        .at(index)
                        .write(&mut host.context(), rank);
                    ram::g_player_health(r)
                        .at(index)
                        .write(&mut host.context(), health);
                    labelled(host, id, &[index as u32, promotion, 0x10], label);
                }
            }
        }
        ids::BUMMER_LIFE_PROMO => {
            for index in 0..2 {
                reset(host);
                labelled(
                    host,
                    id,
                    &[ram::g_player_info(r).at(index).address(), 0x10],
                    "no-display",
                );
                for (cache, phase, random) in [(0, 12, 2), (57, 7, 2), (58, 12, 0)] {
                    reset(host);
                    let display = ram::g_player_info(r).at(6);
                    display.sprite_state().write(&mut host.context(), 11);
                    display.frame_duration().write(&mut host.context(), 1);
                    display.frame().write(&mut host.context(), phase);
                    ram::g_player_promotion_display_entities(r)
                        .at(index)
                        .write(&mut host.context(), 6);
                    ram::g_random_cache_index(r).write(&mut host.context(), cache);
                    let next = if cache == 58 { 0 } else { cache + 1 };
                    ram::g_cached_random_values(r)
                        .at(i32::from(next))
                        .write(&mut host.context(), random);
                    labelled(
                        host,
                        id,
                        &[ram::g_player_info(r).at(index).address(), 0x10],
                        &format!("rank-cache-{cache}-phase-{phase}-random-{random}"),
                    );
                }
            }
        }
        ids::PRESENT_UPDATE_BURPS => {
            for index in 0..2 {
                reset(host);
                labelled(host, id, &[index as u32], "no-burps");
                for cooldown in [0, 5] {
                    reset(host);
                    ram::g_player_burps_left(r)
                        .at(index)
                        .write(&mut host.context(), 1);
                    ram::g_player_burp_timers(r)
                        .at(index)
                        .write(&mut host.context(), 0);
                    ram::g_pcm_effect_cooldown_ticks(r).write(&mut host.context(), cooldown);
                    labelled(
                        host,
                        id,
                        &[index as u32],
                        &format!("burp-expiry-cooldown-{cooldown}"),
                    );
                }
            }
        }
        ids::PRESENT_UPDATE_TERRAIN_EFFECTS => {
            for index in 0..2 {
                for (label, powerup, state, elevator) in [
                    ("retire-powerup", -1, 0, 0),
                    ("retire-ghost", 6, 6, 0),
                    ("retire-elevator", 6, 0, 1),
                ] {
                    reset(host);
                    let p = ram::g_player_info(r).at(index);
                    p.sprite_state().write(&mut host.context(), state);
                    ram::g_player_powerups(r)
                        .at(index)
                        .write(&mut host.context(), powerup);
                    ram::g_player_elevator_flags(r)
                        .at(index)
                        .write(&mut host.context(), elevator);
                    ram::g_player_active_terrain_effect_entities(r)
                        .at(index)
                        .write(&mut host.context(), 6);
                    labelled(host, id, &[p.address(), 0x10], label);
                }
                reset(host);
                ram::g_player_one_shot_terrain_effect_entities(r)
                    .at(index)
                    .write(&mut host.context(), 6);
                ram::g_player_info(r)
                    .at(6)
                    .frame_duration()
                    .write(&mut host.context(), 0);
                labelled(
                    host,
                    id,
                    &[ram::g_player_info(r).at(index).address(), 0x10],
                    "one-shot-finished-cleanup",
                );
            }
        }
        ids::PLAYER_TELEPORT_THROUGH_DOORWAY => {
            for lives in [-1, 3] {
                reset(host);
                ram::g_split_screen_enabled(r).write(&mut host.context(), 1);
                ram::g_player_lives(r)
                    .at(0)
                    .write(&mut host.context(), lives);
                ram::g_camera1_pending_x(r).write(&mut host.context(), -10000);
                ram::g_camera1_pending_y(r).write(&mut host.context(), -10000);
                ram::g_camera2_pending_x(r).write(&mut host.context(), -10000);
                ram::g_camera2_pending_y(r).write(&mut host.context(), -10000);
                labelled(
                    host,
                    id,
                    &[1, ram::g_player_info(r).at(1).address()],
                    &format!("split-earl-toejam-lives-{lives}"),
                );
            }
        }
        ids::PRESENT_UPDATE_DOORWAY => {
            for index in 0..2 {
                reset(host);
                labelled(host, id, &[index as u32], "no-doorway");
                for (label, draw, hidden, contact, y) in [
                    ("offscreen-release", 0, 0, 0, 400),
                    ("hidden-reveal", 6, 0x40, 0, 400),
                    ("centered-crossing", 6, 0, 4 | (index as i8 + 1), 399),
                ] {
                    reset(host);
                    let door = ram::g_player_info(r).at(6);
                    door.x().write(&mut host.context(), 400);
                    door.y().write(&mut host.context(), 400);
                    door.draw_flags().write(&mut host.context(), draw);
                    door.behavior_flags().write(&mut host.context(), hidden);
                    ram::g_player_info(r)
                        .at(index)
                        .x()
                        .write(&mut host.context(), 400);
                    ram::g_player_info(r)
                        .at(index)
                        .y()
                        .write(&mut host.context(), y);
                    ram::g_player_doorway_entities(r)
                        .at(index)
                        .write(&mut host.context(), 6);
                    ram::g_doorway_player_contact_flags(r)
                        .at(index)
                        .write(&mut host.context(), contact);
                    labelled(host, id, &[index as u32], label);
                }
            }
        }
        ids::CHECK_FOR_TELEPHONE => {
            // Acknowledgement is real controller input, shared by the check forks.
            let start = host.frame_count();
            host.set_wait_input_poller(Arc::new(InputScript {
                p2_connected: true,
                holds: (1..100)
                    .map(|pulse| InputHold {
                        start: start + pulse * 30,
                        end: start + pulse * 30 + 5,
                        p1: vec![Button::A],
                        p2: vec![Button::A],
                    })
                    .collect(),
            }));
            for index in 0..2 {
                reset(host);
                labelled(host, id, &[index as u32], "no-telephone");
                for (label, activated, object, x, y, delay, rings, cooldown) in [
                    ("nearby-activation", 0, 6, 500, 450, 60, 2, 0),
                    ("answer-map", 1, 6, 400 + index as i16 * 128, 400, 60, 2, 0),
                    ("offscreen-object-release", 1, 6, 10000, 10000, 60, 2, 0),
                    ("last-ring-expiry", 1, 6, 600, 500, 1, 1, 0),
                    ("pcm-delays-ring", 1, 6, 600, 500, 1, 2, 5),
                ] {
                    reset(host);
                    let phone = ram::g_telephone_states(r).at(index);
                    phone.level().write(&mut host.context(), 1);
                    phone.object().write(&mut host.context(), object);
                    phone.x().write(&mut host.context(), x);
                    phone.y().write(&mut host.context(), y);
                    ram::g_camera1_pending_x(r).write(&mut host.context(), 300);
                    ram::g_camera1_pending_y(r).write(&mut host.context(), 300);
                    ram::g_telephone_activated(r)
                        .at(index)
                        .write(&mut host.context(), activated);
                    ram::g_telephone_time_to_ring(r)
                        .at(index)
                        .write(&mut host.context(), delay);
                    ram::g_telephone_rings_left(r)
                        .at(index)
                        .write(&mut host.context(), rings);
                    ram::g_pcm_effect_cooldown_ticks(r).write(&mut host.context(), cooldown);
                    labelled(host, id, &[index as u32], label);
                }
            }
        }
        ids::PROCESS_PRESENTS => {
            reset(host);
            for index in 0..2 {
                ram::g_player_burps_left(r)
                    .at(index)
                    .write(&mut host.context(), 1);
                ram::g_player_burp_timers(r)
                    .at(index)
                    .write(&mut host.context(), -1);
            }
            labelled(host, id, &[], "both-players-burp-start-order");
        }
        _ => return,
    }
    reset(host);
}
