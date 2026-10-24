use super::*;
use crate::{
    execution::WaitInputPoller,
    input::{Button, InputHold, InputScript},
};
use genesis_config::GenesisEmulatorConfig;
use jgenesis_common::frontend::PartialClone;
use std::{fs, sync::Arc};

struct ProofLedger {
    original: GenesisEmulator,
    clock: WaitClock,
    audio: HeadlessAudio,
    steps: u64,
    source_instructions: u64,
    accesses: usize,
    ram_writes: usize,
    issue_clocks: Vec<(u32, u64)>,
}
impl TimingLedger for ProofLedger {
    fn before_step(&mut self, _owner: ProgressOwner, _clock: &mut WaitClock) -> Result<()> {
        Ok(())
    }
    fn record_step(
        &mut self,
        owner: ProgressOwner,
        step: DeviceStep,
        candidate: &GenesisEmulator,
        _clock: &mut WaitClock,
    ) -> Result<()> {
        let before = self.original.hybrid_bus().cycles.m68k_mclk_cycles;
        if owner == ProgressOwner::RustInstruction {
            self.issue_clocks
                .push((self.original.hybrid_cpu().pc(), before));
        }
        let effect = self
            .original
            .tick(
                &mut HeadlessOutput,
                &mut self.audio,
                &mut Inputs(self.clock.inputs),
                &mut NoCartridgeSaves,
            )
            .map_err(|e| HostError::Emulator(e.to_string()))?;
        let original_bus = self.original.hybrid_bus();
        let candidate_bus = candidate.hybrid_bus();
        let clocks = original_bus.cycles.m68k_mclk_cycles - before;
        if clocks != step.master_clocks
            || original_bus.cycles.m68k_mclk_cycles != candidate_bus.cycles.m68k_mclk_cycles
        {
            return Err(HostError::Metadata(format!(
                "seeded wrong-cycle timing rejected at step {}: original {clocks}, Rust {}",
                self.steps, step.master_clocks
            )));
        }
        if self.original.hybrid_cpu().hybrid_state_words()
            != candidate.hybrid_cpu().hybrid_state_words()
        {
            return Err(HostError::Metadata(format!(
                "timed source CPU state mismatch step {}",
                self.steps
            )));
        }
        let original_accesses = original_bus.native_cpu_accesses.as_ref().unwrap();
        let actual_accesses = candidate_bus.native_cpu_accesses.as_ref().unwrap();
        if original_accesses[self.accesses..] != actual_accesses[self.accesses..] {
            return Err(HostError::Metadata(format!(
                "timed source bus access mismatch step {}",
                self.steps
            )));
        }
        self.accesses = original_accesses.len();
        let original_ram = original_bus.native_ram_writes.as_ref().unwrap();
        let actual_ram = candidate_bus.native_ram_writes.as_ref().unwrap();
        if original_ram[self.ram_writes..] != actual_ram[self.ram_writes..] {
            return Err(HostError::Metadata(format!(
                "timed source RAM commits mismatch step {}",
                self.steps
            )));
        }
        self.ram_writes = original_ram.len();
        assert_eq!(effect == TickEffect::FrameRendered, step.frame_complete);
        self.steps += 1;
        self.source_instructions += u64::from(owner == ProgressOwner::RustInstruction);
        if step.frame_complete {
            self.clock.advance_frame(&mut self.original)?;
        }
        Ok(())
    }
}
fn enable_observers(emulator: &mut GenesisEmulator) {
    let bus = emulator.hybrid_bus_mut();
    bus.native_cpu_accesses = Some(Vec::new());
    bus.native_ram_writes = Some(Vec::new());
    bus.enable_native_write_trace();
}
fn fork(rom: &[u8], entry: &GenesisEmulator) -> GenesisEmulator {
    let mut fork = GenesisEmulator::create(
        Some(rom.to_vec()),
        GenesisEmulatorConfig::default(),
        &mut NoCartridgeSaves,
    );
    fork.load_state(entry.partial_clone());
    enable_observers(&mut fork);
    fork
}
fn capture(rom: &[u8], revision: Revision) -> (GenesisEmulator, WaitClock) {
    capture_with(rom, revision, Arc::new(InputScript::default()), 480)
}
fn capture_with(
    rom: &[u8],
    revision: Revision,
    inputs: Arc<dyn WaitInputPoller>,
    first_frame: u64,
) -> (GenesisEmulator, WaitClock) {
    let mut emulator = GenesisEmulator::create(
        Some(rom.to_vec()),
        GenesisEmulatorConfig::default(),
        &mut NoCartridgeSaves,
    );
    let mut clock = WaitClock {
        frame: 0,
        inputs: inputs.at(0).unwrap(),
        poller: Some(inputs),
        ..WaitClock::default()
    };
    let entry = tje_game::ids::WAIT_FOR_NEXT_V_BLANK
        .metadata()
        .address(revision);
    let mut audio = HeadlessAudio::default();
    for _ in 0..25_000_000 {
        let cpu = emulator.hybrid_cpu();
        let bus = emulator.hybrid_bus();
        if clock.frame >= first_frame
            && cpu.pc() == entry
            && cpu.hybrid_instruction_ready(bus)
            && bus.cycles.m68k_wait_cpu_cycles == 0
            && bus.memory.read_main_ram_word(cpu.stack_pointer() + 6) == 1
            && bus
                .memory
                .read_main_ram_byte(ram::g_mode2_reg_softcopy(revision).address())
                & 0x40
                != 0
        {
            return (emulator, clock);
        }
        let effect = emulator
            .tick(
                &mut HeadlessOutput,
                &mut audio,
                &mut Inputs(clock.inputs),
                &mut NoCartridgeSaves,
            )
            .unwrap();
        audio.samples.clear();
        if effect == TickEffect::FrameRendered {
            clock.advance_frame(&mut emulator).unwrap();
        }
    }
    panic!("natural display-enabled count-one Wait entry not reached");
}
fn run_case(
    rom: &[u8],
    revision: Revision,
    entry: &GenesisEmulator,
    clock: &WaitClock,
    mutant: bool,
) -> Result<(u64, u64, usize, u64)> {
    let mut candidate = fork(rom, entry);
    let original = fork(rom, entry);
    let sp = candidate.hybrid_cpu().stack_pointer();
    let bus = candidate.hybrid_bus();
    let return_pc = (u32::from(bus.memory.read_main_ram_word(sp)) << 16)
        | u32::from(bus.memory.read_main_ram_word(sp + 2));
    let divider = candidate.hybrid_bus().cycles.m68k_divider.get() as u32;
    let polling = candidate.hybrid_bus().memory.read_main_ram_word(sp + 6) as i16 > 0
        && candidate
            .hybrid_bus()
            .memory
            .read_main_ram_byte(ram::g_mode2_reg_softcopy(revision).address())
            & 0x40
            != 0;
    let plan = WaitPlan::compile(rom, revision, return_pc, divider)?;
    let mut storage = tje_engine_runtime::timing_region::RegionStorage::new(plan.timing());
    let mut region = TimingRegion::begin_oracle(
        plan.timing(),
        &state(&candidate),
        plan.timing().fixed_target().unwrap(),
        &mut storage,
    )
    .map_err(timing_error)?;
    let mut clock = clock.clone();
    let mut ledger = ProofLedger {
        original,
        clock: clock.clone(),
        audio: HeadlessAudio::default(),
        steps: 0,
        source_instructions: 0,
        accesses: 0,
        ram_writes: 0,
        issue_clocks: Vec::new(),
    };
    let (effects, samples, receipts) = {
        let mut host = TimingHost::new(&mut candidate, &mut clock, &mut ledger)?;
        if mutant {
            host.seed_wrong_cycle_mutant(2);
        }
        while !region.is_complete() {
            host.admit(&mut region)?;
            let mut source = plan.source_step(host.state())?;
            host.step_source(&mut region, &mut source)?;
        }
        for effect in host.effects() {
            assert_eq!(
                effect.expected.phase,
                tje_engine_core::timing_region::ReferenceBusPhase::Issue
            );
        }
        (
            host.effects().len(),
            host.samples()
                .iter()
                .map(|s| s.map(f64::to_bits))
                .collect::<Vec<_>>(),
            host.effects().to_vec(),
        )
    };
    if polling {
        assert!(
            region.usage().original_isr_instructions > 0,
            "proof must service actual counter-changing ISR"
        );
        assert!(
            region.usage().instructions > 11,
            "proof must execute the polling loop"
        );
    }
    let issue_clocks: std::collections::BTreeSet<_> = ledger.issue_clocks.iter().copied().collect();
    for effect in &receipts {
        assert!(issue_clocks.contains(&(effect.expected.instruction_pc, effect.master_clock)));
    }
    assert_eq!(region.usage().scheduled_isr_instructions, 0);
    assert_eq!(candidate.hybrid_cpu().pc(), return_pc);
    assert_eq!(candidate.hybrid_cpu().stack_pointer(), sp + 4);
    let a = candidate.hybrid_bus();
    let b = ledger.original.hybrid_bus();
    for address in (0..65536).step_by(2) {
        assert_eq!(
            a.memory.read_main_ram_word(address),
            b.memory.read_main_ram_word(address)
        );
    }
    assert!(a.vdp.hybrid_state_eq(&b.vdp));
    assert_eq!(a.native_writes, b.native_writes);
    assert_eq!(
        samples,
        ledger
            .audio
            .samples
            .iter()
            .map(|s| s.map(f64::to_bits))
            .collect::<Vec<_>>()
    );
    let encoded_reference =
        bincode::encode_to_vec(&ledger.original, bincode::config::standard()).unwrap();
    let encoded_candidate =
        bincode::encode_to_vec(&candidate, bincode::config::standard()).unwrap();
    assert!(
        encoded_reference == encoded_candidate,
        "complete CPU/Z80/device/resampler state differs"
    );
    Ok((
        ledger.source_instructions,
        region.usage().original_isr_instructions,
        effects,
        clock.frame,
    ))
}

#[test]
#[ignore = "requires both private ROM revisions; exercises natural frame-480+ timing state"]
fn both_revision_timeaware_source_and_wrong_cycle_mutant() {
    for (revision, key) in [
        (Revision::Rev02, "TJE_REV02_ROM"),
        (Revision::Rev00, "TJE_REV00_ROM"),
    ] {
        let rom = fs::read(std::env::var(key).expect("private ROM path required")).unwrap();
        let verified = tje_rom::Rom::import(&rom).unwrap();
        assert_eq!(verified.revision(), revision);
        let (entry, clock) = capture(&rom, revision);
        let (source, isr, effects, _) = run_case(&rom, revision, &entry, &clock, false).unwrap();
        let error = run_case(&rom, revision, &entry, &clock, true).unwrap_err();
        assert!(
            error.to_string().contains("wrong-cycle timing rejected"),
            "{error}"
        );
        eprintln!(
            "{revision:?}: frame={} source_instructions={source} original_ISR_instructions={isr} effects={effects}; exact CPU/bus/RAM/VDP/audio/stream PASS; +2-cycle prefix mutant rejected",
            clock.frame
        );
        for count in [0, u16::MAX] {
            let mut fixture = fork(&rom, &entry);
            let sp = fixture.hybrid_cpu().stack_pointer();
            fixture
                .hybrid_bus_mut()
                .memory
                .write_main_ram_word(sp + 6, count);
            run_case(&rom, revision, &fixture, &clock, false).unwrap();
        }
        let mut disabled = fork(&rom, &entry);
        let display = ram::g_mode2_reg_softcopy(revision).address();
        let mode = disabled.hybrid_bus().memory.read_main_ram_byte(display);
        disabled
            .hybrid_bus_mut()
            .memory
            .write_main_ram_byte(display, mode & !0x40);
        run_case(&rom, revision, &disabled, &clock, false).unwrap();
    }
}

// These witnesses are controller-only, with fixed controller connectivity. Use
// the ordinary InputScript policy after validating that exact narrower shape;
// do not silently approximate recipes or connection/disconnection events.
fn controller_script(session: &tje_session::Session) -> InputScript {
    assert!(matches!(session.start, tje_session::Start::PowerOn));
    let connected = session.runs[0].2 & tje_session::CONNECTED;
    let buttons = [
        (tje_session::UP, Button::Up),
        (tje_session::DOWN, Button::Down),
        (tje_session::LEFT, Button::Left),
        (tje_session::RIGHT, Button::Right),
        (tje_session::A, Button::A),
        (tje_session::B, Button::B),
        (tje_session::C, Button::C),
        (tje_session::START, Button::Start),
    ];
    let mut start = 0;
    let holds = session
        .runs
        .iter()
        .map(|&tje_session::Run(frames, p1, p2)| {
            assert_eq!(p1 & tje_session::CONNECTED, tje_session::CONNECTED);
            assert_eq!(p2 & tje_session::CONNECTED, connected);
            let end = start + frames;
            let hold = InputHold {
                start,
                end,
                p1: buttons
                    .iter()
                    .filter_map(|&(bit, button)| (p1 & bit != 0).then_some(button))
                    .collect(),
                p2: buttons
                    .iter()
                    .filter_map(|&(bit, button)| (p2 & bit != 0).then_some(button))
                    .collect(),
            };
            start = end;
            hold
        })
        .collect();
    InputScript {
        p2_connected: connected != 0,
        holds,
    }
}

#[test]
#[ignore = "requires both private ROMs and the reported controller-edge session paths"]
fn both_revision_reported_controller_edge_uses_each_original_issue_phase() {
    for (revision, rom_key, session_key) in [
        (Revision::Rev02, "TJE_REV02_ROM", "TJE_TIMING_SESSION_REV02"),
        (Revision::Rev00, "TJE_REV00_ROM", "TJE_TIMING_SESSION_REV00"),
    ] {
        let rom = fs::read(std::env::var(rom_key).expect("private ROM path required")).unwrap();
        assert_eq!(tje_rom::Rom::import(&rom).unwrap().revision(), revision);
        let session = tje_session::Session::from_json(
            &fs::read_to_string(
                std::env::var(session_key).expect("reported session path required"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(Revision::from(session.revision), revision);
        let script = controller_script(&session);
        assert_ne!(script.at(837), script.at(838), "reported input edge");
        let (entry, clock) = capture_with(&rom, revision, Arc::new(script), 837);
        assert_eq!(clock.frame, 837, "must exercise the reported call frame");
        let (source, isr, effects, returned_frame) =
            run_case(&rom, revision, &entry, &clock, false).unwrap();
        assert!(returned_frame >= 838, "must sample the actual input edge");
        let error = run_case(&rom, revision, &entry, &clock, true).unwrap_err();
        assert!(
            error.to_string().contains("wrong-cycle timing rejected"),
            "{error}"
        );
        eprintln!(
            "{revision:?}: reported controller edge {}->{returned_frame}; source_instructions={source} original_ISR_instructions={isr} effects={effects}; exact per-step CPU/bus/RAM and complete devices/audio PASS; +2-cycle prefix mutant rejected",
            clock.frame,
        );
    }
}
