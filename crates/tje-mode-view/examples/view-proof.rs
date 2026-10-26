//! Private composition fixtures from the user's ROM. These are NOT gameplay or
//! four-player campaign evidence; actor/UI arrangements are newly authored here.
use clap::Parser;
use genesis_config::{
    GenesisButton, GenesisController, GenesisControllerType, GenesisEmulatorConfig, GenesisInputs,
};
use sha1::{Digest, Sha1};
use std::{
    error::Error,
    fs::{self, OpenOptions},
    io::{BufWriter, Write},
    path::PathBuf,
};
use tje_engine_cranelift::{CacheConfig, load_engine};
use tje_four_player::{Character, SeatId};
use tje_mem::{Bus, ram};
use tje_mode_view::{
    Actor, Compositor, Publisher, View,
    animation::Program,
    overlay::{Hud, Inventory, InventoryKind, Item, Map, SeatOverlay, Speech},
    source::WorldSource,
};
use tje_rom::Rom;
use tje_simulation::{ActorId, Phase, PublicationStamp};
use tje_world::{Anchors, Cell, CellPosition, Dimensions, LevelGeometry, LevelId};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    rom: PathBuf,
    #[arg(long)]
    session: PathBuf,
    #[arg(long, default_value_t = 1200)]
    frames: u64,
    #[arg(long)]
    output: PathBuf,
    #[arg(long)]
    data_dir: Option<PathBuf>,
}
fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();
    let rom = Rom::import(&fs::read(&args.rom)?)?;
    let revision = rom.revision();
    let session = tje_session::Session::from_json(&fs::read_to_string(args.session)?)?;
    if tje_rom::Revision::from(session.revision) != revision
        || args.frames == 0
        || args.frames > session.frames
    {
        return Err("session/revision/frame mismatch".into());
    }
    fs::create_dir_all(&args.output)?;
    let mut hashes = BufWriter::new(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(args.output.join("faithful-frame-audio.sha1"))?,
    );
    let cache = args
        .data_dir
        .map(|data_dir| CacheConfig { data_dir })
        .map_or_else(CacheConfig::for_current_user, Ok)?;
    let mut engine = load_engine(rom, &GenesisEmulatorConfig::default(), &cache, |_| {})?;
    let mut writes = session.writes().iter().peekable();
    for (frame, words) in session.inputs().take(args.frames as usize).enumerate() {
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
                _ => unreachable!("validated RAM recipe"),
            }
        }
        let pads = words.map(|word| {
            let mut pad = if word & tje_session::CONNECTED != 0 {
                GenesisController::new(GenesisControllerType::ThreeButton)
            } else {
                GenesisController::None
            };
            for (bit, button) in [
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
                pad.set_field(button, word & (1 << bit) != 0);
            }
            pad
        });
        engine.play_frame(GenesisInputs {
            p1: pads[0],
            p2: pads[1],
        })?;
        let (pixels, size) = engine.frame_buffer();
        let mut video = Sha1::new();
        let mut audio = Sha1::new();
        for pixel in &pixels[..size.len() as usize] {
            video.update([pixel.r, pixel.g, pixel.b, pixel.a]);
        }
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
    hashes.flush()?;
    let devices = engine.devices();
    let memory = devices.memory();
    let (vram, cram, _) = devices.vdp().hybrid_memories();
    let mut cells = Vec::with_capacity(Dimensions::ORIGINAL.area());
    for row in 0..26 {
        for column in 0..19 {
            cells.push(Cell::from_word(
                ram::g_player1_level_map(revision)
                    .at(row)
                    .at(column)
                    .read(memory)
                    & 0x1fff,
            )?);
        }
    }
    let geometry = LevelGeometry::new(
        Dimensions::ORIGINAL,
        cells,
        vec![0; 19 * 26],
        Anchors {
            arrival: CellPosition { x: 1, y: 1 },
            exit: CellPosition { x: 17, y: 24 },
            ship_piece: None,
            regions: Vec::new(),
        },
    )?;
    let floor_ids = [2, 3, 4, 5].map(|level| LevelId::new(level).unwrap());
    // A fixture borrows the same imported geometry under independent level IDs;
    // it does not claim that four original map banks or heroes were simulated.
    let levels = std::array::from_fn::<_, 4, _>(|index| (floor_ids[index], &geometry));
    let mut programs = Vec::new();
    for character in Character::DEFAULT_ROSTER {
        let descriptor = character.archetype.animation_descriptors(revision);
        let sequence = memory.read_u32(descriptor);
        let frames = memory.read_u32(descriptor + 4);
        programs.push(Program::import(memory.rom(), sequence, frames)?);
    }
    let mut renderer = Compositor::new();
    for (epoch, name, disjoint, overlays) in [
        (1, "dense-same-level", false, false),
        (2, "dense-disjoint-levels", true, false),
        (3, "four-overlays", true, true),
    ] {
        let stamp = PublicationStamp {
            hardware_frame: args.frames,
            phase: Phase::PresentationA,
            epoch,
        };
        let camera = [
            i32::from(ram::g_camera1_committed_x(revision).read(memory)) - 40,
            i32::from(ram::g_camera1_committed_y(revision).read(memory)),
        ];
        let views = std::array::from_fn(|seat| View::World {
            seat: SeatId::ALL[seat],
            level: floor_ids[if disjoint { seat } else { 0 }],
            origin: [camera[0] + seat as i32 * 16, camera[1]],
            attributes: ram::g_camera_tile_attributes(revision).read(memory),
            starfield: true,
        });
        let mut actors = Vec::with_capacity(516);
        let mut ui: [SeatOverlay; 4] = std::array::from_fn(|_| SeatOverlay::default());
        for seat in 0..4 {
            let (cursor, step) = programs[seat].start(8)?;
            let frame = programs[seat].frame(step.frame)?;
            for index in 0..129 {
                let is_hero = index == 0;
                let x = if is_hero {
                    200
                } else {
                    ((index - 1) % 32) as i32 * 12 + 4
                };
                let y = if is_hero {
                    150
                } else {
                    ((index - 1) / 32) as i32 * 28 + 50
                };
                actors.push(Actor {
                    id: tje_mode_view::packet::DrawId::Actor(
                        ActorId::new((seat * 129 + index) as u16, 1).unwrap(),
                    ),
                    level: floor_ids[if disjoint { seat } else { 0 }],
                    position: [camera[0] + x + seat as i32 * 16, camera[1] + y],
                    elevation: 0,
                    draw_late: false,
                    flip_horizontal: cursor.flip_horizontal,
                    flip_vertical: false,
                    high_priority: true,
                    palette_bank: 0,
                    character: Some(Character::DEFAULT_ROSTER[seat]),
                    seat: is_hero.then_some(SeatId::ALL[seat]),
                    mask_pixels: false,
                    submerged: false,
                    count: frame.count,
                    pieces: frame.pieces,
                    shadow: None,
                });
            }
            ui[seat].hud = Some(Hud {
                seat: SeatId::ALL[seat],
                level: floor_ids[if disjoint { seat } else { 0 }],
                health: 12 + seat as u16 * 4,
                maximum_health: 31,
                lives: 3,
                rank: 1,
                bucks: 10 + seat as u16,
                pause_requested: false,
            });
        }
        if overlays {
            ui[0].inventory = Some(Inventory {
                kind: InventoryKind::Inventory,
                selected: 3,
                slots: [Some(Item {
                    wrapper: 4,
                    identified: false,
                    icon: None,
                    price: None,
                    available: true,
                }); 16],
            });
            ui[1].map = Some(Map {
                level: floor_ids[1],
                dimensions: [19, 26],
                discovery: [u64::MAX; 9],
                discovery_columns: 8,
                discovery_rows: 7,
                markers: [None; 68],
            });
            ui[2].speech = Speech::new(b"PRIVATE COMPOSITION FIXTURE", true, true)?;
            ui[3].hud.as_mut().unwrap().pause_requested = true;
        }
        let mut publisher = Publisher::new();
        publisher.publish(stamp, views, &actors, &ui, &[])?;
        let source = WorldSource::new(stamp, memory, vram, cram, &levels)?;
        renderer.compose(publisher.current(), &source)?;
        let (pixels, size) = renderer.frame();
        let mut bytes = Vec::with_capacity(pixels.len() * 4);
        for pixel in pixels {
            bytes.extend_from_slice(&[pixel.r, pixel.g, pixel.b, pixel.a]);
        }
        let path = args.output.join(format!("{name}.png"));
        let mut encoder = png::Encoder::new(
            OpenOptions::new().write(true).create_new(true).open(path)?,
            size.width,
            size.height,
        );
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&bytes)?;
        println!(
            "{} {name} actors={} canvas={}x{} rgba_sha1={:x}; composition fixture, NOT gameplay",
            revision,
            actors.len(),
            size.width,
            size.height,
            Sha1::digest(&bytes)
        );
        let before = bytes;
        renderer.evict_resources();
        renderer.compose(publisher.current(), &source)?;
        assert!(
            renderer
                .frame()
                .0
                .iter()
                .zip(before.as_chunks::<4>().0)
                .all(|(pixel, bytes)| [pixel.r, pixel.g, pixel.b, pixel.a] == *bytes)
        );
    }
    Ok(())
}
