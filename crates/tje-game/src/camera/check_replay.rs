//! Private-ROM camera replay harness. Build using the private manifest recorded
//! in scratch/W0World/replay-prep.md. This is not a game runtime module.
use genesis_config::{GenesisButton, GenesisController, GenesisControllerType, GenesisInputs};
use std::{env, fs, path::Path, sync::Arc};
use tje_game::{FramedTranslation, FunctionId, Translation, camera, ids};
use tje_host::{
    CheckFailurePolicy, Host, Mode, execution::WaitInputPoller, hooks::HookRegistry,
    input::InputScript,
};
use tje_mem::{Bus, Revision, ram};
use tje_session::Session;

const CANDIDATES: &[Translation] = &[
    Translation {
        id: ids::UPDATE_CAMERA_FOLLOWING,
        function: camera::update_camera_following,
    },
    Translation {
        id: ids::ANIMATE_SPLIT_SCREEN_WIPE,
        function: camera::animate_split_screen_wipe,
    },
    Translation {
        id: ids::ANIMATE_PLAYERS_INTO_SPLIT_VIEWS,
        function: camera::animate_players_into_split_views,
    },
];
const FRAMED_CANDIDATES: &[FramedTranslation] = &[
    FramedTranslation {
        id: ids::TRY_UPDATE_CAMERA_Y,
        function: camera::try_update_camera_y,
    },
    FramedTranslation {
        id: ids::TRY_UPDATE_CAMERA_X,
        function: camera::try_update_camera_x,
    },
    FramedTranslation {
        id: ids::MOVE_CAMERA_UP,
        function: camera::move_camera_up,
    },
    FramedTranslation {
        id: ids::MOVE_CAMERA_DOWN,
        function: camera::move_camera_down,
    },
    FramedTranslation {
        id: ids::MOVE_CAMERA_LEFT,
        function: camera::move_camera_left,
    },
    FramedTranslation {
        id: ids::MOVE_CAMERA_RIGHT,
        function: camera::move_camera_right,
    },
];

// Same indexed controller/recipe adapter as tje-session's runner. Session recipes
// are bounded, validated input writes, never an opaque captured state.
enum Timeline {
    Script(InputScript),
    Session { session: Session, ends: Vec<u64> },
}
impl Timeline {
    fn read(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let text = fs::read_to_string(path)?;
        if path.to_string_lossy().ends_with(".tje.json") {
            let session = Session::from_json(&text)?;
            let mut end = 0;
            let ends = session
                .runs
                .iter()
                .map(|run| {
                    end += run.0;
                    end
                })
                .collect();
            Ok(Self::Session { session, ends })
        } else {
            Ok(Self::Script(InputScript::parse(&text)?))
        }
    }
    fn frames(&self) -> Option<u64> {
        match self {
            Self::Session { session, .. } => Some(session.frames),
            _ => None,
        }
    }
}
impl WaitInputPoller for Timeline {
    fn at(&self, frame: u64) -> tje_host::Result<GenesisInputs> {
        match self {
            Self::Script(script) => Ok(script.at(frame)),
            Self::Session { session, ends } => {
                let index = ends.partition_point(|&end| end <= frame);
                let run = session.runs.get(index).ok_or_else(|| tje_host::HostError::Input(
                    format!("no recorded controller sample at frame {frame}; fixtures need a recorded suffix"),
                ))?;
                let pads = [run.1, run.2].map(|pad| {
                    let mut controller = if pad & tje_session::CONNECTED == 0 {
                        GenesisController::None
                    } else {
                        GenesisController::new(GenesisControllerType::ThreeButton)
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
    }
    fn before_frame(&self, frame: u64, memory: &mut dyn Bus) -> tje_host::Result<()> {
        if let Self::Session { session, .. } = self {
            for write in session
                .writes()
                .iter()
                .filter(|write| write.before_frame == frame)
            {
                match write.width {
                    1 => memory.write_u8(write.address, write.value as u8),
                    2 => memory.write_u16(write.address, write.value as u16),
                    4 => memory.write_u32(write.address, write.value),
                    _ => unreachable!("validated session write width"),
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct Seed {
    address: u32,
    width: u8,
    value: u32,
}
impl Seed {
    fn byte(address: u32, value: u8) -> Self {
        Self {
            address,
            width: 1,
            value: u32::from(value),
        }
    }
    fn word(address: u32, value: i16) -> Self {
        Self {
            address,
            width: 2,
            value: u32::from(value as u16),
        }
    }
    fn long(address: u32, value: u32) -> Self {
        Self {
            address,
            width: 4,
            value,
        }
    }
    fn read(self, bus: &dyn Bus) -> u32 {
        match self.width {
            1 => u32::from(bus.read_u8(self.address)),
            2 => u32::from(bus.read_u16(self.address)),
            4 => bus.read_u32(self.address),
            _ => unreachable!(),
        }
    }
    fn write(self, bus: &mut dyn Bus) {
        match self.width {
            1 => bus.write_u8(self.address, self.value as u8),
            2 => bus.write_u16(self.address, self.value as u16),
            4 => bus.write_u32(self.address, self.value),
            _ => unreachable!(),
        }
    }
}

fn fixture(
    host: &mut Host,
    id: FunctionId,
    label: String,
    seeds: &[Seed],
    receipts: &mut Vec<serde_json::Value>,
) -> bool {
    argument_fixture(host, id, &[], label, seeds, receipts)
}

fn argument_fixture(
    host: &mut Host,
    id: FunctionId,
    arguments: &[u32],
    label: String,
    seeds: &[Seed],
    receipts: &mut Vec<serde_json::Value>,
) -> bool {
    // Restore only the explicitly seeded inputs after the isolated check. CPU,
    // devices and every unseeded game field remain the reached live state.
    let before = host.checks.calls;
    let mismatches = host.checks.mismatches;
    let mut undo = Vec::with_capacity(seeds.len());
    for &seed in seeds {
        let mut context = host.context();
        undo.push(Seed {
            value: seed.read(&context),
            ..seed
        });
        seed.write(&mut context);
    }
    let result = host.check_fixture(id.metadata().address(host.revision()), arguments);
    for seed in undo.into_iter().rev() {
        seed.write(&mut host.context());
    }
    let error = result.err().map(|error| error.to_string());
    receipts.push(serde_json::json!({"label": label, "function": id.metadata().name,
        "arguments": arguments, "entry_frame": host.frame_count(), "calls_checked": host.checks.calls - before,
        "mismatches": host.checks.mismatches - mismatches, "error": error,
        "seeded_inputs": seeds.iter().map(|seed| serde_json::json!({"address": seed.address,
            "width": seed.width, "value": seed.value})).collect::<Vec<_>>() }));
    error.is_none()
}

fn fixtures(host: &mut Host, receipts: &mut Vec<serde_json::Value>) -> bool {
    let revision = host.revision();
    let mut passed = true;
    for split in [0_u8, 1] {
        for lives in [(3_i8, 3_i8), (3, -1), (-1, 3)] {
            for velocity in [-512_i16, 0, 512] {
                let mut seeds = vec![Seed::byte(
                    ram::g_split_screen_enabled(revision).address(),
                    split,
                )];
                for (player, life) in [lives.0, lives.1].into_iter().enumerate() {
                    let player = player as i32;
                    let entity = ram::g_player_info(revision).at(player);
                    seeds.extend([
                        Seed::byte(
                            ram::g_player_lives(revision).at(player).address(),
                            life as u8,
                        ),
                        Seed::byte(
                            ram::g_player_inventory_open(revision).at(player).address(),
                            0,
                        ),
                        Seed::byte(ram::g_player_fall_states(revision).at(player).address(), 0),
                        Seed::word(entity.x().address(), 260),
                        Seed::word(entity.y().address(), 200),
                        Seed::word(entity.height().address(), 0),
                        Seed::byte(entity.height_pixels().address(), 32),
                        Seed::byte(entity.frame_phase().address(), 0),
                        Seed::byte(entity.animation_motion_axes().address(), 0),
                        Seed::long(entity.animation_group_mask().address(), 0),
                        Seed::byte(entity.sprite_state().address(), 0),
                        Seed::word(entity.velocity_x().address(), velocity),
                        Seed::word(entity.velocity_y().address(), velocity),
                        Seed::word(entity.velocity_height().address(), 0),
                    ]);
                }
                for position in [
                    ram::g_camera1_pending_x(revision),
                    ram::g_camera1_pending_y(revision),
                    ram::g_camera2_pending_x(revision),
                    ram::g_camera2_pending_y(revision),
                ] {
                    seeds.push(Seed::word(position.address(), 100));
                }
                for fixed in [
                    ram::g_camera1_follow_x_fixed(revision),
                    ram::g_camera1_follow_y_fixed(revision),
                    ram::g_camera2_follow_x_fixed(revision),
                    ram::g_camera2_follow_y_fixed(revision),
                ] {
                    seeds.push(Seed::long(fixed.address(), 100 << 8));
                }
                passed &= fixture(
                    host,
                    ids::UPDATE_CAMERA_FOLLOWING,
                    format!("following/split-{split}/lives-{lives:?}/velocity-{velocity}"),
                    &seeds,
                    receipts,
                );
            }
        }
    }
    for camera in [0_u32, 1] {
        for (id, vertical) in [
            (ids::TRY_UPDATE_CAMERA_Y, true),
            (ids::TRY_UPDATE_CAMERA_X, false),
        ] {
            let pending = if vertical {
                ram::g_camera_row_upload_pending(revision)
            } else {
                ram::g_camera_column_upload_pending(revision)
            };
            for other in [0_u8, 1] {
                passed &= argument_fixture(
                    host,
                    id,
                    &[camera],
                    format!("flush/camera-{camera}/other-{other}"),
                    &[Seed::byte(pending.at((camera ^ 1) as i32).address(), other)],
                    receipts,
                );
            }
        }
        for position in [8_i16, 9, 100, 1560, 1672, 2360] {
            for split in [0_u8, 1] {
                for pending in [0_u8, 1] {
                    let seeds = [
                        Seed::word(ram::g_camera1_pending_x(revision).address(), position),
                        Seed::word(ram::g_camera2_pending_x(revision).address(), position),
                        Seed::word(ram::g_camera1_pending_y(revision).address(), position),
                        Seed::word(ram::g_camera2_pending_y(revision).address(), position),
                        Seed::byte(ram::g_split_screen_enabled(revision).address(), split),
                        Seed::byte(
                            ram::g_camera_row_upload_pending(revision)
                                .at(camera as i32)
                                .address(),
                            pending,
                        ),
                        Seed::byte(
                            ram::g_camera_column_upload_pending(revision)
                                .at(camera as i32)
                                .address(),
                            pending,
                        ),
                    ];
                    for id in [
                        ids::MOVE_CAMERA_UP,
                        ids::MOVE_CAMERA_DOWN,
                        ids::MOVE_CAMERA_LEFT,
                        ids::MOVE_CAMERA_RIGHT,
                    ] {
                        passed &= argument_fixture(
                            host,
                            id,
                            &[camera],
                            format!(
                                "pixel/camera-{camera}/position-{position}/split-{split}/pending-{pending}"
                            ),
                            &seeds,
                            receipts,
                        );
                    }
                }
            }
        }
    }
    // EnterSplitScreen's actual animation envelope disables both graphics
    // handlers after uploads drain, but it does not mask IRQs or clear display.
    let animation_seeds = [
        Seed::byte(ram::g_v_blank_graphics_enabled(revision).address(), 0),
        Seed::byte(ram::g_h_blank_graphics_enabled(revision).address(), 0),
    ];
    for id in [
        ids::ANIMATE_SPLIT_SCREEN_WIPE,
        ids::ANIMATE_PLAYERS_INTO_SPLIT_VIEWS,
    ] {
        passed &= fixture(
            host,
            id,
            "split/actual-enter-split-graphics-envelope".into(),
            &animation_seeds,
            receipts,
        );
    }
    passed
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().collect();
    let root = Path::new(args.get(1).expect("worktree root"));
    let evidence = Path::new(args.get(2).expect("private evidence directory"));
    let input_path = args.get(3).map_or_else(
        || root.join("sessions/rev00-content-cooperation.tje.json"),
        Into::into,
    );
    let timeline = Arc::new(Timeline::read(&input_path)?);
    let frames = args
        .get(4)
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(10000_u64);
    let frames = if frames == 0 {
        timeline
            .frames()
            .ok_or("full length requires a .tje.json session")?
    } else {
        frames
    };
    let use_fixtures = args.get(5).is_some_and(|value| value == "--fixtures");
    fs::create_dir_all(evidence)?;
    let mut passed = true;
    for revision in [Revision::Rev02, Revision::Rev00] {
        let name = if revision == Revision::Rev02 {
            "rev02"
        } else {
            "rev00"
        };
        let registry =
            HookRegistry::proof_replayed_environment(revision, CANDIDATES, FRAMED_CANDIDATES, &[])?;
        let mut host = Host::new(
            &fs::read(root.join(format!("roms/{name}.bin")))?,
            Mode::Check,
            registry,
        )?;
        host.set_check_failure_policy(CheckFailurePolicy::ContinueOriginal)?;
        host.set_wait_input_poller(timeline.clone());
        for frame in 0..frames {
            if frame < host.frame_count() {
                continue;
            }
            timeline.before_frame(frame, &mut host.context())?;
            host.run_frame(timeline.at(frame)?)?;
        }
        fs::write(
            evidence.join(format!("{name}-natural.json")),
            serde_json::to_vec_pretty(&serde_json::json!({
                "input": input_path, "requested_frames": frames, "actual_frame": host.frame_count(),
                "envelope": "ReplayedEnvironment (modular semantic tier; no replacement/composition approval)",
                "checks": host.checks,
            }))?,
        )?;
        let mut receipts = Vec::new();
        if use_fixtures {
            // Drain actual original upload work BEFORE any fixture inputs are
            // seeded. Never manufacture a ready/pending byte to evade a wait.
            let mut idle = false;
            for _ in 0..32 {
                let boundary = host.settle_fixture_boundary(timeline.at(host.frame_count())?)?;
                receipts.push(serde_json::json!({"label": "settled-original-boundary", "ticks": boundary.ticks,
                    "handlers_completed": boundary.handlers_completed, "entry_frame": host.frame_count()}));
                if ram::g_sprite_graphics_upload_pending(revision).read(&host.context()) == 0 {
                    idle = true;
                    break;
                }
                let frame = host.frame_count();
                timeline.before_frame(frame, &mut host.context())?;
                host.run_frame(timeline.at(frame)?)?;
            }
            if idle {
                passed &= fixtures(&mut host, &mut receipts);
            } else {
                passed = false;
                receipts.push(serde_json::json!({"error":"actual upload service did not become idle; no ready state was fabricated"}));
            }
        }
        fs::write(
            evidence.join(format!("{name}-fixtures.json")),
            serde_json::to_vec_pretty(&receipts)?,
        )?;
        fs::write(
            evidence.join(format!("{name}-final.json")),
            serde_json::to_vec_pretty(&host.checks)?,
        )?;
        passed &= host.checks.mismatches == 0
            && host.checks.uninitialized_stack_reads.is_empty()
            && host.checks.unexercised.is_empty();
        println!(
            "{name}: {} checked calls, {} mismatches, {} unexercised",
            host.checks.calls,
            host.checks.mismatches,
            host.checks.unexercised.len()
        );
    }
    if passed {
        Ok(())
    } else {
        Err("camera replay proof incomplete or rejected; both-revision receipts retained".into())
    }
}
