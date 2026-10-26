use crate::{Phase, session::Session, session_tests::session};
use genesis_config::GenesisEmulatorConfig;
use std::convert::Infallible;
use tje_engine_runtime::{
    Backend, Bus, CpuState, Engine, StepResult,
    enhanced::{DEBUG_PROFILE, EnhancedMode},
};
use tje_four_player::input::InputFrame;
use tje_mem::{AddressSpace, Revision};
use tje_native_platform::DeviceRuntime;

// Authored device workload, not a cartridge fixture or playable profile.
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
fn mode(revision: Revision, seed: u64) -> EnhancedMode {
    let mut mode = EnhancedMode::new(DEBUG_PROFILE, revision).unwrap();
    mode.sidecars_mut().register(session(seed)).unwrap();
    mode
}
fn advance(mode: &mut EnhancedMode, frame: u64) -> InputFrame {
    let state = mode.sidecars_mut().get_mut::<Session>().unwrap();
    let input = InputFrame::new(frame, [0x101, 0x102, 0x104, 0x108], [1, 2, 4, 8]).unwrap();
    state.players.latch_inputs(input).unwrap();
    state.clock.admit(state.clock.next(), input).unwrap();
    state.rngs[1].next_u16();
    input
}

#[test]
fn aggregate_phases_and_hardware_audio_restore_in_one_enhanced_image() {
    for revision in [Revision::Rev00, Revision::Rev02] {
        let mut engine = machine(revision);
        let mut mode = mode(revision, 123);
        for frame in 0..3 {
            let input = advance(&mut mode, frame);
            mode.play_frame(&mut engine, input.hardware_inputs().unwrap())
                .unwrap();
        }
        let checkpoint = mode.snapshot(&engine).unwrap();
        let saved = mode.sidecars().get::<Session>().unwrap().clone();
        assert_eq!(saved.clock.next(), Phase::Players);
        let mut observations = Vec::new();
        for frame in 3..7 {
            let input = advance(&mut mode, frame);
            let audio: Vec<_> = mode
                .play_frame(&mut engine, input.hardware_inputs().unwrap())
                .unwrap()
                .iter()
                .map(|sample| sample.map(f64::to_bits))
                .collect();
            observations.push((mode.snapshot(&engine).unwrap(), audio));
        }
        mode.restore(&mut engine, &checkpoint).unwrap();
        assert_eq!(mode.sidecars().get::<Session>().unwrap(), &saved);
        assert_eq!(
            mode.snapshot(&engine).unwrap().as_bytes(),
            checkpoint.as_bytes()
        );
        for (offset, (expected, audio)) in observations.into_iter().enumerate() {
            let input = advance(&mut mode, offset as u64 + 3);
            let actual: Vec<_> = mode
                .play_frame(&mut engine, input.hardware_inputs().unwrap())
                .unwrap()
                .iter()
                .map(|sample| sample.map(f64::to_bits))
                .collect();
            assert_eq!(actual, audio);
            assert_eq!(
                mode.snapshot(&engine).unwrap().as_bytes(),
                expected.as_bytes()
            );
        }
    }
}

#[test]
fn bad_crossrefs_and_wrong_aggregate_identity_never_commit_hardware() {
    let revision = Revision::Rev02;
    let mut engine = machine(revision);
    let mut current = mode(revision, 123);
    let checkpoint = current.snapshot(&engine).unwrap();
    let hardware = engine.snapshot().unwrap();
    current
        .sidecars_mut()
        .get_mut::<Session>()
        .unwrap()
        .players
        .players[3]
        .level = Some(crate::LevelId::new(31).unwrap());
    assert!(current.snapshot(&engine).is_err());
    assert_eq!(engine.snapshot().unwrap().as_bytes(), hardware.as_bytes());
    current.restore(&mut engine, &checkpoint).unwrap();
    let mut other = mode(revision, 124);
    let input = advance(&mut other, 0);
    other
        .play_frame(&mut engine, input.hardware_inputs().unwrap())
        .unwrap();
    let wrong = other.snapshot(&engine).unwrap();
    current.restore(&mut engine, &checkpoint).unwrap();
    assert!(current.restore(&mut engine, &wrong).is_err());
    assert_eq!(
        current.snapshot(&engine).unwrap().as_bytes(),
        checkpoint.as_bytes()
    );
    let mut rejected = EnhancedMode::new(DEBUG_PROFILE, revision).unwrap();
    let before = rejected.snapshot(&engine).unwrap();
    let mut invalid = session(123);
    invalid.players.players[0].level = Some(crate::LevelId::new(31).unwrap());
    assert!(rejected.sidecars_mut().register(invalid).is_err());
    assert_eq!(
        rejected.snapshot(&engine).unwrap().as_bytes(),
        before.as_bytes()
    );
}

#[test]
fn even_checkpoint_restores_frozen_publication_across_floor_insertion() {
    use crate::publication::{GeometryReference, PublishedKind, PublishedView};
    use tje_world::{
        Cell, CellPosition, generation::Phase as Loading, population::PopulationBaseline,
    };
    for revision in [Revision::Rev00, Revision::Rev02] {
        let mut engine = machine(revision);
        let mut mode = mode(revision, 123);
        let input = advance(&mut mode, 0);
        mode.play_frame(&mut engine, input.hardware_inputs().unwrap())
            .unwrap();
        let level = crate::LevelId::new(2).unwrap();
        let inserted = crate::LevelId::new(3).unwrap();
        {
            let state = mode.sidecars_mut().get_mut::<Session>().unwrap();
            let stamp = crate::PublicationStamp {
                hardware_frame: 0,
                phase: Phase::PresentationA,
                epoch: 1,
            };
            state.publish(stamp).unwrap();
            state.published.kind = PublishedKind::World;
            state.published.stamp = Some(stamp);
            state.published.geometry.push(GeometryReference {
                level,
                epoch: state.world.floor(level).unwrap().geometry_epoch(),
            });
            state.published.views[0] = PublishedView::World {
                seat: crate::SeatId::ALL[0],
                level,
                origin: [0, 0],
                attributes: 0x8000,
                starfield: true,
            };
        }
        let input = advance(&mut mode, 1);
        mode.play_frame(&mut engine, input.hardware_inputs().unwrap())
            .unwrap();
        {
            let state = mode.sidecars_mut().get_mut::<Session>().unwrap();
            state
                .world
                .request_load(
                    inserted,
                    PopulationBaseline {
                        walkable_tiles: 19 * 26 * 64,
                        ordinary_items: 12,
                        earthlings: 10,
                        reserved_items: 2,
                        ship_piece: true,
                        mailbox: true,
                    },
                )
                .unwrap();
            let mut complete = false;
            for _ in 0..100_000 {
                match state
                    .world
                    .advance_loading(&crate::session_tests::Solid, 1024)
                    .unwrap()
                {
                    Loading::Complete => {
                        complete = true;
                        break;
                    }
                    Loading::Rejected => panic!("authored floor insertion rejected"),
                    _ => {}
                }
            }
            assert!(complete);
            crate::session_tests::stage_population(&state.world, &mut state.population, inserted);
        }
        let checkpoint = mode.snapshot(&engine).unwrap();
        let frozen = mode.sidecars().get::<Session>().unwrap().published.clone();
        let hardware = engine.snapshot().unwrap();
        {
            let state = mode.sidecars_mut().get_mut::<Session>().unwrap();
            state
                .world
                .floor_mut(level)
                .unwrap()
                .replace_cell(CellPosition { x: 0, y: 0 }, Cell::new(1, 0).unwrap(), 0)
                .unwrap();
        }
        assert!(mode.snapshot(&engine).is_err());
        assert_eq!(engine.snapshot().unwrap().as_bytes(), hardware.as_bytes());
        mode.restore(&mut engine, &checkpoint).unwrap();
        let restored = mode.sidecars().get::<Session>().unwrap();
        assert!(restored.world.floor(inserted).is_some());
        assert_eq!(restored.published, frozen);
        assert_eq!(restored.clock.epoch(), 2);
        assert_eq!(
            mode.snapshot(&engine).unwrap().as_bytes(),
            checkpoint.as_bytes()
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq, bincode::Encode, bincode::Decode)]
struct WrappedSession {
    shared: Session,
    budget: u16,
}
impl tje_engine_runtime::enhanced::Sidecar for WrappedSession {
    const KEY: &'static str = "test/wrapped-session";
    const VERSION: u32 = 1;
    fn validate(&self) -> tje_game::Result<()> {
        tje_engine_runtime::enhanced::Sidecar::validate(&self.shared)?;
        if self.budget > 100 {
            return Err(tje_game::Error::Platform(
                "authored enclosing budget".into(),
            ));
        }
        Ok(())
    }
    fn configuration_identity(&self) -> &[u8] {
        self.shared.configuration_identity()
    }
}
impl crate::session::SessionStorage for WrappedSession {
    fn session(&self) -> &Session {
        &self.shared
    }
    fn session_mut(&mut self) -> &mut Session {
        &mut self.shared
    }
}
#[test]
fn an_enclosing_owner_restores_shared_and_mode_state_as_one_sidecar() {
    use crate::session::SessionStorage;
    for revision in [Revision::Rev00, Revision::Rev02] {
        let mut engine = machine(revision);
        let mut mode = EnhancedMode::new(DEBUG_PROFILE, revision).unwrap();
        mode.sidecars_mut()
            .register(WrappedSession {
                shared: session(123),
                budget: 7,
            })
            .unwrap();
        assert!(mode.sidecars().get::<Session>().is_err());
        let image = mode.snapshot(&engine).unwrap();
        let owner = mode.sidecars_mut().get_mut::<WrappedSession>().unwrap();
        owner.session_mut().rngs[0].next_u16();
        owner.budget = 99;
        mode.play_frame(
            &mut engine,
            InputFrame::neutral(0).hardware_inputs().unwrap(),
        )
        .unwrap();
        mode.restore(&mut engine, &image).unwrap();
        let owner = mode.sidecars().get::<WrappedSession>().unwrap();
        assert_eq!(owner.budget, 7);
        assert_eq!(owner.session(), &session(123));
        assert_eq!(mode.snapshot(&engine).unwrap().as_bytes(), image.as_bytes());
    }
}
