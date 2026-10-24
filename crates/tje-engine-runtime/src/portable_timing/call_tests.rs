//! Authored JSR/BSR/RTS streams prove protocol mechanics, not retail coverage.
use super::*;
use crate::timing_region::{
    BoundaryAction, BoundarySignals, CalleeKind, RegionCall, RegionWork, ReviewedRegion,
    TimingRegion,
};
use m68000_emu::{BusInterface, M68000, ReadPurpose, debug::DummyM68000Debugger};
use tje_engine_core::timing_region::{RustPathChoice, TimingEvent};

const MAIN: u32 = 0x100;
const CHILD: u32 = 0x300;
const RESUME: u32 = MAIN + 6;
const EXIT: u32 = 0x180;
const STACK: u32 = 0x2000;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Access {
    Read {
        address: u32,
        word: bool,
        value: u16,
        kind: ReadKind,
    },
    Write {
        address: u32,
        word: bool,
        value: u16,
    },
}
fn read_kind(purpose: ReadPurpose) -> ReadKind {
    match purpose {
        ReadPurpose::Semantic => ReadKind::Semantic,
        ReadPurpose::Discarded => ReadKind::Discarded,
        ReadPurpose::Prefetch => ReadKind::Prefetch,
    }
}
#[derive(Clone)]
struct Memory {
    bytes: Vec<u8>,
    accesses: Vec<Access>,
}
impl Memory {
    fn authored() -> Self {
        let mut memory = Self {
            bytes: vec![0; 0x4000],
            accesses: Vec::new(),
        };
        for (pc, word) in [
            (MAIN, 0x4eb9),
            (MAIN + 2, 0),
            (MAIN + 4, CHILD as u16),
            (RESUME, 0x4e75),
            (CHILD, 0x4e75),
            (CHILD + 2, 0x4e71),
            (EXIT, 0x4e71),
        ] {
            memory.bytes[pc as usize..pc as usize + 2].copy_from_slice(&word.to_be_bytes());
        }
        memory.bytes[STACK as usize..STACK as usize + 4].copy_from_slice(&EXIT.to_be_bytes());
        memory
    }
    fn read(&mut self, address: u32, word: bool, kind: ReadKind) -> u16 {
        let at = address as usize;
        let value = if word {
            u16::from_be_bytes([self.bytes[at], self.bytes[at + 1]])
        } else {
            u16::from(self.bytes[at])
        };
        self.accesses.push(Access::Read {
            address,
            word,
            value,
            kind,
        });
        value
    }
    fn write(&mut self, address: u32, word: bool, value: u16) {
        if word {
            self.bytes[address as usize..address as usize + 2]
                .copy_from_slice(&value.to_be_bytes());
        } else {
            self.bytes[address as usize] = value as u8;
        }
        self.accesses.push(Access::Write {
            address,
            word,
            value,
        });
    }
}
impl BusInterface for Memory {
    type DebugView<'a> = DummyM68000Debugger;
    fn read_byte(&mut self, address: u32) -> u8 {
        self.read(address, false, ReadKind::Semantic) as u8
    }
    fn read_word(&mut self, address: u32) -> u16 {
        self.read(address, true, ReadKind::Semantic)
    }
    fn read_byte_tagged(&mut self, address: u32, purpose: ReadPurpose) -> u8 {
        self.read(address, false, read_kind(purpose)) as u8
    }
    fn read_word_tagged(&mut self, address: u32, purpose: ReadPurpose) -> u16 {
        self.read(address, true, read_kind(purpose))
    }
    fn write_byte(&mut self, address: u32, value: u8) {
        self.write(address, false, value.into());
    }
    fn write_word(&mut self, address: u32, value: u16) {
        self.write(address, true, value);
    }
    fn interrupt_level(&self) -> u8 {
        0
    }
    fn acknowledge_interrupt(&mut self, _: u8) {}
    fn halt(&self) -> bool {
        false
    }
    fn reset(&self) -> bool {
        false
    }
}
fn signals() -> BoundarySignals {
    BoundarySignals {
        wait: false,
        reset: false,
        halted: false,
        interrupt_level: 0,
    }
}
fn caller(memory: &Memory) -> TimingProgram {
    let resume = if memory.bytes[MAIN as usize] == 0x61 {
        MAIN + 4
    } else {
        RESUME
    };
    TimingProgram::compile(
        &memory.bytes,
        ReviewedRegion {
            name: "authored caller",
            source: "synthetic JSR child; RTS",
            entry: MAIN,
            instruction_pcs: vec![MAIN, resume],
            edges: Vec::new(),
            calls: vec![RegionCall {
                from: MAIN,
                target: CHILD,
                resume,
            }],
            return_pc: None,
            maximum_steps: 100,
            required_interrupts: &[],
        },
        7,
    )
    .unwrap()
}
fn child(memory: &Memory) -> BoundProgram {
    static SOURCE: game::Program = game::Program {
        name: "authored child",
        source: "synthetic RTS only, not cartridge semantics",
        entry: game::Location {
            function: tje_game::ids::WAIT_FOR_NEXT_V_BLANK,
            role: 0,
        },
        functions: &[game::FunctionRegion {
            function: tje_game::ids::WAIT_FOR_NEXT_V_BLANK,
            roles: 1,
        }],
        edges: &[],
        calls: &[],
    };
    let timing = TimingProgram::compile(
        &memory.bytes,
        ReviewedRegion {
            name: "authored child",
            source: "synthetic RTS only",
            entry: CHILD,
            instruction_pcs: vec![CHILD],
            edges: Vec::new(),
            calls: Vec::new(),
            return_pc: None,
            maximum_steps: 100,
            required_interrupts: &[],
        },
        7,
    )
    .unwrap();
    BoundProgram {
        source: &SOURCE,
        timing,
        sites: BTreeMap::new(),
        locations: BTreeMap::new(),
        revision: Revision::Rev02,
        rom_digest: Revision::Rev02.sha1(),
        divider: 7,
        irq_policy: &[],
        rom_length: memory.bytes.len(),
        call_targets: BTreeMap::new(),
    }
}
fn initial_cpu() -> CpuState {
    CpuState {
        pc: MAIN,
        prefetch: 0x4eb9,
        sr: 0x2700,
        ssp: STACK,
        ..CpuState::default()
    }
}
fn reference(cpu: &CpuState, memory: &mut Memory) -> M68000 {
    let mut reference = M68000::builder().allow_tas_writes(false).build();
    reference.set_data_registers(cpu.d);
    reference.set_address_registers(cpu.a, cpu.usp, cpu.ssp);
    reference.set_status_register(cpu.sr as u16);
    reference.set_pc(cpu.pc, memory);
    memory.accesses.clear();
    reference
}
fn words(cpu: &CpuState) -> [u32; 25] {
    let mut words = [0; 25];
    words[..8].copy_from_slice(&cpu.d);
    words[8..15].copy_from_slice(&cpu.a);
    words[15..].copy_from_slice(&[
        cpu.usp,
        cpu.ssp,
        cpu.pc,
        cpu.sr,
        cpu.prefetch,
        cpu.pending_interrupt,
        cpu.stopped,
        cpu.frozen,
        cpu.address_error,
        cpu.last_muldiv,
    ]);
    words
}
/// Rust owns the two authored instructions' effects. Core only validates their
/// real memory order, path and independently reference-checked cycle result.
fn source_step(
    region: &mut TimingRegion<'_>,
    cpu: &mut CpuState,
    memory: &mut Memory,
    reference: &mut M68000,
    reference_memory: &mut Memory,
) -> crate::DeviceStep {
    let call = cpu.prefetch == 0x4eb9 || cpu.prefetch == 0x6100;
    let resume = if cpu.prefetch == 0x6100 {
        MAIN + 4
    } else {
        RESUME
    };
    let sp = cpu.read_address(7);
    let mut returned = [0_u16; 2];
    let mut read_index = 0;
    let mut write_index = 0;
    let mut cursor = region.cursor(cpu, BoundaryAction::Ready).unwrap();
    let timing = loop {
        match cursor.next_event().unwrap() {
            TimingEvent::Bus { token, expected } => {
                let word = expected.width == IrWidth::Word;
                let value = match expected.kind {
                    AccessKind::Read(kind) => {
                        let value = memory.read(expected.address, word, kind);
                        if kind == ReadKind::Semantic {
                            returned[read_index] = value;
                            read_index += 1;
                        }
                        u32::from(value)
                    }
                    AccessKind::Write => {
                        assert!(call && write_index < 2);
                        assert_eq!(
                            expected.address,
                            sp.wrapping_sub(4).wrapping_add(2 * write_index)
                        );
                        let value = if write_index == 0 {
                            (resume >> 16) as u16
                        } else {
                            resume as u16
                        };
                        memory.write(expected.address, word, value);
                        write_index += 1;
                        u32::from(value)
                    }
                };
                cursor
                    .acknowledge_bus(
                        token,
                        BusObservation {
                            address: expected.address,
                            width: expected.width,
                            kind: expected.kind,
                            value,
                        },
                    )
                    .unwrap();
            }
            TimingEvent::SetReadOffset { token, .. } => cursor.acknowledge_offset(token).unwrap(),
            TimingEvent::InstructionComplete { token, .. } => {
                let next_pc = if call {
                    CHILD
                } else {
                    (u32::from(returned[0]) << 16) | u32::from(returned[1])
                };
                break cursor
                    .complete_instruction(token, RustPathChoice { next_pc })
                    .unwrap();
            }
            TimingEvent::Exception { .. } => {
                panic!("authored legal instruction raised an exception")
            }
        }
    };
    let timing = region.accept_instruction(timing).unwrap();
    cpu.write_address(
        7,
        if call {
            sp.wrapping_sub(4)
        } else {
            sp.wrapping_add(4)
        },
    );
    cpu.pc = timing.pipeline().pc;
    cpu.prefetch = timing.pipeline().prefetch;
    cpu.last_muldiv = timing.pipeline().last_muldiv;
    let expected_cycles = reference.execute_instruction(reference_memory);
    assert_eq!(timing.cycles(), expected_cycles);
    assert_eq!(words(cpu), reference.hybrid_state_words());
    assert_eq!(memory.bytes, reference_memory.bytes);
    assert_eq!(memory.accesses, reference_memory.accesses);
    let step = crate::DeviceStep {
        cpu_cycles: expected_cycles,
        master_clocks: u64::from(expected_cycles) * 7,
        frame_complete: false,
    };
    region.record_progress(step).unwrap();
    step
}

#[test]
fn real_source_call_parks_exclusive_storage_and_consumes_one_child_rts() {
    let mut memory = Memory::authored();
    let mut reference_memory = memory.clone();
    let mut cpu = initial_cpu();
    let mut reference = reference(&cpu, &mut reference_memory);
    let caller = caller(&memory);
    let child = child(&memory);
    let mut caller_storage = RegionStorage::new(&caller);
    let mut child_storage = child.prepare_storage();
    let mut region = TimingRegion::begin_oracle(
        &caller,
        &cpu,
        ReturnTarget::in_rom(memory.bytes.len(), EXIT).unwrap(),
        &mut caller_storage,
    )
    .unwrap();
    let mut owners = CompletionOwners::default();
    owners.begin_root_scope().unwrap();
    let parent = owners
        .begin(tje_game::ids::CLEAR_LOWER_VRAM_TILEMAP, STACK, &region)
        .unwrap();
    source_step(
        &mut region,
        &mut cpu,
        &mut memory,
        &mut reference,
        &mut reference_memory,
    );
    let token = region.authorize_call(&cpu).unwrap();
    owners
        .enter_child(
            &parent,
            &token,
            tje_game::ids::WAIT_FOR_NEXT_V_BLANK,
            CallProfile::NativeStrictReadable,
            CalleeKind::TimedRust,
        )
        .unwrap();
    let mut parked = region.park().unwrap();
    assert!(
        TimingRegion::begin_oracle(
            &caller,
            &initial_cpu(),
            ReturnTarget::in_rom(memory.bytes.len(), EXIT).unwrap(),
            &mut caller_storage
        )
        .is_err()
    );
    owners.bind_frame().unwrap();
    let mut child_region = TimingRegion::begin_oracle(
        child.timing(),
        &cpu,
        ReturnTarget::in_rom(memory.bytes.len(), RESUME).unwrap(),
        &mut child_storage,
    )
    .unwrap();
    let invocation = owners
        .begin(
            tje_game::ids::WAIT_FOR_NEXT_V_BLANK,
            STACK - 4,
            &child_region,
        )
        .unwrap();
    let step = source_step(
        &mut child_region,
        &mut cpu,
        &mut memory,
        &mut reference,
        &mut reference_memory,
    );
    child_region.complete_return(&cpu).unwrap();
    parked
        .observe_nested(step, RegionWork::SourceInstruction)
        .unwrap();
    let certificate = child
        .certify_return(
            &child_region,
            &invocation,
            &cpu,
            game::TimedExit {
                entry: tje_game::ids::WAIT_FOR_NEXT_V_BLANK,
                program_counter: RESUME,
                stack_pointer: STACK,
                value: None,
            },
            signals(),
        )
        .unwrap();
    owners.complete(invocation, certificate).unwrap();
    owners
        .consume(
            game::CompletionRequest {
                entry: tje_game::ids::WAIT_FOR_NEXT_V_BLANK,
                entry_sp: STACK - 4,
                kind: game::CompletionKind::Rts,
                expected_pc: Some(RESUME),
                value: None,
            },
            &cpu,
        )
        .unwrap();
    let returned = owners.finish_child(token, &cpu).unwrap();
    assert_eq!(returned.kind(), CalleeKind::TimedRust);
    assert_eq!(returned.profile(), CallProfile::NativeStrictReadable);
    assert!(!returned.separate_callee_proof_required());
    parked.observe_return(returned.kind()).unwrap();
    let mut region = parked.resume(&caller, &mut caller_storage).unwrap();
    returned.resume(&mut region, &cpu, signals()).unwrap();
    assert_eq!(region.usage().timed_callee_calls, 1);
    assert_eq!(region.usage().nested_source_instructions, 1);
    source_step(
        &mut region,
        &mut cpu,
        &mut memory,
        &mut reference,
        &mut reference_memory,
    );
    region.complete_return(&cpu).unwrap();
    assert!(region.is_complete());
}

fn observed_cpu(reference: &M68000) -> CpuState {
    let words = reference.hybrid_state_words();
    let mut cpu = CpuState::default();
    cpu.d.copy_from_slice(&words[..8]);
    cpu.a.copy_from_slice(&words[8..15]);
    cpu.usp = words[15];
    cpu.ssp = words[16];
    cpu.pc = words[17];
    cpu.sr = words[18];
    cpu.prefetch = words[19];
    cpu.pending_interrupt = words[20];
    cpu.stopped = words[21];
    cpu.frozen = words[22];
    cpu.address_error = words[23];
    cpu.last_muldiv = words[24];
    cpu
}

#[test]
fn actual_original_child_rts_is_labelled_and_never_claims_timed_composition() {
    let mut memory = Memory::authored();
    let mut actual_original_memory = memory.clone();
    let mut cpu = initial_cpu();
    let mut actual_original = reference(&cpu, &mut actual_original_memory);
    let caller = caller(&memory);
    let mut storage = RegionStorage::new(&caller);
    let mut region = TimingRegion::begin_oracle(
        &caller,
        &cpu,
        ReturnTarget::in_rom(memory.bytes.len(), EXIT).unwrap(),
        &mut storage,
    )
    .unwrap();
    let mut owners = CompletionOwners::default();
    owners.begin_root_scope().unwrap();
    let parent = owners
        .begin(tje_game::ids::CLEAR_LOWER_VRAM_TILEMAP, STACK, &region)
        .unwrap();
    source_step(
        &mut region,
        &mut cpu,
        &mut memory,
        &mut actual_original,
        &mut actual_original_memory,
    );
    let token = region.authorize_call(&cpu).unwrap();
    owners
        .enter_child(
            &parent,
            &token,
            tje_game::ids::WAIT_FOR_NEXT_V_BLANK,
            CallProfile::HostCheckOriginalSubstitution,
            CalleeKind::ReferenceOriginalSubstitution,
        )
        .unwrap();
    let mut parked = region.park().unwrap();
    let mut controller = owners.begin_original(&token, &cpu).unwrap();
    let instruction = OriginalInstruction {
        pc: actual_original.pc(),
        opcode: actual_original.next_opcode(),
        stack: actual_original.stack_pointer(),
        interrupt: false,
    };
    let cycles = actual_original.execute_instruction(&mut actual_original_memory);
    // This is a read-only observation of the selected ORIGINAL kernel, never an
    // oracle image imported into a Rust candidate to manufacture a return.
    let actual = observed_cpu(&actual_original);
    let step = crate::DeviceStep {
        cpu_cycles: cycles,
        master_clocks: u64::from(cycles) * 7,
        frame_complete: false,
    };
    controller
        .observe(step, Some(instruction), &actual)
        .unwrap();
    parked
        .observe_nested(step, RegionWork::OriginalCalleeInstruction)
        .unwrap();
    owners
        .complete_original(controller, &actual, signals())
        .unwrap();
    owners
        .consume(
            game::CompletionRequest {
                entry: tje_game::ids::WAIT_FOR_NEXT_V_BLANK,
                entry_sp: STACK - 4,
                kind: game::CompletionKind::Rts,
                expected_pc: Some(RESUME),
                value: None,
            },
            &actual,
        )
        .unwrap();
    let before = actual_original.hybrid_state_words();
    let returned = owners.finish_child(token, &actual).unwrap();
    assert_eq!(
        returned.kind().label(),
        "original-callee substitution (callee proven separately)"
    );
    assert!(returned.separate_callee_proof_required());
    parked.observe_return(returned.kind()).unwrap();
    let mut region = parked.resume(&caller, &mut storage).unwrap();
    returned.resume(&mut region, &actual, signals()).unwrap();
    assert_eq!(before, actual_original.hybrid_state_words());
    assert_eq!(region.usage().original_substitution_calls, 1);
    assert_eq!(region.usage().original_callee_instructions, 1);
    assert_eq!(region.usage().separate_callee_proof_obligations, 1);
    assert_eq!(
        region.usage().original_substitution_label,
        Some("original-callee substitution (callee proven separately)")
    );
}

#[test]
fn a_ready_return_pc_without_an_actual_rts_poisoned_the_admitted_chain() {
    let mut memory = Memory::authored();
    let mut reference_memory = memory.clone();
    let mut cpu = initial_cpu();
    let mut reference = reference(&cpu, &mut reference_memory);
    let caller = caller(&memory);
    let mut storage = RegionStorage::new(&caller);
    let mut region = TimingRegion::begin_oracle(
        &caller,
        &cpu,
        ReturnTarget::in_rom(memory.bytes.len(), EXIT).unwrap(),
        &mut storage,
    )
    .unwrap();
    let mut owners = CompletionOwners::default();
    owners.begin_root_scope().unwrap();
    let parent = owners
        .begin(tje_game::ids::CLEAR_LOWER_VRAM_TILEMAP, STACK, &region)
        .unwrap();
    source_step(
        &mut region,
        &mut cpu,
        &mut memory,
        &mut reference,
        &mut reference_memory,
    );
    let token = region.authorize_call(&cpu).unwrap();
    owners
        .enter_child(
            &parent,
            &token,
            tje_game::ids::WAIT_FOR_NEXT_V_BLANK,
            CallProfile::HostCheckOriginalSubstitution,
            CalleeKind::ReferenceOriginalSubstitution,
        )
        .unwrap();
    let _parked = region.park().unwrap();
    let mut controller = owners.begin_original(&token, &cpu).unwrap();
    cpu.pc = RESUME;
    cpu.prefetch = 0x4e75;
    cpu.ssp = STACK;
    let failed = cpu.clone();
    controller
        .observe(
            crate::DeviceStep {
                cpu_cycles: 1,
                master_clocks: 7,
                frame_complete: false,
            },
            None,
            &cpu,
        )
        .unwrap();
    assert!(
        owners
            .complete_original(controller, &cpu, signals())
            .is_err()
    );
    assert_eq!(
        cpu, failed,
        "authority rejection must not repair the actual failed state"
    );
    assert!(owners.finish_child(token, &cpu).is_err());
    assert!(owners.begin_root_scope().is_err());
    assert!(owners.protected_entry().is_some());
}

#[test]
fn recursive_invocations_lease_distinct_reusable_scratch_without_a_depth_cutoff() {
    let memory = Memory::authored();
    let binding = std::sync::Arc::new(child(&memory));
    let mut pool = PreparedSources::new([std::sync::Arc::clone(&binding)]).unwrap();
    let mut leases = Vec::new();
    let mut identities = Vec::new();
    for depth in 0..12 {
        let mut active = pool.acquire(tje_game::ids::WAIT_FOR_NEXT_V_BLANK).unwrap();
        let cpu = CpuState {
            pc: CHILD,
            prefetch: 0x4e75,
            sr: 0x2700,
            ssp: STACK - 4 * (depth + 1),
            ..CpuState::default()
        };
        {
            let region = TimingRegion::begin_oracle(
                active.bound.timing(),
                &cpu,
                ReturnTarget::in_rom(memory.bytes.len(), RESUME).unwrap(),
                &mut active.storage,
            )
            .unwrap();
            assert!(!identities.contains(&region.identity()));
            identities.push(region.identity());
        }
        leases.push(active);
    }
    for active in leases {
        pool.release(active, Default::default()).unwrap();
    }
    for _ in 0..12 {
        let active = pool.acquire(tje_game::ids::WAIT_FOR_NEXT_V_BLANK).unwrap();
        pool.release(active, Default::default()).unwrap();
    }
}

/// This constructs authored cold-binding fixtures to isolate the selection
/// matrix. No retail opcode body or execution coverage is asserted here.
fn matrix_binding(
    source: &'static game::Program,
    callees: &[FunctionId],
) -> std::sync::Arc<BoundProgram> {
    let memory = Memory::authored();
    let mut binding = child(&memory);
    binding.source = source;
    binding.call_targets = callees.iter().map(|&id| (id, 0x4e75)).collect();
    std::sync::Arc::new(binding)
}

#[test]
fn atomic_and_functional_classes_do_not_grant_zero_time_or_native_original_permission() {
    use tje_game::{FramedTranslation, execution::ExecutionClass, ids, platform::timed};
    static FRAMED: &[FramedTranslation] = &[
        FramedTranslation {
            id: ids::CLEAR_LOWER_VRAM_TILEMAP,
            function: tje_game::cutscenes::timed_clear_lower::clear_lower_vram_tilemap_framed,
        },
        FramedTranslation {
            id: ids::GET_CTRL1_INPUT,
            function: timed::get_ctrl1_input,
        },
        FramedTranslation {
            id: ids::COPY_PALETTE_TO_CRAM_SLOT,
            function: timed::copy_palette_to_cram_slot,
        },
    ];
    assert_eq!(
        tje_game::execution_class(ids::GET_CTRL1_INPUT),
        Some(ExecutionClass::Atomic)
    );
    assert_eq!(
        tje_game::execution_class(ids::COPY_PALETTE_TO_CRAM_SLOT),
        Some(ExecutionClass::Functional)
    );
    let root = matrix_binding(
        tje_game::cutscenes::timed_clear_lower::ClearLowerSource::definition(),
        &[ids::GET_CTRL1_INPUT, ids::COPY_PALETTE_TO_CRAM_SLOT],
    );
    let atomic_source = timed::Source::new(Revision::Rev02, timed::Leaf::Controller1);
    let functional_source = timed::Source::new(Revision::Rev02, timed::Leaf::Palette);
    let atomic = matrix_binding(game::TimingSource::program(&atomic_source), &[]);
    let functional = matrix_binding(game::TimingSource::program(&functional_source), &[]);
    let dispatcher = tje_game::Dispatcher::hybrid_with_frames(&[], FRAMED);
    let complete =
        PreparedSources::new([std::sync::Arc::clone(&root), atomic, functional]).unwrap();
    assert!(
        complete
            .validate_selection(dispatcher, CallProfile::NativeStrictReadable)
            .is_ok()
    );
    let missing = PreparedSources::new([root]).unwrap();
    assert!(
        missing
            .validate_selection(dispatcher, CallProfile::NativeStrictReadable)
            .is_err()
    );
    assert!(
        missing
            .validate_selection(dispatcher, CallProfile::NativeCompiledOriginalCallees)
            .is_err()
    );
    assert!(
        missing
            .validate_selection(dispatcher, CallProfile::HostHybridReference)
            .is_err()
    );
    let check = missing
        .validate_selection(dispatcher, CallProfile::HostCheckOriginalSubstitution)
        .unwrap();
    assert!(check.permits_original(
        CallProfile::HostCheckOriginalSubstitution,
        ids::GET_CTRL1_INPUT
    ));
    assert!(!check.permits_original(CallProfile::NativeStrictReadable, ids::GET_CTRL1_INPUT));
    assert!(!check.permits_original(
        CallProfile::NativeCompiledOriginalCallees,
        ids::GET_CTRL1_INPUT
    ));
}

#[test]
fn actual_word_displacement_bsr_owns_its_real_return_word_and_exact_reference_cycles() {
    let mut memory = Memory::authored();
    memory.bytes[MAIN as usize..MAIN as usize + 2].copy_from_slice(&0x6100_u16.to_be_bytes());
    memory.bytes[MAIN as usize + 2..MAIN as usize + 4]
        .copy_from_slice(&((CHILD - (MAIN + 2)) as u16).to_be_bytes());
    memory.bytes[MAIN as usize + 4..MAIN as usize + 6].copy_from_slice(&0x4e75_u16.to_be_bytes());
    let mut reference_memory = memory.clone();
    let mut cpu = initial_cpu();
    cpu.prefetch = 0x6100;
    let mut reference = reference(&cpu, &mut reference_memory);
    let caller = caller(&memory);
    let mut storage = RegionStorage::new(&caller);
    let mut region = TimingRegion::begin_oracle(
        &caller,
        &cpu,
        ReturnTarget::in_rom(memory.bytes.len(), EXIT).unwrap(),
        &mut storage,
    )
    .unwrap();
    source_step(
        &mut region,
        &mut cpu,
        &mut memory,
        &mut reference,
        &mut reference_memory,
    );
    let token = region.authorize_call(&cpu).unwrap();
    assert_eq!(token.target(), CHILD);
    assert_eq!(token.return_pc(), MAIN + 4);
    assert_eq!(
        &memory.bytes[(STACK - 4) as usize..STACK as usize],
        &(MAIN + 4).to_be_bytes()
    );
}

struct IssuedCall {
    program: TimingProgram,
    storage: RegionStorage,
    parked: crate::timing_region::ParkedRegion,
    owners: CompletionOwners,
    token: Option<crate::timing_region::CallToken>,
    cpu: CpuState,
    memory: Memory,
    reference: M68000,
    reference_memory: Memory,
    caller_step: crate::DeviceStep,
}
fn issued_call() -> IssuedCall {
    issued_call_boundary(false)
}
fn issued_call_boundary(fixed: bool) -> IssuedCall {
    let mut memory = Memory::authored();
    let mut reference_memory = memory.clone();
    let mut cpu = initial_cpu();
    let mut reference = reference(&cpu, &mut reference_memory);
    let program = caller(&memory);
    let program = if fixed {
        let mut reviewed = program.reviewed().clone();
        reviewed.instruction_pcs = vec![MAIN];
        reviewed.return_pc = Some(RESUME);
        TimingProgram::compile(&memory.bytes, reviewed, 7).unwrap()
    } else {
        program
    };
    let mut storage = RegionStorage::new(&program);
    let mut region = TimingRegion::begin_oracle(
        &program,
        &cpu,
        if fixed {
            ReturnTarget::from_rom(&memory.bytes, RESUME).unwrap()
        } else {
            ReturnTarget::in_rom(memory.bytes.len(), EXIT).unwrap()
        },
        &mut storage,
    )
    .unwrap();
    let mut owners = CompletionOwners::default();
    owners.begin_root_scope().unwrap();
    let parent = owners
        .begin(tje_game::ids::CLEAR_LOWER_VRAM_TILEMAP, STACK, &region)
        .unwrap();
    let caller_step = source_step(
        &mut region,
        &mut cpu,
        &mut memory,
        &mut reference,
        &mut reference_memory,
    );
    let token = region.authorize_call(&cpu).unwrap();
    owners
        .enter_child(
            &parent,
            &token,
            tje_game::ids::WAIT_FOR_NEXT_V_BLANK,
            CallProfile::NativeStrictReadable,
            CalleeKind::TimedRust,
        )
        .unwrap();
    let parked = region.park().unwrap();
    IssuedCall {
        program,
        storage,
        parked,
        owners,
        token: Some(token),
        cpu,
        memory,
        reference,
        reference_memory,
        caller_step,
    }
}
fn complete_child(issued: &mut IssuedCall) -> crate::DeviceStep {
    let binding = child(&issued.memory);
    let mut storage = binding.prepare_storage();
    issued.owners.bind_frame().unwrap();
    let mut region = TimingRegion::begin_oracle(
        binding.timing(),
        &issued.cpu,
        ReturnTarget::in_rom(issued.memory.bytes.len(), RESUME).unwrap(),
        &mut storage,
    )
    .unwrap();
    let invocation = issued
        .owners
        .begin(tje_game::ids::WAIT_FOR_NEXT_V_BLANK, STACK - 4, &region)
        .unwrap();
    let step = source_step(
        &mut region,
        &mut issued.cpu,
        &mut issued.memory,
        &mut issued.reference,
        &mut issued.reference_memory,
    );
    region.complete_return(&issued.cpu).unwrap();
    issued
        .parked
        .observe_nested(step, RegionWork::SourceInstruction)
        .unwrap();
    let certificate = binding
        .certify_return(
            &region,
            &invocation,
            &issued.cpu,
            game::TimedExit {
                entry: tje_game::ids::WAIT_FOR_NEXT_V_BLANK,
                program_counter: RESUME,
                stack_pointer: STACK,
                value: None,
            },
            signals(),
        )
        .unwrap();
    issued.owners.complete(invocation, certificate).unwrap();
    step
}
fn child_request() -> game::CompletionRequest {
    game::CompletionRequest {
        entry: tje_game::ids::WAIT_FOR_NEXT_V_BLANK,
        entry_sp: STACK - 4,
        kind: game::CompletionKind::Rts,
        expected_pc: Some(RESUME),
        value: None,
    }
}

#[test]
fn nested_callback_transfer_and_preboxed_fatal_poison_the_suspended_parent_without_repair() {
    use tje_game::{
        Error,
        exits::{ControlTransfer, Exit},
    };
    let entry = tje_game::ids::WAIT_FOR_NEXT_V_BLANK;
    let transfer = Error::ControlTransfer(ControlTransfer {
        entry,
        exit: Exit::Terminal,
    });
    assert_eq!(
        CompletionOwners::default().protect_failure(transfer.clone()),
        transfer,
        "ordinary typed exits outside admitted source ownership remain ordinary"
    );
    for cause in [
        transfer,
        Error::timing_execution_failure(entry, Error::Platform("preboxed nested source".into())),
        Error::ClockAfterTransfer {
            transfer: ControlTransfer {
                entry,
                exit: Exit::Terminal,
            },
            error: tje_game::execution::CallClockError::Unavailable,
        },
    ] {
        let mut issued = issued_call();
        let binding = child(&issued.memory);
        let mut storage = binding.prepare_storage();
        issued.owners.bind_frame().unwrap();
        let region = TimingRegion::begin_oracle(
            binding.timing(),
            &issued.cpu,
            ReturnTarget::in_rom(issued.memory.bytes.len(), RESUME).unwrap(),
            &mut storage,
        )
        .unwrap();
        let _child = issued.owners.begin(entry, STACK - 4, &region).unwrap();
        let actual = issued.cpu.clone();
        let memory = issued.memory.bytes.clone();
        let expected = if cause.is_failed_execution() {
            cause.clone()
        } else {
            Error::timing_execution_failure(entry, cause.clone())
        };
        assert_eq!(issued.owners.protect_failure(cause), expected);
        assert!(issued.owners.begin_root_scope().is_err());
        assert!(issued.owners.bind_frame().is_err());
        assert!(issued.owners.consume(child_request(), &issued.cpu).is_err());
        assert!(
            issued
                .owners
                .finish_child(issued.token.take().unwrap(), &issued.cpu)
                .is_err()
        );
        assert_eq!(issued.cpu, actual);
        assert_eq!(issued.memory.bytes, memory);
        assert_eq!(issued.cpu.read_address(7), STACK - 4);
    }
}

#[test]
fn wrong_and_duplicate_nested_receipt_requests_poison_parent_before_any_return() {
    for fault in 0..4 {
        let mut issued = issued_call();
        complete_child(&mut issued);
        let before = issued.cpu.clone();
        let memory = issued.memory.bytes.clone();
        let mut request = child_request();
        match fault {
            0 => request.kind = game::CompletionKind::ExceptionReturn,
            1 => request.expected_pc = Some(RESUME + 2),
            2 => request.entry = tje_game::ids::ABS,
            _ => {
                issued.owners.consume(request, &issued.cpu).unwrap();
            }
        }
        assert!(issued.owners.consume(request, &issued.cpu).is_err());
        assert!(
            issued
                .owners
                .finish_child(issued.token.take().unwrap(), &issued.cpu)
                .is_err()
        );
        assert!(issued.owners.begin_root_scope().is_err());
        assert_eq!(issued.cpu, before);
        assert_eq!(issued.memory.bytes, memory);
    }
}

#[test]
fn foreign_call_return_and_changed_live_prefetch_never_resume_or_repair_parent() {
    for changed_live in [false, true] {
        let mut first = issued_call();
        complete_child(&mut first);
        first.owners.consume(child_request(), &first.cpu).unwrap();
        let returned = first
            .owners
            .finish_child(first.token.take().unwrap(), &first.cpu)
            .unwrap();
        let mut target = if changed_live {
            first
        } else {
            let mut second = issued_call();
            complete_child(&mut second);
            second.owners.consume(child_request(), &second.cpu).unwrap();
            second
        };
        if changed_live {
            target.cpu.prefetch ^= 2;
        }
        let before = target.cpu.clone();
        let mut region = target
            .parked
            .resume(&target.program, &mut target.storage)
            .unwrap();
        assert!(
            returned
                .resume(&mut region, &target.cpu, signals())
                .is_err()
        );
        let error = target.owners.protect_failure(tje_game::Error::Platform(
            "rejected real child handoff".into(),
        ));
        assert!(error.is_failed_execution());
        assert!(target.owners.begin_root_scope().is_err());
        assert_eq!(target.cpu, before);
    }
}

#[test]
fn a_child_scope_cannot_renew_the_absolute_root_tick_ceiling() {
    use tje_game::execution::{CallCeiling, CallClock, ClockUsage};
    let mut clock = CallClock::default();
    let root = clock
        .enter(
            tje_game::ids::CLEAR_LOWER_VRAM_TILEMAP,
            CallCeiling::new(2, 100).unwrap(),
        )
        .unwrap();
    clock.before_tick().unwrap();
    let mut issued = issued_call();
    clock
        .record(ClockUsage {
            ticks: 1,
            instructions: 0,
            master_clocks: issued.caller_step.master_clocks,
            frames: 0,
            dispatch_steps: 0,
        })
        .unwrap();
    let child = clock
        .enter(tje_game::ids::WAIT_FOR_NEXT_V_BLANK, CallCeiling::DEFAULT)
        .unwrap();
    clock.before_tick().unwrap();
    let step = complete_child(&mut issued);
    clock
        .record(ClockUsage {
            ticks: 1,
            instructions: 0,
            master_clocks: step.master_clocks,
            frames: 0,
            dispatch_steps: 0,
        })
        .unwrap();
    clock.leave(child).unwrap();
    assert!(
        clock.before_tick().is_err(),
        "child completion must not reset the parent's spent ticks"
    );
    assert_eq!(clock.usage().ticks, 2);
    let _ = clock.leave(root);
}

#[test]
fn a_fixed_one_call_boundary_uses_actual_return_prefetch_without_a_root_rts_receipt() {
    let mut issued = issued_call_boundary(true);
    assert_eq!(issued.program.reviewed().instruction_pcs, [MAIN]);
    complete_child(&mut issued);
    issued.owners.consume(child_request(), &issued.cpu).unwrap();
    let returned = issued
        .owners
        .finish_child(issued.token.take().unwrap(), &issued.cpu)
        .unwrap();
    let before = issued.cpu.clone();
    let mut region = issued
        .parked
        .resume(&issued.program, &mut issued.storage)
        .unwrap();
    returned
        .resume(&mut region, &issued.cpu, signals())
        .unwrap();
    assert!(region.is_complete());
    assert_eq!(issued.cpu, before);
    assert_eq!(issued.cpu.pc, RESUME);
    // A completed finite region is not an RTS for the whole caller and cannot
    // manufacture a reusable ordinary source-root completion/authority.
    assert!(issued.owners.begin_root_scope().is_err());
}

#[test]
fn a_consumed_child_remains_protected_until_fallible_call_finalization_finishes() {
    let mut issued = issued_call();
    complete_child(&mut issued);
    issued.owners.consume(child_request(), &issued.cpu).unwrap();
    assert_eq!(
        issued.owners.protected_entry(),
        Some(tje_game::ids::WAIT_FOR_NEXT_V_BLANK)
    );
    let before = issued.cpu.clone();
    let first = issued.owners.protect_failure(tje_game::Error::Clock(
        tje_game::execution::CallClockError::InvalidScope,
    ));
    assert_eq!(
        first,
        tje_game::Error::timing_execution_failure(
            tje_game::ids::WAIT_FOR_NEXT_V_BLANK,
            tje_game::Error::Clock(tje_game::execution::CallClockError::InvalidScope)
        )
    );
    assert!(
        issued
            .owners
            .finish_child(issued.token.take().unwrap(), &issued.cpu)
            .is_err()
    );
    assert!(issued.owners.begin_root_scope().is_err());
    assert_eq!(issued.cpu, before);
}
