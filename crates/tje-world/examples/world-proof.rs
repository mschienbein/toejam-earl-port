//! Private generator/terrain overviews, not gameplay screenshots. Never add its
//! output directory to version control. Only runtime user-ROM identifiers enter
//! the generated world, and only terrain-class colors enter these PNGs.
use sha1::{Digest as _, Sha1};
use sha2::Sha256;
use std::{fs, path::Path};
use tje_mem::AddressSpace;
use tje_rom::Rom;
use tje_world::generation::{GeneratedLevel, Generation, GenerationSpec, Phase};
use tje_world::population::PopulationBaseline;
use tje_world::rom::RomResources;
use tje_world::world::codec;
use tje_world::{Density, Dimensions, LevelId, LevelSize, WorldConfig};

fn generate(
    spec: GenerationSpec,
    resources: &RomResources<'_>,
    quantum: u32,
) -> Result<GeneratedLevel, Box<dyn std::error::Error>> {
    let mut generation = Generation::new(spec)?;
    loop {
        match generation.advance(resources, quantum)? {
            Phase::Complete | Phase::Rejected => return Ok(generation.finish()?),
            _ => (),
        }
    }
}
fn screenshot(
    path: &Path,
    floor: &GeneratedLevel,
    level: LevelId,
    resources: &RomResources<'_>,
) -> Result<(), Box<dyn std::error::Error>> {
    let dimensions = floor.geometry.dimensions();
    let width = u32::from(dimensions.width()) * 16;
    let height = u32::from(dimensions.height()) * 8;
    let mut pixels = vec![0_u8; width as usize * height as usize * 3];
    for row in 0..height {
        for column in 0..width {
            let terrain = floor.geometry.terrain_at(
                level,
                column as i32 * 8 + 128,
                row as i32 * 8 + 64,
                resources,
            )? & 0xf0;
            let color = match terrain {
                0x10 => [55, 135, 72],
                0x20 => [180, 140, 65],
                0x80 => [40, 80, 160],
                _ => [10, 12, 24],
            };
            let offset = (row as usize * width as usize + column as usize) * 3;
            pixels[offset..offset + 3].copy_from_slice(&color);
        }
    }
    let anchors = floor.geometry.anchors();
    for (position, color) in [
        (Some(anchors.arrival), [255, 255, 255]),
        (Some(anchors.exit), [255, 80, 80]),
        (anchors.ship_piece, [255, 235, 40]),
    ] {
        if let Some(position) = position {
            let x = usize::from(position.x) * 16 + 8;
            let y = usize::from(position.y) * 8 + 4;
            for dy in -2..=2 {
                for dx in -2..=2 {
                    let offset = (((y as i32 + dy) as usize) * width as usize
                        + (x as i32 + dx) as usize)
                        * 3;
                    pixels[offset..offset + 3].copy_from_slice(&color);
                }
            }
        }
    }
    let mut encoder = png::Encoder::new(fs::File::create(path)?, width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&pixels)?;
    Ok(())
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<_> = std::env::args_os().collect();
    if arguments.len() != 3 {
        return Err("usage: world-proof USER_ROM PRIVATE_OUTPUT_DIRECTORY".into());
    }
    let rom = Rom::import(&fs::read(&arguments[1])?)?;
    let revision = rom.revision();
    let rom_sha1: [u8; 20] = Sha1::digest(rom.bytes()).into();
    let memory = AddressSpace::new(rom.into_bytes(), revision);
    let resources = RomResources::new(&memory)?;
    let output = Path::new(&arguments[2]);
    fs::create_dir_all(output)?;
    let level = LevelId::new(5).unwrap();
    let initial = PopulationBaseline {
        walkable_tiles: 19 * 26 * 64,
        ordinary_items: 12,
        earthlings: 10,
        reserved_items: 2,
        ship_piece: true,
        mailbox: true,
    };
    let control_config = WorldConfig {
        root_seed: 42,
        rom_sha1,
        size: LevelSize::Original,
        density: Density::Original,
    };
    let control = generate(
        GenerationSpec {
            config: control_config.clone(),
            level,
            baseline: initial,
        },
        &resources,
        1024,
    )?;
    let baseline = PopulationBaseline {
        walkable_tiles: control.walkable_tiles,
        ..initial
    };
    println!(
        "{} stock-dimension ExpandedV1 control: walkable_tiles={} accepted_attempt={} (not original-generator equivalence)",
        revision.name(),
        control.walkable_tiles,
        control.accepted_attempt
    );
    let mut accepted = 0;
    let mut rejected = 0;
    for (label, size) in [
        ("original-control", LevelSize::Original),
        ("expanded", LevelSize::Expanded),
        ("large", LevelSize::Large),
        ("maximum", LevelSize::Custom(Dimensions::new(64, 64)?)),
        ("asymmetric", LevelSize::Custom(Dimensions::new(8, 64)?)),
    ] {
        for seed in [0, 1, 42, u64::MAX] {
            let config = WorldConfig {
                root_seed: seed,
                size,
                density: Density::Normal,
                ..control_config.clone()
            };
            let spec = GenerationSpec {
                config,
                level,
                baseline,
            };
            let first = generate(spec.clone(), &resources, 1024);
            let second = generate(spec, &resources, 257);
            match (first, second) {
                (Ok(first), Ok(second)) => {
                    let a = bincode::encode_to_vec(&first, codec())?;
                    let b = bincode::encode_to_vec(&second, codec())?;
                    if a != b {
                        return Err("generator differs with deterministic work quantum".into());
                    }
                    let (restored, used): (GeneratedLevel, usize) =
                        bincode::decode_from_slice(&a, codec())?;
                    restored.validate()?;
                    if used != a.len() || restored != first {
                        return Err("private generated snapshot round-trip differs".into());
                    }
                    let hash = Sha256::digest(&a);
                    println!(
                        "{label} seed={seed} attempt={} area={} walkable={} items={} earthlings={} bytes={} sha256={hash:x}",
                        first.accepted_attempt,
                        first.geometry.dimensions().area(),
                        first.walkable_tiles,
                        first.population.items.len(),
                        first.population.earthlings.len(),
                        a.len()
                    );
                    if seed == 42 {
                        screenshot(
                            &output.join(format!("{label}-terrain-overview.png")),
                            &first,
                            level,
                            &resources,
                        )?;
                    }
                    accepted += 1;
                }
                (Err(first), Err(second)) if first.to_string() == second.to_string() => {
                    println!("{label} seed={seed} deterministic rejection: {first}");
                    rejected += 1;
                }
                _ => {
                    return Err(
                        "private generation success/failure differs with work quantum".into(),
                    );
                }
            }
        }
    }
    println!(
        "private proof: {accepted} accepted, {rejected} rejected; no native admission or gameplay claimed"
    );
    Ok(())
}
