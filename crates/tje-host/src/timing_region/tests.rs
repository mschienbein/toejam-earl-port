use super::*;
use genesis_config::{GenesisEmulatorConfig, GenesisInputs};
use genesis_core::timing_survey::TimingSurvey;
use tje_engine_runtime::timing_region::{RegionEdge, ReviewedRegion, TimingProgram};

struct OneInstructionProof {
    original: GenesisEmulator,
}
impl TimingLedger for OneInstructionProof {
    fn before_step(&mut self, owner: ProgressOwner, _: &mut WaitClock) -> Result<()> {
        assert_eq!(owner, ProgressOwner::RustInstruction);
        Ok(())
    }
    fn record_step(
        &mut self,
        _: ProgressOwner,
        step: DeviceStep,
        candidate: &GenesisEmulator,
        clock: &mut WaitClock,
    ) -> Result<()> {
        let mut audio = HeadlessAudio::default();
        self.original
            .tick(
                &mut HeadlessOutput,
                &mut audio,
                &mut Inputs(clock.inputs),
                &mut NoCartridgeSaves,
            )
            .unwrap();
        assert_eq!(step.cpu_cycles, 12);
        assert_eq!(
            self.original.hybrid_cpu().hybrid_state_words(),
            candidate.hybrid_cpu().hybrid_state_words()
        );
        assert_eq!(
            self.original.hybrid_bus().native_cpu_accesses,
            candidate.hybrid_bus().native_cpu_accesses
        );
        let expected = bincode::encode_to_vec(&self.original, bincode::config::standard()).unwrap();
        let actual = bincode::encode_to_vec(candidate, bincode::config::standard()).unwrap();
        assert!(
            expected == actual,
            "reference-phased source write changed device/CPU state"
        );
        Ok(())
    }
}
struct TwoSourceWords {
    entry: CpuState,
    words: usize,
}
impl RustInstruction for TwoSourceWords {
    fn entry_state(&self) -> &CpuState {
        &self.entry
    }
    fn effect(
        &mut self,
        emulator: &mut GenesisEmulator,
        expected: ExpectedAccess,
    ) -> Result<BusObservation> {
        assert_eq!(emulator.hybrid_bus().cycles.m68k_mclk_cycles, 0);
        assert!(
            emulator
                .hybrid_bus()
                .native_writes
                .as_ref()
                .unwrap()
                .is_empty(),
            "write committed before the base-cycle advance"
        );
        let value = match expected.kind {
            AccessKind::Read(ReadKind::Prefetch) => None,
            AccessKind::Write => {
                assert_eq!(expected.width, Width::Word);
                assert_eq!(expected.address, self.entry.a[0] + 2 * self.words as u32);
                let value = if self.words == 0 {
                    self.entry.d[0] >> 16
                } else {
                    self.entry.d[0] & 65535
                };
                self.words += 1;
                Some(value)
            }
            _ => panic!("unexpected source effect"),
        };
        rust_bus_effect(emulator, expected, value)
    }
    fn finish(&mut self, emulator: &mut GenesisEmulator) -> Result<RustPathChoice> {
        assert_eq!(self.words, 2);
        // Authored Rust MOVE.L's NZVC effect; X and the register file stay live.
        let sr = (self.entry.sr & !15)
            | (u32::from(self.entry.d[0] == 0) << 2)
            | (u32::from(self.entry.d[0] & 0x80000000 != 0) << 3);
        emulator.hybrid_parts().0.set_status_register(sr as u16);
        Ok(RustPathChoice {
            next_pc: self.entry.pc + 2,
        })
    }
}

#[test]
fn two_word_effects_issue_at_base_and_commit_after_real_device_advance() {
    const ENTRY: u32 = 0x200;
    const STACK: u32 = 0xffff00;
    const MOVE_LONG_D0_INDIRECT_A0: u16 = 0x2080;
    const NOP: u16 = 0x4e71;
    let mut rom = vec![0; 0x400];
    rom[..4].copy_from_slice(&STACK.to_be_bytes());
    rom[4..8].copy_from_slice(&ENTRY.to_be_bytes());
    rom[ENTRY as usize..ENTRY as usize + 2]
        .copy_from_slice(&MOVE_LONG_D0_INDIRECT_A0.to_be_bytes());
    rom[ENTRY as usize + 2..ENTRY as usize + 4].copy_from_slice(&NOP.to_be_bytes());
    let make = || {
        let mut emulator = GenesisEmulator::create(
            Some(rom.clone()),
            GenesisEmulatorConfig::default(),
            &mut NoCartridgeSaves,
        );
        let (cpu, bus) = emulator.hybrid_parts();
        let mut data = cpu.data_registers();
        data[0] = 0x12345678;
        cpu.set_data_registers(data);
        let mut address = cpu.address_registers();
        address[0] = 0xc00000;
        cpu.set_address_registers(
            address,
            cpu.user_stack_pointer(),
            cpu.supervisor_stack_pointer(),
        );
        bus.native_cpu_accesses = Some(Vec::new());
        bus.native_writes = Some(Vec::new());
        bus.timing_survey = Some(TimingSurvey::default());
        emulator
    };
    let mut candidate = make();
    let mut ledger = OneInstructionProof { original: make() };
    let entry = state(&candidate);
    let program = TimingProgram::compile(
        &rom,
        ReviewedRegion {
            name: "authored two-word source store",
            source: "ROM-free MOVE.L D0,(A0) fixture",
            entry: ENTRY,
            instruction_pcs: vec![ENTRY],
            edges: vec![RegionEdge {
                from: ENTRY,
                to: ENTRY + 2,
            }],
            return_pc: Some(ENTRY + 2),
            maximum_steps: 1,
            required_interrupts: &[],
        },
        7,
    )
    .unwrap();
    let mut storage = tje_engine_runtime::timing_region::RegionStorage::new(&program);
    let mut region = TimingRegion::begin_oracle(
        &program,
        &entry,
        program.fixed_target().unwrap(),
        &mut storage,
    )
    .unwrap();
    let mut clock = WaitClock {
        inputs: GenesisInputs::default(),
        ..WaitClock::default()
    };
    let mut host = TimingHost::new(&mut candidate, &mut clock, &mut ledger).unwrap();
    host.step_source(&mut region, &mut TwoSourceWords { entry, words: 0 })
        .unwrap();
    assert!(region.is_complete());
    assert_eq!(host.effects().len(), 3);
    assert!(host.effects().iter().all(|e| e.master_clock == 0));
    let writes = host.emulator().hybrid_bus().native_writes.as_ref().unwrap();
    assert_eq!(writes.len(), 2);
    assert!(writes.iter().all(|e| e.clock == 84));
}

struct RtsSource {
    entry: CpuState,
    reads: [u16; 2],
    count: usize,
}
impl RustInstruction for RtsSource {
    fn entry_state(&self) -> &CpuState {
        &self.entry
    }
    fn effect(
        &mut self,
        emulator: &mut GenesisEmulator,
        expected: ExpectedAccess,
    ) -> Result<BusObservation> {
        if expected.kind == AccessKind::Read(ReadKind::Semantic) {
            assert_eq!(
                expected.address,
                self.entry.read_address(7) + 2 * self.count as u32
            );
            let actual = rust_bus_effect(emulator, expected, None)?;
            self.reads[self.count] = actual.value as u16;
            self.count += 1;
            Ok(actual)
        } else {
            rust_bus_effect(emulator, expected, None)
        }
    }
    fn finish(&mut self, emulator: &mut GenesisEmulator) -> Result<RustPathChoice> {
        assert_eq!(self.count, 2);
        crate::abi::set_sp(
            emulator.hybrid_parts().0,
            self.entry.read_address(7).wrapping_add(4),
        );
        Ok(RustPathChoice {
            next_pc: (u32::from(self.reads[0]) << 16) | u32::from(self.reads[1]),
        })
    }
}
struct ReturnProof {
    original: GenesisEmulator,
    steps: u64,
    audio: HeadlessAudio,
    seed_debt: bool,
    waits: u64,
}
impl TimingLedger for ReturnProof {
    fn before_step(&mut self, _: ProgressOwner, _: &mut WaitClock) -> Result<()> {
        Ok(())
    }
    fn record_step(
        &mut self,
        owner: ProgressOwner,
        step: DeviceStep,
        candidate: &GenesisEmulator,
        clock: &mut WaitClock,
    ) -> Result<()> {
        let before = self.original.hybrid_bus().cycles.m68k_mclk_cycles;
        let event = self
            .original
            .tick(
                &mut HeadlessOutput,
                &mut self.audio,
                &mut Inputs(clock.inputs),
                &mut NoCartridgeSaves,
            )
            .unwrap();
        assert_eq!(event == TickEffect::FrameRendered, step.frame_complete);
        assert_eq!(
            self.original.hybrid_bus().cycles.m68k_mclk_cycles - before,
            step.master_clocks
        );
        assert_eq!(
            self.original.hybrid_cpu().hybrid_state_words(),
            candidate.hybrid_cpu().hybrid_state_words()
        );
        self.steps += 1;
        self.waits += u64::from(owner == ProgressOwner::HardwareWait);
        Ok(())
    }
    fn capture_actual_step(
        &mut self,
        owner: ProgressOwner,
        candidate: &mut GenesisEmulator,
        step: Option<DeviceStep>,
    ) -> Result<()> {
        if self.seed_debt && owner == ProgressOwner::RustInstruction && step.is_some() {
            // Authored device-debt fixture, identical on both sides AFTER the
            // real RTS. This is not an instruction charge or game workaround.
            candidate.hybrid_bus_mut().cycles.m68k_wait_cpu_cycles = 7;
            self.original.hybrid_bus_mut().cycles.m68k_wait_cpu_cycles = 7;
            self.seed_debt = false;
        }
        Ok(())
    }
}
#[test]
fn post_rts_actual_vdp_irq_is_serviced_without_a_caller_opcode_or_state_repair() {
    use jgenesis_common::frontend::PartialClone;
    use tje_engine_runtime::timing_region::{RegionStorage, ReturnTarget};
    const ENTRY: u32 = 0x10000;
    const ISR: u32 = 0x11000;
    const CALLER: u32 = 0x12000;
    const STACK: u32 = 0xffff00;
    const NOP: u16 = 0x4e71;
    const RTS: u16 = 0x4e75;
    const RTE: u16 = 0x4e73;
    const ADDQ_WORD_ABSOLUTE: u16 = 0x5278;
    const COUNTER: u16 = 0x800e;
    const ILLEGAL_CALLER: u16 = 0x4afc;
    let mut rom = vec![0; 0x14000];
    for word in rom.as_chunks_mut::<2>().0 {
        word.copy_from_slice(&NOP.to_be_bytes());
    }
    rom[..4].copy_from_slice(&STACK.to_be_bytes());
    rom[4..8].copy_from_slice(&0x100_u32.to_be_bytes());
    rom[0x78..0x7c].copy_from_slice(&ISR.to_be_bytes());
    for (pc, word) in [
        (ENTRY, RTS),
        (ISR, ADDQ_WORD_ABSOLUTE),
        (ISR + 2, COUNTER),
        (ISR + 4, RTE),
        (CALLER, ILLEGAL_CALLER),
    ] {
        rom[pc as usize..pc as usize + 2].copy_from_slice(&word.to_be_bytes());
    }
    let mut candidate = GenesisEmulator::create(
        Some(rom.clone()),
        GenesisEmulatorConfig::default(),
        &mut NoCartridgeSaves,
    );
    {
        let bus = candidate.hybrid_bus_mut();
        bus.write_word(0xc00004, 0x8004);
        bus.write_word(0xc00004, 0x8164);
        bus.apply_pending_writes();
    }
    let mut captured = false;
    let mut setup_audio = HeadlessAudio::default();
    for _ in 0..100_000 {
        let bus = candidate.hybrid_bus();
        if bus.vdp.scanline() == 223
            && (3380..=3410).contains(&bus.vdp.scanline_mclk())
            && bus.cycles.m68k_wait_cpu_cycles == 0
        {
            assert_eq!(bus.interrupt_level(), 0);
            captured = true;
            break;
        }
        candidate
            .tick(
                &mut HeadlessOutput,
                &mut setup_audio,
                &mut Inputs(GenesisInputs::default()),
                &mut NoCartridgeSaves,
            )
            .unwrap();
        setup_audio.samples.clear();
    }
    assert!(captured, "actual first-frame VDP boundary");
    {
        let (cpu, bus) = candidate.hybrid_parts();
        bus.memory.write_main_ram_word(STACK, (CALLER >> 16) as u16);
        bus.memory.write_main_ram_word(STACK + 2, CALLER as u16);
        cpu.set_pc(ENTRY, bus);
        cpu.set_status_register(0x2000);
    }
    let mut original = GenesisEmulator::create(
        Some(rom.clone()),
        GenesisEmulatorConfig::default(),
        &mut NoCartridgeSaves,
    );
    original.load_state(candidate.partial_clone());
    let program = TimingProgram::compile(
        &rom,
        ReviewedRegion {
            name: "actual authored RTS edge",
            source: "synthetic RTS +real VDP IRQ6 +ADDQ/RTE",
            entry: ENTRY,
            instruction_pcs: vec![ENTRY],
            edges: vec![],
            return_pc: None,
            maximum_steps: 1000,
            required_interrupts: &[6],
        },
        7,
    )
    .unwrap();
    let mut storage = RegionStorage::new(&program);
    let initial = state(&candidate);
    let target = ReturnTarget::in_rom(rom.len(), CALLER).unwrap();
    let mut region = TimingRegion::begin_oracle(&program, &initial, target, &mut storage).unwrap();
    let mut source = RtsSource {
        entry: initial,
        reads: [0; 2],
        count: 0,
    };
    let mut ledger = ReturnProof {
        original,
        steps: 0,
        audio: HeadlessAudio::default(),
        seed_debt: true,
        waits: 0,
    };
    let mut clock = WaitClock {
        inputs: GenesisInputs::default(),
        ..WaitClock::default()
    };
    {
        let mut host = TimingHost::new(&mut candidate, &mut clock, &mut ledger).unwrap();
        host.step_source(&mut region, &mut source).unwrap();
        region.complete_return(&host.state()).unwrap();
        assert_eq!(host.emulator().hybrid_bus().interrupt_level(), 6);
        assert_eq!(host.boundary(&host.state()), BoundaryAction::Wait);
        host.admit(&mut region).unwrap();
        assert_eq!(host.boundary(&host.state()), BoundaryAction::Ready);
        assert_eq!(host.state().pc, CALLER);
        assert_eq!(host.state().ssp, STACK + 4);
        assert_eq!(host.state().frozen, 0);
    }
    assert_eq!(
        candidate.hybrid_bus().memory.read_main_ram_word(0xff800e),
        1
    );
    assert_eq!(region.usage().original_isr_instructions, 2);
    assert!(ledger.steps >= 5);
    assert!(
        ledger.waits > 0,
        "return-edge debt must be drained before IRQ admission"
    );
}
