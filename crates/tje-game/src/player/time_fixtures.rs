//! Labelled live-state checks for pending TimeAware player bodies. This binary
//! never registers them in the production translation list.
use std::{fs, path::PathBuf, sync::Arc};
use tje_game::{Context, FunctionId, ids};
use tje_host::{Host, Mode, hooks::HookRegistry, input::InputScript};
use tje_mem::{Address, ram};

mod proof_support;

fn prepare(host: &mut Host) {
    let r = host.revision();
    for n in 0..2 {
        let p = ram::g_player_info(r).at(n);
        p.entity_index().write(&mut host.context(), n as i8);
        p.level().write(&mut host.context(), 1);
        p.x().write(&mut host.context(), 512);
        p.y().write(&mut host.context(), 512);
        p.height().write(&mut host.context(), 0);
        p.velocity_x().write(&mut host.context(), 0);
        p.velocity_y().write(&mut host.context(), 0);
        p.velocity_height().write(&mut host.context(), 0);
        p.acceleration_x().write(&mut host.context(), 0);
        p.acceleration_y().write(&mut host.context(), 0);
        p.acceleration_height().write(&mut host.context(), 0);
        p.draw_flags().write(
            &mut host.context(),
            if std::env::var_os("PLAYER_TIME_ACTIVE").is_some() {
                3
            } else {
                1
            },
        );
        p.behavior_flags().write(&mut host.context(), 1);
        p.sprite_state().write(&mut host.context(), 0);
        p.animation_group_mask().write(&mut host.context(), 1);
        p.animation_motion_axes().write(&mut host.context(), 0);
        p.frame_duration().write(&mut host.context(), 1);
        ram::g_player_inventory_open(r)
            .at(n)
            .write(&mut host.context(), 0);
        ram::g_player_lives(r).at(n).write(&mut host.context(), 2);
        ram::g_player_health(r).at(n).write(&mut host.context(), 20);
        ram::g_player_pending_hurt_damage(r)
            .at(n)
            .write(&mut host.context(), 2);
        ram::g_player_powerups(r)
            .at(n)
            .write(&mut host.context(), -1);
        ram::g_player_elevator_flags(r)
            .at(n)
            .write(&mut host.context(), 0);
        ram::g_player_fall_states(r)
            .at(n)
            .write(&mut host.context(), 0);
        ram::g_player_terrain_sinking(r)
            .at(n)
            .write(&mut host.context(), 0);
        ram::g_player_interaction_choice_active(r)
            .at(n)
            .write(&mut host.context(), 0);
        ram::g_player_control_remap_entities(r)
            .at(n)
            .write(&mut host.context(), 0);
        ram::g_player_processed_input(r)
            .at(n)
            .write(&mut host.context(), 0);
        ram::g_player_respawn_suppression_ticks(r)
            .at(n)
            .write(&mut host.context(), 0);
        let terrain = if n == 0 {
            ram::g_player1_nearby_terrain(r)
        } else {
            ram::g_player2_nearby_terrain(r)
        }
        .cast::<u8>();
        for cell in 0..25 {
            terrain.offset(cell).write(&mut host.context(), 0x10);
        }
    }
    ram::g_elevator_departure_in_progress(r).write(&mut host.context(), 0);
    ram::g_player_selection(r).write(&mut host.context(), 0);
    ram::g_random_cache_index(r).write(&mut host.context(), 58);
    ram::g_cached_random_values(r)
        .at(0)
        .write(&mut host.context(), 3);
    let e = ram::g_earthling_banks(r).at(0).at(0);
    e.kind().write(&mut host.context(), 1);
    e.object().write(&mut host.context(), 25);
    e.level().write(&mut host.context(), 1);
    e.state().write(&mut host.context(), 0);
    e.x().write(&mut host.context(), 500);
    e.y().write(&mut host.context(), 512);
    let attacker = ram::g_player_info(r).at(25);
    attacker.x().write(&mut host.context(), 500);
    attacker.y().write(&mut host.context(), 512);
    attacker.level().write(&mut host.context(), 1);
    attacker.draw_flags().write(&mut host.context(), 1);
}

fn attempt(host: &mut Host, id: FunctionId, errors: &mut Vec<serde_json::Value>) {
    prepare(host);
    let r = host.revision();
    let p = ram::g_player_info(r).at(0);
    let e = ram::g_earthling_banks(r).at(0).at(0);
    let mut args = vec![p.address(), 0x10];
    let active = std::env::var_os("PLAYER_TIME_ACTIVE").is_some();
    match id {
        ids::KILL_THE_PLAYER => {
            if active {
                ram::g_player_powerups(r)
                    .at(0)
                    .write(&mut host.context(), 5);
            }
            args = vec![0];
        }
        ids::PLAYER_START_FALLING => args = vec![p.address(), 0],
        ids::PLAYER_UPDATE_TERRAIN_SINKING => {
            p.height().write(&mut host.context(), -3);
            args[1] = 0x20;
        }
        ids::PLAYER_UPDATE_HURT_RECOVERY_AND_IDLE => {
            p.sprite_state().write(&mut host.context(), 52);
            p.animation_group_mask().write(&mut host.context(), 0x2000);
            p.frame_duration().write(&mut host.context(), 0);
        }
        ids::PLAYER_RESOLVE_DESCENDING_TERRAIN => {
            p.sprite_state().write(&mut host.context(), 21);
            p.animation_group_mask().write(&mut host.context(), 0x20);
            p.height().write(&mut host.context(), 3);
            if active {
                args[1] = 0x90;
            }
        }
        ids::PLAYER_APPLY_PHASED_JUMP_INPUT => {
            ram::g_player_phased_jump_speech_used(r)
                .at(0)
                .write(&mut host.context(), 1);
            args = vec![p.address(), 0, 1, 0];
            if active {
                p.height().write(&mut host.context(), 20);
                ram::g_player_phased_jump_speech_used(r)
                    .at(0)
                    .write(&mut host.context(), 0);
            }
        }
        ids::PLAYER_UPDATE_PHASED_JUMP_MOTION => {
            ram::g_player_powerups(r)
                .at(0)
                .write(&mut host.context(), 0);
            ram::g_player_phased_jump_armed(r)
                .at(0)
                .write(&mut host.context(), 1);
            ram::g_player_choice_action_release_latches(r)
                .at(0)
                .write(&mut host.context(), 0);
            ram::g_player_processed_input(r)
                .at(0)
                .write(&mut host.context(), 0x40);
            ram::g_player_phased_jump_speech_used(r)
                .at(0)
                .write(&mut host.context(), 1);
            if active {
                p.height().write(&mut host.context(), 20);
                ram::g_player_phased_jump_speech_used(r)
                    .at(0)
                    .write(&mut host.context(), 0);
            }
        }
        ids::PLAYER_YOUCH => args = vec![0, e.address()],
        ids::HULA_ABILITY => {
            e.kind().write(&mut host.context(), 5);
            ram::g_player_hula_contact_delay(r)
                .at(0)
                .write(&mut host.context(), 0);
            ram::g_hula_speech_cooldown(r).write(&mut host.context(), 0);
            args = vec![0, e.address()];
        }
        ids::HOTTUB_ABILITY => {
            e.kind().write(&mut host.context(), 24);
            e.y().write(&mut host.context(), 516);
            p.sprite_state().write(&mut host.context(), 119);
            p.animation_group_mask()
                .write(&mut host.context(), 0x20000000);
            ram::g_player_processed_input(r)
                .at(0)
                .write(&mut host.context(), 4);
            if active {
                // Source RNG seed 32 yields a low-word multiple of 32 on
                // the first Park-Miller draw, selecting the real chat branch.
                ram::g_rng_state(r).write(&mut host.context(), 32);
                ram::g_player_processed_input(r)
                    .at(0)
                    .write(&mut host.context(), 0);
            }
            args = vec![0, e.address()];
        }
        ids::CHICKEN_ABILITY => {
            e.kind().write(&mut host.context(), 15);
            args = vec![0, e.address()];
        }
        ids::AI_BOOGIE_TOUCH => {
            e.kind().write(&mut host.context(), 16);
            args = vec![0, e.address()];
        }
        ids::PLAYER_UPDATE_WHIRLWIND_AND_RECOVERY => {
            p.sprite_state().write(&mut host.context(), 79);
            ram::g_player_whirlwind_timers(r)
                .at(0)
                .write(&mut host.context(), 1);
            if active {
                p.sprite_state().write(&mut host.context(), 78);
                ram::g_player_whirlwind_recovery_timers(r)
                    .at(0)
                    .write(&mut host.context(), 0);
                ram::g_cached_random_values(r)
                    .at(0)
                    .write(&mut host.context(), 1);
            }
        }
        ids::LEMONADE_ABILITY => {
            e.kind().write(&mut host.context(), 23);
            e.y().write(&mut host.context(), 500);
            p.sprite_state().write(&mut host.context(), 8);
            ram::g_player_lemonade_reward_states(r)
                .at(0)
                .write(&mut host.context(), 0);
            args = vec![0, e.address()];
            if active {
                p.sprite_state().write(&mut host.context(), 118);
                p.frame_duration().write(&mut host.context(), 0);
            }
        }
        ids::PLAYER_ADVANCE_TO_NEXT_LEVEL => {
            // Additional labelled level-zero -> cached first-level case;
            // the prior level-one -> generated-two ceiling failure is retained.
            p.level().write(&mut host.context(), 0);
            args = vec![p.address(), 0];
        }
        ids::PLAYER_RELOCATE_TO_PARTNER_OR_NEXT_LEVEL => args = vec![0],
        ids::MAKE_PLAYER_FALL => {
            let flags = ram::g_player_info(r).at(39).address();
            Address::<i8>::new(flags).write(&mut host.context(), 1);
            Address::<i8>::new(flags + 1).write(&mut host.context(), 0);
            p.level().write(&mut host.context(), 2);
            args = vec![flags];
        }
        ids::PLAYER_UPDATE_DEATH_ANIMATION => {
            p.sprite_state().write(&mut host.context(), 5);
            p.height().write(&mut host.context(), -1);
            ram::g_camera1_pending_y(r).write(&mut host.context(), 0);
            ram::g_player_jump_counters(r)
                .at(0)
                .write(&mut host.context(), 1);
        }
        ids::PROCESS_PLAYERS => args.clear(),
        ids::PLAYER_CHOOSE_DIRECTIONAL_MOVEMENT => args = vec![p.address(), 0, 8, 0x10],
        ids::SPRITE_CHECKS => args = vec![0, 8],
        ids::CHECK_PLAYER_FALLING => args.clear(),
        ids::PLAYER_FINALIZE_LAST_LIFE_EXIT => args = vec![p.address(), 0],
        _ => unreachable!(),
    }
    let result = if matches!(
        id,
        ids::PLAYER_CHOOSE_DIRECTIONAL_MOVEMENT | ids::SPRITE_CHECKS
    ) {
        // A labelled fixture prepares the actual source JSR return, not an
        // eager host argument snapshot for a ctx-only frame consumer.
        let mut context = host.context();
        let base = context.stack_pointer().unwrap();
        let mut sp = base;
        for &argument in args.iter().rev() {
            sp = sp.wrapping_sub(4);
            context.write_u32(sp, argument);
        }
        let site = if id == ids::PLAYER_CHOOSE_DIRECTIONAL_MOVEMENT {
            ids::SPRITE_CHECKS.metadata().address(r) + 0x212
        } else {
            ids::PROCESS_INPUT.metadata().address(r) + 0xaa
        };
        sp = sp.wrapping_sub(4);
        context.write_u32(sp, site);
        context.set_stack_pointer(sp).unwrap();
        let result = host
            .check_entry_fixture(id.metadata().address(r))
            .map(|_| 0);
        host.context().set_stack_pointer(base).unwrap();
        result
    } else {
        host.check_fixture(id.metadata().address(r), &args)
    };
    if let Err(error) = result {
        errors.push(serde_json::json!({"function":id.metadata().name,"arguments":args,"error":error.to_string()}));
    }
}

fn main() {
    let out = PathBuf::from(std::env::args_os().nth(1).expect("evidence directory"));
    fs::create_dir_all(&out).unwrap();
    let script = Arc::new(
        InputScript::parse(
            &fs::read_to_string("crates/tje-host/examples/start-game.json").unwrap(),
        )
        .unwrap(),
    );
    for (r, name) in [
        (tje_mem::Revision::Rev02, "rev02"),
        (tje_mem::Revision::Rev00, "rev00"),
    ] {
        let registry = HookRegistry::proof_with_frames(
            r,
            tje_game::player::PENDING_TRANSLATIONS,
            tje_game::player::PENDING_FRAMED_TRANSLATIONS,
        )
        .unwrap();
        let mut host = Host::new(
            &fs::read(format!("roms/{name}.bin")).unwrap(),
            Mode::Reference,
            registry,
        )
        .unwrap();
        host.set_wait_input_poller(script.clone());
        for frame in 0..10000 {
            host.run_frame(script.at(frame)).unwrap();
        }
        assert!(host.measurements.game_frame.is_some());
        let boundary = proof_support::settle(&mut host, &script);
        let mut errors = Vec::new();
        for id in [
            ids::KILL_THE_PLAYER,
            ids::PLAYER_START_FALLING,
            ids::PLAYER_UPDATE_TERRAIN_SINKING,
            ids::PLAYER_UPDATE_HURT_RECOVERY_AND_IDLE,
            ids::PLAYER_RESOLVE_DESCENDING_TERRAIN,
            ids::PLAYER_APPLY_PHASED_JUMP_INPUT,
            ids::PLAYER_UPDATE_PHASED_JUMP_MOTION,
            ids::PLAYER_YOUCH,
            ids::HULA_ABILITY,
            ids::HOTTUB_ABILITY,
            ids::CHICKEN_ABILITY,
            ids::AI_BOOGIE_TOUCH,
            ids::PLAYER_UPDATE_WHIRLWIND_AND_RECOVERY,
            ids::LEMONADE_ABILITY,
            ids::PLAYER_ADVANCE_TO_NEXT_LEVEL,
            ids::PLAYER_RELOCATE_TO_PARTNER_OR_NEXT_LEVEL,
            ids::MAKE_PLAYER_FALL,
            ids::PLAYER_UPDATE_DEATH_ANIMATION,
            ids::PROCESS_PLAYERS,
            ids::PLAYER_CHOOSE_DIRECTIONAL_MOVEMENT,
            ids::SPRITE_CHECKS,
            ids::CHECK_PLAYER_FALLING,
            ids::PLAYER_FINALIZE_LAST_LIFE_EXIT,
        ] {
            if std::env::var("PLAYER_TIME_FUNCTIONS")
                .is_ok_and(|names| !names.split(',').any(|name| name == id.metadata().name))
            {
                continue;
            }
            attempt(&mut host, id, &mut errors);
        }
        let report = serde_json::json!({"revision":name,"fixture_input":"crates/tje-host/examples/start-game.json","fixture_frames":10000,"settled_boundary":boundary,"kind":"labelled live-state fixtures; not natural calls","checks":host.checks,"errors":errors});
        fs::write(
            out.join(format!("{name}-time-proof.json")),
            serde_json::to_string_pretty(&report).unwrap(),
        )
        .unwrap();
        println!(
            "PLAYER_TIME_PROOF {name} checked={} mismatches={} errors={}",
            host.checks.calls,
            host.checks.mismatches,
            errors.len()
        );
    }
}
