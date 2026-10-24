//! Test-only differential observer. It never implements a production directive.
use super::*;
use jgenesis_common::frontend::PartialClone;
use std::cell::{Cell, RefCell};

thread_local! {
    static ENABLED: Cell<bool> = const { Cell::new(false) };
    static MUTATION: Cell<u32> = const { Cell::new(0) };
    static RECEIPTS: RefCell<Vec<Receipt>> = const { RefCell::new(Vec::new()) };
    static FAIL_AFTER: Cell<Option<FailurePoint>> = const { Cell::new(None) };
    static RECEIPT_FAULT: Cell<Option<ReceiptFault>> = const { Cell::new(None) };
    static FAILURE: RefCell<Option<FailureSnapshot>> = const { RefCell::new(None) };
}
#[derive(Debug, Clone)]
pub(super) struct Receipt {
    pub frame: u64,
    pub returned_frame: u64,
    pub source: u64,
    pub original_isr: u64,
    pub bus_effects: usize,
    pub steps: u64,
}
pub(super) struct Enable;
impl Enable {
    pub(super) fn new() -> Self {
        ENABLED.with(|value| value.set(true));
        Self
    }
}
impl Drop for Enable {
    fn drop(&mut self) {
        ENABLED.with(|value| value.set(false));
        MUTATION.with(|value| value.set(0));
        FAIL_AFTER.with(|value| value.set(None));
        RECEIPT_FAULT.with(|value| value.set(None));
        FAILURE.with(|value| {
            value.borrow_mut().take();
        });
    }
}
pub(super) fn mutate_next(cycles: u32) {
    MUTATION.with(|value| value.set(cycles));
}
pub(super) fn take_mutation() -> u32 {
    MUTATION.with(|value| value.replace(0))
}
pub(super) fn receipts() -> Vec<Receipt> {
    RECEIPTS.with(|value| std::mem::take(&mut *value.borrow_mut()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FailurePoint {
    Capture,
    RteTail,
    Frame,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReceiptFault {
    WrongKind,
    Duplicate,
}
pub(super) struct FailureSnapshot {
    pub bytes: Vec<u8>,
    pub cpu: [u32; 25],
    pub frame: FrameState,
    pub input_frame: u64,
}
pub(super) fn fail_after(point: FailurePoint) {
    FAIL_AFTER.with(|value| value.set(Some(point)));
}
pub(super) fn fault_receipt(point: ReceiptFault) {
    RECEIPT_FAULT.with(|value| value.set(Some(point)));
}
pub(crate) fn take_receipt_fault() -> Option<ReceiptFault> {
    RECEIPT_FAULT.with(|value| value.replace(None))
}
pub(crate) fn consume_receipt(
    ledger: &mut CompletionOwners,
    mut request: game::CompletionRequest,
    cpu: &CpuState,
) -> std::result::Result<game::Completion, tje_engine_core::timing_region::TimingError> {
    let fault = take_receipt_fault();
    if fault == Some(ReceiptFault::WrongKind) {
        request.kind = game::CompletionKind::ExceptionReturn;
    }
    let result = ledger.consume(request, cpu);
    if fault == Some(ReceiptFault::Duplicate) && matches!(result, Ok(game::Completion::Consumed)) {
        return ledger.consume(request, cpu);
    }
    result
}
pub(super) fn failure() -> Option<FailureSnapshot> {
    FAILURE.with(|value| value.borrow_mut().take())
}
pub(crate) fn begin_invocation() {
    FAILURE.with(|value| {
        value.borrow_mut().take();
    });
}
pub(crate) fn record_failure(state: &GenesisEmulator, frame: FrameState, input_frame: u64) {
    if !ENABLED.with(Cell::get) {
        return;
    }
    FAILURE.with(|value| {
        let mut first = value.borrow_mut();
        if first.is_none() {
            let bytes = bincode::encode_to_vec(state, bincode::config::standard()).unwrap();
            *first = Some(FailureSnapshot {
                bytes,
                cpu: state.hybrid_cpu().hybrid_state_words(),
                frame,
                input_frame,
            });
        }
    });
}
#[test]
fn first_failure_tap_does_not_replace_earlier_physical_or_frame_evidence() {
    let _enabled = Enable::new();
    let mut host = crate::fixture::tests::synthetic_host(&[0x4e71], &[0x4e73]);
    let frame = FrameState::new(host.cpu().stack_pointer());
    let bytes = bincode::encode_to_vec(&host.emulator, bincode::config::standard()).unwrap();
    begin_invocation();
    record_failure(&host.emulator, frame, 0);
    host.emulator
        .hybrid_bus_mut()
        .memory
        .write_main_ram_byte(0xff8000, 0xa5);
    record_failure(&host.emulator, FrameState::new(frame.entry_sp - 4), 1);
    let first = failure().unwrap();
    assert_eq!(first.bytes, bytes);
    assert_eq!(first.frame, frame);
    assert_eq!(first.input_frame, 0);
}
pub(super) fn maybe_fail(
    owner: ProgressOwner,
    before_depth: u32,
    after_depth: u32,
    step: DeviceStep,
) -> Result<()> {
    let fail = FAIL_AFTER.with(|value| {
        let matched = match value.get() {
            Some(FailurePoint::Capture) => owner == ProgressOwner::RustInstruction,
            Some(FailurePoint::RteTail) => {
                owner == (ProgressOwner::OriginalInterrupt { instruction: true })
                    && after_depth < before_depth
            }
            Some(FailurePoint::Frame) => step.frame_complete,
            None => false,
        };
        if matched {
            value.set(None);
        }
        matched
    });
    if fail {
        return Err(HostError::Metadata(
            "authored actual-step capture callback failure".into(),
        ));
    }
    Ok(())
}

pub(super) struct Oracle {
    reference: GenesisEmulator,
    clock: WaitClock,
    audio: HeadlessAudio,
    accesses: usize,
    commits: usize,
    audio_prefix: usize,
    receipt: Receipt,
    drained_audit: ClockRun,
}
fn observers(state: &mut GenesisEmulator) {
    let bus = state.hybrid_bus_mut();
    bus.native_cpu_accesses = Some(Vec::new());
    bus.native_ram_writes = Some(Vec::new());
    bus.enable_native_write_trace();
}
impl Oracle {
    pub(super) fn begin(
        state: &mut GenesisEmulator,
        clock: &WaitClock,
        audio_prefix: usize,
    ) -> Option<Self> {
        if !ENABLED.with(Cell::get) {
            return None;
        }
        let mut reference = state.partial_clone();
        observers(&mut reference);
        observers(state);
        Some(Self {
            reference,
            clock: clock.clone(),
            audio: HeadlessAudio::default(),
            accesses: 0,
            commits: 0,
            audio_prefix,
            drained_audit: ClockRun::default(),
            receipt: Receipt {
                frame: clock.frame,
                returned_frame: clock.frame,
                source: 0,
                original_isr: 0,
                bus_effects: 0,
                steps: 0,
            },
        })
    }
    pub(super) fn step(
        &mut self,
        candidate: &mut GenesisEmulator,
        owner: ProgressOwner,
        step: DeviceStep,
    ) -> Result<()> {
        // Loan the exact immutable cartridge, just like Host::execute_fork.
        // It is always returned before propagating a Result error.
        self.reference
            .hybrid_bus_mut()
            .cartridge
            .as_mut()
            .expect("reference cartridge")
            .take_rom_from(
                candidate
                    .hybrid_bus_mut()
                    .cartridge
                    .as_mut()
                    .expect("candidate cartridge"),
            );
        let before = self.reference.hybrid_bus().cycles.m68k_mclk_cycles;
        let original_pc = self.reference.hybrid_cpu().pc();
        let result = self
            .reference
            .tick(
                &mut HeadlessOutput,
                &mut self.audio,
                &mut Inputs(self.clock.inputs),
                &mut NoCartridgeSaves,
            )
            .map_err(|error| HostError::Emulator(error.to_string()));
        candidate
            .hybrid_bus_mut()
            .cartridge
            .as_mut()
            .expect("candidate cartridge")
            .take_rom_from(
                self.reference
                    .hybrid_bus_mut()
                    .cartridge
                    .as_mut()
                    .expect("reference cartridge"),
            );
        let effect = result?;
        let expected_clock = self.reference.hybrid_bus().cycles.m68k_mclk_cycles;
        if expected_clock - before != step.master_clocks
            || expected_clock != candidate.hybrid_bus().cycles.m68k_mclk_cycles
        {
            return Err(HostError::Metadata(format!(
                "strict source wrong-cycle mutant rejected at step {}",
                self.receipt.steps
            )));
        }
        if self.reference.hybrid_cpu().hybrid_state_words()
            != candidate.hybrid_cpu().hybrid_state_words()
        {
            return Err(HostError::Metadata(format!(
                "strict source CPU mismatch at step {}",
                self.receipt.steps
            )));
        }
        let original = self.reference.hybrid_bus();
        let rust = candidate.hybrid_bus();
        let expected = original
            .native_cpu_accesses
            .as_ref()
            .expect("original real bus journal");
        let observed = rust
            .native_cpu_accesses
            .as_ref()
            .expect("source real bus journal");
        if expected[self.accesses..] != observed[self.accesses..] {
            return Err(HostError::Metadata(format!(
                "strict source bus mismatch at step {}",
                self.receipt.steps
            )));
        }
        let new_accesses = expected.len() - self.accesses;
        self.accesses = expected.len();
        let expected = original
            .native_ram_writes
            .as_ref()
            .expect("original all-origin commits");
        let observed = rust
            .native_ram_writes
            .as_ref()
            .expect("source all-origin commits");
        if expected[self.commits..] != observed[self.commits..] {
            return Err(HostError::Metadata(format!(
                "strict source RAM commit mismatch at step {}",
                self.receipt.steps
            )));
        }
        self.commits = expected.len();
        if (effect == TickEffect::FrameRendered) != step.frame_complete {
            return Err(HostError::Metadata(
                "strict source frame event mismatch".into(),
            ));
        }
        self.receipt.steps += 1;
        self.receipt.source += u64::from(owner == ProgressOwner::RustInstruction);
        self.receipt.original_isr +=
            u64::from(owner == (ProgressOwner::OriginalInterrupt { instruction: true }));
        if owner == ProgressOwner::RustInstruction {
            self.receipt.bus_effects += new_accesses;
        }
        if step.frame_complete {
            self.clock.advance_frame(&mut self.reference)?;
            if !candidate
                .hybrid_bus()
                .vdp
                .hybrid_state_eq(&self.reference.hybrid_bus().vdp)
            {
                return Err(HostError::Metadata(
                    "strict source video-boundary device mismatch".into(),
                ));
            }
        }
        // The real source collector has already consumed its diagnostic queue.
        // Consume the reference queue through the same non-hardware collector
        // after comparing the independent complete bus/commit journals above.
        // No CPU/device state is normalized or repaired for the final encoding.
        crate::execution::drain_accesses(
            &mut self.reference,
            original_pc,
            before,
            self.clock.frame,
            false,
            &mut self.drained_audit,
            PhysicalObservers::default(),
        );
        Ok(())
    }
    pub(super) fn finish(mut self, candidate: &GenesisEmulator, audio: &[[f64; 2]]) -> Result<()> {
        let expected = bincode::encode_to_vec(&self.reference, bincode::config::standard())
            .map_err(|error| HostError::Metadata(error.to_string()))?;
        let observed = bincode::encode_to_vec(candidate, bincode::config::standard())
            .map_err(|error| HostError::Metadata(error.to_string()))?;
        if expected != observed {
            let first = expected.iter().zip(&observed).position(|(a, b)| a != b);
            let original = self.reference.hybrid_bus();
            let rust = candidate.hybrid_bus();
            return Err(HostError::Metadata(format!(
                "strict source complete state differs: encoded_lengths={}/{}, first_offset={first:?}; audit_cpu={:?}/{:?}, audit_boundaries={:?}/{:?}, audit_writes={:?}/{:?}",
                expected.len(),
                observed.len(),
                original.hybrid_cpu_accesses.as_ref().map(Vec::len),
                rust.hybrid_cpu_accesses.as_ref().map(Vec::len),
                original.hybrid_exception_boundaries.as_ref().map(Vec::len),
                rust.hybrid_exception_boundaries.as_ref().map(Vec::len),
                original.hybrid_writes.as_ref().map(Vec::len),
                rust.hybrid_writes.as_ref().map(Vec::len),
            )));
        }
        if !audio[self.audio_prefix..]
            .iter()
            .map(|sample| sample.map(f64::to_bits))
            .eq(self
                .audio
                .samples
                .iter()
                .map(|sample| sample.map(f64::to_bits)))
        {
            return Err(HostError::Metadata(format!(
                "strict source complete audio state differs: samples={}/{}",
                self.audio.samples.len(),
                audio.len() - self.audio_prefix,
            )));
        }
        self.receipt.returned_frame = self.clock.frame;
        RECEIPTS.with(|receipts| receipts.borrow_mut().push(self.receipt));
        Ok(())
    }
}
