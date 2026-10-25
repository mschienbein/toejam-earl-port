#![cfg(feature = "enhanced-mode")]
//! Authored opcodes exercise the real native compiler and live readable bridge.
use genesis_config::{
    GenesisButton, GenesisController, GenesisControllerType, GenesisEmulatorConfig, GenesisInputs,
};
use sha2::{Digest, Sha256};
use tje_engine_core::{Program, compile_instruction};
use tje_engine_cranelift::{CacheConfig, Engine, NativeProgram, load_engine};
use tje_engine_runtime::{
    StepKind,
    enhanced::{DEBUG_PROFILE, EnhancedMode, EnhancedProfile},
};
use tje_game::{
    Context, FramedTranslation, Result, Translation,
    call_frame::{CallSite, FrameLayout, LocalFrame},
    ids,
};
use tje_mem::{AddressSpace, Bus};
use tje_native_platform::DeviceRuntime;
use tje_rom::{Revision, Rom};
use tje_session::{CONNECTED, Session, Start};

#[path = "enhanced/state.rs"]
mod state;

fn synthetic(id: tje_game::FunctionId, nested: bool) -> Engine {
    let revision = Revision::Rev02;
    let target = id.metadata().address(revision);
    let multiply = ids::LMULT.metadata().address(revision);
    let divide = ids::LDIVT.metadata().address(revision);
    let mut bytes = vec![0; (target.max(multiply).max(divide) + 32) as usize];
    bytes[..4].copy_from_slice(&0xfff000_u32.to_be_bytes());
    bytes[4..8].copy_from_slice(&0x100_u32.to_be_bytes());
    bytes[0x100..0x102].copy_from_slice(&0x4eb9_u16.to_be_bytes()); // JSR absolute long
    bytes[0x102..0x106].copy_from_slice(&target.to_be_bytes());
    bytes[0x106..0x10a].copy_from_slice(&[0x4e, 0x71, 0x60, 0xfe]); // NOP, loop
    let mut pcs = vec![0x100, 0x106, 0x108];
    for pc in [target, multiply, divide] {
        bytes[pc as usize..pc as usize + 4].copy_from_slice(&[0x70, 0x07, 0x4e, 0x75]); // MOVEQ #7,D0; RTS
        pcs.extend([pc, pc + 2]);
    }
    if nested {
        bytes[multiply as usize..multiply as usize + 2].copy_from_slice(&0x4eb9_u16.to_be_bytes());
        bytes[multiply as usize + 2..multiply as usize + 6].copy_from_slice(&divide.to_be_bytes());
        bytes[multiply as usize + 6..multiply as usize + 10]
            .copy_from_slice(&[0x52, 0x80, 0x4e, 0x75]); // ADDQ.L #1,D0; RTS
        pcs.retain(|pc| *pc != multiply + 2);
        pcs.extend([multiply + 6, multiply + 8]);
    }
    pcs.sort_unstable();
    pcs.dedup();
    let program = Program {
        instructions: pcs
            .into_iter()
            .map(|pc| compile_instruction(&bytes, pc).unwrap())
            .collect(),
    };
    let backend = NativeProgram::compile([0; 20], &program).unwrap();
    let mut devices = DeviceRuntime::new(
        AddressSpace::new(bytes, revision),
        &GenesisEmulatorConfig::default(),
    );
    devices.memory_mut().write_u32(0xfff000, 0x1234);
    Engine::from_devices(devices, backend)
}

fn replace_abs(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    assert_eq!(arguments, &[0x1234]);
    context.set_data_register(2, 0xdead_beef);
    context.set_status_register(0x2715);
    context.write_u16(0xff1000, 0x4321);
    Ok(Some(42))
}
static LEAF: &[Translation] = &[Translation {
    id: ids::ABS,
    function: replace_abs,
}];

#[test]
fn actual_compiled_caller_enters_modified_rust_with_live_arguments_and_returns_once() {
    let mut engine = synthetic(ids::ABS, false);
    let mut mode = EnhancedMode::new(
        EnhancedProfile::new("authored-leaf-v1", LEAF, &[]),
        Revision::Rev02,
    )
    .unwrap();
    assert_eq!(
        mode.step(&mut engine, GenesisInputs::default())
            .unwrap()
            .kind,
        StepKind::Instruction
    );
    assert_eq!(engine.cpu().read_address(7), 0xffeffc);
    let step = mode.step(&mut engine, GenesisInputs::default()).unwrap();
    assert_eq!(step.kind, StepKind::EnhancedOverride { function: ids::ABS });
    assert_eq!(step.instruction_pc, None);
    assert_eq!(step.devices.cpu_cycles, 16);
    assert!(step.devices.master_clocks > 0);
    assert_eq!(engine.cpu().pc, 0x106);
    assert_eq!(engine.cpu().prefetch, 0x4e71);
    assert_eq!(engine.cpu().read_address(7), 0xfff000);
    assert_eq!(engine.cpu().d[0], 42);
    assert_eq!(engine.cpu().d[2], 0xdead_beef);
    assert_eq!(engine.cpu().sr, 0x2715);
    assert_eq!(engine.devices().memory().read_u16(0xff1000), 0x4321);
    assert_eq!(engine.devices().memory().read_u32(0xfff000), 0x1234);
    assert_eq!(
        mode.step(&mut engine, GenesisInputs::default())
            .unwrap()
            .kind,
        StepKind::Instruction
    );
}

#[test]
fn stateless_override_snapshots_bind_profile_and_resume_identically() {
    let mut engine = synthetic(ids::ABS, false);
    let mut mode = EnhancedMode::new(
        EnhancedProfile::new("authored-leaf-v1", LEAF, &[]),
        Revision::Rev02,
    )
    .unwrap();
    mode.step(&mut engine, GenesisInputs::default()).unwrap();
    let checkpoint = mode.snapshot(&engine).unwrap();
    let before = engine.cpu().clone();
    let clock = engine.devices().clock();
    assert_eq!(
        engine.restore(&checkpoint),
        Err(tje_engine_runtime::SnapshotError::ProfileMismatch)
    );
    assert_eq!(engine.cpu(), &before);
    assert_eq!(engine.devices().clock(), clock);
    mode.step(&mut engine, GenesisInputs::default()).unwrap();
    let expected = mode.snapshot(&engine).unwrap();
    mode.restore(&mut engine, &checkpoint).unwrap();
    mode.step(&mut engine, GenesisInputs::default()).unwrap();
    assert_eq!(
        mode.snapshot(&engine).unwrap().as_bytes(),
        expected.as_bytes()
    );
}

fn nested_abs(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let value = tje_game::call_at(context, ids::LMULT, &[], CallSite::new(0x106))?;
    Ok(Some(value + 1))
}
fn nested_divide(_: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    Ok(Some(0x20))
}
static NESTED: &[Translation] = &[
    Translation {
        id: ids::ABS,
        function: nested_abs,
    },
    Translation {
        id: ids::LDIVT,
        function: nested_divide,
    },
];

#[test]
fn rust_calls_unselected_compiled_original_which_redispatches_selected_rust() {
    let mut engine = synthetic(ids::ABS, true);
    let mut mode = EnhancedMode::new(
        EnhancedProfile::new("authored-nested-v1", NESTED, &[]),
        Revision::Rev02,
    )
    .unwrap();
    mode.step(&mut engine, GenesisInputs::default()).unwrap();
    mode.step(&mut engine, GenesisInputs::default()).unwrap();
    assert_eq!(engine.cpu().d[0], 0x22);
    assert_eq!(engine.cpu().pc, 0x106);
    assert_eq!(engine.cpu().read_address(7), 0xfff000);
}

fn framed_abs(context: &mut dyn Context) -> Result<Option<u32>> {
    assert!(context.frame_state().unwrap().is_prepared_for(ids::ABS));
    let frame = LocalFrame::enter(context, FrameLayout::link(4, 1 << 2, 1 << 2))?;
    let value = frame.argument_slot(0).read(context);
    frame.argument_slot(0).write(context, value + 1);
    frame.local::<u32>(-4).write(context, 0xaabbccdd);
    context.set_data_register(2, 0x11);
    context.set_address_register(2, 0x22);
    frame.leave(context)?;
    Ok(Some(value))
}
static FRAMED: &[FramedTranslation] = &[FramedTranslation {
    id: ids::ABS,
    function: framed_abs,
}];

#[test]
fn framed_callback_uses_actual_slots_and_restores_only_its_own_locals() {
    let mut engine = synthetic(ids::ABS, false);
    engine.cpu_mut().d[2] = 0x12345678;
    engine.cpu_mut().a[2] = 0xff2000;
    engine.cpu_mut().a[6] = 0xff3000;
    let mut mode = EnhancedMode::new(
        EnhancedProfile::new("authored-framed-v1", &[], FRAMED),
        Revision::Rev02,
    )
    .unwrap();
    mode.step(&mut engine, GenesisInputs::default()).unwrap();
    mode.step(&mut engine, GenesisInputs::default()).unwrap();
    assert_eq!(engine.cpu().d[0], 0x1234);
    assert_eq!(engine.cpu().d[2], 0x12345678);
    assert_eq!(engine.cpu().a[2], 0xff2000);
    assert_eq!(engine.cpu().a[6], 0xff3000);
    assert_eq!(engine.cpu().read_address(7), 0xfff000);
    assert_eq!(engine.devices().memory().read_u32(0xfff000), 0x1235);
}

#[test]
fn debug_profile_writes_new_colour_through_live_vdp_without_rom_palette_assets() {
    let mut engine = synthetic(ids::FORCE_TEXT_BACKGROUND_COLOR_TO_BLACK, false);
    let mut mode = EnhancedMode::new(DEBUG_PROFILE, Revision::Rev02).unwrap();
    mode.step(&mut engine, GenesisInputs::default()).unwrap();
    mode.step(&mut engine, GenesisInputs::default()).unwrap();
    assert_eq!(engine.devices().vdp().hybrid_memories().1[15], 0x0e0e);
    assert_eq!(engine.cpu().pc, 0x106);
}

fn fail(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    context.write_u16(0xff1000, 0x4321);
    Err(tje_game::Error::Platform("authored rejection".into()))
}
static FAIL: &[Translation] = &[Translation {
    id: ids::ABS,
    function: fail,
}];
#[test]
fn failure_never_falls_back_or_retries_partial_callback() {
    let mut engine = synthetic(ids::ABS, false);
    let mut mode = EnhancedMode::new(
        EnhancedProfile::new("authored-fail-v1", FAIL, &[]),
        Revision::Rev02,
    )
    .unwrap();
    mode.step(&mut engine, GenesisInputs::default()).unwrap();
    assert!(mode.step(&mut engine, GenesisInputs::default()).is_err());
    let clock = engine.devices().clock();
    assert!(matches!(
        mode.step(&mut engine, GenesisInputs::default()),
        Err(tje_engine_runtime::enhanced::Error::Failed)
    ));
    assert_eq!(engine.devices().clock(), clock);
    assert_eq!(engine.devices().memory().read_u16(0xff1000), 0x4321);
    mode.reset(&mut engine);
    assert!(mode.step(&mut engine, GenesisInputs::default()).is_ok());
}

fn missing_original(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    tje_game::call_at(context, ids::ULMODT, &[], CallSite::new(0x106)).map(Some)
}
static MISSING_ORIGINAL: &[Translation] = &[Translation {
    id: ids::ABS,
    function: missing_original,
}];
static MISSING_GRANDCHILD: &[Translation] = &[
    Translation {
        id: ids::ABS,
        function: nested_abs,
    },
    Translation {
        id: ids::LDIVT,
        function: missing_original,
    },
];

#[test]
fn compiled_callee_failure_keeps_first_cause_and_actual_nested_stack_and_pc() {
    for (nested, translations, expected_sp) in [
        (false, MISSING_ORIGINAL, 0xffeff8),
        (true, MISSING_GRANDCHILD, 0xffeff0),
    ] {
        let mut engine = synthetic(ids::ABS, nested);
        let mut mode = EnhancedMode::new(
            EnhancedProfile::new("authored-missing-original-v1", translations, &[]),
            Revision::Rev02,
        )
        .unwrap();
        let checkpoint = mode.snapshot(&engine).unwrap();
        mode.step(&mut engine, GenesisInputs::default()).unwrap();
        let failure = mode
            .step(&mut engine, GenesisInputs::default())
            .unwrap_err();
        assert!(matches!(
            failure,
            tje_engine_runtime::enhanced::Error::Override(
                tje_game::Error::EnhancedExecutionFailure { entry, .. }
            ) if entry == ids::ULMODT
        ));
        assert!(mode.is_failed());
        assert_eq!(
            engine.cpu().pc,
            ids::ULMODT.metadata().address(Revision::Rev02)
        );
        assert_eq!(engine.cpu().read_address(7), expected_sp);
        assert_eq!(engine.devices().memory().read_u32(expected_sp), 0x106);
        mode.restore(&mut engine, &checkpoint).unwrap();
        assert!(!mode.is_failed());
        assert!(mode.step(&mut engine, GenesisInputs::default()).is_ok());
    }
}

fn inputs(pads: [u16; 2]) -> GenesisInputs {
    use GenesisButton::*;
    let pads = pads.map(|bits| {
        let mut controller = if bits & CONNECTED != 0 {
            GenesisController::new(GenesisControllerType::ThreeButton)
        } else {
            GenesisController::None
        };
        for (index, button) in [Up, Down, Left, Right, A, B, C, Start]
            .into_iter()
            .enumerate()
        {
            controller.set_field(button, bits & (1 << index) != 0);
        }
        controller
    });
    GenesisInputs {
        p1: pads[0],
        p2: pads[1],
    }
}
#[derive(Default)]
struct Hashes {
    video: Sha256,
    audio: Sha256,
    scratch: Vec<u8>,
}
impl Hashes {
    fn observe(&mut self, engine: &Engine) {
        let (pixels, size) = engine.frame_buffer();
        self.video.update(size.width.to_le_bytes());
        self.video.update(size.height.to_le_bytes());
        self.scratch.clear();
        for pixel in &pixels[..size.len() as usize] {
            self.scratch
                .extend_from_slice(&[pixel.r, pixel.g, pixel.b, pixel.a]);
        }
        self.video.update(&self.scratch);
        self.scratch.clear();
        for sample in engine.devices().audio_samples() {
            for value in sample {
                self.scratch
                    .extend_from_slice(&value.to_bits().to_le_bytes());
            }
        }
        self.audio.update(&self.scratch);
    }
    fn finish(self) -> (String, String) {
        (
            format!("{:x}", self.video.finalize()),
            format!("{:x}", self.audio.finalize()),
        )
    }
}

#[test]
#[ignore = "requires both private ROM revisions; full recorded sessions, two engines maximum"]
fn enhanced_off_recorded_sessions_match_video_and_audio_hashes() {
    for (revision, key, session_name) in [
        (
            Revision::Rev02,
            "TJE_REV02_ROM",
            "rev02-one-player.tje.json",
        ),
        (
            Revision::Rev00,
            "TJE_REV00_ROM",
            "rev00-one-player.tje.json",
        ),
    ] {
        let bytes = std::fs::read(std::env::var(key).unwrap()).unwrap();
        let session_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../sessions")
            .join(session_name);
        let session = Session::from_json(&std::fs::read_to_string(session_path).unwrap()).unwrap();
        assert!(matches!(session.start, Start::PowerOn));
        let dir = tempfile::tempdir().unwrap();
        let cache = CacheConfig {
            data_dir: dir.path().to_owned(),
        };
        let config = GenesisEmulatorConfig::default();
        let mut faithful =
            load_engine(Rom::import(&bytes).unwrap(), &config, &cache, |_| {}).unwrap();
        let mut selected_off =
            load_engine(Rom::import(&bytes).unwrap(), &config, &cache, |_| {}).unwrap();
        let mut mode = EnhancedMode::default();
        let (mut baseline_hashes, mut off_hashes) = (Hashes::default(), Hashes::default());
        for (frame, pads) in session.inputs().enumerate() {
            faithful.play_frame(inputs(pads)).unwrap();
            mode.play_frame(&mut selected_off, inputs(pads)).unwrap();
            assert_eq!(
                faithful.cpu(),
                selected_off.cpu(),
                "{revision:?} frame {frame}"
            );
            assert_eq!(
                faithful.devices().memory().ram(),
                selected_off.devices().memory().ram(),
                "{revision:?} frame {frame}"
            );
            assert_eq!(faithful.devices().clock(), selected_off.devices().clock());
            baseline_hashes.observe(&faithful);
            off_hashes.observe(&selected_off);
        }
        let baseline = baseline_hashes.finish();
        let off = off_hashes.finish();
        assert_eq!(baseline, off);
        eprintln!(
            "{revision:?} {session_name} frames={} mode=faithful video_sha256={} audio_sha256={} identical=true",
            session.frames, off.0, off.1
        );
    }
}

#[test]
#[ignore = "requires both private ROM revisions; recorded menu/gameplay demonstration"]
fn enhanced_debug_recorded_menu_remains_frame_paced_and_deterministic() {
    for (revision, key, session_name) in [
        (
            Revision::Rev02,
            "TJE_REV02_ROM",
            "rev02-one-player.tje.json",
        ),
        (
            Revision::Rev00,
            "TJE_REV00_ROM",
            "rev00-one-player.tje.json",
        ),
    ] {
        let bytes = std::fs::read(std::env::var(key).unwrap()).unwrap();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../sessions")
            .join(session_name);
        let session = Session::from_json(&std::fs::read_to_string(path).unwrap()).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let cache = CacheConfig {
            data_dir: dir.path().to_owned(),
        };
        let config = GenesisEmulatorConfig::default();
        let mut first = load_engine(Rom::import(&bytes).unwrap(), &config, &cache, |_| {}).unwrap();
        let mut second =
            load_engine(Rom::import(&bytes).unwrap(), &config, &cache, |_| {}).unwrap();
        let mut a = EnhancedMode::new(DEBUG_PROFILE, revision).unwrap();
        let mut b = EnhancedMode::new(DEBUG_PROFILE, revision).unwrap();
        let (mut ah, mut bh) = (Hashes::default(), Hashes::default());
        let mut colour_seen = false;
        for pads in session.inputs().take(1200) {
            a.play_frame(&mut first, inputs(pads)).unwrap();
            b.play_frame(&mut second, inputs(pads)).unwrap();
            colour_seen |= first.devices().vdp().hybrid_memories().1[15] == 0x0e0e;
            assert_eq!(first.cpu(), second.cpu());
            assert_eq!(first.frame_count(), second.frame_count());
            ah.observe(&first);
            bh.observe(&second);
        }
        assert!(
            colour_seen,
            "recording did not reach the modified text-background helper"
        );
        assert_eq!(first.frame_count(), 1200);
        let hashes = ah.finish();
        assert_eq!(hashes, bh.finish());
        eprintln!(
            "{revision:?} enhanced-debug frames=1200 magenta_seen=true video_sha256={} audio_sha256={} deterministic=true",
            hashes.0, hashes.1
        );
    }
}
