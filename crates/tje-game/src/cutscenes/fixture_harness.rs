//! Opt-in external host harness, never linked into tje-game. A private manifest
//! names this file as a bin and depends on tje-game, tje-host, tje-mem, serde_json.
//! Arguments: ROM INPUT_JSON METRICS_JSON FRAMES [comma-separated canonical names].
//! Natural check coverage and authored live-state fixture coverage are separate.
//! Reports contain only counters, labels and errors, never ROM-derived state.
use serde_json::{Value, json};
use std::{error::Error, fs, sync::Arc};
use tje_game::{Context, FunctionId, ids};
use tje_host::{Host, Mode, hooks::HookRegistry, input::InputScript};
use tje_mem::{ObjectState, Target, ram};

type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;

const OWNED_SOURCES: &[&str] = &[
    "src/cutscenes/sega_logo.c",
    "src/cutscenes/drifting_entities.c",
    "src/cutscenes/impact_and_planet_entities.c",
    "src/cutscenes/rocketship_pose.c",
    "src/cutscenes/large_player_parts.c",
    "src/cutscenes/intro_frame_entities.c",
    "src/cutscenes/flying_entities.c",
    "src/cutscenes/ending_player_release.c",
    "src/cutscenes/intro.c",
    "src/cutscenes/intro_part11.c",
];

fn entries() -> impl Iterator<Item = &'static tje_game::Translation> {
    let pending = tje_game::cutscenes::PENDING_TRANSLATIONS;
    tje_game::translations().chain(pending)
}

fn registry(revision: tje_mem::Revision, selected: &[FunctionId]) -> Result<HookRegistry> {
    let mut registry = HookRegistry::empty(revision);
    for &id in selected {
        let translation = entries()
            .find(|translation| translation.id == id)
            .ok_or_else(|| format!("untranslated selection: {}", id.metadata().name))?;
        let class = if tje_game::is_atomic_capable(id) {
            tje_game::execution::ExecutionClass::Atomic
        } else {
            tje_game::cutscenes::EXECUTION_CONTRACTS
                .iter()
                .find(|(candidate, _)| *candidate == id)
                .map(|(_, class)| *class)
                .ok_or("unclassified selection")?
        };
        registry.register_translation_with_execution(*translation, class)?;
    }
    Ok(registry)
}

fn check(
    host: &mut Host,
    id: FunctionId,
    arguments: &[u32],
    label: &str,
    outcomes: &mut Vec<Value>,
) {
    if !host.checks.by_function.contains_key(id.metadata().name) {
        return;
    }
    let result = host.check_fixture(id.metadata().address(host.revision()), arguments);
    let error = result.as_ref().err().map(ToString::to_string);
    println!(
        "{} {label}: {}",
        id.metadata().name,
        error.as_deref().unwrap_or("pass")
    );
    outcomes.push(json!({
        "name": id.metadata().name,
        "label": label,
        "arguments": arguments,
        "error": error,
    }));
}

fn original_setup(host: &mut Host, id: FunctionId, arguments: &[u32]) -> Result {
    Context::call_original(&mut host.context(), id, arguments)?;
    Ok(())
}

/// Synthetic interventions on a live gameplay machine: free the dynamic pool,
/// clear only its records and reservation maps, and clear scene selector banks.
/// No game data is embedded; constructors fetch all frames from the user's ROM.
fn clear_scene_pool(host: &mut Host) {
    let revision = host.revision();
    let mut context = host.context();
    for slot in 6..40 {
        let object = ram::g_player_info(revision).at(slot);
        for offset in 0..ObjectState::SIZE {
            context.write_u8(object.address().wrapping_add(offset), 0);
        }
    }
    for index in 0..34 {
        ram::g_dynamic_entity_slot_allocated(revision)
            .at(index)
            .write(&mut context, 0);
    }
    for index in 0..80 {
        ram::g_sprite_tile_group_allocated(revision)
            .at(index)
            .write(&mut context, 0);
    }
    for index in 0..54 {
        ram::g_viewport_sprite_slot_flags(revision)
            .at(index)
            .write(&mut context, 0);
    }
    for index in 0..8 {
        ram::g_intro_large_player_part_entities(revision)
            .at(index)
            .write(&mut context, 0);
        ram::g_cutscene_drifting_entities(revision)
            .at(index)
            .write(&mut context, 0);
    }
    for index in 0..3 {
        ram::g_intro_driving_part_entities(revision)
            .at(index)
            .write(&mut context, 0);
    }
    for index in 0..4 {
        ram::g_intro_flying_entities(revision)
            .at(index)
            .write(&mut context, 0);
    }
    for index in 0..10 {
        ram::g_intro_component_entities(revision)
            .at(index)
            .write(&mut context, 0);
    }
    ram::g_intro_asteroid_entity(revision).write(&mut context, 0);
    ram::g_intro_rocketship_impact_entity(revision).write(&mut context, 0);
    ram::g_cutscene_planet_entity(revision).write(&mut context, 0);
    ram::g_ending_stationary_composite_entity(revision).write(&mut context, 0);
    ram::g_ending_player1_entity(revision).write(&mut context, 0);
    ram::g_ending_player2_entity(revision).write(&mut context, 0);
    ram::g_cutscene_rocketship_entity(revision).write(&mut context, 0);
    ram::g_cutscene_rocketship_exhaust_entity(revision).write(&mut context, 0);
    ram::g_sega_trademark_entity(revision).write(&mut context, 0);
    ram::g_rng_state(revision).write(&mut context, 12345);
}

fn fixtures(host: &mut Host, outcomes: &mut Vec<Value>) -> Result {
    let revision = host.revision();
    check(
        host,
        ids::SEGA_LOGO_CLEAR_VIDEO_PLANES,
        &[],
        "live gameplay VDP; fixed clear transaction stream",
        outcomes,
    );
    for (id, arguments) in [
        (ids::CUTSCENE_CREATE_DRIFTING_ENTITIES, vec![]),
        (ids::CUTSCENE_CREATE_PLANET_ENTITY, vec![160, 180]),
        (
            ids::ENDING_CREATE_STATIONARY_COMPOSITE,
            vec![0xffff_8000, 0x1234_7fff],
        ),
        (ids::INTRO_ALLOCATE_DRIVING_PARTS, vec![]),
        (ids::INTRO_ALLOCATE_FLYING_ENTITY_SLOTS, vec![]),
        (ids::INTRO_ALLOCATE_LARGE_PLAYER_PARTS, vec![]),
        (ids::INTRO_CREATE_ASTEROID_ENTITY, vec![]),
        (ids::INTRO_CREATE_HIDDEN_FRAME_ENTITIES, vec![]),
        (
            ids::INTRO_CREATE_ROCKETSHIP_IMPACT,
            vec![0x1234_ffff, 0x5678_8000],
        ),
        (ids::SEGA_LOGO_CREATE_TRADEMARK_ENTITY, vec![]),
    ] {
        clear_scene_pool(host);
        check(
            host,
            id,
            &arguments,
            "live-state fixture: empty dynamic pool and reservation maps",
            outcomes,
        );
        for index in 0..34 {
            ram::g_dynamic_entity_slot_allocated(revision)
                .at(index)
                .write(&mut host.context(), 1);
        }
        check(
            host,
            id,
            &arguments,
            "live-state fixture: exhausted dynamic pool",
            outcomes,
        );
    }
    clear_scene_pool(host);
    original_setup(host, ids::INTRO_ALLOCATE_LARGE_PLAYER_PARTS, &[])?;
    for index in 0..9 {
        check(
            host,
            ids::INTRO_POSITION_LARGE_PLAYER_PART,
            &[index, 0x1234_7fff, 0xabcd_fffb],
            "live-state fixture: each selector, signed coordinates, upper guard",
            outcomes,
        );
    }
    original_setup(host, ids::INTRO_POSITION_LARGE_PLAYER_PARTS, &[])?;
    check(
        host,
        ids::INTRO_POSITION_LARGE_PLAYER_PARTS,
        &[],
        "live-state fixture: seven active allocated actor records",
        outcomes,
    );
    for index in 0..9 {
        check(
            host,
            ids::INTRO_RELEASE_LARGE_PLAYER_PART_SPRITES,
            &[index],
            "live-state fixture: active selectors and upper guard",
            outcomes,
        );
    }
    check(
        host,
        ids::INTRO_RELEASE_LARGE_PLAYER_SPRITES_AND_REFRESH,
        &[],
        "live-state fixture: active large actors and real IRQ waits",
        outcomes,
    );
    check(
        host,
        ids::INTRO_RELEASE_LARGE_PLAYER_PARTS,
        &[],
        "live-state fixture: eight allocated actor records",
        outcomes,
    );
    clear_scene_pool(host);
    original_setup(host, ids::INTRO_ALLOCATE_DRIVING_PARTS, &[])?;
    for index in 0..4 {
        check(
            host,
            ids::INTRO_POSITION_DRIVING_PART,
            &[index, 0x1234_8000, 0xffff_7fff],
            "live-state fixture: each driving selector, narrowing and upper guard",
            outcomes,
        );
    }
    for index in 0..3 {
        original_setup(host, ids::INTRO_POSITION_DRIVING_PART, &[index, 104, 151])?;
    }
    for index in 0..4 {
        check(
            host,
            ids::INTRO_RELEASE_DRIVING_PART_SPRITES,
            &[index],
            "live-state fixture: resident driving actors and upper guard",
            outcomes,
        );
    }
    check(
        host,
        ids::INTRO_RELEASE_DRIVING_PARTS,
        &[],
        "live-state fixture: three allocated driving actors",
        outcomes,
    );
    clear_scene_pool(host);
    original_setup(host, ids::CUTSCENE_ALLOCATE_ROCKETSHIP_PAIR, &[])?;
    for animation in 0..4 {
        check(
            host,
            ids::CUTSCENE_SET_ROCKETSHIP_PAIR_POSE,
            &[animation, 0x1234_8000, 0x5678_7fff],
            "live-state fixture: each rocket pose; nonresident sprites",
            outcomes,
        );
    }
    original_setup(host, ids::CUTSCENE_SET_ROCKETSHIP_PAIR_POSE, &[0, 104, 151])?;
    check(
        host,
        ids::CUTSCENE_SET_ROCKETSHIP_PAIR_POSE,
        &[3, 104, 151],
        "live-state fixture: release/reallocate both resident viewports",
        outcomes,
    );
    clear_scene_pool(host);
    original_setup(host, ids::CUTSCENE_CREATE_DRIFTING_ENTITIES, &[])?;
    check(
        host,
        ids::CUTSCENE_UPDATE_DRIFTING_ENTITIES,
        &[0xffff_ffff],
        "live-state fixture: natural allocated drifters",
        outcomes,
    );
    for index in 0..8 {
        let slot = ram::g_cutscene_drifting_entities(revision)
            .at(index)
            .read(&host.context());
        let object = ram::g_player_info(revision).at(i32::from(slot));
        let mut context = host.context();
        object.x().write(&mut context, 370);
        object.fraction_x().write(&mut context, 254);
        object
            .velocity_x()
            .write(&mut context, if index & 1 == 0 { 2 } else { -1 });
    }
    check(
        host,
        ids::CUTSCENE_UPDATE_DRIFTING_ENTITIES,
        &[0],
        "live-state fixture: wrap/reseed versus unsigned fraction-negative velocity",
        outcomes,
    );
    check(
        host,
        ids::CUTSCENE_RELEASE_DRIFTING_ENTITIES,
        &[],
        "live-state fixture: eight resident drifters",
        outcomes,
    );
    clear_scene_pool(host);
    original_setup(host, ids::INTRO_ALLOCATE_FLYING_ENTITY_SLOTS, &[])?;
    for scene in 0..6 {
        check(
            host,
            ids::INTRO_UPDATE_RIGHTWARD_FLYING_ENTITY,
            &[1, scene],
            "live-state fixture: restart each scene/default",
            outcomes,
        );
        check(
            host,
            ids::INTRO_UPDATE_FLYING_ENTITY_MOTION,
            &[scene],
            "live-state fixture: respawn each scene/default",
            outcomes,
        );
    }
    original_setup(host, ids::INTRO_UPDATE_RIGHTWARD_FLYING_ENTITY, &[1, 1])?;
    let first = ram::g_intro_flying_entities(revision)
        .at(0)
        .read(&host.context());
    ram::g_player_info(revision)
        .at(i32::from(first))
        .x()
        .write(&mut host.context(), 230);
    check(
        host,
        ids::INTRO_UPDATE_RIGHTWARD_FLYING_ENTITY,
        &[0, 1],
        "live-state fixture: right-edge sprite release without restart",
        outcomes,
    );
    for index in 0..4 {
        let slot = ram::g_intro_flying_entities(revision)
            .at(index)
            .read(&host.context());
        let object = ram::g_player_info(revision).at(i32::from(slot));
        let mut context = host.context();
        object.x().write(&mut context, 168);
        object.y().write(&mut context, 0);
        object.height().write(&mut context, -100);
        object.velocity_x().write(&mut context, -3);
        object.velocity_y().write(&mut context, 4);
        object.velocity_height().write(&mut context, -1);
    }
    check(
        host,
        ids::INTRO_UPDATE_FLYING_ENTITY_MOTION,
        &[2],
        "live-state fixture: in-bounds motion; delay reaches zero and allocates sprites",
        outcomes,
    );
    check(
        host,
        ids::INTRO_RELEASE_FLYING_ENTITY_SLOTS,
        &[],
        "live-state fixture: allocated flying selectors",
        outcomes,
    );
    for (create, release, arguments) in [
        (
            ids::INTRO_CREATE_ASTEROID_ENTITY,
            ids::INTRO_RELEASE_ASTEROID_ENTITY,
            vec![],
        ),
        (
            ids::INTRO_CREATE_ROCKETSHIP_IMPACT,
            ids::INTRO_RELEASE_ROCKETSHIP_IMPACT,
            vec![160, 180],
        ),
        (
            ids::CUTSCENE_CREATE_PLANET_ENTITY,
            ids::CUTSCENE_RELEASE_PLANET_ENTITY,
            vec![160, 180],
        ),
        (
            ids::ENDING_CREATE_STATIONARY_COMPOSITE,
            ids::ENDING_RELEASE_STATIONARY_COMPOSITE,
            vec![160, 180],
        ),
        (
            ids::SEGA_LOGO_CREATE_TRADEMARK_ENTITY,
            ids::SEGA_LOGO_RELEASE_TRADEMARK_ENTITY,
            vec![],
        ),
    ] {
        clear_scene_pool(host);
        check(
            host,
            release,
            &[],
            "live-state fixture: null selector",
            outcomes,
        );
        original_setup(host, create, &arguments)?;
        check(
            host,
            release,
            &[],
            "live-state fixture: allocated resident object",
            outcomes,
        );
    }
    clear_scene_pool(host);
    check(
        host,
        ids::ENDING_RELEASE_ACTIVE_PLAYER_ENTITIES,
        &[],
        "live-state fixture: both handles zero",
        outcomes,
    );
    original_setup(host, ids::INTRO_ALLOCATE_DRIVING_PARTS, &[])?;
    let first = ram::g_intro_driving_part_entities(revision)
        .at(0)
        .read(&host.context());
    let second = ram::g_intro_driving_part_entities(revision)
        .at(1)
        .read(&host.context());
    ram::g_ending_player1_entity(revision).write(&mut host.context(), first);
    ram::g_ending_player2_entity(revision).write(&mut host.context(), second);
    check(
        host,
        ids::ENDING_RELEASE_ACTIVE_PLAYER_ENTITIES,
        &[],
        "live-state fixture: both active handles",
        outcomes,
    );
    clear_scene_pool(host);
    check(
        host,
        ids::INTRO_BEGIN_LARGE_PLAYER_SCENE,
        &[7],
        "live-state fixture: empty actor pool, intro background seven, real IRQ service",
        outcomes,
    );
    for id in [
        ids::INTRO_PART5,
        ids::INTRO_PART6,
        ids::INTRO_PART7,
        ids::INTRO_PART8,
        ids::INTRO_PART9,
        ids::INTRO_PART1,
        ids::INTRO_PART10,
        ids::INTRO_PART11,
        ids::INTRO_PART2,
        ids::INTRO_PART3,
        ids::INTRO_PART4,
        ids::INTRO_VIDEO,
    ] {
        clear_scene_pool(host);
        check(
            host,
            id,
            &[],
            "live-state fixture: initialized pool; unheld controller timeline; complete scene",
            outcomes,
        );
    }
    clear_scene_pool(host);
    ram::g_sega_rocket_pass_shown(revision).write(&mut host.context(), 1);
    check(
        host,
        ids::SEGA_VIDEO,
        &[],
        "live-state fixture: palette cycle, rocket pass already shown",
        outcomes,
    );
    ram::g_sega_rocket_pass_shown(revision).write(&mut host.context(), 0);
    check(
        host,
        ids::SEGA_VIDEO,
        &[],
        "live-state fixture: palette cycle and first rocket pass",
        outcomes,
    );
    let frame = host.frame_count();
    let abort_timeline = Arc::new(InputScript::parse(&format!(
        r#"{{"holds": [
            {{"start": {frame}, "end": {}, "p1": ["start"]}},
            {{"start": {}, "end": {}, "p1": ["start"]}}
        ]}}"#,
        frame + 30,
        frame + 60,
        frame + 200
    ))?);
    host.set_wait_input_poller(abort_timeline);
    for id in [
        ids::INTRO_PART5,
        ids::INTRO_PART6,
        ids::INTRO_PART7,
        ids::INTRO_PART8,
        ids::INTRO_PART9,
        ids::INTRO_PART1,
        ids::INTRO_PART10,
        ids::INTRO_PART11,
        ids::INTRO_PART2,
        ids::INTRO_PART3,
        ids::INTRO_PART4,
        ids::INTRO_VIDEO,
    ] {
        clear_scene_pool(host);
        check(
            host,
            id,
            &[],
            "live-state fixture: held/released Start timeline; early scene interruption",
            outcomes,
        );
    }
    Ok(())
}

fn main() -> Result {
    let arguments: Vec<_> = std::env::args().collect();
    if arguments.len() < 5 {
        return Err("usage: ROM INPUT_JSON METRICS_JSON FRAMES [NAMES]".into());
    }
    let bytes = fs::read(&arguments[1])?;
    let revision = tje_mem::Revision::Rev02;
    let revision = if arguments[1].contains("rev00") {
        tje_mem::Revision::Rev00
    } else {
        revision
    };
    let input_json = fs::read_to_string(&arguments[2])?;
    let script = Arc::new(InputScript::parse(&input_json)?);
    let frames: u64 = arguments[4].parse()?;
    let names = arguments.get(5).map(String::as_str).unwrap_or("");
    let selected: Vec<_> = entries()
        .filter(|translation| {
            let metadata = translation.id.metadata();
            OWNED_SOURCES.contains(&metadata.source)
                && metadata.wave > 0
                && (names.is_empty()
                    || (names == "atomic" && tje_game::is_atomic_capable(translation.id))
                    || names
                        .split(',')
                        .any(|name| name == translation.id.metadata().name))
        })
        .map(|translation| translation.id)
        .collect();
    let mut natural = Host::new(&bytes, Mode::Check, registry(revision, &selected)?)?;
    natural.set_wait_input_poller(script.clone());
    let mut natural_error = None;
    for frame in 0..frames {
        if let Err(error) = natural.run_frame(script.at(frame)) {
            natural_error = Some(error.to_string());
            break;
        }
    }
    let mut live = Host::new(&bytes, Mode::Reference, registry(revision, &selected)?)?;
    live.set_wait_input_poller(script.clone());
    for frame in 0..frames {
        live.run_frame(script.at(frame))?;
    }
    let gameplay = live.measurements.game_frame;
    let mut outcomes = Vec::new();
    let fixture_error = fixtures(&mut live, &mut outcomes)
        .err()
        .map(|error| error.to_string());
    let native_missing: Vec<_> = tje_game::Dispatcher::native(&selected)
        .err()
        .unwrap_or_default()
        .iter()
        .map(|id| id.metadata().name)
        .collect();
    let report = json!({
        "revision": format!("{revision:?}"),
        "requested_frames": frames,
        "natural_frames": natural.frame_count(),
        "natural_error": natural_error,
        "natural": natural.checks,
        "live_state_frames": live.frame_count(),
        "live_state_gameplay": gameplay,
        "fixture_error": fixture_error,
        "fixtures": live.checks,
        "outcomes": outcomes,
        "native_missing": native_missing,
    });
    fs::write(&arguments[3], serde_json::to_string_pretty(&report)?)?;
    if report["natural_error"].is_null()
        && report["fixture_error"].is_null()
        && outcomes.iter().all(|outcome| outcome["error"].is_null())
    {
        Ok(())
    } else {
        Err("proof failures retained in metrics".into())
    }
}
