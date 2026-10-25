//! Private-ROM differential proof. Inputs are committed controller sessions;
//! labelled fixtures change live gameplay state without publishing state bytes.

use genesis_config::{GenesisButton, GenesisController, GenesisControllerType, GenesisInputs};
use std::{
    collections::BTreeMap,
    error::Error,
    fs,
    path::Path,
    sync::{Arc, LazyLock},
};
use tje_game::{
    FramedTranslation, FunctionId, Translation, ids,
    ui::{hud_all, menus},
};
use tje_host::{Host, Mode, execution::WaitInputPoller, hooks::HookRegistry};
use tje_mem::{Revision, ram};
use tje_session::{CONNECTED, Session, Start};

type Result<T, E = Box<dyn Error>> = std::result::Result<T, E>;

struct ProofEntries {
    legacy: &'static [Translation],
    framed: &'static [FramedTranslation],
    roots: Vec<FunctionId>,
    replayed: bool,
}

struct Timeline {
    session: Session,
    ends: Vec<u64>,
}

impl Timeline {
    fn new(session: Session) -> Self {
        let mut end = 0;
        let ends = session
            .runs
            .iter()
            .map(|run| {
                end += run.0;
                end
            })
            .collect();
        Self { session, ends }
    }

    fn inputs(&self, frame: u64) -> tje_host::Result<GenesisInputs> {
        let index = self.ends.partition_point(|&end| end <= frame);
        let run = self.session.runs.get(index).ok_or_else(|| {
            tje_host::HostError::Input(format!("no recorded controller sample at frame {frame}"))
        })?;
        let pads = [run.1, run.2].map(|pad| {
            let mut controller = if pad & CONNECTED != 0 {
                GenesisController::new(GenesisControllerType::ThreeButton)
            } else {
                GenesisController::None
            };
            for (index, button) in [
                GenesisButton::Up,
                GenesisButton::Down,
                GenesisButton::Left,
                GenesisButton::Right,
                GenesisButton::A,
                GenesisButton::B,
                GenesisButton::C,
                GenesisButton::Start,
            ]
            .into_iter()
            .enumerate()
            {
                controller.set_field(button, pad & (1 << index) != 0);
            }
            controller
        });
        Ok(GenesisInputs {
            p1: pads[0],
            p2: pads[1],
        })
    }
}

impl WaitInputPoller for Timeline {
    fn at(&self, frame: u64) -> tje_host::Result<GenesisInputs> {
        self.inputs(frame)
    }
}

fn save(path: &Path, value: &serde_json::Value) -> Result<()> {
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

fn run_session(
    roms: &Path,
    path: &Path,
    output: &Path,
    entries: &ProofEntries,
) -> Result<(Host, Arc<Timeline>)> {
    let session = Session::from_json(&fs::read_to_string(path)?)?;
    if !matches!(session.start, Start::PowerOn) {
        return Err("natural-session proof rejects recipe writes".into());
    }
    let revision: Revision = session.revision.into();
    let bytes = fs::read(roms.join(match revision {
        Revision::Rev02 => "rev02.bin",
        Revision::Rev00 => "rev00.bin",
    }))?;
    let frames = match std::env::var("TJE_HUD_FRAMES") {
        Ok(value) => value.parse::<u64>()?,
        Err(std::env::VarError::NotPresent) => session.frames,
        Err(error) => return Err(error.into()),
    };
    if frames == 0 || frames > session.frames {
        return Err("TJE_HUD_FRAMES must select a nonempty recorded prefix".into());
    }
    if std::env::var_os("TJE_HUD_SKIP_FIXTURES").is_none() && frames == session.frames {
        return Err("fixture settling needs a recorded prefix with later input samples; set TJE_HUD_FRAMES or TJE_HUD_SKIP_FIXTURES".into());
    }
    let timeline = Arc::new(Timeline::new(session));
    let registry = if entries.replayed {
        HookRegistry::proof_replayed_environment(
            revision,
            entries.legacy,
            entries.framed,
            &entries.roots,
        )?
    } else {
        HookRegistry::proof_filtered_with_frames(
            revision,
            entries.legacy,
            entries.framed,
            &entries.roots,
        )?
    };
    let reference_advance = std::env::var_os("TJE_HUD_REFERENCE_ADVANCE").is_some();
    let advance_mode = if reference_advance {
        Mode::Reference
    } else {
        Mode::Check
    };
    let mut checked = Host::new(&bytes, advance_mode, registry.clone())?;
    checked.set_wait_input_poller(timeline.clone());
    let mut reference = Host::new(&bytes, Mode::Reference, registry)?;
    let mut inventory_frames = [0_u64; 2];
    let mut mail_order_frames = [0_u64; 2];
    let mut split_frames = 0_u64;
    let result = (|| -> Result<()> {
        for frame in 0..frames {
            let a = checked.run_frame(timeline.inputs(frame)?)?;
            let b = reference.run_frame(timeline.inputs(frame)?)?;
            let equal = a.frame_counter == b.frame_counter
                && a.ram == b.ram
                && a.vdp == b.vdp
                && a.writes == b.writes
                && a.idle_random_draws == b.idle_random_draws
                && a.lag_events == b.lag_events
                && a.audio_samples
                    .iter()
                    .flatten()
                    .map(|sample| sample.to_bits())
                    .eq(b
                        .audio_samples
                        .iter()
                        .flatten()
                        .map(|sample| sample.to_bits()));
            let (a_pixels, a_size) = checked.frame_buffer();
            let (b_pixels, b_size) = reference.frame_buffer();
            if !equal
                || a_size != b_size
                || a_pixels[..a_size.len() as usize] != b_pixels[..b_size.len() as usize]
            {
                return Err(format!("whole-reference state differs at frame {frame}").into());
            }
            let context = checked.context();
            for player in [0, 1] {
                let open = ram::g_player_inventory_open(revision)
                    .at(player)
                    .read(&context);
                inventory_frames[player as usize] += u64::from(open != 0);
                mail_order_frames[player as usize] += u64::from(open == 2);
            }
            split_frames += u64::from(ram::g_split_screen_enabled(revision).read(&context) != 0);
        }
        if checked.checks.mismatches != 0 || !checked.checks.uninitialized_stack_reads.is_empty() {
            return Err("mismatch or dependent stack read recorded".into());
        }
        Ok(())
    })();
    let name = path
        .file_stem()
        .ok_or("session has no filename")?
        .to_string_lossy();
    save(
        &output.join(format!("{name}.json")),
        &serde_json::json!({
            "session": path, "revision": revision.name(), "recording_frames": timeline.session.frames,
            "frames": checked.frame_count(),
            "advance_mode": if reference_advance { "reference-only; fixture setup, no natural checks" } else { "check" },
            "proof_envelope": if entries.replayed { "ReplayedEnvironment" } else { "ClassDefined" },
            "whole_session_reference_equal": result.is_ok(),
            "error": result.as_ref().err().map(ToString::to_string),
            "inventory_open_frames": inventory_frames, "mail_order_open_frames": mail_order_frames,
            "split_screen_frames": split_frames, "checks": checked.checks,
            "measurements": checked.measurements,
        }),
    )?;
    result?;
    println!(
        "{name}: {frames} frames, {} checked calls, mailbox={mail_order_frames:?}",
        checked.checks.calls
    );
    Ok((checked, timeline))
}

fn fixture(
    host: &mut Host,
    entries: &[FunctionId],
    records: &mut Vec<serde_json::Value>,
    label: &str,
    id: FunctionId,
    arguments: &[u32],
) {
    if !entries.contains(&id) {
        return;
    }
    let result = host.check_fixture(id.metadata().address(host.revision()), arguments);
    records.push(serde_json::json!({
        "label": label, "function": id.metadata().name, "arguments": arguments,
        "passed": result.is_ok(), "error": result.err().map(|error| error.to_string()),
    }));
}

fn resource_state(host: &mut Host, player: i32, rank: i8, points: u16, lives: i8) {
    let revision = host.revision();
    let mut context = host.context();
    ram::g_player1_map_level(revision).write(&mut context, 1);
    ram::g_v_blank_graphics_enabled(revision).write(&mut context, 1);
    ram::g_v_blank_graphics_paused(revision).write(&mut context, 0);
    ram::g_inventory_redraw_phase(revision).write(&mut context, 0);
    ram::g_sprite_graphics_upload_pending(revision).write(&mut context, 0);
    ram::g_sprite_tile_buffer_offset(revision).write(&mut context, 0);
    ram::g_sprite_tile_upload_queue_word_count(revision).write(&mut context, 0);
    ram::g_player_inventory_open(revision)
        .at(player)
        .write(&mut context, 0);
    ram::g_player_rank(revision)
        .at(player)
        .write(&mut context, rank);
    ram::g_player_points(revision)
        .at(player)
        .write(&mut context, points);
    ram::g_player_lives(revision)
        .at(player)
        .write(&mut context, lives);
    ram::g_hud_displayed_lives(revision)
        .at(player)
        .write(&mut context, lives);
    ram::g_player_health(revision)
        .at(player)
        .write(&mut context, 4);
    ram::g_hud_displayed_health(revision)
        .at(player)
        .write(&mut context, 4);
    ram::g_player_health_before_swimming(revision)
        .at(player)
        .write(&mut context, -1);
    ram::g_player_powerups(revision)
        .at(player)
        .write(&mut context, -1);
    ram::g_player_promotion_display_entities(revision)
        .at(player)
        .write(&mut context, 0);
    let object = ram::g_player_info(revision).at(player);
    object.entity_index().write(&mut context, player as i8);
    object.sprite_state().write(&mut context, 0);
    object.animation_group_mask().write(&mut context, 0);
    object.draw_flags().write(&mut context, 0);
}

fn resource_fixtures(
    host: &mut Host,
    entries: &[FunctionId],
    records: &mut Vec<serde_json::Value>,
) {
    if !entries.iter().any(|id| {
        [
            ids::CHECK_PROMOTION_SCORE,
            ids::PLAYER_RESET_RANK_AND_RESOURCES,
            ids::SETUP_THE_MAIN_GAME,
            ids::PLAYER_UPDATE_HEALTH_AND_LIVES,
        ]
        .contains(id)
    }) {
        return;
    }
    let revision = host.revision();
    for player in [0, 1] {
        for (rank, points, lives) in [
            (0, 39, 3),
            (0, 40, 3),
            (7, 880, 3),
            (8, u16::MAX, 3),
            (0, u16::MAX, -1),
        ] {
            resource_state(host, player, rank, points, lives);
            fixture(
                host,
                entries,
                records,
                "promotion threshold, rank cap and absent player",
                ids::CHECK_PROMOTION_SCORE,
                &[player as u32],
            );
        }
        // Rank reset intentionally never terminates at points>=880. Such inputs
        // are not disguised as returning fixtures or fixed by a rank-eight cap.
        for points in [0, 39, 40, 879] {
            for selection in [0, 2] {
                resource_state(host, player, 7, points, 3);
                ram::g_player_selection(revision).write(&mut host.context(), selection);
                fixture(
                    host,
                    entries,
                    records,
                    "returning rank reset below the original endless-loop threshold",
                    ids::PLAYER_RESET_RANK_AND_RESOURCES,
                    &[player as u32],
                );
            }
        }
        for (state, health, displayed, saved, lives, flags, counter) in [
            (5, 0, 0, -1, 3, 0, 0),
            (6, 0, 0, -1, 3, 0, 0),
            (3, 4, 4, -1, 3, 0, 0),
            (7, 4, 4, 4, 3, 0, 12),
            (0, 4, 4, 9, 3, 0x80, 0),
            (2, 4, 4, -1, 3, 0x80, 0),
            (0, 0, 0, -1, 0, 0, 0),
            (0, 0, 0, 4, i8::MIN, 0, 0),
        ] {
            resource_state(host, player, 8, 0, lives);
            let mut context = host.context();
            let object = ram::g_player_info(revision).at(player);
            object.sprite_state().write(&mut context, state);
            object.draw_flags().write(&mut context, flags);
            ram::g_player_health(revision)
                .at(player)
                .write(&mut context, health);
            ram::g_hud_displayed_health(revision)
                .at(player)
                .write(&mut context, displayed);
            ram::g_player_health_before_swimming(revision)
                .at(player)
                .write(&mut context, saved);
            ram::g_frame_counter(revision).write(&mut context, counter);
            fixture(
                host,
                entries,
                records,
                "ghost, swim, restoration, draw-late and signed death branches",
                ids::PLAYER_UPDATE_HEALTH_AND_LIVES,
                &[object.address(), u16::MAX as u32],
            );
        }
    }
    for (lives_a, lives_b) in [(3, -1), (-1, 3), (3, 3), (-1, -1)] {
        resource_state(host, 0, 0, 0, lives_a);
        resource_state(host, 1, 0, 0, lives_b);
        // An absent player with positive health makes the original immediate
        // health constructor loop forever. Preserve that bug; use its returning
        // natural precondition instead of inventing a successful return.
        let mut context = host.context();
        ram::g_player_health(revision).at(0).write(&mut context, 0);
        ram::g_player_health(revision).at(1).write(&mut context, 0);
        fixture(
            host,
            entries,
            records,
            "startup active/absent resource matrix with original returning preconditions",
            ids::SETUP_THE_MAIN_GAME,
            &[],
        );
    }
}

fn drive_fixtures(
    host: &mut Host,
    timeline: &Timeline,
    entries: &[FunctionId],
    output: &Path,
) -> Result<()> {
    // Settling must execute original outstanding work before any fixture write.
    // Select an explicit recorded prefix with TJE_HUD_FRAMES so the current and
    // crossed video frames still have authentic recorded controller samples.
    let boundary = host.settle_fixture_boundary(timeline.inputs(host.frame_count())?)?;
    let completed_frames: Vec<_> = boundary
        .completed_frames
        .iter()
        .map(|frame| {
            let audio_bits: Vec<_> = frame
                .audio_samples
                .iter()
                .map(|sample| sample.map(f64::to_bits))
                .collect();
            serde_json::json!({
                "frame": frame.frame, "frame_counter": frame.frame_counter, "pc": frame.pc,
                "ram": frame.ram, "vdp": {
                    "vram": frame.vdp.vram, "cram": frame.vdp.cram,
                    "vsram": frame.vdp.vsram, "registers": frame.vdp.registers,
                },
                "writes": frame.writes, "audio_bits": audio_bits,
                "idle_random_draws": frame.idle_random_draws, "lag_events": frame.lag_events,
            })
        })
        .collect();
    // This private ignored evidence contains any complete settling snapshots;
    // none of these live machine bytes belong in the public repository.
    save(
        &output.join(format!("{}-fixture-boundary.json", host.revision().name())),
        &serde_json::json!({
            "start_pc": boundary.start_pc, "end_pc": boundary.end_pc,
            "start_frame": boundary.start_frame, "end_frame": boundary.end_frame,
            "ticks": boundary.ticks, "master_clocks": boundary.master_clocks,
            "handlers_completed": boundary.handlers_completed,
            "completed_frames": completed_frames,
        }),
    )?;
    let revision = host.revision();
    let before: BTreeMap<_, _> = host
        .checks
        .by_function
        .iter()
        .map(|(name, coverage)| (name.to_owned(), coverage.calls_checked))
        .collect();
    let mut records = Vec::new();
    resource_fixtures(host, entries, &mut records);
    if std::env::var_os("TJE_HUD_UPLOAD_WAIT_FIXTURES").is_some() {
        for player in [0, 1] {
            for rank in [0, 8] {
                let mut context = host.context();
                ram::g_mode2_reg_softcopy(revision).write(&mut context, 0x40);
                ram::g_v_blank_func(revision).write(&mut context, tje_mem::Ptr::new(0));
                ram::g_v_blank_graphics_enabled(revision).write(&mut context, 1);
                ram::g_h_blank_graphics_enabled(revision).write(&mut context, 0);
                ram::g_v_blank_graphics_paused(revision).write(&mut context, 0);
                ram::g_inventory_redraw_phase(revision).write(&mut context, 0);
                ram::g_terrain_priority_update_pending(revision).write(&mut context, 0);
                ram::g_sprite_tile_buffer_offset(revision).write(&mut context, 0);
                ram::g_sprite_tile_upload_queue_word_count(revision).write(&mut context, 0);
                ram::g_sprite_graphics_upload_pending(revision).write(&mut context, 0);
                ram::g_player_rank(revision)
                    .at(player)
                    .write(&mut context, rank);
                ram::g_player_lives(revision)
                    .at(player)
                    .write(&mut context, 3);
                ram::g_player_health(revision)
                    .at(player)
                    .write(&mut context, 0);
                fixture(
                    host,
                    entries,
                    &mut records,
                    "queued health initialization waits for original upload ISR",
                    ids::HUD_INITIALIZE_HEALTH_BAR,
                    &[player as u32, 0],
                );
            }
            let mut context = host.context();
            ram::g_player_powerups(revision)
                .at(player)
                .write(&mut context, -1);
            ram::g_sprite_tile_buffer_offset(revision).write(&mut context, 0);
            ram::g_sprite_tile_upload_queue_word_count(revision).write(&mut context, 0);
            ram::g_sprite_graphics_upload_pending(revision).write(&mut context, 1);
            fixture(
                host,
                entries,
                &mut records,
                "action icon waits for a previously queued original upload ISR",
                ids::HUD_LOAD_ACTION_ICON_SPRITE,
                &[player as u32],
            );
        }
    } else {
        // Every currency byte is meaningful: bucks are signed bytes, not a 999 cap.
        for player in [0, 1] {
            for bucks in i8::MIN..=i8::MAX {
                ram::g_player_bucks(revision)
                    .at(player)
                    .write(&mut host.context(), bucks);
                fixture(
                    host,
                    entries,
                    &mut records,
                    "all signed-byte currency values",
                    ids::INVENTORY_DRAW_BUCKS,
                    &[player as u32],
                );
            }
            for points in [0, 9, 10, 99, 100, 999, 1000, 32767, 32768, 65535] {
                for rank in 0..=8 {
                    let mut context = host.context();
                    ram::g_player_bucks(revision)
                        .at(player)
                        .write(&mut context, 127);
                    ram::g_player_points(revision)
                        .at(player)
                        .write(&mut context, points);
                    ram::g_player_rank(revision)
                        .at(player)
                        .write(&mut context, rank);
                    fixture(
                        host,
                        entries,
                        &mut records,
                        "points width and rank transitions",
                        ids::INVENTORY_DRAW_PLAYER_STATS,
                        &[player as u32],
                    );
                }
            }
            // Saved swimming health has its own upper clamp, not a lower clamp.
            for rank in [-128, -1, 0, 1, 8, 127] {
                for actual in [-128, -1, 0, 1, 23, 31, 63, 127] {
                    for displayed in [-128, -1, 0, 1, 3, 4, 7, 8, 22, 23, 31, 63, 126, 127] {
                        let mut context = host.context();
                        ram::g_player1_map_level(revision).write(&mut context, 1);
                        ram::g_player_lives(revision)
                            .at(player)
                            .write(&mut context, 3);
                        ram::g_hud_displayed_lives(revision)
                            .at(player)
                            .write(&mut context, 3);
                        ram::g_player_info(revision)
                            .at(player)
                            .sprite_state()
                            .write(&mut context, 0);
                        ram::g_player_rank(revision)
                            .at(player)
                            .write(&mut context, rank);
                        ram::g_player_health(revision)
                            .at(player)
                            .write(&mut context, actual);
                        ram::g_player_health_before_swimming(revision)
                            .at(player)
                            .write(&mut context, 127);
                        ram::g_hud_displayed_health(revision)
                            .at(player)
                            .write(&mut context, displayed);
                        fixture(
                            host,
                            entries,
                            &mut records,
                            "health clamp, byte limits and nibble transitions",
                            ids::HUD_ANIMATE_HEALTH_BAR,
                            &[player as u32],
                        );
                    }
                    let mut context = host.context();
                    ram::g_sprite_tile_buffer_offset(revision).write(&mut context, 0);
                    ram::g_sprite_tile_upload_queue_word_count(revision).write(&mut context, 0);
                    ram::g_sprite_graphics_upload_pending(revision).write(&mut context, 0);
                    ram::g_v_blank_graphics_enabled(revision).write(&mut context, 0);
                    ram::g_player_health(revision)
                        .at(player)
                        .write(&mut context, actual);
                    ram::g_hud_displayed_health(revision)
                        .at(player)
                        .write(&mut context, 0);
                    fixture(
                        host,
                        entries,
                        &mut records,
                        "immediate health initialization extremes",
                        ids::HUD_INITIALIZE_HEALTH_BAR,
                        &[player as u32, 1],
                    );
                }
            }
            for lives in [0, 4, 5, 127] {
                let mut context = host.context();
                ram::g_player1_map_level(revision).write(&mut context, 1);
                ram::g_player_lives(revision)
                    .at(player)
                    .write(&mut context, lives);
                ram::g_hud_displayed_lives(revision)
                    .at(player)
                    .write(&mut context, -1);
                ram::g_player_info(revision)
                    .at(player)
                    .sprite_state()
                    .write(&mut context, 0);
                ram::g_player_rank(revision)
                    .at(player)
                    .write(&mut context, 0);
                ram::g_player_health(revision)
                    .at(player)
                    .write(&mut context, 1);
                ram::g_hud_displayed_health(revision)
                    .at(player)
                    .write(&mut context, 0);
                fixture(
                    host,
                    entries,
                    &mut records,
                    "life icon refresh and four/five-life boundary",
                    ids::HUD_ANIMATE_HEALTH_BAR,
                    &[player as u32],
                );
            }
            for (lives, state, level) in [(-1, 0, 1), (3, 5, 1), (3, 6, 1), (3, 0, 26)] {
                let mut context = host.context();
                ram::g_player_lives(revision)
                    .at(player)
                    .write(&mut context, lives);
                ram::g_hud_displayed_lives(revision)
                    .at(player)
                    .write(&mut context, lives);
                ram::g_player_info(revision)
                    .at(player)
                    .sprite_state()
                    .write(&mut context, state);
                ram::g_player1_map_level(revision).write(&mut context, level);
                fixture(
                    host,
                    entries,
                    &mut records,
                    "absent, ghost and final-level early returns",
                    ids::HUD_ANIMATE_HEALTH_BAR,
                    &[player as u32],
                );
            }
            for powerup in -1..=7 {
                let mut context = host.context();
                ram::g_player_powerups(revision)
                    .at(player)
                    .write(&mut context, powerup);
                ram::g_v_blank_graphics_enabled(revision).write(&mut context, 0);
                ram::g_sprite_graphics_upload_pending(revision).write(&mut context, 0);
                ram::g_sprite_tile_buffer_offset(revision).write(&mut context, 0);
                ram::g_sprite_tile_upload_queue_word_count(revision).write(&mut context, 0);
                fixture(
                    host,
                    entries,
                    &mut records,
                    "default and every action-icon descriptor",
                    ids::HUD_LOAD_ACTION_ICON_SPRITE,
                    &[player as u32],
                );
            }
            for suppressed in [0, 1] {
                for identified in [0, 1] {
                    for partner_lives in [-1, 3] {
                        let mut context = host.context();
                        ram::g_inventory_labels_suppressed(revision)
                            .write(&mut context, suppressed);
                        ram::g_player_lives(revision)
                            .at(player ^ 1)
                            .write(&mut context, partner_lives);
                        for (row, present) in [(0, 12), (1, 0), (2, 26)] {
                            ram::g_player_mailbox_items(revision)
                                .at(player)
                                .at(row)
                                .write(&mut context, present);
                            ram::g_player_mailbox_prices(revision)
                                .at(player)
                                .at(row)
                                .write(&mut context, [-128, 0, 127][row as usize]);
                            ram::g_present_info(revision)
                                .at(i32::from(present))
                                .identified()
                                .write(&mut context, identified);
                        }
                        fixture(
                            host,
                            entries,
                            &mut records,
                            "mailbox labels, togetherness and signed prices",
                            ids::INVENTORY_DRAW_MAIL_ORDER_OFFERS,
                            &[player as u32],
                        );
                    }
                }
            }
        }
        for selection in [-1, 0, 1, 2] {
            for rank in 0..=8 {
                let mut context = host.context();
                ram::g_player_selection(revision).write(&mut context, selection);
                ram::g_player_rank(revision).at(0).write(&mut context, rank);
                ram::g_player_rank(revision)
                    .at(1)
                    .write(&mut context, 8 - rank);
                fixture(
                    host,
                    entries,
                    &mut records,
                    "active/absent player ranks and rank changes",
                    ids::HUD_DRAW_RANK_LABELS,
                    &[],
                );
            }
        }
        for split in [0, 1] {
            for (open_a, open_b) in [(0, 0), (1, 0), (0, 2), (1, 2)] {
                for lives in [-1, 3] {
                    for level in [1, 26] {
                        let mut context = host.context();
                        ram::g_player1_map_level(revision).write(&mut context, level);
                        ram::g_split_screen_enabled(revision).write(&mut context, split);
                        ram::g_player_inventory_open(revision)
                            .at(0)
                            .write(&mut context, open_a);
                        ram::g_player_inventory_open(revision)
                            .at(1)
                            .write(&mut context, open_b);
                        ram::g_player_lives(revision)
                            .at(0)
                            .write(&mut context, lives);
                        ram::g_player_lives(revision).at(1).write(&mut context, 3);
                        ram::g_player_powerups(revision)
                            .at(0)
                            .write(&mut context, -1);
                        ram::g_player_powerups(revision)
                            .at(1)
                            .write(&mut context, -1);
                        fixture(
                            host,
                            entries,
                            &mut records,
                            "combined, split, inventory and final-level HUD",
                            ids::DRAW_HUD,
                            &[],
                        );
                    }
                }
            }
        }
        for filled in [0, 1, u32::MAX, 0x10000, i16::MAX as u32] {
            fixture(
                host,
                entries,
                &mut records,
                "border RNG and short truth narrowing",
                ids::DRAW_MENU_WINDOW_FRAME,
                &[filled],
            );
        }
    }
    let counts: BTreeMap<_, _> = host
        .checks
        .by_function
        .iter()
        .map(|(name, coverage)| {
            (
                name,
                coverage.calls_checked - before.get(name).copied().unwrap_or(0),
            )
        })
        .collect();
    let failures = records
        .iter()
        .filter(|record| record["passed"] != true)
        .count();
    save(
        &output.join(format!("{}-live-fixtures.json", revision.name())),
        &serde_json::json!({
            "revision": revision.name(), "coverage_source": "labelled synthetic inputs on live gameplay state",
            "fixture_calls": counts, "fixture_failures": failures, "records": records,
            "checks_including_session": host.checks,
        }),
    )?;
    println!(
        "{revision:?} fixtures: {} cases, {failures} failures",
        records.len()
    );
    if failures != 0 {
        return Err(format!("{failures} labelled fixtures failed").into());
    }
    Ok(())
}

fn main() -> Result<()> {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if arguments.len() < 3 {
        return Err("usage: tje-ui-hud-check <ROM-directory> <evidence-directory> <session.tje.json>...; TJE_HUD_FUNCTIONS optionally selects comma-separated names".into());
    }
    let roms = Path::new(&arguments[0]);
    let output = Path::new(&arguments[1]);
    fs::create_dir_all(output)?;
    let selection = std::env::var("TJE_HUD_FUNCTIONS").ok();
    let names: Vec<_> = selection
        .as_deref()
        .map_or_else(Vec::new, |text| text.split(',').collect());
    static FRAMED: LazyLock<Vec<FramedTranslation>> = LazyLock::new(|| {
        hud_all::FRAME_CANDIDATES
            .iter()
            .chain(menus::RESOURCE_FRAME_CANDIDATES)
            .copied()
            .collect()
    });
    let replayed = std::env::var_os("TJE_HUD_REPLAYED_ENVIRONMENT").is_some();
    let candidates: Vec<_> = hud_all::PROVEN_ENTRIES
        .iter()
        .map(|entry| entry.id)
        .chain(FRAMED.iter().map(|entry| entry.id))
        .collect();
    for name in &names {
        if !candidates.iter().any(|id| id.metadata().name == *name) {
            return Err(format!("unknown HUD function {name}").into());
        }
    }
    let roots = candidates
        .into_iter()
        .filter(|id| {
            if !names.is_empty() {
                names.contains(&id.metadata().name)
            } else if replayed {
                FRAMED.iter().any(|entry| entry.id == *id)
            } else {
                hud_all::PROVEN_ENTRIES.iter().any(|entry| entry.id == *id)
            }
        })
        .collect();
    let entries = ProofEntries {
        legacy: hud_all::PROVEN_ENTRIES,
        framed: &FRAMED,
        roots,
        replayed,
    };
    let mut hosts = BTreeMap::new();
    let mut failures = Vec::new();
    for path in &arguments[2..] {
        match run_session(roms, Path::new(path), output, &entries) {
            Ok((host, timeline)) => {
                hosts.insert(host.revision().name(), (host, timeline));
            }
            Err(error) => {
                eprintln!("{}: {error}", Path::new(path).display());
                failures.push(error.to_string());
            }
        }
    }
    if std::env::var_os("TJE_HUD_SKIP_FIXTURES").is_none() {
        for (_, (mut host, timeline)) in hosts {
            if host.measurements.game_frame.is_some()
                && let Err(error) = drive_fixtures(&mut host, &timeline, &entries.roots, output)
            {
                failures.push(error.to_string());
            }
        }
    }
    if !failures.is_empty() {
        return Err(failures.join("; ").into());
    }
    Ok(())
}
