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
