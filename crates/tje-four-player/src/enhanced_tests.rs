use super::*;
use crate::{
    Error, LevelId, SEATS, SeatId,
    input::{PadWord, SourceId, SourceSample},
    state::{Arrival, Position},
};
use genesis_config::GenesisEmulatorConfig;
use std::convert::Infallible;
use tje_engine_runtime::{
    Backend, Bus, CpuState, Engine, RewindBuffer, StepResult, enhanced::DEBUG_PROFILE,
};
use tje_mem::AddressSpace;
use tje_native_platform::DeviceRuntime;
use tje_rom::Revision;

// Authored deterministic device workload, NOT a cartridge/gameplay fixture. It
// advances real video/audio devices so the sidecar is tested in the SAME atomic
// enhanced image and rewind ring as hardware, not in an isolated bincode clone.
struct ClockWorkload;
impl Backend for ClockWorkload {
    type Error = Infallible;
    fn execute(
        &mut self,
        cpu: &mut CpuState,
        bus: &mut dyn Bus,
    ) -> std::result::Result<StepResult, Infallible> {
        cpu.d[0] = cpu.d[0].wrapping_add(1);
        bus.write16(0xff0100, cpu.d[0] as u16);
        bus.write8(0xc00011, 0x90 | (cpu.d[0] & 15) as u8);
        Ok(StepResult {
            cycles: 44,
            ..StepResult::default()
        })
    }
}
fn machine(revision: Revision) -> Engine<ClockWorkload> {
    let mut bytes = vec![0; 1024];
    bytes[..4].copy_from_slice(&0xffff00_u32.to_be_bytes());
    bytes[4..8].copy_from_slice(&0x100_u32.to_be_bytes());
    bytes[0x100..0x102].copy_from_slice(&0x4e71_u16.to_be_bytes());
    Engine::from_devices(
        DeviceRuntime::new(
            AddressSpace::new(bytes, revision),
            &GenesisEmulatorConfig::default(),
        ),
        ClockWorkload,
    )
}
fn model(seed: u64) -> CoopState {
    let mut state = CoopState::new(Config::four_seats(seed)).unwrap();
    for seat in SeatId::ALL {
        state
            .router
            .reassign(&[(seat, Some(SourceId([seat.get(); 16])))])
            .unwrap();
    }
    state
}
fn mode(revision: Revision, seed: u64) -> EnhancedMode {
    // The existing debug registry is only a snapshot envelope identity here.
    // No four-player gameplay registry is published or entered by this workload.
    let mut mode = EnhancedMode::new(DEBUG_PROFILE, revision).unwrap();
    mode.sidecars_mut().register(model(seed)).unwrap();
    mode
}
fn advance_bookkeeping(mode: &mut EnhancedMode, frame: u64) -> crate::input::InputFrame {
    let state = mode.sidecars_mut().get_mut::<CoopState>().unwrap();
    let samples: [SourceSample; SEATS] = std::array::from_fn(|index| SourceSample {
        source: SourceId([index as u8; 16]),
        pad: PadWord::new(256 | (1 << ((index + frame as usize) % 8))).unwrap(),
        actions: (index as u16 + 1) << 8,
    });
    let input = state.router.route(frame, &samples).unwrap();
    state.latch_inputs(input).unwrap();
    let arrivals: [Arrival; SEATS] = std::array::from_fn(|index| Arrival {
        seat: SeatId::ALL[index],
        level: LevelId::new(1 + ((index + frame as usize) % 4) as u16).unwrap(),
        position: Position {
            x: (100 + index as i32) * 256,
            y: (200 + frame as i32) * 256,
            elevation: 0,
        },
    });
    state.record_arrivals(&arrivals).unwrap();
    input
}

#[test]
fn snapshots_and_rewind_include_all_four_seats_inputs_and_persistent_level_history() {
    for revision in Revision::ALL {
        let mut engine = machine(revision);
        let mut mode = mode(revision, 123);
        for frame in 0..2 {
            let inputs = advance_bookkeeping(&mut mode, frame);
            mode.play_frame(&mut engine, inputs.hardware_inputs().unwrap())
                .unwrap();
        }
        let checkpoint = mode.snapshot(&engine).unwrap();
        let checkpoint_state = mode.sidecars().get::<CoopState>().unwrap().clone();
        let mut rewind = RewindBuffer::new(4 * 1024 * 1024);
        rewind.push(checkpoint.clone());
        let mut expected = Vec::new();
        for frame in 2..6 {
            let inputs = advance_bookkeeping(&mut mode, frame);
            let audio: Vec<_> = mode
                .play_frame(&mut engine, inputs.hardware_inputs().unwrap())
                .unwrap()
                .iter()
                .map(|sample| sample.map(f64::to_bits))
                .collect();
            expected.push((mode.snapshot(&engine).unwrap(), audio));
        }
        let image = rewind.pop_before(engine.frame_count()).unwrap();
        mode.restore(&mut engine, &image).unwrap();
        assert_eq!(
            mode.sidecars().get::<CoopState>().unwrap(),
            &checkpoint_state
        );
        assert_eq!(
            mode.snapshot(&engine).unwrap().as_bytes(),
            checkpoint.as_bytes()
        );
        for (offset, (image, audio)) in expected.into_iter().enumerate() {
            let inputs = advance_bookkeeping(&mut mode, offset as u64 + 2);
            let actual: Vec<_> = mode
                .play_frame(&mut engine, inputs.hardware_inputs().unwrap())
                .unwrap()
                .iter()
                .map(|sample| sample.map(f64::to_bits))
                .collect();
            assert_eq!(actual, audio);
            assert_eq!(mode.snapshot(&engine).unwrap().as_bytes(), image.as_bytes());
        }
    }
}

#[test]
fn sidecar_only_changes_affect_complete_hash_not_hardware_and_reset_restores_initial_values() {
    let mut engine = machine(Revision::Rev02);
    let mut mode = mode(Revision::Rev02, 123);
    let before = mode.snapshot(&engine).unwrap();
    let hardware = engine.snapshot().unwrap();
    advance_bookkeeping(&mut mode, 0);
    let after = mode.snapshot(&engine).unwrap();
    assert_ne!(before.state_hash(), after.state_hash());
    assert_eq!(engine.snapshot().unwrap().as_bytes(), hardware.as_bytes());
    mode.reset(&mut engine);
    assert_eq!(mode.sidecars().get::<CoopState>().unwrap(), &model(123));
}

#[test]
fn mismatched_config_and_invalid_current_state_reject_transactionally() {
    for revision in Revision::ALL {
        let mut engine = machine(revision);
        let mut current = mode(revision, 1);
        let other = mode(revision, 2).snapshot(&engine).unwrap();
        let before = current.snapshot(&engine).unwrap();
        assert!(current.restore(&mut engine, &other).is_err());
        assert_eq!(
            current.snapshot(&engine).unwrap().as_bytes(),
            before.as_bytes()
        );
        current
            .sidecars_mut()
            .get_mut::<CoopState>()
            .unwrap()
            .players[3]
            .rank = 9;
        assert!(current.snapshot(&engine).is_err());
        current.restore(&mut engine, &before).unwrap();
        assert_eq!(
            current.snapshot(&engine).unwrap().as_bytes(),
            before.as_bytes()
        );
    }
}

#[test]
fn enabling_rejects_before_mutation_for_faithful_or_already_enhanced_state() {
    for revision in Revision::ALL {
        let engine = machine(revision);
        for mut mode in [EnhancedMode::default(), mode(revision, 5)] {
            let before = mode.snapshot(&engine).unwrap();
            let config = Config::four_seats(6);
            assert_eq!(
                request(&mut mode, true, &config),
                Err(Error::SimulationUnavailable)
            );
            assert_eq!(
                mode.snapshot(&engine).unwrap().as_bytes(),
                before.as_bytes()
            );
            request(&mut mode, false, &config).unwrap();
            assert_eq!(
                mode.snapshot(&engine).unwrap().as_bytes(),
                before.as_bytes()
            );
            let mut invalid = config;
            invalid.ruleset = 99;
            assert!(request(&mut mode, true, &invalid).is_err());
            request(&mut mode, false, &invalid).unwrap();
            assert_eq!(
                mode.snapshot(&engine).unwrap().as_bytes(),
                before.as_bytes()
            );
        }
    }
}

// Authored resource definitions for persistence integration, not cartridge
// terrain/AI/loot data and not a four-player gameplay session.
struct SolidResources;
impl tje_world::Resources for SolidResources {
    fn parameter(&self, _: LevelId, parameter: u8) -> tje_world::Result<(u8, u8)> {
        Ok(if parameter == 6 { (3, 6) } else { (55, 70) })
    }
    fn shape(&self, _: tje_world::Shape, _: u8, alternate: bool) -> tje_world::Result<u8> {
        Ok(if alternate { 2 } else { 1 })
    }
    fn alternative(&self, shape: u8, _: u32) -> tje_world::Result<tje_world::Cell> {
        tje_world::Cell::new(shape, 0)
    }
    fn transition(&self, _: u8, _: u8, _: u32) -> tje_world::Result<Option<tje_world::Cell>> {
        Ok(None)
    }
    fn base_tile(&self, base: u8, _: u8, _: u8) -> tje_world::Result<u16> {
        Ok(u16::from(base & 127 != 0))
    }
    fn overlay_tile(&self, _: u8, _: u8, _: u8) -> tje_world::Result<u16> {
        Ok(0)
    }
    fn property(&self, tile: u16) -> tje_world::Result<u8> {
        Ok(if tile == 1 { 0x10 } else { 0x90 })
    }
}

#[test]
fn shared_level_mutations_and_four_seat_references_restore_in_one_rewind_image() {
    use sha1::Digest;
    use tje_world::{
        Cell, Density, Dimensions, LevelSize, WorldConfig, generation::Phase,
        population::PopulationBaseline, world::WorldState,
    };
    for revision in Revision::ALL {
        let mut engine = machine(revision);
        let mut mode = mode(revision, 7);
        let dimensions = Dimensions::new(8, 8).unwrap();
        let mut world = WorldState::new(WorldConfig {
            root_seed: 7,
            rom_sha1: sha1::Sha1::digest(engine.devices().memory().rom()).into(),
            size: LevelSize::Custom(dimensions),
            density: Density::Normal,
        })
        .unwrap();
        let baseline = PopulationBaseline {
            walkable_tiles: 19 * 26 * 64,
            ordinary_items: 12,
            earthlings: 10,
            reserved_items: 2,
            ship_piece: true,
            mailbox: true,
        };
        let mut arrivals = Vec::new();
        for seat in SeatId::ALL {
            let level = LevelId::new(2 + u16::from(seat.get())).unwrap();
            world.request_load(level, baseline).unwrap();
            let mut complete = false;
            for _ in 0..10_000 {
                match world.advance_loading(&SolidResources, 1024).unwrap() {
                    Phase::Complete => {
                        complete = true;
                        break;
                    }
                    Phase::Rejected => panic!("synthetic persistence floor rejected"),
                    _ => {}
                }
            }
            assert!(complete, "bounded synthetic floor did not finish");
            world.set_active(seat.get(), level).unwrap();
            let (x, y) = world
                .floor(level)
                .unwrap()
                .generated
                .geometry
                .anchors()
                .arrival
                .center();
            arrivals.push(Arrival {
                seat,
                level,
                position: Position {
                    x: x * 256,
                    y: y * 256,
                    elevation: 0,
                },
            });
        }
        mode.sidecars_mut()
            .get_mut::<CoopState>()
            .unwrap()
            .record_arrivals(&arrivals)
            .unwrap();
        let first = LevelId::new(2).unwrap();
        let position = world
            .floor(first)
            .unwrap()
            .generated
            .geometry
            .anchors()
            .arrival;
        let (x, y) = position.center();
        let floor = world.floor_mut(first).unwrap();
        floor
            .replace_cell(position, Cell::new(2, 0).unwrap(), 3)
            .unwrap();
        let claimed_item = floor.generated.population.items.last().unwrap().id;
        let removed_actor = floor.generated.population.earthlings.last().unwrap().id;
        floor
            .generated
            .population
            .collected
            .insert(claimed_item)
            .unwrap();
        floor
            .generated
            .population
            .removed
            .insert(removed_actor)
            .unwrap();
        assert!(floor.discovery.visit(x, y, dimensions).unwrap());
        let drop = world.drop_item(first, x, y, 5).unwrap();
        mode.sidecars_mut().register(world.clone()).unwrap();
        let seats = mode.sidecars().get::<CoopState>().unwrap().clone();
        let checkpoint = mode.snapshot(&engine).unwrap();
        let mut rewind = RewindBuffer::new(4 * 1024 * 1024);
        rewind.push(checkpoint.clone());
        mode.play_frame(
            &mut engine,
            crate::input::InputFrame::neutral(0)
                .hardware_inputs()
                .unwrap(),
        )
        .unwrap();
        // Reassign presentation-independent active floor references, then revisit.
        // These are model bookkeeping operations, NOT original elevator calls.
        for arrival in &mut arrivals {
            arrival.level = LevelId::new(2 + u16::from((arrival.seat.get() + 1) % 4)).unwrap();
            mode.sidecars_mut()
                .get_mut::<WorldState>()
                .unwrap()
                .set_active(arrival.seat.get(), arrival.level)
                .unwrap();
        }
        mode.sidecars_mut()
            .get_mut::<CoopState>()
            .unwrap()
            .record_arrivals(&arrivals)
            .unwrap();
        assert_ne!(
            mode.snapshot(&engine).unwrap().state_hash(),
            checkpoint.state_hash()
        );
        let image = rewind.pop_before(engine.frame_count()).unwrap();
        mode.restore(&mut engine, &image).unwrap();
        assert_eq!(mode.sidecars().get::<WorldState>().unwrap(), &world);
        assert_eq!(mode.sidecars().get::<CoopState>().unwrap(), &seats);
        assert_eq!(
            mode.snapshot(&engine).unwrap().as_bytes(),
            checkpoint.as_bytes()
        );
        let restored = mode.sidecars().get::<WorldState>().unwrap();
        let floor = restored.floor(first).unwrap();
        assert!(floor.generated.population.collected.contains(claimed_item));
        assert!(floor.generated.population.removed.contains(removed_actor));
        assert_eq!(floor.discovery.awards(), 1);
        assert_eq!(restored.dropped()[0].id, drop);
        assert_eq!(
            floor
                .generated
                .geometry
                .cell(i32::from(position.x), i32::from(position.y))
                .unwrap()
                .base(),
            2
        );
        assert_eq!(
            floor.generated.geometry.path_state()[position.index(dimensions).unwrap()],
            3
        );
        for seat in SeatId::ALL {
            assert_eq!(
                seats.players[seat.index()].level,
                restored.active()[seat.index()]
            );
        }
    }
}
