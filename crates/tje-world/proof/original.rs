//! Private, ROM-required OriginalV1 semantic proof. Compile this file as an
//! external binary with path dependencies tje-world (private-proof), tje-game,
//! tje-mem, tje-rom, tje-engine-verify, sha1, sha2, serde_json, bincode and anyhow.
//! Usage: original-proof REV00_ROM REV02_ROM PRIVATE_RECEIPT_JSON.
//! Only receipts are written: never maps, item tables, ROM bytes or screenshots.
use anyhow::{Result, bail, ensure};
use sha1::{Digest, Sha1};
use sha2::Sha256;
use std::{fs, path::Path};
use tje_engine_verify::{Machine, RuntimeMachine, ir_backend::IrBackend};
use tje_game::{FunctionId, ids};
use tje_mem::{Bus, ram};
use tje_rom::Rom;
use tje_world::{
    LevelId,
    original::{
        CampaignSeeds,
        geometry::{OriginalGeneration, OriginalGeometry, OriginalPhase, OriginalSpec},
        items::{ItemPhase, OriginalItems, StockSpecialPositions},
    },
    rom::RomResources,
    world::codec,
};

const RETURN: u32 = 0xfffffe;
fn call(machine: &mut RuntimeMachine<IrBackend>, id: FunctionId, args: &[u32]) -> Result<u64> {
    let revision = machine.engine().devices().memory().revision();
    let pc = id.metadata().address(revision);
    let stack = 0xfffe00 - 4 * (args.len() as u32 + 1);
    {
        let memory = machine.engine_mut().devices_mut().memory_mut();
        memory.write_u32(stack, RETURN);
        for (index, arg) in args.iter().enumerate() {
            memory.write_u32(stack + 4 + index as u32 * 4, *arg);
        }
    }
    let prefetch = machine.engine().devices().memory().read_u16(pc);
    {
        let cpu = machine.engine_mut().cpu_mut();
        cpu.sr = 0x2700;
        cpu.write_address(7, stack);
        cpu.pc = pc;
        cpu.prefetch = u32::from(prefetch);
        cpu.pending_interrupt = 0;
        cpu.stopped = 0;
        cpu.frozen = 0;
    }
    for steps in 0..50_000_000_u64 {
        if machine.engine().cpu().pc == RETURN {
            return Ok(steps);
        }
        Machine::step(machine, [0; 2])?;
    }
    bail!(
        "original {} exceeded bounded proof instruction count at {:06x}",
        id.metadata().name,
        machine.engine().cpu().pc
    )
}
fn typed(
    spec: OriginalSpec,
    resources: &RomResources<'_>,
    quantum: u32,
) -> Result<(OriginalGeometry, u64)> {
    let mut generation = OriginalGeneration::new(spec)?;
    let mut previous = generation.phase();
    while !matches!(
        generation.phase(),
        OriginalPhase::Complete | OriginalPhase::Rejected
    ) {
        generation.advance(resources, quantum)?;
        if generation.phase() != previous {
            let bytes = bincode::encode_to_vec(&generation, codec())?;
            let (restored, used): (OriginalGeneration, usize) =
                bincode::decode_from_slice(&bytes, codec())?;
            ensure!(used == bytes.len());
            restored.validate()?;
            generation = restored;
            previous = generation.phase();
        }
    }
    let work = generation.total_work();
    Ok((generation.finish()?, work))
}
fn setup(machine: &mut RuntimeMachine<IrBackend>, level: LevelId, seed: u16) -> Result<()> {
    let revision = machine.engine().devices().memory().revision();
    {
        let memory = machine.engine_mut().devices_mut().memory_mut();
        memory.ram_mut().fill(0);
        ram::g_world_type(revision).write(memory, 1);
        ram::g_rng_state(revision).write(memory, 12345);
        for seat in 0..2 {
            ram::g_player_lives(revision).at(seat).write(memory, 3);
        }
        for bank in 0..2 {
            for slot in 0..32 {
                ram::g_object_table(revision)
                    .at(bank)
                    .at(slot)
                    .kind()
                    .write(memory, -1);
            }
            for slot in 0..20 {
                ram::g_earthling_banks(revision)
                    .at(bank)
                    .at(slot)
                    .kind()
                    .write(memory, -1);
            }
        }
    }
    call(machine, ids::GENERATE_LEVEL_SEEDS, &[])?;
    call(machine, ids::ASSIGN_SHIP_PIECE_LEVELS, &[])?;
    {
        let memory = machine.engine_mut().devices_mut().memory_mut();
        ram::g_player1_map_level(revision).write(memory, -1);
        ram::g_player2_map_level(revision).write(memory, -1);
        ram::g_saved_map_level(revision).write(memory, -1);
        if level.get() < 26 {
            ram::g_level_seeds(revision)
                .at(i32::from(level.get()))
                .write(memory, seed);
        }
    }
    Ok(())
}
fn prove_rom(path: &Path, rows: &mut Vec<serde_json::Value>) -> Result<()> {
    let rom_file = Rom::import(&fs::read(path)?)?;
    let revision = rom_file.revision();
    let rom_sha1: [u8; 20] = Sha1::digest(rom_file.bytes()).into();
    let mut machine = RuntimeMachine::new(
        &rom_file,
        IrBackend::new(&rom_file),
        "original Engine with IR oracle",
    );
    let mut original_steps = 0_u64;
    for level_number in 0..=26_u16 {
        let level = LevelId::new(level_number).unwrap();
        let fixed = {
            let resources = RomResources::new(machine.engine().devices().memory())?;
            if level_number < 26 {
                resources.fixed_seed(level)?
            } else {
                0
            }
        };
        let seeds = if level.ordinary() {
            vec![
                ("fixed", fixed),
                ("authored-zero", 0),
                ("authored-42", 42),
                ("authored-max", u16::MAX),
            ]
        } else {
            vec![("fixed", fixed)]
        };
        for (seed_label, seed) in seeds {
            setup(&mut machine, level, seed)?;
            {
                let resources = RomResources::new(machine.engine().devices().memory())?;
                let bank = CampaignSeeds::fixed(&resources)?;
                for index in 0..26 {
                    if index != level_number {
                        ensure!(
                            ram::g_level_seeds(revision)
                                .at(i32::from(index))
                                .read(machine.engine().devices().memory())
                                == bank.get(LevelId::new(index).unwrap())?
                        );
                    }
                }
            }
            let spec = OriginalSpec {
                revision: revision.into(),
                rom_sha1,
                level,
                seed,
                ambient_rng: 12345,
                directional_path_mode: false,
            };
            let (geometry, work) = {
                let resources = RomResources::new(machine.engine().devices().memory())?;
                typed(spec.clone(), &resources, 256)?
            };
            let (single, _work_single) = {
                let resources = RomResources::new(machine.engine().devices().memory())?;
                typed(spec, &resources, 1)?
            };
            ensure!(geometry == single, "work quantum changed OriginalV1 output");
            original_steps += call(
                &mut machine,
                ids::MAP_LOAD_PLAYER_LEVEL,
                &[0, u32::from(level_number)],
            )?;
            {
                let memory = machine.engine().devices().memory();
                for index in 0..19 * 26 {
                    let row = (index / 19) as i32;
                    let column = (index % 19) as i32;
                    let observed = ram::g_player1_level_map(revision)
                        .at(row)
                        .at(column)
                        .read(memory);
                    let expected = geometry.raw_cell(index)?;
                    ensure!(
                        observed == expected,
                        "{} level{} {seed_label} map[{row},{column}] expected{expected:04x} observed{observed:04x}",
                        revision.name(),
                        level_number
                    );
                    let observed = ram::g_map_path_direction_banks(revision)
                        .at(row)
                        .at(column)
                        .read(memory)
                        & 15;
                    ensure!(
                        observed == geometry.geometry.path_state()[index],
                        "{} level{} path[{row},{column}]",
                        revision.name(),
                        level_number
                    );
                }
                if level_number < 26 {
                    ensure!(
                        ram::g_level_seeds(revision)
                            .at(i32::from(level_number))
                            .read(memory)
                            == geometry.accepted_seed,
                        "accepted seed differs"
                    );
                }
                ensure!(
                    ram::g_rng_state(revision).read(memory) == geometry.ambient_rng_after,
                    "ambient RNG after geometry differs"
                );
                if level.ordinary() {
                    for index in 0..usize::from(geometry.anchor_count) {
                        let observed = ram::g_level_anchor_positions(revision).at(index as i32);
                        ensure!(
                            observed.x().read(memory) as u16 == geometry.anchors[index].x
                                && observed.y().read(memory) as u16 == geometry.anchors[index].y,
                            "stock anchor differs"
                        );
                    }
                }
            }
            let assigned = {
                let memory = machine.engine().devices().memory();
                level_number != 0
                    && (0..10).any(|index| {
                        ram::g_ship_piece_level_assignments(revision)
                            .at(index)
                            .read(memory) as u16
                            == level_number
                    })
            };
            let specials = {
                let resources = RomResources::new(machine.engine().devices().memory())?;
                StockSpecialPositions::prepare(
                    level,
                    geometry.accepted_seed,
                    &geometry.geometry,
                    if assigned {
                        geometry.ship_position()
                    } else {
                        None
                    },
                    &resources,
                )?
            };
            original_steps += call(
                &mut machine,
                ids::EARTHLING_RETAIN_LEVEL_BANK,
                &[u32::from(level_number)],
            )?;
            if level.ordinary() {
                let memory = machine.engine().devices().memory();
                for index in 0..2 {
                    let record = ram::g_earthling_banks(revision).at(0).at(index as i32);
                    let kind = record.kind().read(memory);
                    if let Some((x, y)) = specials.positions[index] {
                        ensure!(
                            kind == if index == 0 { 9 } else { 0 },
                            "stock special prefix kind differs"
                        );
                        ensure!(
                            (
                                i32::from(record.x().read(memory)),
                                i32::from(record.y().read(memory))
                            ) == (x, y),
                            "{} level{} special{} position differs",
                            revision.name(),
                            level_number,
                            index
                        );
                    } else if index == 0 {
                        ensure!(
                            kind == -1,
                            "failed mailbox should leave an unused prefix slot"
                        );
                    }
                }
            }
            let content = {
                let resources = RomResources::new(machine.engine().devices().memory())?;
                let mut items = OriginalItems::new(&geometry, true, specials)?;
                let mut previous = items.phase();
                while !matches!(items.phase(), ItemPhase::Complete | ItemPhase::Rejected) {
                    items.advance(&geometry, &resources, 127)?;
                    if items.phase() != previous {
                        let bytes = bincode::encode_to_vec(&items, codec())?;
                        let (restored, used): (OriginalItems, usize) =
                            bincode::decode_from_slice(&bytes, codec())?;
                        ensure!(used == bytes.len());
                        restored.validate(&geometry)?;
                        items = restored;
                        previous = items.phase();
                    }
                }
                items.finish(&geometry)?
            };
            original_steps += call(
                &mut machine,
                ids::LOAD_LEVEL_ITEM_BANK,
                &[u32::from(level_number)],
            )?;
            {
                let memory = machine.engine().devices().memory();
                for (slot, expected) in content.items.iter().enumerate() {
                    let item = ram::g_object_table(revision).at(0).at(slot as i32);
                    ensure!(
                        item.kind().read(memory) == expected.kind,
                        "{} level{} {seed_label} item{} expectedkind{} observed{}",
                        revision.name(),
                        level_number,
                        slot,
                        expected.kind,
                        item.kind().read(memory)
                    );
                    if expected.kind != -1 {
                        ensure!(
                            item.level().read(memory) as u16 == level_number
                                && i32::from(item.x().read(memory)) == expected.x
                                && i32::from(item.y().read(memory)) == expected.y,
                            "{} level{} {seed_label} item{} position differs",
                            revision.name(),
                            level_number,
                            slot
                        );
                    }
                }
            }
            let content_bytes = bincode::encode_to_vec(&content, codec())?;
            let digest = format!("{:x}", Sha256::digest(&content_bytes));
            rows.push(serde_json::json!({"revision":revision.name(),"floor":level_number,"seed_case":seed_label,"geometry_work":work,"attempt":geometry.accepted_attempt,"cells_compared":494,"paths_compared":494,"item_slots_compared":32,"active_items":content.items.iter().filter(|item|item.kind!=-1).count(),"source_digest":digest,"phase_roundtrips":true,"work_quanta":[1,256]}));
            eprintln!(
                "{} level{} {} OriginalV1 map/path/items match",
                revision.name(),
                level_number,
                seed_label
            );
        }
    }
    eprintln!(
        "{} original instructions executed: {}",
        revision.name(),
        original_steps
    );
    Ok(())
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().collect();
    ensure!(
        args.len() == 4,
        "usage: original-proof REV00_ROM REV02_ROM PRIVATE_RECEIPT_JSON"
    );
    let mut rows = Vec::new();
    prove_rom(Path::new(&args[1]), &mut rows)?;
    prove_rom(Path::new(&args[2]), &mut rows)?;
    let receipt = serde_json::json!({"format":"tje-original-v1-proof/1","backend":"original Engine / IR oracle","comparison":"semantic generated cells, low path nibble, accepted seeds, active item kind/level/position; not CPU timing or item padding","cases":rows,"passed":true});
    if let Some(parent) = Path::new(&args[3]).parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&args[3], serde_json::to_vec_pretty(&receipt)?)?;
    Ok(())
}
