//! Opt-in differential evidence, not a production game dependency.
//! Run with `<private-ROM-directory> <private-evidence-directory>`.
//! Sessions run real controller scripts; fixtures are explicitly synthetic inputs
//! applied to live gameplay state. No ROM-derived state is written publicly.

use std::{collections::BTreeMap, error::Error, fs, path::Path};
use tje_game::{Dispatcher, FunctionId, ids, ui};
use tje_host::{Host, Mode, hooks::HookRegistry, input::InputScript};
use tje_mem::{Revision, ram};

type Result<T, E = Box<dyn Error>> = std::result::Result<T, E>;

fn save(path: &Path, value: &serde_json::Value) -> Result<()> {
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

fn run_session(
    bytes: &[u8],
    revision: Revision,
    name: &str,
    source: &str,
    frames: u64,
    output: &Path,
) -> Result<Host> {
    let script = InputScript::parse(source)?;
    let registry = HookRegistry::filtered(revision, &["ui"], &[])?;
    let mut checked = Host::new(bytes, Mode::Check, registry.clone())?;
    let mut reference = Host::new(bytes, Mode::Reference, registry)?;
    let mut inventory_frames = [0_u64; 2];
    let mut split_frames = 0_u64;
    let result = (|| -> Result<()> {
        for frame in 0..frames {
            let a = checked.run_frame(script.at(frame))?;
            let b = reference.run_frame(script.at(frame))?;
            if a.frame_counter != b.frame_counter
                || a.ram != b.ram
                || a.vdp != b.vdp
                || a.writes != b.writes
                || a.idle_random_draws != b.idle_random_draws
                || a.lag_events != b.lag_events
                || !a
                    .audio_samples
                    .iter()
                    .flatten()
                    .map(|value| value.to_bits())
                    .eq(b
                        .audio_samples
                        .iter()
                        .flatten()
                        .map(|value| value.to_bits()))
            {
                return Err(format!(
                    "{revision:?} {name} frame {frame}: whole-session state differs"
                )
                .into());
            }
            let (pixels_a, size_a) = checked.frame_buffer();
            let (pixels_b, size_b) = reference.frame_buffer();
            if size_a != size_b
                || pixels_a[..size_a.len() as usize] != pixels_b[..size_b.len() as usize]
            {
                return Err(format!("{revision:?} {name} frame {frame}: pixels differ").into());
            }
            let context = checked.context();
            for player in [0, 1] {
                if ram::g_player_inventory_open(revision)
                    .at(player)
                    .read(&context)
                    != 0
                {
                    inventory_frames[player as usize] += 1;
                }
            }
            if ram::g_split_screen_enabled(revision).read(&context) != 0 {
                split_frames += 1;
            }
        }
        if checked.measurements.title_frame.is_none() || checked.measurements.game_frame.is_none() {
            return Err(
                format!("{revision:?} {name}: title/gameplay milestone not reached").into(),
            );
        }
        if name == "inventory" && inventory_frames[0] == 0 {
            return Err(format!("{revision:?} inventory script did not open inventory").into());
        }
        if name == "two-player" && inventory_frames.contains(&0) {
            return Err(
                format!("{revision:?} two-player script did not open both inventories").into(),
            );
        }
        Ok(())
    })();
    save(
        &output.join(format!("{}-{name}.json", revision.name())),
        &serde_json::json!({
            "revision": revision.name(), "session": name, "frames": checked.frame_count(),
            "whole_session_reference_equal": result.is_ok(),
            "error": result.as_ref().err().map(ToString::to_string),
            "inventory_open_frames": inventory_frames, "split_screen_frames": split_frames,
            "checks": checked.checks, "measurements": checked.measurements,
        }),
    )?;
    result?;
    println!(
        "{revision:?} {name}: {} frames, {} checked calls, inventory={inventory_frames:?}, split={split_frames}",
        checked.frame_count(),
        checked.checks.calls
    );
    Ok(checked)
}

fn fixture(host: &mut Host, id: FunctionId, arguments: &[u32]) -> Result<u32> {
    host.check_fixture(id.metadata().address(host.revision()), arguments)
        .map_err(|error| {
            format!(
                "live-state fixture {} {arguments:?}: {error}",
                id.metadata().name
            )
            .into()
        })
}

fn drive_fixtures(host: &mut Host) -> Result<()> {
    let revision = host.revision();
    fixture(host, ids::TEXT_COPY_FONT_BANKS_TO_RAM, &[])?;
    for value in [
        i16::MIN as i32 as u32,
        u32::MAX,
        0,
        1,
        i16::MAX as u32,
        0x10000,
    ] {
        fixture(host, ids::FORCE_TEXT_BACKGROUND_COLOR_TO_BLACK, &[value])?;
    }
    // Every possible signed-char input, including all embedded control writes.
    for character in 0..=255 {
        fixture(host, ids::TEXT_MAP_CHARACTER_TO_GLYPH, &[character])?;
    }
    for index in [0, 57, 58, 59, u16::MAX - 1, u16::MAX] {
        ram::g_random_cache_index(revision).write(&mut host.context(), index);
        fixture(host, ids::TEXT_MAP_CHARACTER_TO_GLYPH, &[u32::from(b'?')])?;
    }
    for split in [0, -1] {
        for (open_a, open_b) in [(0, 0), (-1, 0), (0, 2), (1, 3)] {
            for suspend in [0, 1, 0x10000, u32::MAX] {
                let mut context = host.context();
                ram::g_split_screen_enabled(revision).write(&mut context, split);
                ram::g_player_inventory_open(revision)
                    .at(0)
                    .write(&mut context, open_a);
                ram::g_player_inventory_open(revision)
                    .at(1)
                    .write(&mut context, open_b);
                ram::g_camera1_pending_x(revision).write(&mut context, i16::MIN);
                ram::g_camera1_pending_y(revision).write(&mut context, i16::MAX);
                ram::g_camera2_pending_x(revision).write(&mut context, -123);
                ram::g_camera2_pending_y(revision).write(&mut context, 456);
                ram::g_inventory_saved_camera1_x(revision).write(&mut context, 123);
                ram::g_inventory_saved_camera1_y(revision).write(&mut context, -456);
                ram::g_inventory_saved_camera2_x(revision).write(&mut context, i16::MAX);
                ram::g_inventory_saved_camera2_y(revision).write(&mut context, i16::MIN);
                fixture(host, ids::INVENTORY_SET_CAMERA_SUSPENSION, &[suspend])?;
            }
        }
    }
    for player in [0, 1] {
        for (scroll, row, column) in [
            (0, 0, 0),
            (5, 2, 1),
            (0, 64, 0),
            (-1, 0, 1),
            (127, 127, 127),
            (-128, -128, -128),
        ] {
            let mut context = host.context();
            ram::g_player_inventory_scroll_row(revision)
                .at(player)
                .write(&mut context, scroll);
            ram::g_player_inventory_selection_row(revision)
                .at(player)
                .write(&mut context, row);
            ram::g_player_inventory_selection_column(revision)
                .at(player)
                .write(&mut context, column);
            fixture(host, ids::INVENTORY_GET_SELECTED_PRESENT, &[player as u32])?;
        }
        // Full, exhausted, retreat-over-empty, and mailbox selector states.
        for (open, occupied) in [(1, -1), (1, 0), (1, 1), (1, 4), (1, 15), (2, -1)] {
            let mut context = host.context();
            ram::g_player_inventory_open(revision)
                .at(player)
                .write(&mut context, open);
            ram::g_player_inventory_scroll_row(revision)
                .at(player)
                .write(&mut context, 0);
            ram::g_player_inventory_selection_row(revision)
                .at(player)
                .write(&mut context, 2);
            ram::g_player_inventory_selection_column(revision)
                .at(player)
                .write(&mut context, 1);
            for slot in 0..16 {
                ram::g_player_inventory(revision)
                    .at(player)
                    .at(slot)
                    .write(&mut context, if slot == occupied { 7 } else { -1 });
            }
            fixture(
                host,
                ids::INVENTORY_UPDATE_SELECTOR_ENTITY,
                &[player as u32],
            )?;
        }
        for split in [0, -1] {
            for own in [0, 1] {
                for other in [0, 3] {
                    for lives in [-128, -1, 0, 4, 5, 127] {
                        for level in [-128, -1, 0, 9, 10, 25, 127] {
                            let mut context = host.context();
                            ram::g_split_screen_enabled(revision).write(&mut context, split);
                            ram::g_player_inventory_open(revision)
                                .at(player)
                                .write(&mut context, own);
                            ram::g_player_inventory_open(revision)
                                .at(player ^ 1)
                                .write(&mut context, other);
                            ram::g_player_lives(revision)
                                .at(player)
                                .write(&mut context, lives);
                            let level_address = if player == 0 {
                                ram::g_toejam_level(revision)
                            } else {
                                ram::g_earl_level(revision)
                            };
                            level_address.write(&mut context, level);
                            fixture(
                                host,
                                ids::HUD_POSITION_LEVEL_NUMBER_SPRITES,
                                &[player as u32],
                            )?;
                            fixture(host, ids::HUD_POSITION_HEALTH_BAR_SPRITES, &[player as u32])?;
                            fixture(host, ids::HUD_POSITION_LIVES_SPRITE, &[player as u32])?;
                            for powerup in [-1, 0, 7] {
                                ram::g_player_powerups(revision)
                                    .at(player)
                                    .write(&mut host.context(), powerup);
                                fixture(
                                    host,
                                    ids::HUD_POSITION_ACTION_ICON_SPRITE,
                                    &[player as u32],
                                )?;
                            }
                        }
                    }
                }
            }
        }
    }
    fixture(host, ids::ARE_BOTH_CONTROLLERS_CONNECTED, &[])?;
    for handle in [0, 1, 39, 255, 256, 512, i16::MAX, i16::MIN] {
        ram::g_menu_cursor_entity(revision).write(&mut host.context(), handle);
        for (column, row) in [(0, 0), (-1, -1), (i16::MIN, i16::MAX), (i16::MAX, i16::MIN)] {
            fixture(
                host,
                ids::POSITION_MENU_CURSOR_ENTITY,
                &[column as i32 as u32, row as i32 as u32],
            )?;
        }
    }
    fixture(host, ids::ENABLE_TEXT_FONT_ANIMATION, &[])?;
    fixture(host, ids::DISABLE_TEXT_FONT_ANIMATION_IF_MENUS_CLOSED, &[])?;
    fixture(host, ids::PLAYER_GET_MAXIMUM_HEALTH, &[0])?;
    fixture(host, ids::GET_SCORE_FOR_PROMO, &[0])?;
    Ok(())
}

fn main() -> Result<()> {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if arguments.len() != 2 {
        return Err(
            "usage: tje-ui-check <private-ROM-directory> <private-evidence-directory>".into(),
        );
    }
    let rom_directory = Path::new(&arguments[0]);
    let output = Path::new(&arguments[1]);
    fs::create_dir_all(output)?;
    let roots: Vec<_> = ui::TRANSLATIONS.iter().map(|entry| entry.id).collect();
    if let Err(missing) = Dispatcher::native(&roots) {
        return Err(format!("native UI closure missing {missing:?}").into());
    }
    for revision in [Revision::Rev02, Revision::Rev00] {
        let rom_name = match revision {
            Revision::Rev02 => "rev02.bin",
            Revision::Rev00 => "rev00.bin",
        };
        let bytes = fs::read(rom_directory.join(rom_name))?;
        run_session(
            &bytes,
            revision,
            "standard",
            include_str!("../../../../tje-host/examples/start-game.json"),
            10_000,
            output,
        )?;
        run_session(
            &bytes,
            revision,
            "inventory",
            include_str!("inventory.json"),
            2_500,
            output,
        )?;
        let mut host = run_session(
            &bytes,
            revision,
            "two-player",
            include_str!("two-player.json"),
            2_500,
            output,
        )?;
        let before: BTreeMap<_, _> = host
            .checks
            .by_function
            .iter()
            .map(|(name, coverage)| (name.clone(), coverage.calls_checked))
            .collect();
        let result = drive_fixtures(&mut host);
        let fixture_calls: BTreeMap<_, _> = host
            .checks
            .by_function
            .iter()
            .map(|(name, coverage)| (name.clone(), coverage.calls_checked - before[name]))
            .collect();
        save(
            &output.join(format!("{}-live-fixtures.json", revision.name())),
            &serde_json::json!({
                "revision": revision.name(), "coverage_source": "labelled live-state fixtures after 2500-frame two-player session",
                "error": result.as_ref().err().map(ToString::to_string), "fixture_calls": fixture_calls,
                "checks_including_two_player_session": host.checks, "native_ui_closure_complete": true,
            }),
        )?;
        result?;
        if host.checks.mismatches != 0
            || !host.checks.uninitialized_stack_reads.is_empty()
            || !host.checks.unexercised.is_empty()
        {
            return Err(format!("{revision:?}: incomplete UI fixture evidence").into());
        }
        println!("{revision:?}: every UI hook exercised, zero mismatches; native closure complete");
    }
    Ok(())
}
