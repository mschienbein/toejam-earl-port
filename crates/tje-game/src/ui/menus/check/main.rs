//! Private ROM differential runner. Usage: ROM_DIRECTORY EVIDENCE_DIRECTORY
//! [comma-separated function names] [comma-separated recording suffixes].
//! Fixtures are explicitly labelled RAM recipes on live state, never ROM blobs.
use genesis_config::{GenesisButton, GenesisController, GenesisControllerType, GenesisInputs};
use std::{
    error::Error,
    fs,
    path::Path,
    sync::{Arc, OnceLock},
};
use tje_game::{
    FramedTranslation, FunctionId, Translation, ids,
    ui::{self, menus},
};
use tje_host::{Host, Mode, execution::WaitInputPoller, hooks::HookRegistry};
use tje_mem::{Bus, Revision, ram, rom};
use tje_session::{CONNECTED, Session};

type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;

fn inputs(pads: [u16; 2]) -> GenesisInputs {
    use GenesisButton::*;
    let pads = pads.map(|pad| {
        let mut controller = if pad & CONNECTED != 0 {
            GenesisController::new(GenesisControllerType::ThreeButton)
        } else {
            GenesisController::None
        };
        for (index, button) in [Up, Down, Left, Right, A, B, C, Start]
            .into_iter()
            .enumerate()
        {
            controller.set_field(button, pad & (1 << index) != 0);
        }
        controller
    });
    GenesisInputs {
        p1: pads[0],
        p2: pads[1],
    }
}

struct Recording {
    session: Session,
    pads: Vec<[u16; 2]>,
}
impl Recording {
    fn load(path: &Path) -> Result<Self> {
        let session = Session::from_json(&fs::read_to_string(path)?)?;
        let pads = session.inputs().collect();
        Ok(Self { session, pads })
    }
}
impl WaitInputPoller for Recording {
    fn at(&self, frame: u64) -> tje_host::Result<GenesisInputs> {
        let pads = self
            .pads
            .get(frame as usize)
            .ok_or_else(|| tje_host::HostError::Input(format!("recording has no frame {frame}")))?;
        Ok(inputs(*pads))
    }
    fn before_frame(&self, frame: u64, memory: &mut dyn Bus) -> tje_host::Result<()> {
        for write in self
            .session
            .writes()
            .iter()
            .filter(|write| write.before_frame == frame)
        {
            match write.width {
                1 => memory.write_u8(write.address, write.value as u8),
                2 => memory.write_u16(write.address, write.value as u16),
                4 => memory.write_u32(write.address, write.value),
                _ => unreachable!("validated recipe width"),
            }
        }
        Ok(())
    }
}

fn save(path: &Path, value: &serde_json::Value) -> Result {
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

fn menu_ids() -> impl Iterator<Item = FunctionId> {
    menus::PENDING_IDS
        .iter()
        .copied()
        .chain(menus::PROVEN_ENTRIES.iter().map(|entry| entry.id))
}

fn registry(revision: Revision, names: &[String]) -> Result<HookRegistry> {
    static PROOF: OnceLock<Vec<Translation>> = OnceLock::new();
    static FRAMED: OnceLock<Vec<FramedTranslation>> = OnceLock::new();
    let selected = |id: FunctionId| names.iter().any(|name| name == id.metadata().name);
    let pending = PROOF.get_or_init(|| {
        ui::PENDING_TRANSLATIONS
            .iter()
            .chain(ui::TRANSLATIONS)
            .filter(|entry| selected(entry.id))
            .copied()
            .collect()
    });
    // The root literal table also includes newly framed resource candidates,
    // without binding this harness to a sibling's owner-manifest alias.
    let framed = FRAMED.get_or_init(|| {
        ui::PENDING_FRAMED_TRANSLATIONS
            .iter()
            .filter(|entry| selected(entry.id))
            .copied()
            .collect()
    });
    Ok(HookRegistry::proof_with_frames(revision, pending, framed)?)
}

fn session(
    bytes: &[u8],
    revision: Revision,
    name: &str,
    source: Arc<Recording>,
    registry: HookRegistry,
    output: &Path,
) -> Result<Host> {
    let mut checked = Host::new(bytes, Mode::Check, registry.clone())?;
    let mut reference = Host::new(bytes, Mode::Reference, registry)?;
    checked.set_wait_input_poller(source.clone());
    reference.set_wait_input_poller(source.clone());
    let mut error = None;
    for frame in 0..source.session.frames {
        source.before_frame(frame, &mut checked.context())?;
        source.before_frame(frame, &mut reference.context())?;
        let input = source.at(frame)?;
        let a = match checked.run_frame(input) {
            Ok(snapshot) => snapshot,
            Err(failure) => {
                error = Some(failure.to_string());
                break;
            }
        };
        let b = reference.run_frame(input)?;
        if a.ram != b.ram
            || a.vdp != b.vdp
            || a.writes != b.writes
            || a.frame_counter != b.frame_counter
            || a.idle_random_draws != b.idle_random_draws
            || a.lag_events != b.lag_events
            || !a
                .audio_samples
                .iter()
                .flatten()
                .map(|sample| sample.to_bits())
                .eq(b
                    .audio_samples
                    .iter()
                    .flatten()
                    .map(|sample| sample.to_bits()))
        {
            error = Some(format!("whole-session state differs at frame {frame}"));
            break;
        }
        let (pixels_a, size_a) = checked.frame_buffer();
        let (pixels_b, size_b) = reference.frame_buffer();
        if size_a != size_b
            || pixels_a[..size_a.len() as usize] != pixels_b[..size_b.len() as usize]
        {
            error = Some(format!("whole-session pixels differ at frame {frame}"));
            break;
        }
    }
    save(
        &output.join(format!("{}-{name}.json", revision.name())),
        &serde_json::json!({
            "recording": name, "revision": revision.name(), "frames": checked.frame_count(),
            "whole_session_reference_equal": error.is_none(), "error": error,
            "checks": checked.checks, "measurements": checked.measurements,
        }),
    )?;
    if let Some(error) = error {
        return Err(error.into());
    }
    println!(
        "{} {name}: {} checked calls",
        revision.name(),
        checked.checks.calls
    );
    Ok(checked)
}

struct Fixtures<'a> {
    host: &'a mut Host,
    names: &'a [String],
    observations: Vec<serde_json::Value>,
}
impl Fixtures<'_> {
    fn check(&mut self, label: &str, id: FunctionId, arguments: &[u32]) {
        if !self.names.iter().any(|name| name == id.metadata().name) {
            return;
        }
        let result = self
            .host
            .check_fixture(id.metadata().address(self.host.revision()), arguments);
        self.observations.push(serde_json::json!({
            "label": label, "function": id.metadata().name, "arguments": arguments,
            "passed": result.is_ok(), "error": result.err().map(|error| error.to_string()),
        }));
    }
}

fn fixtures(host: &mut Host, names: &[String], output: &Path) -> Result<usize> {
    let revision = host.revision();
    let mut f = Fixtures {
        host,
        names,
        observations: Vec::new(),
    };
    // A fresh conversion uses genuine live RAM/frame state and the original local buffer.
    for value in [
        i16::MIN,
        -10000,
        -1000,
        -100,
        -10,
        -1,
        0,
        1,
        9,
        10,
        99,
        100,
        999,
        1000,
        9999,
        10000,
        i16::MAX,
    ] {
        f.check(
            "decimal signed boundary and leading-zero suppression",
            ids::TEXT_WRITE_DECIMAL_TILES,
            &[i32::from(value) as u32],
        );
    }
    for (items, count) in [
        (
            rom::menu_who_items(revision).address(),
            rom::menu_who_item_count(revision).read(&f.host.context()),
        ),
        (
            rom::menu_what_items(revision).address(),
            rom::menu_what_item_count(revision).read(&f.host.context()),
        ),
        (
            rom::menu_controller_options_items(revision).address(),
            rom::menu_controller_options_item_count(revision).read(&f.host.context()),
        ),
        (
            rom::menu_jam_out_items(revision).address(),
            rom::menu_jam_out_item_count(revision).read(&f.host.context()),
        ),
    ] {
        f.check(
            "all retail menu records",
            ids::MENU_DRAW_TEXT_ITEMS,
            &[items, i32::from(count) as u32],
        );
        f.check(
            "negative menu item count",
            ids::MENU_DRAW_TEXT_ITEMS,
            &[items, u32::MAX],
        );
    }
    f.check(
        "live cursor allocation",
        ids::CREATE_MENU_CURSOR_ENTITY,
        &[],
    );
    for header in [i16::MIN, -1, 0, 1, 2, 3, i16::MAX] {
        f.check(
            "word-wrapped header animation displacement",
            ids::ADD_MENU_HEADER,
            &[i32::from(header) as u32],
        );
    }
    for handle in [0, 6, 25] {
        ram::g_menu_cursor_entity(revision).write(&mut f.host.context(), handle);
        f.check(
            "cursor zero/live handle release",
            ids::RELEASE_MENU_CURSOR_ENTITY,
            &[],
        );
        ram::g_menu_header_entity(revision).write(&mut f.host.context(), handle);
        f.check(
            "header zero/live handle release",
            ids::REMOVE_MENU_HEADER,
            &[],
        );
    }
    for player in 0..2 {
        for seed in [0, 1, 0x8000, u16::MAX] {
            ram::g_level_seeds(revision)
                .at(1)
                .write(&mut f.host.context(), seed);
            f.check(
                "mail-order seeded exclusions, uniqueness and prices",
                ids::SET_MAIL_ORDER_PRESENTS,
                &[player, 1],
            );
        }
        for level in [i16::MIN, -1, i16::MAX] {
            f.check(
                "mail-order word-wrapped seed displacement",
                ids::SET_MAIL_ORDER_PRESENTS,
                &[player, i32::from(level) as u32],
            );
        }
        for (present, bucks, identified) in [
            (-1, 3, 0),
            (0, 1, 0),
            (0, 2, 0),
            (0, 3, 1),
            (26, 3, 0),
            (27, 127, 0),
        ] {
            let mut context = f.host.context();
            ram::g_player_inventory_scroll_row(revision)
                .at(player as i32)
                .write(&mut context, 0);
            ram::g_player_inventory_selection_row(revision)
                .at(player as i32)
                .write(&mut context, 0);
            ram::g_player_inventory_selection_column(revision)
                .at(player as i32)
                .write(&mut context, 0);
            ram::g_player_inventory(revision)
                .at(player as i32)
                .at(0)
                .write(&mut context, present);
            ram::g_player_bucks(revision)
                .at(player as i32)
                .write(&mut context, bucks);
            if present >= 0 {
                ram::g_present_info(revision)
                    .at(i32::from(present))
                    .identified()
                    .write(&mut context, identified);
            }
            f.check(
                "identification affordability, mystery, unchanged and charge paths",
                ids::INVENTORY_IDENTIFY_SELECTED_PRESENT,
                &[player],
            );
        }
        for (elevator, occupied) in [(1, 16), (0, 0), (0, 1), (0, 8), (0, 16)] {
            let mut context = f.host.context();
            ram::g_player_elevator_flags(revision)
                .at(player as i32)
                .write(&mut context, elevator);
            for slot in 0..16 {
                ram::g_player_inventory(revision)
                    .at(player as i32)
                    .at(slot)
                    .write(&mut context, if slot < occupied { 0 } else { -1 });
            }
            f.check(
                "drop elevator rejection and inventory compaction",
                ids::INVENTORY_DROP_SELECTED_PRESENT,
                &[player],
            );
        }
        for (lives, rank, score) in [(-1, 0, 65535), (0, 8, 65535), (0, 0, 0), (0, 0, 39)] {
            let mut context = f.host.context();
            ram::g_player_lives(revision)
                .at(player as i32)
                .write(&mut context, lives);
            ram::g_player_rank(revision)
                .at(player as i32)
                .write(&mut context, rank);
            ram::g_player_points(revision)
                .at(player as i32)
                .write(&mut context, score);
            f.check(
                "promotion absent, rank-cap and below-threshold paths",
                ids::CHECK_PROMOTION_SCORE,
                &[player],
            );
        }
        for (state, group, displayed, health, saved) in [
            (5, 0, 0, 0, -1),
            (6, 0, 0, 0, -1),
            (0, 0x20000, 0, 0, -1),
            (3, 0, 1, 8, -1),
            (7, 0, 1, 8, 9),
            (0, 0x800, 1, 8, -1),
            (0, 0, 1, 8, 9),
            (2, 0, 1, 8, -1),
            (0, 0, 1, 8, -1),
        ] {
            let mut context = f.host.context();
            let object = ram::g_player_info(revision).at(player as i32);
            object.entity_index().write(&mut context, player as i8);
            object.sprite_state().write(&mut context, state);
            object.animation_group_mask().write(&mut context, group);
            object.draw_flags().write(&mut context, 0x81);
            ram::g_player_promotion_display_entities(revision)
                .at(player as i32)
                .write(&mut context, 0);
            ram::g_player_rank(revision)
                .at(player as i32)
                .write(&mut context, 8);
            ram::g_hud_displayed_health(revision)
                .at(player as i32)
                .write(&mut context, displayed);
            ram::g_player_health(revision)
                .at(player as i32)
                .write(&mut context, health);
            ram::g_player_health_before_swimming(revision)
                .at(player as i32)
                .write(&mut context, saved);
            ram::g_frame_counter(revision).write(&mut context, 0);
            let address = object.address();
            f.check(
                "health reaction, swim, restoration and draw-late branches",
                ids::PLAYER_UPDATE_HEALTH_AND_LIVES,
                &[address, 0],
            );
        }
        for bits in 0..=255_u8 {
            let mut context = f.host.context();
            let object = ram::g_player_info(revision).at(player as i32);
            object.entity_index().write(&mut context, player as i8);
            object.sprite_state().write(&mut context, 0);
            object.animation_group_mask().write(&mut context, 0);
            ram::g_player_inventory_open(revision)
                .at(player as i32)
                .write(&mut context, 0);
            ram::g_player_powerups(revision)
                .at(player as i32)
                .write(&mut context, -1);
            ram::g_player_lives(revision)
                .at(player as i32)
                .write(&mut context, bits as i8);
            ram::g_hud_displayed_health(revision)
                .at(player as i32)
                .write(&mut context, 0);
            ram::g_player_health(revision)
                .at(player as i32)
                .write(&mut context, 0);
            ram::g_player_health_before_swimming(revision)
                .at(player as i32)
                .write(&mut context, 42);
            let address = object.address();
            f.check(
                "death SUBQ.B signed overflow: every stored life byte",
                ids::PLAYER_UPDATE_HEALTH_AND_LIVES,
                &[address, 0],
            );
        }
        for selection in [0, 1, 2] {
            for score in [0, 39, 40, 99, 100, 179, 879] {
                let mut context = f.host.context();
                ram::g_v_blank_graphics_enabled(revision).write(&mut context, 0);
                ram::g_sprite_graphics_upload_pending(revision).write(&mut context, 0);
                ram::g_sprite_tile_buffer_offset(revision).write(&mut context, 0);
                ram::g_player_selection(revision).write(&mut context, selection);
                ram::g_player_points(revision)
                    .at(player as i32)
                    .write(&mut context, score);
                f.check(
                    "graphics-disabled resource reset below original endless-loop threshold",
                    ids::PLAYER_RESET_RANK_AND_RESOURCES,
                    &[player],
                );
            }
        }
    }
    for pattern in 0..3 {
        let mut context = f.host.context();
        for player in 0..2 {
            for slot in 0..16 {
                let present = if pattern == 0 {
                    -1
                } else if pattern == 1 {
                    27
                } else {
                    ((slot + player) % 28) as i8
                };
                ram::g_player_inventory(revision)
                    .at(player)
                    .at(slot)
                    .write(&mut context, present);
            }
        }
        f.check(
            "randomization empty, protected and mixed inventories",
            ids::RANDOMIZE_INVENTORY,
            &[],
        );
    }
    for (first_lives, second_lives) in [(-1, -1), (0, -1), (-1, 0), (0, 0)] {
        let mut context = f.host.context();
        ram::g_v_blank_graphics_enabled(revision).write(&mut context, 0);
        ram::g_sprite_graphics_upload_pending(revision).write(&mut context, 0);
        ram::g_sprite_tile_buffer_offset(revision).write(&mut context, 0);
        ram::g_player_rank(revision).at(0).write(&mut context, 0);
        ram::g_player_rank(revision).at(1).write(&mut context, 0);
        ram::g_player_lives(revision)
            .at(0)
            .write(&mut context, first_lives);
        ram::g_player_lives(revision)
            .at(1)
            .write(&mut context, second_lives);
        f.check(
            "setup retains absent-player resources and initializes every HUD",
            ids::SETUP_THE_MAIN_GAME,
            &[],
        );
    }
    f.check(
        "transient entity cleanup on live gameplay handles",
        ids::CLEAR_TRANSIENT_ENTITIES,
        &[],
    );
    let mut handles = Vec::new();
    for (base, count) in [
        (ram::g_shared_projectile_entities(revision).at(0), 12),
        (ram::g_control_remap_projectile_entities(revision).at(0), 4),
        (ram::g_mole_effect_entities(revision).at(0), 6),
        (ram::g_speech_bubble_entities(revision).at(0), 4),
    ] {
        handles.extend((0..count).map(|index| base.offset(index)));
    }
    for player in 0..2 {
        handles.extend([
            ram::g_player_active_terrain_effect_entities(revision).at(player),
            ram::g_player_one_shot_terrain_effect_entities(revision).at(player),
            ram::g_player_transient_effect_entities(revision).at(player),
            ram::g_player_decoy_entities(revision).at(player),
            ram::g_player_promotion_display_entities(revision).at(player),
            ram::g_player_doorway_entities(revision).at(player),
            ram::g_player_rain_cloud_states(revision).at(player),
            ram::g_timed_effect_entities(revision).at(player),
            ram::g_player_control_remap_entities(revision).at(player),
            ram::g_player_wake_effect_entity_handles(revision).at(player),
            ram::g_player_boom_box_entities(revision).at(player),
            ram::g_player_interaction_choice_entities(revision).at(player),
        ]);
    }
    for &occupied in &handles {
        let mut context = f.host.context();
        for &handle in &handles {
            handle.write(&mut context, 0);
        }
        occupied.write(&mut context, 6);
        ram::g_dynamic_entity_slot_allocated(revision)
            .at(0)
            .write(&mut context, 1);
        let object = ram::g_player_info(revision).at(6);
        object.draw_flags().write(&mut context, 1);
        object.sprite_count().write(&mut context, 0);
        object.tile_count().write(&mut context, 0);
        ram::g_life_request_pending(revision).write(&mut context, 1);
        for player in 0..2 {
            ram::g_player_interaction_choice_active(revision)
                .at(player)
                .write(&mut context, 1);
            ram::g_player_interaction_responses(revision)
                .at(player)
                .write(&mut context, 1);
            ram::g_player_tomato_rain_ticks(revision)
                .at(player)
                .write(&mut context, 7);
        }
        f.check(
            "each transient handle family occupied, including conditional state clears",
            ids::CLEAR_TRANSIENT_ENTITIES,
            &[],
        );
    }
    for handle in handles {
        handle.write(&mut f.host.context(), 0);
    }
    ram::g_life_request_display_entity(revision).write(&mut f.host.context(), 6);
    f.check(
        "occupied word-sized life-request display handle and pending flag",
        ids::CLEAR_TRANSIENT_ENTITIES,
        &[],
    );
    let failures = f
        .observations
        .iter()
        .filter(|observation| observation["passed"] == false)
        .count();
    save(
        &output.join(format!("{}-live-fixtures.json", revision.name())),
        &serde_json::json!({
            "revision": revision.name(), "fixture_results": f.observations, "failures": failures,
            "checks": f.host.checks,
        }),
    )?;
    println!("{} live fixtures: {failures} failed", revision.name());
    Ok(failures)
}

fn main() -> Result {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    let roms = Path::new(arguments.first().ok_or("missing ROM directory")?);
    let output = Path::new(arguments.get(1).ok_or("missing evidence directory")?);
    fs::create_dir_all(output)?;
    let names: Vec<String> = arguments.get(2).map_or_else(
        || {
            menu_ids()
                .filter(|&id| {
                    !matches!(
                        id,
                        ids::NEW_GAME_MAIN_MENU
                            | ids::SHOW_GAME_OVER_AND_RESET
                            | ids::MENU_WHO_HANDLER
                            | ids::MENU_WHAT_HANDLER
                            | ids::MENU_CONTROLLER_OPTIONS_HANDLER
                            | ids::MENU_JAM_OUT_HANDLER
                            | ids::GENERAL_MENU_HANDLER
                            | ids::RUN_FINAL_LEVEL_PAUSE_MENU
                            | ids::GAME_PAUSED
                    )
                })
                .map(|id| id.metadata().name.to_owned())
                .collect()
        },
        |names| names.split(',').map(str::to_owned).collect(),
    );
    for name in &names {
        if !menu_ids().any(|id| id.metadata().name == name) {
            return Err(format!("unknown menu translation {name}").into());
        }
    }
    let recordings: Vec<_> = arguments.get(3).map_or_else(
        || vec!["menus", "one-player", "two-player", "game-over"],
        |names| names.split(',').collect(),
    );
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../../..");
    let mut failures = 0;
    for (revision, file) in [(Revision::Rev02, "rev02"), (Revision::Rev00, "rev00")] {
        let bytes = fs::read(roms.join(format!("{file}.bin")))?;
        let registry = registry(revision, &names)?;
        for name in &recordings {
            let source = Arc::new(Recording::load(
                &root.join(format!("sessions/{file}-{name}.tje.json")),
            )?);
            let result = session(&bytes, revision, name, source, registry.clone(), output);
            if let Err(error) = result {
                eprintln!("{file} {name}: {error}");
                failures += 1;
            }
        }
        let source = Arc::new(Recording::load(
            &root.join(format!("sessions/{file}-one-player.tje.json")),
        )?);
        let mut live = Host::new(&bytes, Mode::Reference, registry)?;
        live.set_wait_input_poller(source.clone());
        for frame in 0..1600 {
            source.before_frame(frame, &mut live.context())?;
            live.run_frame(source.at(frame)?)?;
        }
        failures += fixtures(&mut live, &names, output)?;
    }
    if failures == 0 {
        Ok(())
    } else {
        Err(
            format!("{failures} recorded sessions or labelled fixtures failed; inspect evidence")
                .into(),
        )
    }
}
