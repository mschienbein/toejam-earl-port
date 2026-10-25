//! Private evidence runner: ROM SESSION HASH_OUTPUT fixed|fixed-pilot|disabled|faithful [FRAMES].
//! A faithful controller session is only an input source for a Fixed playtest,
//! never relabelled as an enhanced recording. Outputs contain hashes, not assets.
use genesis_config::{
    GenesisButton, GenesisController, GenesisControllerType, GenesisEmulatorConfig, GenesisInputs,
};
use sha1::{Digest, Sha1};
use std::{
    fs::OpenOptions,
    io::{BufWriter, Write},
};
use tje_engine_cranelift::{CacheConfig, load_engine};
use tje_engine_runtime::enhanced::EnhancedMode;
use tje_extras::fixed::{self, Settings};
use tje_mem::{Bus, ram};
use tje_rom::Rom;
use tje_session::{CONNECTED, Session};
fn inputs(pads: [u16; 2]) -> GenesisInputs {
    let [p1, p2] = pads.map(|bits| {
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
    GenesisInputs { p1, p2 }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if !(5..=6).contains(&args.len()) {
        return Err(
            "usage: profile_session ROM SESSION HASH_OUTPUT fixed|fixed-pilot|disabled|faithful [FRAMES]"
                .into(),
        );
    }
    let rom = Rom::import(&std::fs::read(&args[1])?)?;
    let revision = rom.revision();
    let session = Session::from_json(&std::fs::read_to_string(&args[2])?)?;
    if tje_mem::Revision::from(session.revision) != revision {
        return Err("session/ROM revisions differ".into());
    }
    let frames = args
        .get(5)
        .map(|s| s.parse::<u64>())
        .transpose()?
        .unwrap_or(session.frames);
    if frames == 0 || frames > session.frames {
        return Err("frame count must be a nonempty committed session prefix".into());
    }
    let mut engine = load_engine(
        rom,
        &GenesisEmulatorConfig::default(),
        &CacheConfig::for_current_user()?,
        |_| {},
    )?;
    engine.set_audio_sample_rate(48000)?;
    let settings = Settings {
        enabled: matches!(args[4].as_str(), "fixed" | "fixed-pilot"),
        ..Default::default()
    };
    let mut mode = match args[4].as_str() {
        "faithful" => EnhancedMode::default(),
        "fixed" | "fixed-pilot" | "disabled" => fixed::new_mode(revision, settings)?,
        _ => return Err("unknown profile selection".into()),
    };
    let mut out = BufWriter::new(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&args[3])?,
    );
    let mut all_video = Sha1::new();
    let mut all_audio = Sha1::new();
    let pilot = args[4] == "fixed-pilot";
    if pilot && !session.writes().is_empty() {
        return Err("the Fixed pilot requires power-on inputs without RAM recipes".into());
    }
    let mut entered_world = None;
    let mut world_frames = 0_u64;
    let mut moved_frames = 0_u64;
    let mut writes = session.writes().iter().peekable();
    for (frame, pads) in session.inputs().take(frames as usize).enumerate() {
        while writes
            .peek()
            .is_some_and(|w| w.before_frame == frame as u64)
        {
            let write = writes.next().unwrap();
            let memory = engine.devices_mut().memory_mut();
            match write.width {
                1 => memory.write_u8(write.address, write.value as u8),
                2 => memory.write_u16(write.address, write.value as u16),
                4 => memory.write_u32(write.address, write.value),
                _ => unreachable!("validated recipe"),
            }
        }
        let memory = engine.devices().memory();
        let player = ram::g_player_info(revision).at(0);
        let live_world = player.level().read(memory) == 1
            && ram::g_player1_map_level(revision).read(memory) == 1
            && ram::g_player_lives(revision).at(0).read(memory) >= 0
            && ram::g_player_health(revision).at(0).read(memory) > 0;
        let before_position = (player.x().read(memory), player.y().read(memory));
        let pads = if pilot {
            if live_world {
                world_frames += 1;
                entered_world.get_or_insert(frame);
            }
            let buttons = if let Some(start) = entered_world {
                match frame.saturating_sub(start + 120) % 48 {
                    _ if frame < start + 120 => 0,
                    0..=11 => 8,
                    12..=23 => 4,
                    _ => 0,
                }
            } else if frame % 60 < 8 {
                128 // Start skips scenes and accepts each default menu; release between edges.
            } else {
                0
            };
            [CONNECTED | buttons, CONNECTED]
        } else {
            pads
        };
        let samples = mode.play_frame(&mut engine, inputs(pads))?;
        let mut audio = Sha1::new();
        for sample in samples {
            for channel in sample {
                audio.update(channel.to_bits().to_le_bytes());
            }
        }
        let mut video = Sha1::new();
        let (pixels, size) = engine.frame_buffer();
        for color in &pixels[..size.len() as usize] {
            video.update([color.r, color.g, color.b, color.a]);
        }
        let video = video.finalize();
        let audio = audio.finalize();
        all_video.update(video);
        all_audio.update(audio);
        writeln!(out, "{} {:x} {:x}", frame + 1, video, audio)?;
        if pilot && live_world {
            let memory = engine.devices().memory();
            if before_position != (player.x().read(memory), player.y().read(memory)) {
                moved_frames += 1;
            }
        }
        if frame == frames as usize / 2 {
            let image = mode.snapshot(&engine)?;
            mode.restore(&mut engine, &image)?;
            assert_eq!(
                mode.snapshot(&engine)?.as_bytes(),
                image.as_bytes(),
                "atomic save/restore canonical bytes"
            );
            if settings.enabled {
                let mut other_settings = settings;
                other_settings.set("B001", false)?;
                let mut other = fixed::new_mode(revision, other_settings)?;
                assert!(
                    other.restore(&mut engine, &image).is_err(),
                    "immutable fix selection must reject another mask"
                );
                mode.restore(&mut engine, &image)?;
            }
        }
    }
    out.flush()?;
    if pilot && (world_frames < 600 || moved_frames < 20) {
        return Err(format!("Fixed pilot did not establish normal live play: world_frames={world_frames} moved_frames={moved_frames}").into());
    }
    let memory = engine.devices().memory();
    println!(
        "revision={} profile={} frames={} video={:x} audio={:x} players={:?} save_restore=pass input={} world_frames={} moved_frames={}",
        revision,
        mode.profile_id(),
        engine.frame_count(),
        all_video.finalize(),
        all_audio.finalize(),
        (0..2)
            .map(|p| {
                let a = ram::g_player_info(revision).at(p);
                (
                    a.level().read(memory),
                    a.x().read(memory),
                    a.y().read(memory),
                )
            })
            .collect::<Vec<_>>(),
        if pilot {
            "adaptive-fixed-pilot"
        } else {
            "committed-session"
        },
        world_frames,
        moved_frames
    );
    Ok(())
}
