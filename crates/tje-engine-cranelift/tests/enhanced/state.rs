use super::*;
use bincode::{Decode, Encode};
use tje_engine_runtime::{
    MachineSnapshot, RewindBuffer, SnapshotError,
    enhanced::{
        Context as ExtraContext, Continuation, EnhancedInputs, OverrideStep, ResumableTranslation,
        Sidecar, Wait,
    },
};

#[derive(Debug, PartialEq, Eq, Encode, Decode)]
struct ExtraState {
    course: u32,
    visits: u32,
    seats: [u32; 4],
    pads: [GenesisController; 4],
}
impl Sidecar for ExtraState {
    const KEY: &'static str = "authored-test-state";
    const VERSION: u32 = 1;
}
fn initial(course: u32) -> ExtraState {
    ExtraState {
        course,
        visits: 0,
        seats: [0; 4],
        pads: [GenesisController::None; 4],
    }
}
fn profile(entries: &'static [ResumableTranslation]) -> EnhancedProfile {
    EnhancedProfile::new("authored-resumable-v1", &[], &[]).with_resumable(entries)
}
fn mode(entries: &'static [ResumableTranslation]) -> EnhancedMode {
    let mut mode = EnhancedMode::new(profile(entries), Revision::Rev02).unwrap();
    mode.sidecars_mut().register(initial(7)).unwrap();
    mode
}

fn wait_frames(
    ctx: &mut dyn ExtraContext,
    continuation: &mut Continuation,
) -> Result<OverrideStep> {
    ctx.sidecars_mut().get_mut::<ExtraState>()?.visits += 1;
    match continuation.stage {
        0 => {
            continuation.stage = 1;
            continuation.locals[0] = ctx.entry_arguments()?.first().copied().unwrap_or(0);
            Ok(OverrideStep::Wait(Wait::Frames(2)))
        }
        1 => {
            assert_eq!(continuation.locals[0], 0x1234);
            Ok(OverrideStep::Return(Some(42)))
        }
        _ => unreachable!(),
    }
}
static FRAME_WAIT: &[ResumableTranslation] = &[ResumableTranslation {
    id: ids::ABS,
    function: wait_frames,
}];

#[test]
fn frame_wait_is_paced_and_snapshot_rewind_preserves_activation_and_sidecars() {
    let mut engine = synthetic(ids::ABS, false);
    let mut mode = mode(FRAME_WAIT);
    mode.step(&mut engine, GenesisInputs::default()).unwrap();
    mode.step(&mut engine, GenesisInputs::default()).unwrap();
    assert_eq!(
        engine.cpu().pc,
        ids::ABS.metadata().address(Revision::Rev02)
    );
    assert_eq!(engine.cpu().read_address(7), 0xffeffc);
    assert_eq!(mode.sidecars().get::<ExtraState>().unwrap().visits, 1);
    let checkpoint = mode.snapshot(&engine).unwrap();
    assert_eq!(
        u32::from_le_bytes(checkpoint.as_bytes()[8..12].try_into().unwrap()),
        3
    );
    let decoded = MachineSnapshot::from_bytes(checkpoint.as_bytes().to_vec()).unwrap();
    let mut rewind = RewindBuffer::new(8 * 1024 * 1024);
    rewind.push(decoded);
    let mut expected = Vec::new();
    for frame in 1..=3 {
        mode.play_frame(&mut engine, GenesisInputs::default())
            .unwrap();
        assert_eq!(engine.frame_count(), frame);
        expected.push(mode.snapshot(&engine).unwrap());
    }
    assert_eq!(mode.sidecars().get::<ExtraState>().unwrap().visits, 2);
    assert_eq!(engine.cpu().d[0], 42);
    let image = rewind.pop_before(engine.frame_count()).unwrap();
    mode.restore(&mut engine, &image).unwrap();
    assert_eq!(mode.sidecars().get::<ExtraState>().unwrap().visits, 1);
    for expected in expected {
        mode.play_frame(&mut engine, GenesisInputs::default())
            .unwrap();
        assert_eq!(
            mode.snapshot(&engine).unwrap().as_bytes(),
            expected.as_bytes()
        );
    }
}

fn authored_interrupt_engine(id: tje_game::FunctionId, long_original: bool) -> Engine {
    let (body, offsets): (&[u8], &[u32]) = if long_original {
        // MOVE.L #100000,D0; SUBQ.L #1,D0; BNE loop; RTS.
        (
            &[
                0x20, 0x3c, 0x00, 0x01, 0x86, 0xa0, 0x53, 0x80, 0x66, 0xfc, 0x4e, 0x75,
            ],
            &[0, 6, 8, 10],
        )
    } else {
        (&[0x4e, 0x75], &[0])
    };
    authored_interrupt_engine_code(id, body, offsets)
}

fn authored_interrupt_engine_code(
    id: tje_game::FunctionId,
    body: &[u8],
    offsets: &[u32],
) -> Engine {
    let target = id.metadata().address(Revision::Rev02);
    let mut bytes = vec![0; target as usize + 32];
    bytes[..4].copy_from_slice(&0xfff000_u32.to_be_bytes());
    bytes[4..8].copy_from_slice(&0x100_u32.to_be_bytes());
    bytes[0x78..0x7c].copy_from_slice(&0x200_u32.to_be_bytes());
    bytes[0x100..0x102].copy_from_slice(&0x4eb9_u16.to_be_bytes());
    bytes[0x102..0x106].copy_from_slice(&target.to_be_bytes());
    bytes[0x106..0x108].copy_from_slice(&[0x60, 0xfe]);
    // ADDQ.W #1,($ff1000).L; RTE. Actual accepted VBlank handlers update RAM.
    bytes[0x200..0x208].copy_from_slice(&[0x52, 0x79, 0x00, 0xff, 0x10, 0x00, 0x4e, 0x73]);
    let mut pcs = vec![0x100, 0x106, 0x200, 0x206];
    bytes[target as usize..target as usize + body.len()].copy_from_slice(body);
    pcs.extend(offsets.iter().map(|offset| target + offset));
    let program = Program {
        instructions: pcs
            .into_iter()
            .map(|pc| compile_instruction(&bytes, pc).unwrap())
            .collect(),
    };
    let backend = NativeProgram::compile([0; 20], &program).unwrap();
    let mut devices = DeviceRuntime::new(
        AddressSpace::new(bytes, Revision::Rev02),
        &GenesisEmulatorConfig::default(),
    );
    devices.memory_mut().write_u32(0xfff000, 0x1234);
    let mut engine = Engine::from_devices(devices, backend);
    engine.cpu_mut().set_sr(0x2000);
    engine.devices_mut().write_immediate_word(0xc00004, 0x8120);
    engine
}

fn wait_interrupt(
    ctx: &mut dyn ExtraContext,
    continuation: &mut Continuation,
) -> Result<OverrideStep> {
    if continuation.stage == 0 {
        continuation.stage = 1;
        return Ok(OverrideStep::Wait(Wait::Interrupt));
    }
    assert_ne!(ctx.read_u16(0xff1000), 0);
    assert_eq!(ctx.status_register() & 0x0700, 0);
    ctx.sidecars_mut().get_mut::<ExtraState>()?.visits += 1;
    Ok(OverrideStep::Return(None))
}
static INTERRUPT_WAIT: &[ResumableTranslation] = &[ResumableTranslation {
    id: ids::WAIT_FOR_NEXT_V_BLANK,
    function: wait_interrupt,
}];

#[test]
fn timeaware_wait_runs_actual_vblank_handler_and_resumes_after_rte_deterministically() {
    let mut engine = authored_interrupt_engine(ids::WAIT_FOR_NEXT_V_BLANK, false);
    let mut mode = mode(INTERRUPT_WAIT);
    mode.step(&mut engine, GenesisInputs::default()).unwrap();
    mode.step(&mut engine, GenesisInputs::default()).unwrap();
    let saved = mode.snapshot(&engine).unwrap();
    for _ in 0..2 {
        mode.play_frame(&mut engine, GenesisInputs::default())
            .unwrap();
    }
    assert_eq!(mode.sidecars().get::<ExtraState>().unwrap().visits, 1);
    let expected = mode.snapshot(&engine).unwrap();
    mode.restore(&mut engine, &saved).unwrap();
    for _ in 0..2 {
        mode.play_frame(&mut engine, GenesisInputs::default())
            .unwrap();
    }
    assert_eq!(
        mode.snapshot(&engine).unwrap().as_bytes(),
        expected.as_bytes()
    );
}

fn poll_vblank(
    ctx: &mut dyn ExtraContext,
    continuation: &mut Continuation,
) -> Result<OverrideStep> {
    ctx.sidecars_mut().get_mut::<ExtraState>()?.visits += 1;
    if ctx.read_u16(0xff1000) == 0 {
        continuation.stage += 1;
        Ok(OverrideStep::Wait(Wait::Poll))
    } else {
        assert!(continuation.stage > 1);
        Ok(OverrideStep::Return(None))
    }
}
static POLLING: &[ResumableTranslation] = &[ResumableTranslation {
    id: ids::WAIT_FOR_NEXT_V_BLANK,
    function: poll_vblank,
}];
#[test]
fn polling_rechecks_real_ram_while_devices_and_interrupts_progress() {
    let mut engine = authored_interrupt_engine(ids::WAIT_FOR_NEXT_V_BLANK, false);
    let mut mode = mode(POLLING);
    for _ in 0..2 {
        mode.play_frame(&mut engine, GenesisInputs::default())
            .unwrap();
    }
    assert!(mode.sidecars().get::<ExtraState>().unwrap().visits > 1);
    assert_eq!(engine.cpu().pc, 0x106);
}

fn forward_original(
    ctx: &mut dyn ExtraContext,
    continuation: &mut Continuation,
) -> Result<OverrideStep> {
    ctx.sidecars_mut().get_mut::<ExtraState>()?.visits += 1;
    if continuation.stage == 0 {
        continuation.stage = 1;
        Ok(OverrideStep::CallOriginal {
            function: ids::WAIT_FOR_NEXT_V_BLANK,
            arguments: ctx.entry_arguments()?,
        })
    } else {
        Ok(OverrideStep::Return(None))
    }
}
static FORWARD: &[ResumableTranslation] = &[ResumableTranslation {
    id: ids::WAIT_FOR_NEXT_V_BLANK,
    function: forward_original,
}];
#[test]
fn original_timeaware_call_yields_every_frame_and_pending_call_roundtrips() {
    let mut engine = authored_interrupt_engine(ids::WAIT_FOR_NEXT_V_BLANK, true);
    let mut mode = mode(FORWARD);
    mode.play_frame(&mut engine, GenesisInputs::default())
        .unwrap();
    assert_eq!(engine.frame_count(), 1);
    assert_eq!(mode.sidecars().get::<ExtraState>().unwrap().visits, 1);
    let saved = mode.snapshot(&engine).unwrap();
    let mut expected = Vec::new();
    for frame in 2..=20 {
        mode.play_frame(&mut engine, GenesisInputs::default())
            .unwrap();
        assert_eq!(engine.frame_count(), frame);
        expected.push(mode.snapshot(&engine).unwrap());
    }
    assert_eq!(mode.sidecars().get::<ExtraState>().unwrap().visits, 2);
    assert_eq!(engine.cpu().read_address(7), 0xfff000);
    mode.restore(&mut engine, &saved).unwrap();
    for image in expected {
        mode.play_frame(&mut engine, GenesisInputs::default())
            .unwrap();
        assert_eq!(mode.snapshot(&engine).unwrap().as_bytes(), image.as_bytes());
    }
}

fn nested_parent(ctx: &mut dyn ExtraContext, state: &mut Continuation) -> Result<OverrideStep> {
    if state.stage == 0 {
        state.stage = 1;
        Ok(OverrideStep::CallOriginal {
            function: ids::LMULT,
            arguments: vec![],
        })
    } else {
        Ok(OverrideStep::Return(Some(ctx.data_register(0) + 1)))
    }
}
fn nested_child(_: &mut dyn ExtraContext, state: &mut Continuation) -> Result<OverrideStep> {
    if state.stage == 0 {
        state.stage = 1;
        Ok(OverrideStep::Wait(Wait::Frames(1)))
    } else {
        Ok(OverrideStep::Return(Some(0x20)))
    }
}
static NESTED_RESUMABLE: &[ResumableTranslation] = &[
    ResumableTranslation {
        id: ids::ABS,
        function: nested_parent,
    },
    ResumableTranslation {
        id: ids::LDIVT,
        function: nested_child,
    },
];
#[test]
fn scheduler_original_keeps_nested_dispatch_and_nested_continuations_in_snapshots() {
    let mut engine = synthetic(ids::ABS, true);
    let mut mode = mode(NESTED_RESUMABLE);
    mode.play_frame(&mut engine, GenesisInputs::default())
        .unwrap();
    let saved = mode.snapshot(&engine).unwrap();
    mode.play_frame(&mut engine, GenesisInputs::default())
        .unwrap();
    assert_eq!(engine.cpu().d[0], 0x22);
    assert_eq!(engine.cpu().read_address(7), 0xfff000);
    let expected = mode.snapshot(&engine).unwrap();
    mode.restore(&mut engine, &saved).unwrap();
    mode.play_frame(&mut engine, GenesisInputs::default())
        .unwrap();
    assert_eq!(
        mode.snapshot(&engine).unwrap().as_bytes(),
        expected.as_bytes()
    );
}

fn record_input(ctx: &mut dyn ExtraContext, _: &mut Continuation) -> Result<OverrideStep> {
    let pads = ctx.inputs().pads;
    ctx.sidecars_mut().get_mut::<ExtraState>()?.pads = pads;
    Ok(OverrideStep::Return(None))
}
static INPUT: &[ResumableTranslation] = &[ResumableTranslation {
    id: ids::ABS,
    function: record_input,
}];
fn four_pads() -> EnhancedInputs {
    let pads = std::array::from_fn(|index| {
        let mut pad = GenesisController::new(GenesisControllerType::ThreeButton);
        pad.set_field(
            [
                GenesisButton::A,
                GenesisButton::B,
                GenesisButton::C,
                GenesisButton::Start,
            ][index],
            true,
        );
        pad
    });
    EnhancedInputs { pads }
}
#[test]
fn four_pads_route_to_override_and_are_included_in_snapshot_hash() {
    let mut engine = synthetic(ids::ABS, false);
    let mut mode = mode(INPUT);
    let inputs = four_pads();
    mode.step(&mut engine, inputs).unwrap();
    let saved = mode.snapshot(&engine).unwrap();
    mode.step(&mut engine, inputs).unwrap();
    assert_eq!(
        mode.sidecars().get::<ExtraState>().unwrap().pads,
        inputs.pads
    );
    let first = mode.snapshot(&engine).unwrap();
    mode.restore(&mut engine, &saved).unwrap();
    let mut changed = inputs;
    changed.pads[3] = GenesisController::None;
    mode.step(&mut engine, changed).unwrap();
    assert_ne!(
        mode.snapshot(&engine).unwrap().state_hash(),
        first.state_hash()
    );
    assert_eq!(changed.original(), inputs.original());
    mode.restore(&mut engine, &first).unwrap();
    assert_eq!(
        mode.sidecars().get::<ExtraState>().unwrap().pads,
        inputs.pads
    );
    assert_eq!(mode.snapshot(&engine).unwrap().as_bytes(), first.as_bytes());
}

#[test]
fn faithful_driver_ignores_extra_pads_and_preserves_exact_format2_state() {
    let mut a = synthetic(ids::ABS, false);
    let mut b = synthetic(ids::ABS, false);
    let mut off = EnhancedMode::default();
    let inputs = four_pads();
    a.play_frame(inputs.original()).unwrap();
    off.play_frame(&mut b, inputs).unwrap();
    let expected = a.snapshot().unwrap();
    assert_eq!(off.snapshot(&b).unwrap().as_bytes(), expected.as_bytes());
    assert_eq!(
        u32::from_le_bytes(expected.as_bytes()[8..12].try_into().unwrap()),
        2
    );
    assert_eq!(
        off.snapshot(&b).unwrap().state_hash(),
        expected.state_hash()
    );
}

#[test]
fn course_profile_schema_and_plain_machine_mismatch_reject_transactionally() {
    let mut engine = synthetic(ids::ABS, false);
    let mut source = mode(INPUT);
    source.step(&mut engine, four_pads()).unwrap();
    let image = source.snapshot(&engine).unwrap();
    let before = engine.snapshot().unwrap();
    let mut wrong_course = EnhancedMode::new(profile(INPUT), Revision::Rev02).unwrap();
    wrong_course.sidecars_mut().register(initial(8)).unwrap();
    assert!(wrong_course.restore(&mut engine, &image).is_err());
    assert_eq!(
        wrong_course.sidecars().get::<ExtraState>().unwrap().course,
        8
    );
    assert_eq!(engine.snapshot().unwrap().as_bytes(), before.as_bytes());
    let mut wrong_profile = EnhancedMode::new(
        EnhancedProfile::new("other-profile", &[], &[]).with_resumable(INPUT),
        Revision::Rev02,
    )
    .unwrap();
    wrong_profile.sidecars_mut().register(initial(7)).unwrap();
    assert_eq!(
        wrong_profile.restore(&mut engine, &image),
        Err(SnapshotError::ProfileMismatch)
    );
    assert_eq!(
        engine.restore_with_profile(&image, source.profile_id()),
        Err(SnapshotError::ProfileMismatch)
    );
    assert_eq!(
        source.restore(&mut engine, &before),
        Err(SnapshotError::ProfileMismatch)
    );
    assert_eq!(engine.snapshot().unwrap().as_bytes(), before.as_bytes());
    source
        .sidecars_mut()
        .get_mut::<ExtraState>()
        .unwrap()
        .visits = 99;
    source.reset(&mut engine);
    assert_eq!(source.sidecars().get::<ExtraState>().unwrap(), &initial(7));
}

#[test]
fn registry_rejects_duplicate_and_wrong_typed_keys() {
    #[derive(Encode, Decode)]
    struct Other(u32);
    impl Sidecar for Other {
        const KEY: &'static str = ExtraState::KEY;
        const VERSION: u32 = 2;
    }
    let mut mode = mode(INPUT);
    assert!(mode.sidecars_mut().register(initial(7)).is_err());
    assert!(mode.sidecars().get::<Other>().is_err());
    let mut engine = synthetic(ids::ABS, false);
    let image = mode.snapshot(&engine).unwrap();
    let mut wrong_schema = EnhancedMode::new(profile(INPUT), Revision::Rev02).unwrap();
    wrong_schema.sidecars_mut().register(Other(0)).unwrap();
    assert!(wrong_schema.restore(&mut engine, &image).is_err());
    assert_eq!(mode.snapshot(&engine).unwrap().as_bytes(), image.as_bytes());
}

fn observe_original(ctx: &mut dyn ExtraContext, _: &mut Continuation) -> Result<OverrideStep> {
    ctx.sidecars_mut().get_mut::<ExtraState>()?.visits += 1;
    Ok(OverrideStep::ContinueOriginal)
}
static OBSERVE: &[ResumableTranslation] = &[ResumableTranslation {
    id: ids::WAIT_FOR_NEXT_V_BLANK,
    function: observe_original,
}];

#[test]
fn before_only_original_bypass_survives_interrupt_and_snapshot_without_reentering_hook() {
    let mut engine = authored_interrupt_engine(ids::WAIT_FOR_NEXT_V_BLANK, false);
    let mut mode = mode(OBSERVE);
    mode.step(&mut engine, GenesisInputs::default()).unwrap();
    mode.step(&mut engine, GenesisInputs::default()).unwrap();
    engine.cpu_mut().pending_interrupt = 6;
    let saved = mode.snapshot(&engine).unwrap();
    mode.play_frame(&mut engine, GenesisInputs::default())
        .unwrap();
    assert_eq!(mode.sidecars().get::<ExtraState>().unwrap().visits, 1);
    assert_eq!(engine.cpu().pc, 0x106);
    assert_eq!(engine.cpu().read_address(7), 0xfff000);
    let expected = mode.snapshot(&engine).unwrap();
    mode.restore(&mut engine, &saved).unwrap();
    mode.play_frame(&mut engine, GenesisInputs::default())
        .unwrap();
    assert_eq!(
        mode.snapshot(&engine).unwrap().as_bytes(),
        expected.as_bytes()
    );
}

fn fail_resumable(ctx: &mut dyn ExtraContext, _: &mut Continuation) -> Result<OverrideStep> {
    ctx.sidecars_mut().get_mut::<ExtraState>()?.visits += 1;
    Err(tje_game::Error::Platform(
        "authored resumable failure".into(),
    ))
}
static FAIL_RESUMABLE: &[ResumableTranslation] = &[ResumableTranslation {
    id: ids::ABS,
    function: fail_resumable,
}];

#[test]
fn resumable_failure_is_latched_and_atomic_restore_releases_it_without_repeating_effects() {
    let mut engine = synthetic(ids::ABS, false);
    let mut mode = mode(FAIL_RESUMABLE);
    let saved = mode.snapshot(&engine).unwrap();
    mode.step(&mut engine, GenesisInputs::default()).unwrap();
    assert!(mode.step(&mut engine, GenesisInputs::default()).is_err());
    let clock = engine.devices().clock();
    assert!(mode.snapshot(&engine).is_err());
    assert!(matches!(
        mode.step(&mut engine, GenesisInputs::default()),
        Err(tje_engine_runtime::enhanced::Error::Failed)
    ));
    assert_eq!(mode.sidecars().get::<ExtraState>().unwrap().visits, 1);
    assert_eq!(engine.devices().clock(), clock);
    mode.restore(&mut engine, &saved).unwrap();
    assert!(!mode.is_failed());
    assert_eq!(mode.sidecars().get::<ExtraState>().unwrap().visits, 0);
}

#[test]
fn enhanced_payload_version_corruption_and_hardware_failures_do_not_partially_restore() {
    use sha1::{Digest, Sha1};
    let mut engine = synthetic(ids::ABS, false);
    let mut mode = mode(INPUT);
    let saved = mode.snapshot(&engine).unwrap();
    let mut bad_version = saved.as_bytes().to_vec();
    let checksum_at = bad_version.len() - 20;
    let length_at = checksum_at - 4;
    let length =
        u32::from_le_bytes(bad_version[length_at..checksum_at].try_into().unwrap()) as usize;
    let payload_at = length_at - length;
    bad_version[payload_at..payload_at + 4].copy_from_slice(&99_u32.to_le_bytes());
    let checksum = Sha1::digest(&bad_version[..checksum_at]);
    bad_version[checksum_at..].copy_from_slice(&checksum);
    let version_image = MachineSnapshot::from_bytes(bad_version).unwrap();
    mode.sidecars_mut().get_mut::<ExtraState>().unwrap().visits = 99;
    let before = mode.snapshot(&engine).unwrap();
    assert!(mode.restore(&mut engine, &version_image).is_err());
    assert_eq!(
        mode.snapshot(&engine).unwrap().as_bytes(),
        before.as_bytes()
    );
    let mut bad_length = saved.as_bytes().to_vec();
    bad_length[length_at..checksum_at].copy_from_slice(&u32::MAX.to_le_bytes());
    let checksum = Sha1::digest(&bad_length[..checksum_at]);
    bad_length[checksum_at..].copy_from_slice(&checksum);
    assert_eq!(
        MachineSnapshot::from_bytes(bad_length).unwrap_err(),
        SnapshotError::Corrupt
    );
    let mut bad_hardware = saved.as_bytes().to_vec();
    // Truncate only the hardware payload, leaving the enhanced envelope valid.
    bad_hardware.remove(payload_at - 1);
    let hardware_checksum_at = bad_hardware.len() - 20;
    let checksum = Sha1::digest(&bad_hardware[..hardware_checksum_at]);
    bad_hardware[hardware_checksum_at..].copy_from_slice(&checksum);
    let hardware_image = MachineSnapshot::from_bytes(bad_hardware).unwrap();
    assert!(mode.restore(&mut engine, &hardware_image).is_err());
    assert_eq!(
        mode.snapshot(&engine).unwrap().as_bytes(),
        before.as_bytes()
    );
}

fn forward_abs(ctx: &mut dyn ExtraContext, state: &mut Continuation) -> Result<OverrideStep> {
    assert_eq!(ctx.entry_arguments()?, vec![0x1234]);
    if state.stage == 0 {
        state.stage = 1;
        Ok(OverrideStep::CallOriginal {
            function: ids::ABS,
            arguments: ctx.entry_arguments()?,
        })
    } else {
        Ok(OverrideStep::Return(Some(ctx.data_register(0) + 1)))
    }
}
static FORWARD_ABS: &[ResumableTranslation] = &[ResumableTranslation {
    id: ids::ABS,
    function: forward_abs,
}];

#[test]
fn managed_original_releases_only_its_argument_area_and_roundtrips_at_completed_rts() {
    let mut engine = synthetic(ids::ABS, false);
    let mut mode = mode(FORWARD_ABS);
    for _ in 0..4 {
        mode.step(&mut engine, GenesisInputs::default()).unwrap();
    }
    assert_eq!(
        engine.cpu().pc,
        ids::ABS.metadata().address(Revision::Rev02)
    );
    assert_eq!(engine.cpu().read_address(7), 0xffeff8);
    let saved = mode.snapshot(&engine).unwrap();
    mode.step(&mut engine, GenesisInputs::default()).unwrap();
    assert_eq!(engine.cpu().pc, 0x106);
    assert_eq!(engine.cpu().read_address(7), 0xfff000);
    assert_eq!(engine.cpu().d[0], 8);
    assert_eq!(engine.devices().memory().read_u32(0xfff000), 0x1234);
    let expected = mode.snapshot(&engine).unwrap();
    mode.restore(&mut engine, &saved).unwrap();
    mode.step(&mut engine, GenesisInputs::default()).unwrap();
    assert_eq!(
        mode.snapshot(&engine).unwrap().as_bytes(),
        expected.as_bytes()
    );
}

#[test]
fn sidecar_only_mutation_changes_whole_image_netplay_hash_without_touching_original_ram() {
    let mut engine = synthetic(ids::ABS, false);
    let mut mode = mode(INPUT);
    let hardware = engine.snapshot().unwrap();
    let saved = mode.snapshot(&engine).unwrap();
    mode.sidecars_mut().get_mut::<ExtraState>().unwrap().seats[3] = 99;
    assert_ne!(
        mode.snapshot(&engine).unwrap().state_hash(),
        saved.state_hash()
    );
    assert_eq!(engine.snapshot().unwrap().as_bytes(), hardware.as_bytes());
    mode.restore(&mut engine, &saved).unwrap();
    assert_eq!(mode.sidecars().get::<ExtraState>().unwrap().seats[3], 0);
    assert_eq!(
        mode.snapshot(&engine).unwrap().state_hash(),
        saved.state_hash()
    );
}

#[test]
fn original_polling_backedge_to_entry_is_not_mistaken_for_a_nested_override() {
    // SUBQ.L #1,D0; BNE function_entry; RTS. The first opcode is the loop head.
    let mut engine = authored_interrupt_engine_code(
        ids::WAIT_FOR_NEXT_V_BLANK,
        &[0x53, 0x80, 0x66, 0xfc, 0x4e, 0x75],
        &[0, 2, 4],
    );
    engine.cpu_mut().d[0] = 100_000;
    let mut mode = mode(FORWARD);
    mode.play_frame(&mut engine, GenesisInputs::default())
        .unwrap();
    assert_eq!(mode.sidecars().get::<ExtraState>().unwrap().visits, 1);
    let saved = mode.snapshot(&engine).unwrap();
    for _ in 0..19 {
        mode.play_frame(&mut engine, GenesisInputs::default())
            .unwrap();
    }
    assert_eq!(mode.sidecars().get::<ExtraState>().unwrap().visits, 2);
    assert_eq!(engine.cpu().pc, 0x106);
    assert_eq!(engine.cpu().read_address(7), 0xfff000);
    let expected = mode.snapshot(&engine).unwrap();
    mode.restore(&mut engine, &saved).unwrap();
    for _ in 0..19 {
        mode.play_frame(&mut engine, GenesisInputs::default())
            .unwrap();
    }
    assert_eq!(
        mode.snapshot(&engine).unwrap().as_bytes(),
        expected.as_bytes()
    );
}

#[test]
#[ignore = "requires both private ROM revisions; actual TimeAware VBlank forwarding and full state replay"]
fn recorded_timeaware_original_calls_and_sidecars_replay_identically_in_both_revisions() {
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
        assert!(matches!(session.start, Start::PowerOn));
        let pads: Vec<_> = session.inputs().take(600).collect();
        assert_eq!(pads.len(), 600);
        let dir = tempfile::tempdir().unwrap();
        let cache = CacheConfig {
            data_dir: dir.path().to_owned(),
        };
        let mut engine = load_engine(
            Rom::import(&bytes).unwrap(),
            &GenesisEmulatorConfig::default(),
            &cache,
            |_| {},
        )
        .unwrap();
        let mut mode = EnhancedMode::new(profile(FORWARD), revision).unwrap();
        mode.sidecars_mut().register(initial(7)).unwrap();
        for &pad in &pads[..300] {
            mode.play_frame(&mut engine, inputs(pad)).unwrap();
        }
        let checkpoint = mode.snapshot(&engine).unwrap();
        let mut expected = Vec::new();
        let mut first = Hashes::default();
        for &pad in &pads[300..] {
            mode.play_frame(&mut engine, inputs(pad)).unwrap();
            first.observe(&engine);
            expected.push(mode.snapshot(&engine).unwrap().state_hash());
        }
        assert_eq!(engine.frame_count(), 600);
        let calls = mode.sidecars().get::<ExtraState>().unwrap().visits;
        assert!(calls > 0);
        mode.restore(&mut engine, &checkpoint).unwrap();
        let mut replay = Hashes::default();
        for (&pad, expected) in pads[300..].iter().zip(expected) {
            mode.play_frame(&mut engine, inputs(pad)).unwrap();
            replay.observe(&engine);
            assert_eq!(mode.snapshot(&engine).unwrap().state_hash(), expected);
        }
        assert_eq!(mode.sidecars().get::<ExtraState>().unwrap().visits, calls);
        let hashes = first.finish();
        assert_eq!(replay.finish(), hashes);
        eprintln!(
            "{revision:?} resumable-vblank frames=600 replayed=300 callbacks={calls} video_sha256={} audio_sha256={} snapshot_hashes_identical=true",
            hashes.0, hashes.1
        );
    }
}
