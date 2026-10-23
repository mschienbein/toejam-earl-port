//! Opt-in ROM-backed executable: arguments are ROM root, output root, Jam Out
//! input JSON and frame count. Built with the lane-private Cargo manifest.
//! Every additional fixture is an authored mutation of a completed live state.

use std::{
    fs,
    path::Path,
    sync::{Arc, LazyLock},
};
use tje_game::{Context, FunctionId, ids};
use tje_host::{Host, Mode, hooks::HookRegistry, input::InputScript};
use tje_mem::{Platform, Ptr, Revision, ram};

const ROOTS: &[FunctionId] = &[
    ids::DRAW_STARFIELD_PLANE,
    ids::WAIT_FOR_NEXT_V_BLANK,
    ids::SELECT_VIEWPORT_SPRITE_CHAIN,
    ids::HANDLE_H_BLANK,
    ids::GET_PLAYER_INPUT,
    ids::INITIALISE_THE_Z80,
    ids::JAM_OUT_DRAW_PACKED_BACKGROUND_BLOCK,
    ids::JAM_OUT_RELEASE_CHARACTER_COPIES,
];

fn proof_roots(include_wait: bool) -> Vec<tje_game::Translation> {
    ROOTS
        .iter()
        .filter(|&&id| include_wait || id != ids::WAIT_FOR_NEXT_V_BLANK)
        .map(|&id| {
            *tje_game::translations()
                .chain(tje_game::core::PENDING_TRANSLATIONS)
                .chain(tje_game::jamout::PENDING_TRANSLATIONS)
                .find(|entry| entry.id == id)
                .unwrap()
        })
        .collect()
}

static NATURAL_ROOTS: LazyLock<Vec<tje_game::Translation>> = LazyLock::new(|| proof_roots(false));
static FIXTURE_ROOTS: LazyLock<Vec<tje_game::Translation>> = LazyLock::new(|| proof_roots(true));

fn registry(revision: Revision, include_wait: bool) -> HookRegistry {
    // The shared proof API overlays these bodies on all proven callees; no
    // selected pending body silently falls back to its original implementation.
    let roots = if include_wait {
        &*FIXTURE_ROOTS
    } else {
        &*NATURAL_ROOTS
    };
    HookRegistry::proof(revision, roots.as_slice()).unwrap()
}

fn check(host: &mut Host, id: FunctionId, arguments: &[u32]) {
    host.check_fixture(id.metadata().address(host.revision()), arguments)
        .unwrap_or_else(|error| {
            panic!(
                "{:?} live fixture {} {arguments:?}: {error}",
                host.revision(),
                id.metadata().name
            )
        });
}

fn fixtures(host: &mut Host) {
    let revision = host.revision();
    check(host, ids::DRAW_STARFIELD_PLANE, &[]);
    check(host, ids::INITIALISE_THE_Z80, &[]);
    for (x, y) in [(0, 0), (20, 14), (-1_i16, -1_i16), (i16::MIN, i16::MAX)] {
        check(
            host,
            ids::JAM_OUT_DRAW_PACKED_BACKGROUND_BLOCK,
            &[x as i32 as u32, y as i32 as u32],
        );
    }
    for split in [0, 1] {
        for first_open in [0, -1] {
            for second_open in [0, -1] {
                ram::g_split_screen_enabled(revision).write(&mut host.context(), split);
                ram::g_player_inventory_open(revision)
                    .at(0)
                    .write(&mut host.context(), first_open);
                ram::g_player_inventory_open(revision)
                    .at(1)
                    .write(&mut host.context(), second_open);
                for player in [0, 1, 2, 0xffff, 0x10000] {
                    check(host, ids::SELECT_VIEWPORT_SPRITE_CHAIN, &[player]);
                }
                for enabled in [0, 1] {
                    ram::g_h_blank_graphics_enabled(revision).write(&mut host.context(), enabled);
                    for level in [25, 26] {
                        ram::g_player1_map_level(revision).write(&mut host.context(), level);
                        for (x, y) in [(1600, 1792), (-1, -3), (i16::MIN, i16::MAX)] {
                            ram::g_camera2_committed_x(revision).write(&mut host.context(), x);
                            ram::g_camera2_committed_y(revision).write(&mut host.context(), y);
                            check(host, ids::HANDLE_H_BLANK, &[]);
                        }
                    }
                }
            }
        }
    }
    // Source-named masks/states from G/include/player_animation{,_groups}.h.
    const REACTION: u32 = 0x20000;
    const STATIONARY: u32 = 0x20000000;
    const FALL: u32 = 0x20;
    const LAND: u32 = 0x40;
    const SKATES: u32 = 0x07800000;
    const WHIRLWIND: i8 = 79;
    const RECOVERY: i8 = 78;
    const GHOST: i8 = 5;
    const LAST_LIFE_GHOST: i8 = 6;
    const FLATTENED: i8 = 2;
    for (mask, state) in [
        (0, 0),
        (REACTION, 0),
        (0, WHIRLWIND),
        (0, RECOVERY),
        (0, GHOST),
        (0, LAST_LIFE_GHOST),
        (STATIONARY, 0),
        (FALL, 0),
        (LAND, 0),
        (SKATES, 0),
        (0, FLATTENED),
    ] {
        for alive in [false, true] {
            for player in 0..2 {
                let entity = ram::g_player_info(revision).at(player);
                entity
                    .animation_group_mask()
                    .write(&mut host.context(), mask);
                entity.sprite_state().write(&mut host.context(), state);
                ram::g_player_lives(revision)
                    .at(player)
                    .write(&mut host.context(), if alive { 0 } else { -1 });
            }
            ram::g_controller1_raw_input(revision).write(&mut host.context(), 0xff);
            ram::g_controller2_raw_input(revision).write(&mut host.context(), 0x55);
            check(host, ids::GET_PLAYER_INPUT, &[]);
        }
    }
    for slot in 0..9 {
        ram::g_jam_toejam_entities(revision)
            .at(slot)
            .write(&mut host.context(), 0);
        ram::g_jam_earl_entities(revision)
            .at(slot)
            .write(&mut host.context(), 0);
    }
    check(host, ids::JAM_OUT_RELEASE_CHARACTER_COPIES, &[]);
    for handle in [-1, 1, 6] {
        ram::g_jam_toejam_entities(revision)
            .at(0)
            .write(&mut host.context(), handle);
        ram::g_jam_earl_entities(revision)
            .at(8)
            .write(&mut host.context(), handle);
        if handle == 6 {
            ram::g_player_info(revision)
                .at(6)
                .draw_flags()
                .write(&mut host.context(), 1);
        }
        check(host, ids::JAM_OUT_RELEASE_CHARACTER_COPIES, &[]);
    }
}

fn wait_fixtures(host: &mut Host) {
    let revision = host.revision();
    ram::g_mode2_reg_softcopy(revision).write(&mut host.context(), 0);
    check(host, ids::WAIT_FOR_NEXT_V_BLANK, &[2]);
    // Counter-only real IRQ path, deliberately distinct from natural gameplay.
    ram::g_mode2_reg_softcopy(revision).write(&mut host.context(), 0x40);
    ram::g_v_blank_func(revision).write(&mut host.context(), Ptr::new(0));
    ram::g_v_blank_graphics_enabled(revision).write(&mut host.context(), 0);
    ram::g_h_blank_graphics_enabled(revision).write(&mut host.context(), 0);
    ram::g_v_blank_graphics_paused(revision).write(&mut host.context(), 1);
    ram::g_v_blank_audio_input_override(revision).write(&mut host.context(), 0);
    let sr = host.context().status_register() & !0x0700;
    host.context().set_status_register(sr);
    host.context().write_vdp_control(0x8164);
    for count in [0, 0xffff, 1, 2] {
        check(host, ids::WAIT_FOR_NEXT_V_BLANK, &[count]);
    }
}

fn session(
    revision: Revision,
    file: &str,
    label: &str,
    bytes: &[u8],
    script: Arc<InputScript>,
    output: &Path,
    frames: u64,
) {
    let mut host = Host::new(bytes, Mode::Check, registry(revision, false)).unwrap();
    let mut reference = Host::new(bytes, Mode::Reference, registry(revision, true)).unwrap();
    host.set_wait_input_poller(script.clone());
    reference.set_wait_input_poller(script.clone());
    for frame in 0..frames {
        let a = match host.run_frame(script.at(frame)) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                fs::write(output.join(format!("batch1-{label}-{file}-failure.json")),
                    serde_json::to_string_pretty(&serde_json::json!({"revision":revision.name(),"frame":frame,"error":error.to_string(),"checks":host.checks})).unwrap()).unwrap();
                panic!("{revision:?} {label} frame {frame}: {error}");
            }
        };
        let b = reference.run_frame(script.at(frame)).unwrap();
        assert_eq!(a.ram, b.ram, "whole RAM {revision:?} {frame}");
        assert_eq!(a.vdp, b.vdp, "VDP {revision:?} {frame}");
        assert_eq!(a.writes, b.writes, "hardware writes {revision:?} {frame}");
        assert_eq!(a.frame_counter, b.frame_counter);
        assert_eq!(a.idle_random_draws, b.idle_random_draws);
        assert_eq!(a.lag_events, b.lag_events);
        assert!(
            a.audio_samples.iter().flatten().map(|v| v.to_bits()).eq(b
                .audio_samples
                .iter()
                .flatten()
                .map(|v| v.to_bits())),
            "audio {revision:?} {frame}"
        );
        let (a, sa) = host.frame_buffer();
        let (b, sb) = reference.frame_buffer();
        assert_eq!(sa, sb);
        assert_eq!(&a[..sa.len() as usize], &b[..sb.len() as usize]);
    }
    let natural = serde_json::to_value(&host.checks).unwrap();
    if label == "gameplay" {
        fixtures(&mut host);
        wait_fixtures(&mut reference);
    }
    assert_eq!(host.checks.mismatches, 0);
    if label == "gameplay" {
        assert!(host.checks.unexercised.is_empty());
    }
    let missing_native = tje_game::Dispatcher::native(ROOTS)
        .err()
        .unwrap_or_default()
        .iter()
        .map(|id| id.metadata().name)
        .collect::<Vec<_>>();
    fs::write(output.join(format!("batch1-{label}-{file}.json")),
        serde_json::to_string_pretty(&serde_json::json!({"revision":revision.name(),"frames":frames,"whole_session_equal":true,"natural":natural,"with_live_fixtures":host.checks,"wait_counter_only_fixtures":reference.checks,"wait_natural_status":"blocked on timing-fidelity decision; REV02 frame492 YM read 0 versus128","measurements":host.measurements,"missing_native":missing_native})).unwrap()).unwrap();
    println!("PASS batch1 {label} {file}");
}

fn main() {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    assert_eq!(arguments.len(), 4);
    let output = Path::new(&arguments[1]);
    let gameplay = include_str!("../../../tje-host/examples/start-game.json");
    let jam = fs::read_to_string(&arguments[2]).unwrap();
    let frames = arguments[3].parse().unwrap();
    for (file, revision) in [("rev02", Revision::Rev02), ("rev00", Revision::Rev00)] {
        let bytes = fs::read(Path::new(&arguments[0]).join(format!("{file}.bin"))).unwrap();
        session(
            revision,
            file,
            "gameplay",
            &bytes,
            Arc::new(InputScript::parse(gameplay).unwrap()),
            output,
            frames,
        );
        session(
            revision,
            file,
            "jamout",
            &bytes,
            Arc::new(InputScript::parse(&jam).unwrap()),
            output,
            frames,
        );
    }
}
