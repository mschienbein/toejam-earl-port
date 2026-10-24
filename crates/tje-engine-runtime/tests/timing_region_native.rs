#![cfg(feature = "devices")]
//! Authored CPU streams and Rust ISR effects on the actual DeviceRuntime.
//! IRQ4/IRQ6 come from a real programmed VDP, not injected CPU pending bits.
use bincode::{Decode, Encode};
use genesis_config::{GenesisEmulatorConfig, GenesisInputs, GenesisRegion};
use genesis_core::{GenesisEmulator, native::CpuAccess};
use jgenesis_common::frontend::{
    AudioOutput, Color, EmulatorTrait, FrameSize, InputPoller, RenderFrameOptions, Renderer,
    SaveWriter, TickEffect,
};
use m68000_emu::BusInterface;
use std::{convert::Infallible, io, sync::Arc};
use tje_engine_core::{
    CpuState, ReadKind,
    ir::{InstructionIr, Op, StateField, Value, Width},
    timing_region::{
        AccessKind, BusObservation, RustPathChoice, TimingCursor, TimingError, TimingEvent,
        TimingInstruction, TimingScratch,
    },
};
use tje_engine_runtime::{
    BusAccess, PlatformDevices,
    timing_region::{
        BoundaryAction, InterruptEntry, IsrAction, IsrSourceContext, NativeIsrContract,
        NativeIsrProgram, RegionEdge, ReviewedRegion, ScheduledIsrClosure, TimingProgram,
        TimingRegion,
    },
};
use tje_mem::AddressSpace;
use tje_native_platform::DeviceRuntime;
use tje_rom::Revision;

const MAIN: u32 = 0x200;
const ISR: u32 = 0x300;
const RTE_PC: u32 = 0x304;
const COUNTER: u32 = 0xff800e;
const STACK: u32 = 0xffff00;
const USER_STACK: u32 = 0xfff000;
const NOP: u16 = 0x4e71;
const BRA_MAIN: u16 = 0x60fc;
const ADDQ_WORD_COUNTER: u16 = 0x5278;
const COUNTER_SHORT: u16 = 0x800e;
const RTE: u16 = 0x4e73;

struct Sink;
impl Renderer for Sink {
    type Err = Infallible;
    fn render_frame(
        &mut self,
        _: &[Color],
        _: FrameSize,
        _: f64,
        _: RenderFrameOptions,
    ) -> Result<(), Self::Err> {
        Ok(())
    }
}
impl SaveWriter for Sink {
    type Err = io::Error;
    fn load_bytes(&mut self, _: &str) -> io::Result<Vec<u8>> {
        Err(io::ErrorKind::NotFound.into())
    }
    fn persist_bytes(&mut self, _: &str, _: &[u8]) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn load_serialized<D: Decode<()>>(&mut self, _: &str) -> io::Result<D> {
        Err(io::ErrorKind::NotFound.into())
    }
    fn persist_serialized<E: Encode>(&mut self, _: &str, _: E) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
}
#[derive(Default)]
struct Audio(Vec<[u64; 2]>);
impl AudioOutput for Audio {
    type Err = Infallible;
    fn push_sample(&mut self, l: f64, r: f64) -> Result<(), Self::Err> {
        self.0.push([l.to_bits(), r.to_bits()]);
        Ok(())
    }
}
#[derive(Default)]
struct Inputs(GenesisInputs);
impl InputPoller<GenesisInputs> for Inputs {
    fn poll(&mut self) -> &GenesisInputs {
        &self.0
    }
}
fn rom() -> Vec<u8> {
    let mut rom = vec![0; 0x800];
    rom[..4].copy_from_slice(&STACK.to_be_bytes());
    rom[4..8].copy_from_slice(&MAIN.to_be_bytes());
    for level in [4_u32, 6] {
        let p = (0x60 + 4 * level) as usize;
        rom[p..p + 4].copy_from_slice(&ISR.to_be_bytes());
    }
    for (pc, word) in [
        (MAIN, NOP),
        (MAIN + 2, BRA_MAIN),
        (MAIN + 4, NOP),
        (ISR, ADDQ_WORD_COUNTER),
        (ISR + 2, COUNTER_SHORT),
        (RTE_PC, RTE),
        (RTE_PC + 2, NOP),
    ] {
        rom[pc as usize..pc as usize + 2].copy_from_slice(&word.to_be_bytes());
    }
    rom
}
fn native_program(rom: &[u8]) -> Arc<NativeIsrProgram> {
    Arc::new(
        NativeIsrProgram::compile(
            rom,
            NativeIsrContract {
                name: "authored nested VDP IRQ closure",
                source: "synthetic counter ADDQ.W and explicit platform RTE",
                entries: vec![
                    InterruptEntry { level: 4, pc: ISR },
                    InterruptEntry { level: 6, pc: ISR },
                ],
                instruction_pcs: vec![ISR, RTE_PC],
                edges: vec![RegionEdge {
                    from: ISR,
                    to: RTE_PC,
                }],
            },
            7,
        )
        .unwrap(),
    )
}
fn main_program(rom: &[u8]) -> TimingProgram {
    TimingProgram::compile(
        rom,
        ReviewedRegion {
            name: "authored main suspension",
            source: "synthetic NOP/BRA source loop",
            entry: MAIN,
            instruction_pcs: vec![MAIN, MAIN + 2],
            edges: vec![
                RegionEdge {
                    from: MAIN,
                    to: MAIN + 2,
                },
                RegionEdge {
                    from: MAIN + 2,
                    to: MAIN,
                },
            ],
            return_pc: MAIN + 4,
            maximum_steps: 1000,
            required_interrupts: &[4, 6],
        },
        7,
    )
    .unwrap()
}
fn prepared(rom: &[u8]) -> GenesisEmulator {
    let mut reference = GenesisEmulator::create(
        Some(rom.to_vec()),
        GenesisEmulatorConfig {
            forced_region: Some(GenesisRegion::Americas),
            ..GenesisEmulatorConfig::default()
        },
        &mut Sink,
    );
    {
        let (cpu, bus) = reference.hybrid_parts();
        cpu.set_address_registers(cpu.address_registers(), USER_STACK, STACK);
        bus.write_word(0xc00004, 0x8014); // actual H interrupt enable
        bus.write_word(0xc00004, 0x8164); // actual V interrupt + display enable
        bus.apply_pending_writes();
    }
    let mut audio = Audio::default();
    let mut captured = false;
    for _ in 0..100_000 {
        let bus = reference.hybrid_bus();
        if bus.vdp.scanline() == 223
            && (3000..=3100).contains(&bus.vdp.scanline_mclk())
            && reference.hybrid_cpu().pc() == MAIN
            && bus.cycles.m68k_wait_cpu_cycles == 0
            && bus.interrupt_level() == 4
        {
            captured = true;
            break;
        }
        reference
            .tick(&mut Sink, &mut audio, &mut Inputs::default(), &mut Sink)
            .unwrap();
        audio.0.clear();
    }
    assert!(captured, "authored VDP IRQ4 boundary not reached");
    reference.hybrid_parts().0.set_status_register(0); // labelled user-mode entry fixture
    let bus = reference.hybrid_bus_mut();
    bus.native_cpu_accesses = Some(Vec::new());
    bus.native_ram_writes = Some(Vec::new());
    bus.enable_native_write_trace();
    reference
}

fn prepared_before_renderer_event(rom: &[u8]) -> GenesisEmulator {
    // Derive this reference event from the actual renderer, not VBlank IRQ
    // timing: the pinned VDP renders the bottom border after VBlank begins.
    let mut probe = prepared(rom);
    probe.hybrid_parts().0.set_status_register(0x2700);
    let mut audio = Audio::default();
    let event_clock = (0..100_000)
        .find_map(|_| {
            let before = probe.hybrid_bus().cycles.m68k_mclk_cycles;
            let effect = probe
                .tick(&mut Sink, &mut audio, &mut Inputs::default(), &mut Sink)
                .unwrap();
            audio.0.clear();
            (effect == TickEffect::FrameRendered).then_some(before)
        })
        .expect("actual renderer event");
    let mut reference = prepared(rom);
    reference.hybrid_parts().0.set_status_register(0x2700);
    let mut captured = false;
    for _ in 0..100_000 {
        let bus = reference.hybrid_bus();
        if bus.cycles.m68k_mclk_cycles >= event_clock - 200
            && reference.hybrid_cpu().pc() == MAIN
            && bus.cycles.m68k_wait_cpu_cycles == 0
        {
            assert_eq!(bus.interrupt_level(), 6);
            assert!(bus.cycles.m68k_mclk_cycles < event_clock);
            captured = true;
            break;
        }
        reference
            .tick(&mut Sink, &mut audio, &mut Inputs::default(), &mut Sink)
            .unwrap();
        audio.0.clear();
    }
    assert!(captured, "source entry before actual renderer event");
    reference.hybrid_parts().0.set_status_register(0);
    let bus = reference.hybrid_bus_mut();
    bus.native_cpu_accesses.as_mut().unwrap().clear();
    bus.native_ram_writes.as_mut().unwrap().clear();
    bus.native_writes.as_mut().unwrap().clear();
    reference
}
fn cpu(reference: &GenesisEmulator) -> CpuState {
    let original = reference.hybrid_cpu();
    let words = original.hybrid_state_words();
    CpuState {
        d: original.data_registers(),
        a: original.address_registers(),
        usp: words[15],
        ssp: words[16],
        pc: words[17],
        sr: words[18],
        prefetch: words[19],
        pending_interrupt: words[20],
        stopped: words[21],
        frozen: words[22],
        address_error: words[23],
        last_muldiv: words[24],
    }
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
fn devices(rom: &[u8], reference: &GenesisEmulator) -> PlatformDevices {
    let mut devices = DeviceRuntime::from_state(
        AddressSpace::new(rom.to_vec(), Revision::Rev02),
        reference.capture_native_state(),
    )
    .unwrap();
    devices.enable_trace();
    devices.enable_ram_write_trace();
    let mut devices = PlatformDevices::from(devices);
    devices.enable_exception_trace();
    devices
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mutation {
    None,
    MissingBody,
    NoProgress,
    FalseRte,
    WrongUsp,
    WrongSsp,
    ForeignPlan,
    ZeroCycles,
    BypassBus,
}
struct RustCounter {
    program: Arc<NativeIsrProgram>,
    mutation: Mutation,
    calls: usize,
}
impl ScheduledIsrClosure for RustCounter {
    fn timing_program(&self) -> &Arc<NativeIsrProgram> {
        &self.program
    }
    fn supports_pc(&self, pc: u32) -> bool {
        [ISR, RTE_PC].contains(&pc) && !(self.mutation == Mutation::MissingBody && pc == ISR)
    }
    fn step(&mut self, source: &mut IsrSourceContext<'_>) -> Result<IsrAction, TimingError> {
        self.calls += 1;
        if self.mutation == Mutation::NoProgress {
            return Ok(IsrAction::NoProgress);
        }
        if self.mutation == Mutation::FalseRte {
            return Ok(IsrAction::ExceptionReturn);
        }
        let entry = source.state().clone();
        if entry.pc == RTE_PC {
            return Ok(IsrAction::ExceptionReturn);
        }
        let (mut cursor, mut effects) = source.admit_source(&entry)?;
        if matches!(self.mutation, Mutation::ForeignPlan | Mutation::ZeroCycles) {
            // Deliberately forged raw Core IR: correct opcode/next PC, but not
            // the controller-admitted user-ROM plan or cursor generation.
            let fake = InstructionIr {
                pc: ISR,
                next_pc: RTE_PC,
                opcode: ADDQ_WORD_COUNTER,
                value_count: 3,
                ops: vec![
                    Op::Constant {
                        dst: Value(0),
                        value: if self.mutation == Mutation::ZeroCycles {
                            0
                        } else {
                            16
                        },
                    },
                    Op::Constant {
                        dst: Value(1),
                        value: RTE_PC,
                    },
                    Op::Constant {
                        dst: Value(2),
                        value: u32::from(RTE),
                    },
                    Op::StoreState {
                        field: StateField::Pc,
                        value: Value(1),
                    },
                    Op::StoreState {
                        field: StateField::Prefetch,
                        value: Value(2),
                    },
                    Op::Finish { cycles: Value(0) },
                ],
            };
            let fake = TimingInstruction::compile(&fake)?;
            let mut scratch = TimingScratch::default();
            let mut fake = TimingCursor::begin(&fake, &entry, &mut scratch)?;
            let TimingEvent::InstructionComplete { token, .. } = fake.next_event()? else {
                panic!("forged finish");
            };
            return Ok(IsrAction::SourceInstruction(fake.complete_instruction(
                token,
                RustPathChoice { next_pc: RTE_PC },
            )?));
        }
        let mut old = None;
        loop {
            match cursor.next_event()? {
                TimingEvent::SetReadOffset { token, .. } => {
                    if self.mutation == Mutation::BypassBus {
                        cursor.acknowledge_offset(token)?;
                    } else {
                        effects.read_offset(&mut cursor, token)?;
                    }
                }
                TimingEvent::Bus { token, expected } => {
                    if self.mutation == Mutation::BypassBus {
                        let value = if expected.kind == AccessKind::Read(ReadKind::Prefetch) {
                            match expected.address {
                                p if p == ISR + 2 => u32::from(COUNTER_SHORT),
                                RTE_PC => u32::from(RTE),
                                _ => 0,
                            }
                        } else {
                            0
                        };
                        cursor.acknowledge_bus(
                            token,
                            BusObservation {
                                address: expected.address,
                                width: expected.width,
                                kind: expected.kind,
                                value,
                            },
                        )?;
                    } else {
                        match expected.kind {
                            AccessKind::Read(ReadKind::Prefetch) => {
                                effects.prefetch(&mut cursor, token)?
                            }
                            AccessKind::Read(ReadKind::Semantic) => {
                                old = Some(effects.read(
                                    &mut cursor,
                                    token,
                                    COUNTER,
                                    Width::Word,
                                    ReadKind::Semantic,
                                )? as u16)
                            }
                            AccessKind::Write => effects.write(
                                &mut cursor,
                                token,
                                COUNTER,
                                Width::Word,
                                u32::from(old.unwrap().wrapping_add(1)),
                            )?,
                            _ => panic!("unexpected authored counter effect"),
                        }
                    }
                }
                TimingEvent::InstructionComplete { token, .. } => {
                    if self.mutation != Mutation::BypassBus {
                        let old = old.unwrap();
                        let result = old.wrapping_add(1);
                        let carry = old == u16::MAX;
                        let overflow = old == i16::MAX as u16;
                        let flags = u32::from(carry)
                            | (u32::from(overflow) << 1)
                            | (u32::from(result == 0) << 2)
                            | (u32::from(result & 0x8000 != 0) << 3)
                            | (u32::from(carry) << 4);
                        effects.set_status((entry.sr & !31) | flags);
                        if self.mutation == Mutation::WrongUsp {
                            effects.set_user_stack(USER_STACK ^ 0x100);
                        }
                        if self.mutation == Mutation::WrongSsp {
                            effects.set_supervisor_stack(STACK.wrapping_sub(6) ^ 0x100);
                        }
                    }
                    return Ok(IsrAction::SourceInstruction(cursor.complete_instruction(
                        token,
                        RustPathChoice { next_pc: RTE_PC },
                    )?));
                }
                other => panic!("unexpected authored counter exit {other:?}"),
            }
        }
    }
}
fn normalize(access: CpuAccess) -> BusAccess {
    match access {
        CpuAccess::Read {
            address,
            word,
            value,
            purpose,
        } => BusAccess::Read {
            address,
            word,
            value,
            kind: match purpose {
                0 => ReadKind::Semantic,
                1 => ReadKind::Prefetch,
                2 => ReadKind::Discarded,
                _ => panic!("read purpose"),
            },
        },
        CpuAccess::Write {
            address,
            word,
            value,
        } => BusAccess::Write {
            address,
            word,
            value,
        },
        CpuAccess::AcknowledgeInterrupt { level } => BusAccess::AcknowledgeInterrupt { level },
    }
}
fn assert_native_service(
    rom: &[u8],
    mut original: GenesisEmulator,
    level: u8,
    expected_frame_yields: u64,
    expected_instructions: u64,
    expected_counter: u8,
) {
    let mut state = cpu(&original);
    let initial = state.clone();
    let mut devices = devices(rom, &original);
    let main = main_program(rom);
    let mut handler = RustCounter {
        program: native_program(rom),
        mutation: Mutation::None,
        calls: 0,
    };
    let mut region = TimingRegion::begin_native(&main, &state, Some(&mut handler)).unwrap();
    let token = region
        .suspend_interrupt(&state, BoundaryAction::InterruptAdmission { level })
        .unwrap();
    region.start_native_interrupt(token, &state).unwrap();
    let mut audio = Audio::default();
    let mut frame_yields = 0;
    let receipt = loop {
        let before_clock = devices.clock();
        let before_boundaries = devices.exception_trace().unwrap().len();
        let step = region
            .step_native_interrupt(&mut state, &mut devices, GenesisInputs::default())
            .unwrap();
        let expected = original
            .tick(&mut Sink, &mut audio, &mut Inputs::default(), &mut Sink)
            .unwrap();
        assert_eq!(
            step.devices.frame_complete,
            expected == TickEffect::FrameRendered
        );
        assert_eq!(words(&state), original.hybrid_cpu().hybrid_state_words());
        assert_eq!(
            devices.clock(),
            original.hybrid_bus().cycles.m68k_mclk_cycles
        );
        let actual = devices.instruction_trace().unwrap();
        let expected = original.hybrid_bus().native_cpu_accesses.as_ref().unwrap();
        assert_eq!(actual.len(), expected.len());
        assert!(
            expected
                .iter()
                .copied()
                .map(normalize)
                .eq(actual.iter().copied())
        );
        for boundary in &devices.exception_trace().unwrap()[before_boundaries..] {
            assert_eq!(boundary.master_clock, before_clock);
            assert!(boundary.master_clock < devices.clock());
            assert!(boundary.bus_access_index <= actual.len());
        }
        assert_eq!(
            devices.ram_write_trace().unwrap(),
            original.hybrid_bus().native_ram_writes.as_ref().unwrap()
        );
        if step.devices.frame_complete {
            frame_yields += 1;
            assert!(
                step.returned.is_none(),
                "must yield frame before finishing nested ISR service"
            );
        }
        if let Some(receipt) = step.returned {
            break receipt;
        }
    };
    assert_eq!(
        (state.usp, state.ssp, state.sr, state.pc, state.prefetch),
        (
            initial.usp,
            initial.ssp,
            initial.sr,
            initial.pc,
            initial.prefetch
        )
    );
    assert_eq!(frame_yields, expected_frame_yields);
    assert_eq!(receipt.scheduled_instructions(), expected_instructions);
    assert!(receipt.steps() >= expected_instructions + 2);
    assert!(receipt.master_clocks() > 0);
    assert!(matches!(
        devices
            .exception_trace()
            .unwrap()
            .last()
            .map(|entry| entry.boundary),
        Some(tje_engine_core::ExceptionBoundary::Returned)
    ));
    assert_eq!(region.usage().original_isr_instructions, 0);
    assert!(devices.vdp().hybrid_state_eq(&original.hybrid_bus().vdp));
    assert_eq!(
        devices.memory().ram()[0x800e..0x8010],
        [0, expected_counter]
    );
    assert_eq!(
        devices
            .audio_samples()
            .iter()
            .map(|s| s.map(f64::to_bits))
            .collect::<Vec<_>>(),
        audio.0
    );
}

#[test]
fn native_rte_receipt_requires_real_nested_entry_bus_clock_and_both_stack_banks() {
    let rom = rom();
    assert_native_service(&rom, prepared(&rom), 4, 0, 4, 2);
}

#[test]
fn native_service_yields_the_actual_frame_event_before_rte_without_retaining_devices() {
    let rom = rom();
    assert_native_service(&rom, prepared_before_renderer_event(&rom), 6, 1, 2, 1);
}
#[test]
fn noop_false_rte_foreign_plan_zero_cycles_fake_bus_and_wrong_banks_never_mint_receipt() {
    let rom = rom();
    for mutation in [
        Mutation::NoProgress,
        Mutation::FalseRte,
        Mutation::WrongUsp,
        Mutation::WrongSsp,
        Mutation::ForeignPlan,
        Mutation::ZeroCycles,
        Mutation::BypassBus,
    ] {
        let original = prepared(&rom);
        let mut state = cpu(&original);
        let initial_usp = state.usp;
        let initial_ssp = state.ssp;
        let mut devices = devices(&rom, &original);
        let main = main_program(&rom);
        let mut handler = RustCounter {
            program: native_program(&rom),
            mutation,
            calls: 0,
        };
        let mut region = TimingRegion::begin_native(&main, &state, Some(&mut handler)).unwrap();
        let token = region
            .suspend_interrupt(&state, BoundaryAction::InterruptAdmission { level: 4 })
            .unwrap();
        region.start_native_interrupt(token, &state).unwrap();
        let error = loop {
            let before = state.clone();
            let clock_before = devices.clock();
            let marker_count = devices.exception_trace().unwrap().len();
            let depth = devices
                .exception_trace()
                .unwrap()
                .iter()
                .fold(0_u32, |depth, marker| match marker.boundary {
                    tje_engine_core::ExceptionBoundary::Entered { .. } => depth + 1,
                    tje_engine_core::ExceptionBoundary::Returned => depth - 1,
                });
            let rejected_before_rte = (mutation == Mutation::FalseRte && before.pc != RTE_PC)
                || (before.pc == RTE_PC
                    && (before.usp != initial_usp
                        || before.ssp != initial_ssp.wrapping_sub(6 * depth)));
            let instructions_before = region.usage().scheduled_isr_instructions;
            match region.step_native_interrupt(&mut state, &mut devices, GenesisInputs::default()) {
                Ok(step) => assert!(step.returned.is_none()),
                Err(error) => {
                    if rejected_before_rte {
                        assert!(
                            devices.exception_trace().unwrap()[marker_count..]
                                .iter()
                                .all(|entry| {
                                    entry.boundary != tje_engine_core::ExceptionBoundary::Returned
                                })
                        );
                        assert_eq!(devices.clock(), clock_before);
                        assert_eq!(
                            (state.pc, state.prefetch, state.usp, state.ssp),
                            (before.pc, before.prefetch, before.usp, before.ssp),
                            "pre-RTE validation cannot perform or repair a return",
                        );
                        assert_eq!(
                            region.usage().scheduled_isr_instructions,
                            instructions_before
                        );
                    }
                    break error;
                }
            }
        };
        assert!(matches!(
            error,
            TimingError::WrongEntry | TimingError::WrongToken | TimingError::InvalidIr(_)
        ));
        assert!(
            region.usage().scheduler_steps > 0,
            "actual accepted IRQ entry is not skipped"
        );
        if mutation == Mutation::WrongUsp {
            assert_ne!(state.usp, initial_usp, "never repair wrong bank to fit RTE");
        }
        if mutation == Mutation::WrongSsp {
            assert_eq!(
                state.ssp,
                STACK.wrapping_sub(6) ^ 0x100,
                "never repair the corrupted supervisor bank before RTE"
            );
        }
        let clock = devices.clock();
        assert!(
            region
                .step_native_interrupt(&mut state, &mut devices, GenesisInputs::default())
                .is_err()
        );
        assert_eq!(devices.clock(), clock);
    }
}
#[test]
fn missing_native_body_and_callee_closure_fail_before_device_progress() {
    let rom = rom();
    let original = prepared(&rom);
    let state = cpu(&original);
    let devices = devices(&rom, &original);
    let clock = devices.clock();
    let main = main_program(&rom);
    let mut handler = RustCounter {
        program: native_program(&rom),
        mutation: Mutation::MissingBody,
        calls: 0,
    };
    assert!(matches!(
        TimingRegion::begin_native(&main, &state, Some(&mut handler)),
        Err(TimingError::MissingIsrCapability)
    ));
    assert_eq!(handler.calls, 0);
    assert_eq!(devices.clock(), clock);
    let incomplete = NativeIsrContract {
        name: "missing authored callee continuation",
        source: "negative closure fixture",
        entries: vec![InterruptEntry { level: 6, pc: ISR }],
        instruction_pcs: vec![ISR],
        edges: Vec::new(),
    };
    assert!(NativeIsrProgram::compile(&rom, incomplete, 7).is_err());
}

#[test]
fn actual_rte_marker_survives_proof_boundary_failure_but_not_an_architectural_fault() {
    use tje_mem::Bus as MemoryBus;
    for (target, successful_rte) in [(MAIN + 4, true), (MAIN + 1, false)] {
        let rom = rom();
        let original = prepared(&rom);
        let mut state = cpu(&original);
        let mut devices = devices(&rom, &original);
        let main = main_program(&rom);
        let mut handler = RustCounter {
            program: native_program(&rom),
            mutation: Mutation::None,
            calls: 0,
        };
        let mut region = TimingRegion::begin_native(&main, &state, Some(&mut handler)).unwrap();
        let token = region
            .suspend_interrupt(&state, BoundaryAction::InterruptAdmission { level: 4 })
            .unwrap();
        region.start_native_interrupt(token, &state).unwrap();
        for _ in 0..100_000 {
            if state.pc == RTE_PC && state.sr & 0x2000 != 0 {
                break;
            }
            let step = region
                .step_native_interrupt(&mut state, &mut devices, GenesisInputs::default())
                .unwrap();
            assert!(step.returned.is_none());
        }
        assert_eq!(state.pc, RTE_PC);
        let old_sp = state.ssp;
        // Explicit fixture mutation AFTER a real admitted IRQ. This is not a
        // source-game write, and it never patches the CPU's PC/SP to fit proof.
        MemoryBus::write_u32(devices.memory_mut(), old_sp + 2, target);
        let mut failed_at = None;
        for _ in 0..1000 {
            let old_clock = devices.clock();
            let old_markers = devices.exception_trace().unwrap().len();
            match region.step_native_interrupt(&mut state, &mut devices, GenesisInputs::default()) {
                Ok(step) => assert!(step.returned.is_none()),
                Err(_) => {
                    failed_at = Some((old_clock, old_markers));
                    break;
                }
            }
        }
        let (old_clock, old_markers) = failed_at.expect("the altered actual frame must reject");
        let markers = &devices.exception_trace().unwrap()[old_markers..];
        assert_eq!(
            markers
                .iter()
                .any(|entry| entry.boundary == tje_engine_core::ExceptionBoundary::Returned),
            successful_rte,
        );
        if successful_rte {
            assert_eq!(state.pc, target);
            assert_eq!(state.ssp, old_sp + 6);
            assert_eq!(state.prefetch, u32::from(NOP));
            assert_eq!(
                devices.clock(),
                old_clock,
                "the later ownership error aborts before device advance"
            );
        } else {
            assert_ne!(state.address_error, 0);
            assert!(markers.iter().any(|entry| matches!(
                entry.boundary,
                tje_engine_core::ExceptionBoundary::Entered { vector: 3, .. }
            )));
        }
    }
}
