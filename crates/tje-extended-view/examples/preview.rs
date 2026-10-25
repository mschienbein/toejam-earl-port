//! Private completed-frame view proof. All images and observations stay private.
use clap::Parser;
use genesis_config::{
    GenesisButton, GenesisController, GenesisControllerType, GenesisEmulatorConfig, GenesisInputs,
};
use sha1::{Digest, Sha1};
use std::{
    error::Error,
    fs::OpenOptions,
    io::{BufWriter, Write},
    path::PathBuf,
};
use tje_engine_cranelift::{CacheConfig, load_engine};
use tje_engine_runtime::enhanced::EnhancedMode;
use tje_extended_view::{
    ExperimentalWidth, ViewMode, enhanced,
    presentation::{Presenter, PublishedView},
};
use tje_mem::Bus;
use tje_rom::{Revision, Rom};
use tje_session::{CONNECTED, Session};

#[derive(Parser)]
#[command(about = "Private complete extended-view sprite/scene replay; supply your own ROM")]
struct Args {
    #[arg(long)]
    rom: PathBuf,
    #[arg(long)]
    session: PathBuf,
    #[arg(long)]
    frame: u64,
    /// Private PPM output. Existing files are never overwritten.
    #[arg(long)]
    output: PathBuf,
    #[arg(long)]
    data_dir: Option<PathBuf>,
    /// Explicitly experimental 408..512, in 8-pixel steps; ordinary mode is 400.
    #[arg(long, conflicts_with = "faithful")]
    experimental_width: Option<u16>,
    /// Original engine and exact 320x224 output, for before/after evidence.
    #[arg(long)]
    faithful: bool,
    /// Private per-frame original framebuffer/audio SHA-1 observations.
    #[arg(long)]
    hash_output: Option<PathBuf>,
}
fn inputs(pads: [u16; 2]) -> GenesisInputs {
    use GenesisButton::*;
    let [p1, p2] = pads.map(|pad| {
        let mut controller = if pad & CONNECTED != 0 {
            GenesisController::new(GenesisControllerType::ThreeButton)
        } else {
            GenesisController::None
        };
        for (bit, button) in [Up, Down, Left, Right, A, B, C, Start]
            .into_iter()
            .enumerate()
        {
            controller.set_field(button, pad & (1 << bit) != 0);
        }
        controller
    });
    GenesisInputs { p1, p2 }
}
fn private_writer(path: &PathBuf) -> std::io::Result<BufWriter<std::fs::File>> {
    Ok(BufWriter::new(
        OpenOptions::new().write(true).create_new(true).open(path)?,
    ))
}
fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();
    let mode = if args.faithful {
        ViewMode::Faithful
    } else {
        match args.experimental_width {
            Some(width) => ViewMode::Experimental(ExperimentalWidth::new(width)?),
            None => ViewMode::Widescreen,
        }
    };
    let rom = Rom::import(&std::fs::read(&args.rom)?)?;
    let revision = rom.revision();
    let session = Session::from_json(&std::fs::read_to_string(&args.session)?)?;
    if Revision::from(session.revision) != revision {
        return Err("session and cartridge revisions differ".into());
    }
    if args.frame == 0 || args.frame > session.frames {
        return Err("--frame must be 1..=session completed frame count".into());
    }
    let cache = match args.data_dir {
        Some(data_dir) => CacheConfig { data_dir },
        None => CacheConfig::for_current_user()?,
    };
    let mut engine = load_engine(rom, &GenesisEmulatorConfig::default(), &cache, |_| {})?;
    let mut enhanced = if args.faithful {
        EnhancedMode::default()
    } else {
        let mut enhanced = EnhancedMode::new(
            enhanced::profile(mode.width() as u16).ok_or("unsupported view profile")?,
            revision,
        )?;
        enhanced::configure(&mut enhanced, mode.width() as u16)?;
        enhanced
    };
    let mut hashes = args.hash_output.as_ref().map(private_writer).transpose()?;
    let mut writes = session.writes().iter().peekable();
    for (frame, pads) in session.inputs().take(args.frame as usize).enumerate() {
        while writes
            .peek()
            .is_some_and(|write| write.before_frame == frame as u64)
        {
            let write = writes.next().unwrap();
            let memory = engine.devices_mut().memory_mut();
            match write.width {
                1 => memory.write_u8(write.address, write.value as u8),
                2 => memory.write_u16(write.address, write.value as u16),
                4 => memory.write_u32(write.address, write.value),
                _ => unreachable!("validated RAM recipe width"),
            }
        }
        enhanced.play_frame(&mut engine, inputs(pads))?;
        if let Some(hashes) = &mut hashes {
            let (pixels, size) = engine.frame_buffer();
            let mut video = Sha1::new();
            for pixel in &pixels[..size.len() as usize] {
                video.update([pixel.r, pixel.g, pixel.b, pixel.a]);
            }
            let mut audio = Sha1::new();
            for sample in engine.devices().audio_samples() {
                for channel in sample {
                    audio.update(channel.to_bits().to_le_bytes());
                }
            }
            writeln!(
                hashes,
                "{} {:x} {:x}",
                frame + 1,
                video.finalize(),
                audio.finalize()
            )?;
        }
    }
    if let Some(hashes) = &mut hashes {
        hashes.flush()?;
    }
    let devices = engine.devices();
    let (original, size) = engine.frame_buffer();
    if size.width != 320 || size.height != 224 {
        return Err("expected borderless original 320x224 output".into());
    }
    let mut presenter = (!args.faithful).then(|| Presenter::new(mode));
    if let Some(presenter) = &mut presenter {
        let (vram, cram, _) = devices.vdp().hybrid_memories();
        presenter.set_shadow_highlight(devices.vdp().hybrid_registers().shadow_highlight_flag);
        presenter.compose(
            devices.memory(),
            vram,
            cram,
            original,
            enhanced.sidecars().get::<PublishedView>()?,
            &[],
        )?;
    }
    let (pixels, size) = presenter
        .as_ref()
        .map_or((original, size), Presenter::frame);
    let mut writer = private_writer(&args.output)?;
    write!(writer, "P6\n{} {}\n255\n", size.width, size.height)?;
    for pixel in pixels {
        writer.write_all(&[pixel.r, pixel.g, pixel.b])?;
    }
    writer.flush()?;
    println!(
        "{}: completed frame {}, {}x{}, {}",
        revision,
        args.frame,
        size.width,
        size.height,
        enhanced.profile_id()
    );
    Ok(())
}
