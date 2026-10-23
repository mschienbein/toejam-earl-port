use super::*;
use crate::{
    CheckReport, Mode,
    abi::{AtomicRun, HookContext},
    fixture::tests::synthetic_host,
    hooks::HookFn,
    input::InputScript,
};
use journal::{DerivedReads, JournalRead};
use m68000_emu::{BusInterface, ReadPurpose};
use std::sync::Arc;
use tje_game::{
    Context, Dispatcher,
    call_frame::{FrameState, LocalFrame},
    execution::{ExecutionClass, WaitRequest},
    exits::{MainLoopStage, StackBanks},
};
use tje_mem::{Bus, Revision, ram};

const CODE: u32 = 0x100;
const OUTPUT: u32 = 0xff9000;
const STORED: u32 = 0xff9020;
const PROGRAM: &[u16] = &[
    // Authored LINK #-4; MOVE.W -2(A6),D0; EXT.L D0; MOVE.W D0,$ff9000;
    // UNLK A6; RTS. These are synthetic instructions, not cartridge bytes.
    0x4e56, 0xfffc, 0x302e, 0xfffe, 0x48c0, 0x33c0, 0x00ff, 0x9000, 0x4e5e, 0x4e75,
];
static SYNTHETIC: DependentContract = DependentContract {
    function: ids::GET_RANDOM_NUMBER,
    frame: FrameLayout::link(4, 0, 0),
    ranges: &[FrameByteRange {
        entry_sp_offset: -6,
        bytes: 2,
    }],
    producer: CompiledProducer {
        function: ids::GET_RANDOM_NUMBER,
        caller: ids::GET_RANDOM_NUMBER,
        caller_frame: FrameLayout::link(4, 0, 0),
        frame: FrameLayout::link(4, 0, 0),
        entry_sp_delta: 0,
        source: "authored ROM-free frame input",
    },
    source: "authored LINK #-4; semantic word at -2(A6)",
};

#[derive(Clone, Copy)]
enum Mutation {
    None,
    Offset,
    Width,
    Mirror,
    Stored,
    ExtraWrite,
    Purpose,
    Flags,
    Independent,
}
fn body(context: &mut HookContext<'_>, mutation: Mutation) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::link(4, 0, 0))?;
    let address = frame.local::<i16>(-2).address();
    let value = match mutation {
        Mutation::Offset => context.read_u16(address - 2),
        Mutation::Width => {
            u16::from_be_bytes([context.read_u8(address), context.read_u8(address + 1)])
        }
        Mutation::Mirror => context.read_u16(address & 0xefffff),
        Mutation::Purpose => context
            .bus()
            .read_word_tagged(address, ReadPurpose::Discarded),
        Mutation::Stored | Mutation::Independent => {
            let _actual = context.read_u16(address);
            if matches!(mutation, Mutation::Stored) {
                context.read_u16(STORED)
            } else {
                0
            }
        }
        _ => context.read_u16(address),
    } as i16;
    context.write_u16(OUTPUT, value as u16);
    if matches!(mutation, Mutation::ExtraWrite) {
        context.write_u16(OUTPUT, value as u16);
    }
    context.set_status_register(
        (context.status_register() & 0xfff0)
            | if value == 0 {
                4
            } else if value < 0 {
                8
            } else {
                0
            },
    );
    if matches!(mutation, Mutation::Flags) {
        context.set_status_register(context.status_register() ^ 1);
    }
    frame.leave(context)?;
    Ok(Some(i32::from(value) as u32))
}
fn correct(c: &mut HookContext<'_>) -> Result<Option<u32>> {
    body(c, Mutation::None)
}
fn wrong_offset(c: &mut HookContext<'_>) -> Result<Option<u32>> {
    body(c, Mutation::Offset)
}
fn wrong_width(c: &mut HookContext<'_>) -> Result<Option<u32>> {
    body(c, Mutation::Width)
}
fn wrong_mirror(c: &mut HookContext<'_>) -> Result<Option<u32>> {
    body(c, Mutation::Mirror)
}
fn stored_value(c: &mut HookContext<'_>) -> Result<Option<u32>> {
    body(c, Mutation::Stored)
}
fn extra_write(c: &mut HookContext<'_>) -> Result<Option<u32>> {
    body(c, Mutation::ExtraWrite)
}
fn wrong_purpose(c: &mut HookContext<'_>) -> Result<Option<u32>> {
    body(c, Mutation::Purpose)
}
fn wrong_flags(c: &mut HookContext<'_>) -> Result<Option<u32>> {
    body(c, Mutation::Flags)
}
fn independent(c: &mut HookContext<'_>) -> Result<Option<u32>> {
    body(c, Mutation::Independent)
}
fn read_after_write(c: &mut HookContext<'_>) -> Result<Option<u32>> {
    let result = body(c, Mutation::None)?;
    let address = c.frame_state().unwrap().entry_sp - 6;
    c.write_u16(address, 0);
    let _extra = c.read_u16(address);
    Ok(result)
}

fn host(program: &[u16], function: HookFn, marked: bool) -> Host {
    let mut host = synthetic_host(program, &[0x4e73]);
    let mut symbol = ids::GET_RANDOM_NUMBER.metadata().clone();
    symbol.name = "AuthoredDependent";
    symbol.addresses = [CODE; 2];
    symbol.sizes = [2 * program.len() as u32; 2];
    symbol.abi.argument_count = 0;
    let mut registry = HookRegistry::empty(Revision::Rev02);
    registry.register_contract(symbol, true, function).unwrap();
    if marked {
        // Authored unit contracts are private test inputs, not additions to the
        // production reviewed list or a public arbitrary-address waiver.
        registry.set_authored_dependent_proof(
            CODE,
            ProofSetting {
                contract: &SYNTHETIC,
                config: DependentProofConfig::default(),
            },
        );
    }
    host.registry = registry;
    host.checks = CheckReport::registered(&host.registry);
    let root_sp = host.cpu().stack_pointer() - 4;
    host.context().write_u16(root_sp - 6, 0x1234);
    host.context().write_u16(root_sp - 8, 0x1234);
    host.context().write_u16(STORED, 0x1234);
    host
}

#[test]
fn genuine_dependence_is_coupled_not_independent_or_informational() {
    let mut rejected = host(PROGRAM, correct, false);
    assert!(rejected.check_fixture(CODE, &[]).is_err());
    assert!(!rejected.checks.uninitialized_stack_reads.is_empty());
    let mut proven = host(PROGRAM, correct, true);
    assert_eq!(proven.check_fixture(CODE, &[]).unwrap(), 0x1234);
    assert!(proven.checks.uninitialized_stack_reads.is_empty());
    assert!(proven.checks.nondependent_stack_reads.is_empty());
    assert_eq!(proven.checks.dependent_stack_reads.len(), 4); // original/Rust PCs, two bytes each
    let evidence = &proven.checks.dependent_equivalence["AuthoredDependent"];
    assert_eq!(evidence.calls_proven, 1);
    assert_eq!(evidence.seeded_probes_completed, 16);
    assert_eq!(evidence.byte_probes_completed, 8);
    assert_eq!(evidence.probe_pairs_matched, 24);
    assert!(evidence.sensitive_probe_pairs > 0);
    assert_eq!(evidence.original_runs_completed, 24);
    assert_eq!(evidence.rust_runs_completed, 24);
    assert_eq!(proven.checks.stack_dependency_reruns, 48);
    assert_eq!(evidence.last_read_trace.len(), 1);
    assert_eq!(evidence.last_read_trace[0].width, 2);
}

#[test]
fn offset_width_mirror_and_redundant_write_mutants_fail() {
    for (function, detail) in [
        (wrong_offset as HookFn, "outside the reviewed"),
        (wrong_width, "read trace differs"),
        (wrong_mirror, "read trace differs"),
        (extra_write, "write/value footprint differs"),
        (wrong_purpose, "read trace differs"),
        (wrong_flags, "full SR/CCR"),
    ] {
        let mut mutant = host(PROGRAM, function, true);
        let error = mutant.check_fixture(CODE, &[]).unwrap_err();
        assert!(error.to_string().contains(detail), "{error}");
        assert_eq!(
            mutant.checks.dependent_equivalence["AuthoredDependent"].calls_rejected,
            1
        );
    }
}

#[test]
fn reading_the_right_byte_but_substituting_a_stored_value_fails_seeded_probe() {
    let mut mutant = host(PROGRAM, stored_value, true);
    let error = mutant.check_fixture(CODE, &[]).unwrap_err();
    assert!(
        error.to_string().contains("write/value footprint differs"),
        "{error}"
    );
    let evidence = &mutant.checks.dependent_equivalence["AuthoredDependent"];
    assert_eq!(evidence.seeded_probes_completed, 1);
    assert_eq!(evidence.probe_pairs_matched, 0);
    assert_eq!(evidence.original_runs_completed, 1);
    assert_eq!(evidence.rust_runs_completed, 1);
}

#[test]
fn initialized_reads_of_the_same_dependent_bytes_are_not_dropped() {
    // After the real residual read/store, CLR.W -2(A6) initializes both bytes.
    // Rust's extra read after that store has no effect on outputs or writes.
    let mut program = PROGRAM.to_vec();
    program.splice(8..8, [0x426e, 0xfffe]);
    let mut mutant = host(&program, read_after_write, true);
    let error = mutant.check_fixture(CODE, &[]).unwrap_err();
    assert!(error.to_string().contains("read trace differs"), "{error}");
    let evidence = &mutant.checks.dependent_equivalence["AuthoredDependent"];
    assert_eq!(evidence.seeded_probes_completed, 0);
    assert_eq!(evidence.calls_rejected, 1);
}

#[test]
fn unchanged_original_outputs_do_not_establish_true_dependence() {
    // The authored variant consumes the old local but then CLR.L D0.
    let mut program = PROGRAM.to_vec();
    program.splice(5..5, [0x4280]);
    let mut host = host(&program, independent, true);
    let error = host.check_fixture(CODE, &[]).unwrap_err();
    assert!(
        error.to_string().contains("no original output sensitivity"),
        "{error}"
    );
    let evidence = &host.checks.dependent_equivalence["AuthoredDependent"];
    assert_eq!(evidence.probe_pairs_matched, 24);
    assert_eq!(evidence.sensitive_probe_pairs, 0);
}

#[test]
fn native_bus_widths_feed_stack_and_outer_timed_ram_journals_once() {
    let mut host = synthetic_host(&[0x60fe], &[0x4e73]);
    let mut context = host
        .context()
        .with_execution(ExecutionClass::TimeAware, Default::default());
    context.enable_stack_audit();
    context.enable_dependent_read_journal();
    context.write_u32(0xff7fe0, 0x1234_5678);
    assert_eq!(Bus::read_u32(&context, 0xff7fe0), 0x1234_5678);
    {
        let journal = context.stack_reads.borrow();
        let reads = journal.as_ref().unwrap();
        assert_eq!(reads.len(), 2);
        assert_eq!(
            reads
                .iter()
                .map(|event| (event.read.raw_address, event.read.width, event.read.value))
                .collect::<Vec<_>>(),
            [(0xff7fe0, 2, 0x1234), (0xff7fe2, 2, 0x5678)]
        );
    }
    let count = |trace: &crate::check_projection::LogicTrace| {
        trace
            .events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    crate::check_projection::LogicEvent::Access {
                        access: genesis_core::HybridAccess::Read {
                            address: 0xff7fe0 | 0xff7fe2,
                            ..
                        },
                        ..
                    }
                )
            })
            .count()
    };
    assert_eq!(count(&context.logic_trace.borrow()), 2);
    context.set_execution_class(ExecutionClass::Atomic).unwrap();
    assert_eq!(Bus::read_u16(&context, 0xff7fe0), 0x1234);
    assert_eq!(count(&context.logic_trace.borrow()), 3);
}

#[test]
fn activation_is_reviewed_explicit_atomic_and_preserved_for_continuations() {
    fn never(_: &mut HookContext<'_>) -> Result<Option<u32>> {
        panic!("configuration executed an entry")
    }
    let revision = Revision::Rev02;
    let mut registry = HookRegistry::empty(revision);
    registry
        .register_contract(
            ids::EARTHLING_STEER_TOWARD_NEAREST_TARGET
                .metadata()
                .clone(),
            true,
            never,
        )
        .unwrap();
    let id = ids::EARTHLING_STEER_TOWARD_NEAREST_TARGET;
    let config = DependentProofConfig {
        seed: 123,
        seeded_probes: 16,
    };
    assert!(
        registry
            .enable_dependent_equivalence(
                &[id],
                DependentProofConfig {
                    seeded_probes: 15,
                    ..config
                }
            )
            .is_err()
    );
    registry.enable_dependent_equivalence(&[], config).unwrap();
    assert!(
        registry
            .at(id.metadata().address(revision))
            .unwrap()
            .dependent_proof
            .is_none()
    );
    assert!(
        registry
            .enable_dependent_equivalence(&[id, ids::ABS], config)
            .is_err()
    );
    assert!(
        registry
            .at(id.metadata().address(revision))
            .unwrap()
            .dependent_proof
            .is_none()
    );
    registry
        .enable_dependent_equivalence(&[id], config)
        .unwrap();
    let mut hook = registry
        .at(id.metadata().address(revision))
        .unwrap()
        .clone();
    hook.exit_contract = tje_game::exits::contract(ids::FURTHER_INITIALISATION);
    fn continuation(_: &mut dyn Context, _: &[u32]) -> tje_game::Result<Option<u32>> {
        Ok(None)
    }
    let (copy, _) = hook
        .continuation(
            MainLoopStage::Idle,
            continuation,
            revision,
            FrameState::new(0xff7ff0),
            0xff7fd0,
        )
        .unwrap();
    assert_eq!(copy.dependent_proof.unwrap().config, config);
    assert!(std::ptr::eq(
        copy.dependent_proof.unwrap().contract,
        hook.dependent_proof.unwrap().contract
    ));
    assert_eq!(copy.dependent_frame_sp(0xff7fd0), 0xff7ff0);
}

#[test]
fn journal_derives_each_side_before_write_and_keeps_purposes_and_raw_aliases() {
    let address = 0xff7fe0;
    let mut audit = AtomicRun {
        stack_reads: Some(Vec::new()),
        ..AtomicRun::default()
    };
    let reads = audit.stack_reads.as_mut().unwrap();
    reads.push(JournalRead::new(1, address, true, ReadPurpose::Prefetch, 0x1234, 0).unwrap());
    reads.push(JournalRead::new(2, address, true, ReadPurpose::Semantic, 0x1234, 0).unwrap());
    reads.push(JournalRead::new(3, address, true, ReadPurpose::Discarded, 0x1234, 0).unwrap());
    audit.writes.push(OrderedWrite::Ram { address, value: 1 });
    reads.push(JournalRead::new(4, address, true, ReadPurpose::Semantic, 0x0134, 1).unwrap());
    let derived = DerivedReads::derive(&audit, 0xff7ff0);
    assert_eq!(derived.bytes, BTreeSet::from([address, address + 1]));
    let trace = derived.trace(&derived.bytes);
    assert_eq!(
        trace.iter().map(|read| read.purpose).collect::<Vec<_>>(),
        ["prefetch", "semantic", "discarded", "semantic"]
    );
    let mirror = JournalRead::new(1, 0xef7fe0, true, ReadPurpose::Semantic, 0x1234, 0).unwrap();
    assert_eq!(mirror.read.canonical_address, address);
    assert_ne!(mirror.read.raw_address, address);
    let dependent_bytes = derived.bytes.clone();
    drop(derived);
    // Same-side interleaving is replayed, not compared as a numeric index.
    audit.writes.push(OrderedWrite::Ram {
        address: address + 1,
        value: 2,
    });
    audit
        .stack_reads
        .as_mut()
        .unwrap()
        .push(JournalRead::new(5, address, true, ReadPurpose::Semantic, 0x0102, 2).unwrap());
    assert_eq!(
        DerivedReads::derive(&audit, 0xff7ff0)
            .trace(&dependent_bytes)
            .len(),
        5
    );
}

#[test]
fn boundary_values_cover_each_byte_and_seeded_assignments_are_reproducible() {
    let addresses = [0xff7fe0, 0xff7fe3];
    let mut a = synthetic_host(&[0x60fe], &[0x4e73]);
    let mut b = synthetic_host(&[0x60fe], &[0x4e73]);
    for index in 0..16 {
        perturb(&mut a.emulator, &addresses, Probe::Seeded(index), 1234);
        perturb(&mut b.emulator, &addresses, Probe::Seeded(index), 1234);
        assert_eq!(
            a.context().read_u32(addresses[0]),
            b.context().read_u32(addresses[0])
        );
    }
    for address in addresses {
        for value in [0x00, 0xff, 0x7f, 0x80] {
            perturb(
                &mut a.emulator,
                &addresses,
                Probe::Byte { address, value },
                1234,
            );
            assert_eq!(a.context().read_u8(address), value);
        }
    }
}

fn nearest_function() -> tje_game::TranslationFn {
    tje_game::earthlings::PENDING_TRANSLATIONS
        .iter()
        .find(|entry| entry.id == ids::EARTHLING_STEER_TOWARD_NEAREST_TARGET)
        .expect("complete pending nearest-target body")
        .function
}
fn nearest(c: &mut HookContext<'_>) -> Result<Option<u32>> {
    let args = std::array::from_fn::<_, 4, _>(|index| c.argument_u32(index as u32));
    Ok(nearest_function()(c, &args)?)
}

struct MutantContext<'a> {
    inner: &'a mut dyn Context,
    residual: [u32; 2],
    stored: [u16; 2],
    substitute: bool,
}
impl Bus for MutantContext<'_> {
    fn read_u8(&self, address: u32) -> u8 {
        self.inner.read_u8(address)
    }
    fn read_u16(&self, address: u32) -> u16 {
        if let Some(index) = self
            .residual
            .iter()
            .position(|candidate| *candidate == address)
        {
            if self.substitute {
                let _actual = self.inner.read_u16(address);
                self.stored[index]
            } else {
                self.inner.read_u16(address - 2)
            }
        } else {
            self.inner.read_u16(address)
        }
    }
    fn read_u32(&self, address: u32) -> u32 {
        self.inner.read_u32(address)
    }
    fn write_u8(&mut self, address: u32, value: u8) {
        self.inner.write_u8(address, value);
    }
    fn write_u16(&mut self, address: u32, value: u16) {
        self.inner.write_u16(address, value);
    }
    fn write_u32(&mut self, address: u32, value: u32) {
        self.inner.write_u32(address, value);
    }
}
impl tje_mem::Platform for MutantContext<'_> {
    fn write_vdp_control(&mut self, value: u16) {
        self.inner.write_vdp_control(value);
    }
    fn write_vdp_data(&mut self, value: u16) {
        self.inner.write_vdp_data(value);
    }
    fn write_vdp_command(&mut self, value: u32) {
        self.inner.write_vdp_command(value);
    }
}
impl tje_game::Platform for MutantContext<'_> {
    fn read_device_u8(&mut self, address: u32) -> u8 {
        self.inner.read_device_u8(address)
    }
    fn read_device_u16(&mut self, address: u32) -> u16 {
        self.inner.read_device_u16(address)
    }
    fn write_device_u8(&mut self, address: u32, value: u8) {
        self.inner.write_device_u8(address, value);
    }
    fn write_device_u16(&mut self, address: u32, value: u16) {
        self.inner.write_device_u16(address, value);
    }
    fn wait_until(&mut self, request: WaitRequest) -> tje_game::Result<()> {
        self.inner.wait_until(request)
    }
}
impl Context for MutantContext<'_> {
    fn revision(&self) -> Revision {
        self.inner.revision()
    }
    fn data_register(&self, index: usize) -> u32 {
        self.inner.data_register(index)
    }
    fn set_data_register(&mut self, index: usize, value: u32) {
        self.inner.set_data_register(index, value);
    }
    fn address_register(&self, index: usize) -> u32 {
        self.inner.address_register(index)
    }
    fn set_address_register(&mut self, index: usize, value: u32) {
        self.inner.set_address_register(index, value);
    }
    fn status_register(&self) -> u16 {
        self.inner.status_register()
    }
    fn set_status_register(&mut self, value: u16) {
        self.inner.set_status_register(value);
    }
    fn stack_pointer(&self) -> tje_game::Result<u32> {
        self.inner.stack_pointer()
    }
    fn set_stack_pointer(&mut self, value: u32) -> tje_game::Result<()> {
        self.inner.set_stack_pointer(value)
    }
    fn frame_state(&self) -> Option<FrameState> {
        self.inner.frame_state()
    }
    fn set_frame_state(&mut self, value: FrameState) -> tje_game::Result<()> {
        self.inner.set_frame_state(value)
    }
    fn stack_banks(&self) -> tje_game::Result<StackBanks> {
        self.inner.stack_banks()
    }
    fn set_stack_banks(&mut self, value: StackBanks) -> tje_game::Result<()> {
        self.inner.set_stack_banks(value)
    }
    fn dispatcher(&self) -> Dispatcher {
        self.inner.dispatcher()
    }
    fn execution_class(&self) -> ExecutionClass {
        self.inner.execution_class()
    }
    fn set_execution_class(&mut self, class: ExecutionClass) -> tje_game::Result<()> {
        self.inner.set_execution_class(class)
    }
    fn call_original(&mut self, id: FunctionId, args: &[u32]) -> tje_game::Result<u32> {
        self.inner.call_original(id, args)
    }
    fn call_original_entry(
        &mut self,
        id: FunctionId,
        class: ExecutionClass,
    ) -> tje_game::Result<u32> {
        self.inner.call_original_entry(id, class)
    }
    fn call_original_with_execution(
        &mut self,
        id: FunctionId,
        args: &[u32],
        class: ExecutionClass,
    ) -> tje_game::Result<u32> {
        self.inner.call_original_with_execution(id, args, class)
    }
}
fn nearest_mutant(c: &mut HookContext<'_>, substitute: bool) -> Result<Option<u32>> {
    let args = std::array::from_fn::<_, 4, _>(|index| c.argument_u32(index as u32));
    let sp = c.stack_pointer()?;
    let stored = [c.read_u16(STORED), c.read_u16(STORED + 2)];
    let mut proxy = MutantContext {
        inner: c,
        residual: [sp - 6, sp - 10],
        stored,
        substitute,
    };
    Ok(nearest_function()(&mut proxy, &args)?)
}
fn nearest_offset(c: &mut HookContext<'_>) -> Result<Option<u32>> {
    nearest_mutant(c, false)
}
fn nearest_stored(c: &mut HookContext<'_>) -> Result<Option<u32>> {
    nearest_mutant(c, true)
}

#[test]
#[ignore = "requires both private ROMs; real dependent producer/consumer and seeded mutants"]
fn nearest_target_tracks_real_producer_in_both_revisions() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let (key, slug) = match revision {
            Revision::Rev02 => ("TJE_REV02_ROM", "rev02"),
            Revision::Rev00 => ("TJE_REV00_ROM", "rev00"),
        };
        let path = std::env::var_os(key)
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join(format!("../../roms/{slug}.bin"))
            });
        let bytes = std::fs::read(path).unwrap();
        for (function, mutant) in [
            (nearest as HookFn, false),
            (nearest_offset, true),
            (nearest_stored, true),
        ] {
            let mut registry = HookRegistry::empty(revision);
            registry
                .register_contract(
                    ids::EARTHLING_STEER_TOWARD_NEAREST_TARGET
                        .metadata()
                        .clone(),
                    true,
                    function,
                )
                .unwrap();
            registry
                .enable_dependent_equivalence(
                    &[ids::EARTHLING_STEER_TOWARD_NEAREST_TARGET],
                    DependentProofConfig::default(),
                )
                .unwrap();
            let mut host = Host::new(&bytes, Mode::Reference, registry).unwrap();
            let script = Arc::new(
                InputScript::parse(include_str!("../../examples/start-game.json")).unwrap(),
            );
            host.set_wait_input_poller(script.clone());
            for frame in 0..1300 {
                host.run_frame(script.at(frame)).unwrap();
            }
            let advance = host
                .settle_fixture_boundary(script.at(host.frame_count()))
                .unwrap();
            // Authored AI_GenericEnemyThink frame, not a claim that natural
            // gameplay selected this deliberately ineligible target fixture.
            let caller =
                LocalFrame::enter(&mut host.context(), FrameLayout::link(4, 0x7c, 0x3c)).unwrap();
            let steady_sp = host.cpu().stack_pointer();
            let producer = ram::g_earthling_banks(revision).at(0).at(0);
            producer.object().write(&mut host.context(), 0);
            let mut registers = host.cpu().data_registers();
            registers[2] = 0x1357_2468;
            registers[3] = 0x89ab_cdef;
            host.context().cpu().set_data_registers(registers);
            {
                let mut context = host.context();
                context.enable_stack_audit();
                context
                    .call_original(
                        ids::SET_EARTHLING_STATE.metadata().address(revision),
                        &[producer.address(), 0],
                    )
                    .unwrap();
                assert_eq!(context.read_u16(steady_sp - 30), 0x2468);
                assert_eq!(context.read_u16(steady_sp - 26), 0xcdef);
                for (address, value) in [
                    (steady_sp - 30, 0x24),
                    (steady_sp - 29, 0x68),
                    (steady_sp - 26, 0xcd),
                    (steady_sp - 25, 0xef),
                ] {
                    assert!(context.writes.iter().any(|write| matches!(write, OrderedWrite::Ram { address: a, value: v } if *a == address && *v == value)));
                }
            }
            host.context().write_u16(STORED, 0xcdef);
            host.context().write_u16(STORED + 2, 0x2468);
            let state = ram::g_player_info(revision).at(6);
            state.x().write(&mut host.context(), 500);
            state.y().write(&mut host.context(), 500);
            state.level().write(&mut host.context(), 3);
            state.sprite_state().write(&mut host.context(), 0);
            for player in 0..2 {
                ram::g_player_info(revision)
                    .at(player)
                    .level()
                    .write(&mut host.context(), 1);
                ram::g_player_decoy_entities(revision)
                    .at(player)
                    .write(&mut host.context(), 0);
            }
            let result = host.check_fixture(
                ids::EARTHLING_STEER_TOWARD_NEAREST_TARGET
                    .metadata()
                    .address(revision),
                &[state.address(), 255, 0, 1],
            );
            if mutant {
                assert!(result.is_err(), "mutant passed in {revision:?}");
                assert_eq!(
                    host.checks.dependent_equivalence["EarthlingSteerTowardNearestTarget"]
                        .calls_proven,
                    0
                );
            } else {
                result.unwrap();
                let evidence =
                    &host.checks.dependent_equivalence["EarthlingSteerTowardNearestTarget"];
                assert_eq!(evidence.calls_proven, 1);
                assert_eq!(evidence.seeded_probes_completed, 16);
                assert_eq!(evidence.byte_probes_completed, 16);
                assert_eq!(
                    evidence
                        .last_read_trace
                        .iter()
                        .map(|read| read.raw_address)
                        .collect::<Vec<_>>(),
                    [steady_sp - 26, steady_sp - 30]
                );
                assert!(evidence.sensitive_probe_pairs > 0);
                assert_eq!(host.checks.mismatches, 0);
                assert!(host.checks.uninitialized_stack_reads.is_empty());
            }
            println!(
                "DEPENDENT_NEAREST {}",
                serde_json::json!({"revision":slug,"mutant":mutant,"settling_ticks":advance.ticks,"evidence":host.checks.dependent_equivalence})
            );
            caller.leave(&mut host.context()).unwrap();
        }
    }
}
