//! Authored proof-capable timelines isolate the verification runner's contract.
use anyhow::Result;
use tje_engine_core::ReadKind;
use tje_engine_runtime::BusAccess;
use tje_engine_verify::{
    FrameState, InstructionMachine, InstructionProof, Machine, Position, Step,
    verify_instruction_session, verify_session,
};
use tje_rom::Revision;
use tje_session::{CONNECTED, RamWrite, Session};

struct Timeline {
    tick: u64,
    corrupt_at: Option<u64>,
    journal: Vec<BusAccess>,
}
impl Timeline {
    fn new(corrupt_at: Option<u64>) -> Self {
        Self {
            tick: 0,
            corrupt_at,
            journal: Vec::new(),
        }
    }
}
impl Machine for Timeline {
    fn backend_name(&self) -> &str {
        "authored-instruction-proof"
    }
    fn position(&self) -> Position {
        Position {
            pc: 0x100 + 2 * (self.tick as u32 % 4),
            master_clock: self.tick * 28,
            scanline: 0,
            line_master_clock: self.tick * 28,
        }
    }
    fn begin_frame(&mut self) {}
    fn write_ram(&mut self, _: &RamWrite) -> Result<()> {
        anyhow::bail!("this authored timeline has no recipe")
    }
    fn step(&mut self, _: [u16; 2]) -> Result<Step> {
        let pc = self.position().pc;
        self.journal.clear();
        self.journal.push(BusAccess::Read {
            address: pc + 2,
            word: true,
            value: 0x4e71,
            kind: ReadKind::Prefetch,
        });
        self.tick += 1;
        Ok(Step {
            frame_complete: self.tick.is_multiple_of(4),
            instruction_pc: Some(pc),
        })
    }
    fn capture(&self) -> Result<FrameState> {
        Ok(FrameState {
            position: self.position(),
            work_ram: vec![0; 4],
            ..FrameState::default()
        })
    }
}
impl InstructionMachine for Timeline {
    fn enable_instruction_proof(&mut self) {
        self.journal.clear();
    }
    fn instruction_proof(&self) -> InstructionProof<'_> {
        let mut cpu = [0; 25];
        cpu[17] = self.position().pc;
        // A hidden CPU latch intentionally does not change the frame projection.
        cpu[22] = u32::from(self.corrupt_at == Some(self.tick));
        InstructionProof {
            cpu,
            base_cycles: if self.tick == 0 { 0 } else { 4 },
            accesses: &self.journal,
            ram_writes: &[],
        }
    }
}
fn session() -> Session {
    let mut session = Session::new(Revision::Rev02, "authored instruction proof timeline");
    for _ in 0..3 {
        session.record([CONNECTED, 0]).unwrap();
    }
    session
}

#[test]
fn proof_has_exact_steps_instruction_and_bus_denominators() {
    let mut report = verify_instruction_session(
        &session(),
        &mut Timeline::new(None),
        &mut Timeline::new(None),
    )
    .unwrap();
    assert!(report.passed());
    let proof = report.instruction_proof.as_ref().unwrap();
    assert!(proof.initial_state_compared);
    assert_eq!(proof.scheduler_steps_compared, 12);
    assert_eq!(proof.instructions_compared, 12);
    assert_eq!(proof.bus_accesses_compared, 12);
    report
        .instruction_proof
        .as_mut()
        .unwrap()
        .scheduler_steps_compared -= 1;
    assert!(!report.passed(), "a partial instruction proof cannot pass");
}

#[test]
fn full_hidden_cpu_state_is_checked_before_the_first_frame_finishes() {
    assert!(
        verify_session(
            &session(),
            &mut Timeline::new(None),
            &mut Timeline::new(Some(2))
        )
        .unwrap()
        .passed()
    );
    let report = verify_instruction_session(
        &session(),
        &mut Timeline::new(None),
        &mut Timeline::new(Some(2)),
    )
    .unwrap();
    assert!(!report.passed());
    assert_eq!(report.frames_compared, 0);
    let mismatch = report.divergence.unwrap();
    assert_eq!(mismatch.frame, 0);
    assert_eq!(mismatch.step, 1);
    assert_eq!(mismatch.instruction_pc, Some(0x102));
    assert_eq!(mismatch.difference.domain, "instruction_cpu.frozen");
}

#[test]
fn initial_hidden_state_is_compared_before_executing_any_instruction() {
    let report = verify_instruction_session(
        &session(),
        &mut Timeline::new(None),
        &mut Timeline::new(Some(0)),
    )
    .unwrap();
    assert!(!report.passed());
    assert_eq!(report.scheduler_steps, 0);
    assert_eq!(
        report.instruction_proof.unwrap().scheduler_steps_compared,
        0
    );
    assert_eq!(
        report.divergence.unwrap().difference.domain,
        "instruction_cpu.frozen"
    );
}
