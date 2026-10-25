//! Explicit local/headless playtest: ROM COURSE RECORDED_INPUTS [FRAMES].
//! The input recording supplies normal pads/menu navigation; it is not a course
//! export or an enhanced replay identity. No file is uploaded or written.
use genesis_config::{
    GenesisButton, GenesisController, GenesisControllerType, GenesisEmulatorConfig, GenesisInputs,
};
use std::{fs::File, io::Read};
use tje_engine_cranelift::{CacheConfig, load_engine};
use tje_maker::{
    MAX_BODY_BYTES, ValidatedCourse,
    enhanced::{MakerState, RunStatus, new_mode},
};
use tje_rom::Rom;
use tje_session::{CONNECTED, Session};

fn bounded(path: &str, max: usize) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(max as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > max {
        return Err(format!("{path}: input exceeds {max} bytes").into());
    }
    Ok(bytes)
}
fn inputs(pads: [u16; 2]) -> GenesisInputs {
    let pads = pads.map(|bits| {
        let mut pad = if bits & CONNECTED != 0 {
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
            pad.set_field(button, bits & (1 << index) != 0);
        }
        pad
    });
    GenesisInputs {
        p1: pads[0],
        p2: pads[1],
    }
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if !(4..=5).contains(&args.len()) {
        return Err("usage: play_course ROM COURSE.tjemaker.json INPUTS.tje.json [FRAMES]".into());
    }
    let rom = Rom::import(&bounded(&args[1], 2 * 1024 * 1024)?)?;
    let course = ValidatedCourse::from_json(&bounded(&args[2], MAX_BODY_BYTES)?)?;
    let session = Session::from_json(std::str::from_utf8(&bounded(&args[3], 16 * 1024 * 1024)?)?)?;
    let frames = args
        .get(4)
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(session.frames);
    let mut engine = load_engine(
        rom,
        &GenesisEmulatorConfig::default(),
        &CacheConfig::for_current_user()?,
        |_| {},
    )?;
    let mut mode = new_mode(&course, engine.devices().memory())?;
    for pads in session.inputs().take(frames as usize) {
        mode.play_frame(&mut engine, inputs(pads))?;
        let state = mode.sidecars().get::<MakerState>()?;
        if matches!(state.status(), RunStatus::Complete | RunStatus::Failed) {
            break;
        }
    }
    println!(
        "course={} sha256={} frames={} status={:?}",
        course.course().title,
        course.digest_hex(),
        engine.frame_count(),
        mode.sidecars().get::<MakerState>()?.status()
    );
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
