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
            return_pc: ENTRY + 2,
            maximum_steps: 1,
            required_interrupts: &[],
        },
        7,
    )
    .unwrap();
    let mut region = TimingRegion::begin_oracle(&program, &entry).unwrap();
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
