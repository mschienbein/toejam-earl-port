//! Private, background-only extended-view proof using a user's ROM/session.
//! No frontend installs or enables this example. Output must remain private.
use clap::Parser;
use genesis_config::{
    GenesisButton, GenesisController, GenesisControllerType, GenesisEmulatorConfig, GenesisInputs,
};
use std::{
    error::Error,
    fs::OpenOptions,
    io::{BufWriter, Write},
    path::PathBuf,
};
use tje_engine_cranelift::{CacheConfig, load_engine};
use tje_extended_view::{
    ExperimentalWidth, FULL_HEIGHT, IndexedPixel, MapRenderer, ViewMode, Viewport, WorldView,
};
use tje_mem::Bus;
use tje_rom::{Revision, Rom};
use tje_session::{CONNECTED, Session};

#[derive(Parser)]
#[command(about = "Private terrain-only widescreen proof; not a selectable game mode")]
struct Args {
    #[arg(long)]
    rom: PathBuf,
    #[arg(long)]
    session: PathBuf,
    /// Number of completed frames from power-on; choose a normal gameplay frame.
    #[arg(long)]
    frame: u64,
    /// User-private PPM output; an existing file will never be overwritten.
    #[arg(long)]
    output: PathBuf,
    #[arg(long)]
    data_dir: Option<PathBuf>,
    /// Explicitly experimental; otherwise the extra view is 400x224.
    #[arg(long)]
    experimental_width: Option<u16>,
    /// Omit the original center, showing that this proof has no entities or HUD.
    #[arg(long)]
    background_only: bool,
    /// Diagnostic gameplay-changing visibility hooks; still no host sprite renderer.
    #[cfg(feature = "enhanced-mode")]
    #[arg(long)]
    enhanced_culling: bool,
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

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();
    let mode = match args.experimental_width {
        Some(width) => ViewMode::Experimental(ExperimentalWidth::new(width)?),
        None => ViewMode::Widescreen,
    };
    let rom = Rom::import(&std::fs::read(&args.rom)?)?;
    let revision = rom.revision();
    let session = Session::from_json(&std::fs::read_to_string(&args.session)?)?;
    if Revision::from(session.revision) != revision {
        return Err("session and cartridge revisions differ".into());
    }
    if args.frame == 0 || args.frame > session.frames {
        return Err("--frame must be between 1 and the session's completed frame count".into());
    }
    let cache = match args.data_dir {
        Some(data_dir) => CacheConfig { data_dir },
        None => CacheConfig::for_current_user()?,
    };
    let mut engine = load_engine(rom, &GenesisEmulatorConfig::default(), &cache, |_| {})?;
    #[cfg(feature = "enhanced-mode")]
    let mut enhanced = if args.enhanced_culling {
        let profile = match mode.width() {
            400 => tje_extended_view::enhanced::WIDESCREEN_PROFILE,
            512 => tje_extended_view::enhanced::EXPERIMENTAL_512_PROFILE,
            _ => return Err("behavioral profiles exist only for widths 400 and 512".into()),
        };
        tje_engine_runtime::enhanced::EnhancedMode::new(profile, revision)?
    } else {
        tje_engine_runtime::enhanced::EnhancedMode::default()
    };
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
                _ => unreachable!("validated session write width"),
            }
        }
        #[cfg(feature = "enhanced-mode")]
        enhanced.play_frame(&mut engine, inputs(pads))?;
        #[cfg(not(feature = "enhanced-mode"))]
        engine.play_frame(inputs(pads))?;
    }
    let devices = engine.devices();
    let memory = devices.memory();
    if tje_mem::ram::g_v_blank_func(revision)
        .read(memory)
        .address()
        != 0
    {
        return Err("custom scene VBlank is active; choose an ordinary gameplay frame".into());
    }
    let (vram, cram, _) = devices.vdp().hybrid_memories();
    let renderer = MapRenderer::new(memory, vram)?;
    let mut output = vec![IndexedPixel::default(); mode.width() * FULL_HEIGHT];
    let primary = WorldView::committed(memory, Viewport::Primary)?;
    let primary_len = primary.height() * mode.width();
    renderer.render_background(primary, mode, &mut output[..primary_len])?;
    if primary.height() != FULL_HEIGHT {
        renderer.render_background(
            WorldView::committed(memory, Viewport::Secondary)?,
            mode,
            &mut output[primary_len..],
        )?;
    }
    let (original, size) = engine.frame_buffer();
    if size.width != 320 || size.height != 224 {
        return Err("preview requires the original borderless 320x224 output".into());
    }
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args.output)?;
    let mut writer = BufWriter::new(file);
    write!(writer, "P6\n{} {}\n255\n", mode.width(), FULL_HEIGHT)?;
    // Hardware color scaling, not a game palette. Matches the default VDP's
    // normal non-linear scale; this diagnostic does not model shadow/highlight.
    let channel = [0, 52, 87, 116, 144, 172, 206, 255];
    let margin = mode.margin() as usize;
    for y in 0..FULL_HEIGHT {
        for x in 0..mode.width() {
            let rgb = if !args.background_only && (margin..margin + 320).contains(&x) {
                let color = original[y * 320 + x - margin];
                [color.r, color.g, color.b]
            } else {
                let pixel = output[y * mode.width() + x];
                let index = if pixel.transparent() {
                    0
                } else {
                    pixel.palette_index()
                };
                let color = cram[index];
                [
                    channel[((color >> 1) & 7) as usize],
                    channel[((color >> 5) & 7) as usize],
                    channel[((color >> 9) & 7) as usize],
                ]
            };
            writer.write_all(&rgb)?;
        }
    }
    writer.flush()?;
    println!(
        "{}: frame {}, {}x{}; wings contain background only, not widened entities or HUD",
        revision,
        args.frame,
        mode.width(),
        FULL_HEIGHT
    );
    Ok(())
}
