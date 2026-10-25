//! Opt-in external host harness, never linked into tje-game. A private manifest
//! names this file as a bin and depends on tje-game, tje-host, tje-mem, serde_json.
//! Arguments: ROM INPUT_JSON METRICS_JSON FRAMES [comma-separated canonical names].
//! Natural check coverage and authored live-state fixture coverage are separate.
//! Reports contain only counters, labels and errors, never ROM-derived state.
use serde_json::{Value, json};
use std::{
    error::Error,
    fs,
    sync::{Arc, OnceLock},
};
use tje_game::{Context, FunctionId, ids};
use tje_host::{Host, Mode, hooks::HookRegistry, input::InputScript};
use tje_mem::{ObjectState, Target, ram};

type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;

static PROOF_ROOTS: OnceLock<Vec<tje_game::Translation>> = OnceLock::new();
static FRAMED_PROOF_ROOTS: OnceLock<Vec<tje_game::FramedTranslation>> = OnceLock::new();

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

fn framed_entries() -> impl Iterator<Item = &'static tje_game::FramedTranslation> {
    tje_game::framed_translations().chain(tje_game::cutscenes::PENDING_FRAMED_TRANSLATIONS)
}

fn registry(revision: tje_mem::Revision, selected: &[FunctionId]) -> Result<HookRegistry> {
    let mut roots = Vec::with_capacity(selected.len());
    let mut framed = Vec::with_capacity(selected.len());
    for &id in selected {
        if let Some(translation) = entries().find(|translation| translation.id == id) {
            roots.push(*translation);
        } else if let Some(translation) = framed_entries().find(|translation| translation.id == id)
        {
            framed.push(*translation);
        } else {
            return Err(format!("untranslated selection: {}", id.metadata().name).into());
        }
    }
    // The proof API requires static entry pointers. This command-line process
    // creates one bounded root table, shared by its two hosts, never per call.
    let roots = PROOF_ROOTS.get_or_init(|| roots);
    let framed = FRAMED_PROOF_ROOTS.get_or_init(|| framed);
    Ok(HookRegistry::proof_with_frames(revision, roots, framed)?)
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
    let entry_pc = host.cpu().pc();
    let entry_sr = host.cpu().status_register();
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
        "entry_pc": entry_pc,
        "entry_sr": entry_sr,
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

fn shared_tile_allocator_failure(host: &mut Host, slots: &[u8]) {
    let revision = host.revision();
    let mut context = host.context();
    for index in 0..54 {
        ram::g_viewport_sprite_slot_flags(revision)
            .at(index)
            .write(&mut context, 1);
    }
    for index in 0..80 {
        ram::g_sprite_tile_group_allocated(revision)
            .at(index)
            .write(&mut context, 1);
    }
    for &slot in slots {
        let object = ram::g_player_info(revision).at(i32::from(slot));
        object.draw_flags().write(&mut context, 1);
        object.behavior_flags().write(&mut context, 4);
    }
    Context::set_data_register(&mut context, 4, 0xface_05ff);
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
    // Preserve the released current-frame sample supplied while settling.
    // The first held sample is explicitly on the next crossed frame.
    let first_press = frame + 1;
    let abort_timeline = Arc::new(InputScript::parse(&format!(
        r#"{{"holds": [
            {{"start": {first_press}, "end": {}, "p1": ["start"]}},
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
    clear_scene_pool(host);
    original_setup(host, ids::INTRO_ALLOCATE_LARGE_PLAYER_PARTS, &[])?;
    for index in [0, 3] {
        let slot = ram::g_intro_large_player_part_entities(revision)
            .at(index)
            .read(&host.context());
        shared_tile_allocator_failure(host, &[slot]);
        check(
            host,
            ids::INTRO_POSITION_LARGE_PLAYER_PART,
            &[index as u32, 100, 672],
            "live-state fixture: shared tiles, no resident view, exhausted slots; D4 is unbiased Y",
            outcomes,
        );
    }
    let slots: Vec<_> = (0..8)
        .map(|index| {
            ram::g_intro_large_player_part_entities(revision)
                .at(index)
                .read(&host.context())
        })
        .collect();
    shared_tile_allocator_failure(host, &slots);
    check(
        host,
        ids::INTRO_POSITION_LARGE_PLAYER_PARTS,
        &[],
        "live-state fixture: seven shared-tile selector failures compose through Y-residue positioners",
        outcomes,
    );
    clear_scene_pool(host);
    original_setup(host, ids::INTRO_ALLOCATE_DRIVING_PARTS, &[])?;
    for index in 0..3 {
        let slot = ram::g_intro_driving_part_entities(revision)
            .at(index)
            .read(&host.context());
        shared_tile_allocator_failure(host, &[slot]);
        check(
            host,
            ids::INTRO_POSITION_DRIVING_PART,
            &[index as u32, 100, 672],
            "live-state fixture: shared tiles, no resident view, exhausted slots; D4 is pre-bias Y",
            outcomes,
        );
    }
    clear_scene_pool(host);
    original_setup(host, ids::CUTSCENE_ALLOCATE_ROCKETSHIP_PAIR, &[])?;
    let ship = ram::g_cutscene_rocketship_entity(revision).read(&host.context());
    let exhaust = ram::g_cutscene_rocketship_exhaust_entity(revision).read(&host.context());
    shared_tile_allocator_failure(host, &[ship, exhaust]);
    check(
        host,
        ids::CUTSCENE_SET_ROCKETSHIP_PAIR_POSE,
        &[0, 100, 672],
        "live-state fixture: both shared-tile allocations fail; caller D4.low is Y",
        outcomes,
    );
    clear_scene_pool(host);
    original_setup(host, ids::INTRO_ALLOCATE_FLYING_ENTITY_SLOTS, &[])?;
    let slots: Vec<_> = (0..4)
        .map(|index| {
            ram::g_intro_flying_entities(revision)
                .at(index)
                .read(&host.context())
        })
        .collect();
    for scene in [2, 3, 4] {
        shared_tile_allocator_failure(host, &slots);
        for &slot in &slots {
            let object = ram::g_player_info(revision).at(i32::from(slot));
            let mut context = host.context();
            object.x().write(&mut context, 168);
            object.height().write(&mut context, -100);
            object.velocity_x().write(&mut context, 0);
            object.velocity_y().write(&mut context, 0);
            object.velocity_height().write(&mut context, -1);
        }
        check(
            host,
            ids::INTRO_UPDATE_FLYING_ENTITY_MOTION,
            &[scene],
            "live-state fixture: shared-tile failure when delay reaches zero; D4 is MOVEQ/CLR.W drift",
            outcomes,
        );
    }
    shared_tile_allocator_failure(host, &slots[..1]);
    Context::set_data_register(&mut host.context(), 4, 0xface_02a0);
    check(
        host,
        ids::INTRO_UPDATE_RIGHTWARD_FLYING_ENTITY,
        &[1, 1],
        "live-state fixture: shared-tile failure retains unchanged nonzero entry D4",
        outcomes,
    );
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
        .map(|translation| translation.id)
        .chain(framed_entries().map(|translation| translation.id))
        .filter(|id| {
            let metadata = id.metadata();
            OWNED_SOURCES.contains(&metadata.source)
                && metadata.wave > 0
                && (names.is_empty()
                    || (names == "atomic" && tje_game::is_atomic_capable(*id))
                    || (names == "time-aware"
                        && tje_game::execution_class(*id)
                            == Some(tje_game::execution::ExecutionClass::TimeAware))
                    || names.split(',').any(|name| name == id.metadata().name))
        })
        .collect();
    let registry = registry(revision, &selected)?;
    let mut natural = Host::new(&bytes, Mode::Check, registry.clone())?;
    natural.set_wait_input_poller(script.clone());
    let mut natural_error = None;
    for frame in 0..frames {
        if let Err(error) = natural.run_frame(script.at(frame)) {
            natural_error = Some(error.to_string());
            break;
        }
    }
    let mut live = Host::new(&bytes, Mode::Reference, registry)?;
    live.set_wait_input_poller(script.clone());
    for frame in 0..frames {
        live.run_frame(script.at(frame))?;
    }
    // Settle original work before any synthetic fixture writes or argument slots.
    // Keep the complete returned snapshots, including audio/device observations,
    // in this owned advance through reporting; publish only bounded metadata.
    let settle_inputs = script.at(live.frame_count());
    let settled = live.settle_fixture_boundary(settle_inputs)?;
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
        "fixture_boundary": {
            "start_pc": settled.start_pc,
            "end_pc": settled.end_pc,
            "start_frame": settled.start_frame,
            "end_frame": settled.end_frame,
            "ticks": settled.ticks,
            "master_clocks": settled.master_clocks,
            "handlers_completed": settled.handlers_completed,
            "completed_frames": settled.completed_frames.iter().map(|frame| json!({
                "frame": frame.frame,
                "frame_counter": frame.frame_counter,
                "audio_samples": frame.audio_samples.len(),
                "writes": frame.writes.len(),
                "idle_random_draws": frame.idle_random_draws,
                "lag_events": frame.lag_events,
            })).collect::<Vec<_>>(),
        },
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
