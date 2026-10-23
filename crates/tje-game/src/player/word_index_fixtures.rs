//! Both-revision labelled large/negative index fixtures for ASL.W/ADDA.W.
use std::{fs, path::PathBuf};
use tje_game::{FunctionId, ids};
use tje_host::{Host, Mode, hooks::HookRegistry, input::InputScript};
use tje_mem::{Revision, ram};

fn check(host: &mut Host, id: FunctionId, args: &[u32]) {
    host.check_fixture(id.metadata().address(host.revision()), args)
        .unwrap_or_else(|error| panic!("{} {args:08x?}: {error}", id.metadata().name));
}

fn fixtures(host: &mut Host) {
    let r = host.revision();
    let e = ram::g_earthling_banks(r).at(0).at(0);
    for number in [512_i16, 513, -512] {
        // Independent, explicit low-word scale selects record zero/one.
        let p = ram::g_player_info(r)
            .byte_offset(i32::from(number.wrapping_mul(128)))
            .cast::<tje_mem::ObjectState>();
        let n = i32::from(number);
        p.entity_index()
            .write(&mut host.context(), if number == 513 { 1 } else { 0 });
        p.x().write(&mut host.context(), 512);
        p.y().write(&mut host.context(), 512);
        p.level().write(&mut host.context(), 1);
        p.height().write(&mut host.context(), 20);
        p.draw_flags().write(&mut host.context(), 1);
        p.sprite_state().write(&mut host.context(), 0);
        p.animation_group_mask().write(&mut host.context(), 1);
        ram::g_player_interaction_choice_active(r)
            .at(n)
            .write(&mut host.context(), 0);
        ram::g_player_terrain_sinking(r)
            .at(n)
            .write(&mut host.context(), 0);
        ram::g_player_health(r).at(n).write(&mut host.context(), 20);
        ram::g_terrain_priority_update_player(r).write(&mut host.context(), number);
        check(host, ids::MAP_CLEAR_TILE_PRIORITY_NEAR_PLAYER, &[]);
        e.kind().write(&mut host.context(), 7);
        e.state().write(&mut host.context(), 0);
        e.object().write(&mut host.context(), 25);
        e.x().write(&mut host.context(), 512);
        e.y().write(&mut host.context(), 512);
        check(host, ids::CUPID_ABILITY, &[number as u32, e.address()]);
        e.kind().write(&mut host.context(), 1);
        check(host, ids::PLAYER_SQUISH, &[number as u32, e.address()]);
        e.kind().write(&mut host.context(), 8);
        e.state().write(&mut host.context(), 4);
        ram::g_player_info(r)
            .at(25)
            .draw_flags()
            .write(&mut host.context(), 1);
        check(host, ids::WHIRL_WIND_ABILITY, &[number as u32, e.address()]);
        e.kind().write(&mut host.context(), 18);
        e.state().write(&mut host.context(), 3);
        e.timer().write(&mut host.context(), 14);
        p.sprite_state().write(&mut host.context(), 70);
        check(host, ids::MOLE_ABILITY, &[number as u32, e.address()]);
    }
}

fn main() {
    let out = PathBuf::from(std::env::args_os().nth(1).expect("evidence directory"));
    fs::create_dir_all(&out).unwrap();
    let script = InputScript::parse(
        &fs::read_to_string("crates/tje-host/examples/start-game.json").unwrap(),
    )
    .unwrap();
    for (r, name) in [(Revision::Rev02, "rev02"), (Revision::Rev00, "rev00")] {
        let registry = HookRegistry::filtered(r, &["player"], &[]).unwrap();
        let mut host = Host::new(
            &fs::read(format!("roms/{name}.bin")).unwrap(),
            Mode::Reference,
            registry,
        )
        .unwrap();
        for frame in 0..10000 {
            host.run_frame(script.at(frame)).unwrap();
        }
        assert!(host.measurements.game_frame.is_some());
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| fixtures(&mut host)));
        fs::write(out.join(format!("{name}-index-proof.json")),serde_json::to_string_pretty(&serde_json::json!({"revision":name,"kind":"labelled live-state large/negative index fixtures","checks":host.checks,"completed":result.is_ok()})).unwrap()).unwrap();
        if let Err(error) = result {
            std::panic::resume_unwind(error);
        }
        assert_eq!(host.checks.mismatches, 0);
        println!(
            "PLAYER_WORD_INDEX_PROOF {name} checked={} mismatches={}",
            host.checks.calls, host.checks.mismatches
        );
    }
}
