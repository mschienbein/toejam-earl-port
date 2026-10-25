//! Labelled interventions on a separately reached, settled original gameplay state.
//! These are not natural-session coverage, reset emulation or replacement admission.
use tje_game::{FunctionId, ids};
use tje_host::Host;
use tje_mem::{Address, ram};

pub(super) const ROOTS: &[FunctionId] = &[
    ids::PLAYER_START_FALLING,
    ids::CHECK_PLAYER_FALLING,
    ids::MAKE_PLAYER_FALL,
    ids::PLAYER_UPDATE_TERRAIN_SINKING,
    ids::PLAYER_UPDATE_HURT_RECOVERY_AND_IDLE,
    ids::PLAYER_RESOLVE_DESCENDING_TERRAIN,
    ids::PLAYER_ADVANCE_TO_NEXT_LEVEL,
    ids::PLAYER_RELOCATE_TO_PARTNER_OR_NEXT_LEVEL,
    ids::KILL_THE_PLAYER,
    ids::PLAYER_FINALIZE_LAST_LIFE_EXIT,
    ids::PLAYER_UPDATE_DEATH_ANIMATION,
    ids::PROCESS_PLAYERS,
];

fn prepare(host: &mut Host) {
    let r = host.revision();
    for n in 0..2 {
        let p = ram::g_player_info(r).at(n);
        let mut c = host.context();
        p.entity_index().write(&mut c, n as i8);
        p.level().write(&mut c, 1);
        p.x().write(&mut c, 512);
        p.y().write(&mut c, 512);
        p.height().write(&mut c, 0);
        p.velocity_x().write(&mut c, 0);
        p.velocity_y().write(&mut c, 0);
        p.velocity_height().write(&mut c, 0);
        p.acceleration_x().write(&mut c, 0);
        p.acceleration_y().write(&mut c, 0);
        p.acceleration_height().write(&mut c, 0);
        p.draw_flags().write(&mut c, 3);
        p.behavior_flags().write(&mut c, 1);
        p.sprite_state().write(&mut c, 0);
        p.animation_group_mask().write(&mut c, 1);
        p.animation_motion_axes().write(&mut c, 0);
        p.frame_duration().write(&mut c, 1);
        ram::g_player_lives(r).at(n).write(&mut c, 2);
        ram::g_player_health(r).at(n).write(&mut c, 20);
        ram::g_player_inventory_open(r).at(n).write(&mut c, 0);
        ram::g_player_pending_hurt_damage(r).at(n).write(&mut c, 2);
        ram::g_player_powerups(r).at(n).write(&mut c, -1);
        ram::g_player_elevator_flags(r).at(n).write(&mut c, 0);
        ram::g_player_fall_states(r).at(n).write(&mut c, 0);
        ram::g_player_terrain_sinking(r).at(n).write(&mut c, 0);
        ram::g_player_interaction_choice_active(r)
            .at(n)
            .write(&mut c, 0);
        ram::g_player_control_remap_entities(r)
            .at(n)
            .write(&mut c, 0);
        ram::g_player_processed_input(r).at(n).write(&mut c, 0);
        ram::g_player_respawn_suppression_ticks(r)
            .at(n)
            .write(&mut c, 0);
        let terrain = (if n == 0 {
            ram::g_player1_nearby_terrain(r)
        } else {
            ram::g_player2_nearby_terrain(r)
        })
        .cast::<u8>();
        for cell in 0..25 {
            terrain.offset(cell).write(&mut c, 0x10);
        }
    }
    let mut c = host.context();
    ram::g_player1_terrain_underfoot(r).write(&mut c, 0x10);
    ram::g_player2_terrain_underfoot(r).write(&mut c, 0x10);
    ram::g_elevator_departure_in_progress(r).write(&mut c, 0);
    ram::g_player_selection(r).write(&mut c, 0);
    ram::g_random_cache_index(r).write(&mut c, 58);
    ram::g_cached_random_values(r).at(0).write(&mut c, 3);
    // No SR, frame-counter, pending-upload or device-ready value is fabricated.
}
fn attempt(
    host: &mut Host,
    id: FunctionId,
    label: &str,
    arguments: &[u32],
    rows: &mut Vec<serde_json::Value>,
) {
    let name = id.metadata().name;
    let before = host.checks.by_function[name].calls_checked;
    let before_mismatches = host.checks.by_function[name].mismatches;
    let frame_counter = ram::g_frame_counter(host.revision()).read(&host.context());
    let result = super::fixture_entries::check_argument_root(host, id, arguments)
        .unwrap_or_else(|| host.check_fixture(id.metadata().address(host.revision()), arguments));
    rows.push(serde_json::json!({"function":name,"label":label,"arguments":arguments,"frame":host.frame_count(),"frame_counter":frame_counter,"checked":host.checks.by_function[name].calls_checked-before,"mismatches":host.checks.by_function[name].mismatches-before_mismatches,"error":result.err().map(|error|error.to_string())}));
}

pub(super) fn run(host: &mut Host) -> Vec<serde_json::Value> {
    let r = host.revision();
    let mut rows = Vec::new();
    for number in 0..2 {
        let p = ram::g_player_info(r).at(number);
        for variant in 0..3 {
            prepare(host);
            if variant == 1 {
                ram::g_player_inventory_open(r)
                    .at(number)
                    .write(&mut host.context(), 1);
            }
            if variant == 2 {
                ram::g_player_powerups(r)
                    .at(number)
                    .write(&mut host.context(), 5);
            }
            attempt(
                host,
                ids::KILL_THE_PLAYER,
                &format!("player{number}-kill-plain-inventory-skates-{variant}"),
                &[number as u32],
                &mut rows,
            );
        }
        for direction in 0..4 {
            prepare(host);
            p.sprite_state().write(&mut host.context(), direction);
            if direction == 1 {
                let nearby = (if number == 0 {
                    ram::g_player1_nearby_terrain(r)
                } else {
                    ram::g_player2_nearby_terrain(r)
                })
                .cast::<u8>();
                nearby.offset(11).write(&mut host.context(), 0x90);
            }
            attempt(
                host,
                ids::PLAYER_START_FALLING,
                &format!("player{number}-fall-direction{direction}-real-priority-irq"),
                &[p.address(), number as u32],
                &mut rows,
            );
        }
        for state in [79, 78, 5, 6, 0] {
            prepare(host);
            p.sprite_state().write(&mut host.context(), state);
            attempt(
                host,
                ids::PLAYER_UPDATE_TERRAIN_SINKING,
                &format!("player{number}-sinking-state-guard{state}"),
                &[p.address(), 0x10],
                &mut rows,
            );
        }
        for (height, sinking, land) in [(0, 0, 0x20), (-3, 1, 0x10), (-24, 1, 0x20)] {
            prepare(host);
            p.height().write(&mut host.context(), height);
            ram::g_player_terrain_sinking(r)
                .at(number)
                .write(&mut host.context(), sinking);
            attempt(
                host,
                ids::PLAYER_UPDATE_TERRAIN_SINKING,
                &format!("player{number}-sinking-height{height}-state{sinking}-terrain{land:02x}"),
                &[p.address(), land],
                &mut rows,
            );
        }
        for variant in 0..5 {
            prepare(host);
            p.sprite_state().write(&mut host.context(), 52);
            p.animation_group_mask().write(&mut host.context(), 0x2000);
            p.frame_duration().write(&mut host.context(), 0);
            if variant == 1 {
                p.height().write(&mut host.context(), 10);
                p.sprite_state().write(&mut host.context(), 68);
            }
            if variant == 2 || variant == 3 {
                let nearby = (if number == 0 {
                    ram::g_player1_nearby_terrain(r)
                } else {
                    ram::g_player2_nearby_terrain(r)
                })
                .cast::<u8>();
                nearby
                    .offset(12)
                    .write(&mut host.context(), if variant == 2 { 0x80 } else { 0x90 });
            }
            if variant == 4 {
                ram::g_player_powerups(r)
                    .at(number)
                    .write(&mut host.context(), 5);
            }
            attempt(
                host,
                ids::PLAYER_UPDATE_HURT_RECOVERY_AND_IDLE,
                &format!("player{number}-hurt-ground-air-water-void-skates-{variant}"),
                &[p.address(), 0x10],
                &mut rows,
            );
        }
        for (land, power, height, input) in [
            (0x10, -1, 100, 0),
            (0x10, 1, 61, 0x40),
            (0x80, -1, 3, 0),
            (0x90, -1, 3, 0),
            (0x10, 7, 3, 0),
            (0x10, 7, 3, 1),
            (0x10, -1, 3, 0),
        ] {
            prepare(host);
            p.sprite_state().write(&mut host.context(), 21);
            p.animation_group_mask().write(&mut host.context(), 0x20);
            p.height().write(&mut host.context(), height);
            p.velocity_height()
                .write(&mut host.context(), if power == 1 { 256 } else { -128 });
            ram::g_player_powerups(r)
                .at(number)
                .write(&mut host.context(), power);
            ram::g_player_processed_input(r)
                .at(number)
                .write(&mut host.context(), input);
            attempt(
                host,
                ids::PLAYER_RESOLVE_DESCENDING_TERRAIN,
                &format!(
                    "player{number}-resolve-{land:02x}-power{power}-height{height}-input{input}"
                ),
                &[p.address(), land],
                &mut rows,
            );
        }
        prepare(host);
        p.level().write(&mut host.context(), 0);
        attempt(
            host,
            ids::PLAYER_ADVANCE_TO_NEXT_LEVEL,
            &format!("player{number}-cached-level-zero-to-one"),
            &[p.address(), number as u32],
            &mut rows,
        );
        for variant in 0..3 {
            prepare(host);
            if variant == 1 {
                ram::g_elevator_departure_in_progress(r).write(&mut host.context(), 1);
                ram::g_player_elevator_flags(r)
                    .at(number ^ 1)
                    .write(&mut host.context(), 1);
            }
            if variant == 2 {
                ram::g_player_lives(r)
                    .at(number ^ 1)
                    .write(&mut host.context(), -1);
                ram::g_highest_level(r).write(&mut host.context(), 1);
            }
            attempt(
                host,
                ids::PLAYER_RELOCATE_TO_PARTNER_OR_NEXT_LEVEL,
                &format!("player{number}-relocate-partner-elevator-alone-{variant}"),
                &[number as u32],
                &mut rows,
            );
        }
        prepare(host);
        ram::g_player_lives(r)
            .at(number)
            .write(&mut host.context(), 0);
        attempt(
            host,
            ids::PLAYER_FINALIZE_LAST_LIFE_EXIT,
            &format!("player{number}-finalize-surviving-partner-return-only"),
            &[p.address(), number as u32],
            &mut rows,
        );
        for variant in 0..4 {
            prepare(host);
            p.sprite_state().write(&mut host.context(), 5);
            if variant == 0 {
                p.height().write(&mut host.context(), -1);
                ram::g_player_jump_counters(r)
                    .at(number)
                    .write(&mut host.context(), 2);
            }
            if variant == 1 {
                p.behavior_flags().write(&mut host.context(), 0x80);
                p.height().write(&mut host.context(), 20);
            }
            if variant == 2 {
                p.sprite_state().write(&mut host.context(), 6);
                p.behavior_flags().write(&mut host.context(), 0x80);
                p.height().write(&mut host.context(), 20);
                ram::g_player_jump_counters(r)
                    .at(number)
                    .write(&mut host.context(), 1);
            }
            if variant == 3 {
                p.sprite_state().write(&mut host.context(), 0);
                ram::g_player_respawn_suppression_ticks(r)
                    .at(number)
                    .write(&mut host.context(), 1);
            }
            attempt(
                host,
                ids::PLAYER_UPDATE_DEATH_ANIMATION,
                &format!("player{number}-death-blink-speech-restart-suppression-{variant}"),
                &[p.address(), 0x10],
                &mut rows,
            );
        }
    }
    for variant in 0..3 {
        prepare(host);
        for number in 0..2 {
            ram::g_player_fall_states(r)
                .at(number)
                .write(&mut host.context(), variant);
            ram::g_player_info(r)
                .at(number)
                .height()
                .write(&mut host.context(), if variant == 1 { -225 } else { 10 });
            ram::g_player_jump_counters(r)
                .at(number)
                .write(&mut host.context(), 2);
        }
        attempt(
            host,
            ids::CHECK_PLAYER_FALLING,
            &format!("check-falling-state{variant}-actual-address-taken-local"),
            &[],
            &mut rows,
        );
    }
    let readiness = Address::<u8>::new(ram::g_player_info(r).at(39).x().address());
    for pair in [[0, 0], [1, 0], [0, 1], [1, 1]] {
        prepare(host);
        readiness.write(&mut host.context(), pair[0]);
        readiness.offset(1).write(&mut host.context(), pair[1]);
        attempt(
            host,
            ids::MAKE_PLAYER_FALL,
            &format!(
                "make-fall-readiness{}{}-labelled-unused-object-bytes",
                pair[0], pair[1]
            ),
            &[readiness.address()],
            &mut rows,
        );
    }
    for inventory in [0, 1] {
        prepare(host);
        for number in 0..2 {
            ram::g_player_inventory_open(r)
                .at(number)
                .write(&mut host.context(), inventory);
        }
        attempt(
            host,
            ids::PROCESS_PLAYERS,
            &format!("process-both-players-inventory{inventory}"),
            &[],
            &mut rows,
        );
    }
    rows
}
