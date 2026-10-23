//! Development-only cycle-exact engine verification. No emulator/interpreter
//! dependency from this crate belongs in the shipped native or browser frontend.

pub mod audio;
pub mod coverage;
pub mod instruction_proof;
pub mod interpreter;
pub mod ir_backend;
pub mod reference;
pub mod runtime;
pub mod state;

pub use coverage::{Catalogue, CoverageReport};
pub use instruction_proof::{InstructionMachine, InstructionProof, InstructionProofCounts};
pub use runtime::RuntimeMachine;
pub use state::{CpuSnapshot, Difference, FrameState, Position, compare_frames};

use anyhow::{Context, Result, ensure};
use coverage::CoverageCollector;
use instruction_proof::compare_instruction_proofs;
use serde::{Deserialize, Serialize};
use tje_session::{RamWrite, Session};

pub const REPORT_FORMAT: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    pub frame_complete: bool,
    /// None for refresh/DMA waits, interrupt entry, STOP or other noninstructions.
    pub instruction_pc: Option<u32>,
}

/// A machine advances exactly one original instruction or reference wait/IRQ
/// step. Snapshots and observation buffers must not mutate its hardware state.
pub trait Machine {
    fn backend_name(&self) -> &str;
    fn position(&self) -> Position;
    fn begin_frame(&mut self);
    fn write_ram(&mut self, write: &RamWrite) -> Result<()>;
    fn step(&mut self, pads: [u16; 2]) -> Result<Step>;
    fn capture(&self) -> Result<FrameState>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Granularity {
    FrameBoundary,
    OriginalInstructionBoundary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Divergence {
    /// Zero-based completed-video-frame interval from the session input format.
    pub frame: u64,
    /// Zero-based scheduler step within that interval (includes explicit waits).
    pub step: u64,
    pub granularity: Granularity,
    pub reference: Position,
    pub candidate: Position,
    /// The original instruction just executed, not the following PC.
    pub instruction_pc: Option<u32>,
    pub function: String,
    pub candidate_instruction_pc: Option<u32>,
    pub candidate_function: String,
    pub difference: Difference,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayReport {
    pub format: u32,
    pub revision: String,
    pub reference_backend: String,
    pub candidate_backend: String,
    pub requested_frames: u64,
    pub frames_compared: u64,
    pub scheduler_steps: u64,
    pub divergence: Option<Divergence>,
    /// Only actual original instructions executed by the candidate contribute.
    pub coverage: CoverageReport,
    /// Present only when full architectural state, base charge and all ordered
    /// 68000 bus accesses were compared after every scheduler step.
    pub instruction_proof: Option<InstructionProofCounts>,
}

impl ReplayReport {
    pub fn passed(&self) -> bool {
        self.divergence.is_none()
            && self.frames_compared == self.requested_frames
            && self.instruction_proof.as_ref().is_none_or(|proof| {
                proof.initial_state_compared
                    && proof.scheduler_steps_compared == self.scheduler_steps
                    && proof.instructions_compared == self.coverage.instructions_executed
            })
    }
}

/// Compare two independent machines. The library is filesystem-free and usable
/// from a browser proof wrapper. Use `verify_session_with` when restart factories
/// are available to refine a frame mismatch to its first instruction boundary.
pub fn verify_session(
    session: &Session,
    reference: &mut impl Machine,
    candidate: &mut impl Machine,
) -> Result<ReplayReport> {
    replay(session, reference, candidate, None, false, |_, _, _| None)
}

/// A frame mismatch is replayed from power-on, comparing full state after every
/// scheduler step only in the first divergent interval. This avoids claiming a
/// frame-end PC caused an earlier RAM/pixel/sample/latch divergence.
pub fn verify_session_with<R: Machine, C: Machine>(
    session: &Session,
    mut factory: impl FnMut() -> Result<(R, C)>,
) -> Result<ReplayReport> {
    let (mut reference, mut candidate) = factory()?;
    let mut report = verify_session(session, &mut reference, &mut candidate)?;
    if let Some(divergence) = &report.divergence
        && divergence.granularity == Granularity::FrameBoundary
    {
        let frame = divergence.frame;
        drop((reference, candidate));
        let (mut reference, mut candidate) = factory()?;
        let refined = replay(
            session,
            &mut reference,
            &mut candidate,
            Some(frame),
            false,
            |_, _, _| None,
        )?;
        ensure!(
            refined.divergence.is_some(),
            "divergence did not reproduce during localization"
        );
        report.divergence = refined.divergence;
    }
    Ok(report)
}

/// Full instruction proof on the same live timeline as mandatory frame checks.
/// Equal initial RAM plus identical ordered actual CPU/Z80 RAM commits establishes
/// full RAM equality at each boundary, without copying/scanning 64 KiB per step.
/// Issued CPU accesses and complete device/output frames are checked separately.
pub fn verify_instruction_session<R: InstructionMachine, C: InstructionMachine>(
    session: &Session,
    reference: &mut R,
    candidate: &mut C,
) -> Result<ReplayReport> {
    replay_instruction(session, reference, candidate, None)
}

pub fn verify_instruction_session_with<R: InstructionMachine, C: InstructionMachine>(
    session: &Session,
    mut factory: impl FnMut() -> Result<(R, C)>,
) -> Result<ReplayReport> {
    let (mut reference, mut candidate) = factory()?;
    let mut report = verify_instruction_session(session, &mut reference, &mut candidate)?;
    if let Some(divergence) = &report.divergence
        && divergence.granularity == Granularity::FrameBoundary
    {
        let frame = divergence.frame;
        drop((reference, candidate));
        let (mut reference, mut candidate) = factory()?;
        let refined = replay_instruction(session, &mut reference, &mut candidate, Some(frame))?;
        ensure!(
            refined.divergence.is_some(),
            "divergence did not reproduce during localization"
        );
        report.divergence = refined.divergence;
    }
    Ok(report)
}

fn replay_instruction<R: InstructionMachine, C: InstructionMachine>(
    session: &Session,
    reference: &mut R,
    candidate: &mut C,
    diagnose_frame: Option<u64>,
) -> Result<ReplayReport> {
    reference.enable_instruction_proof();
    candidate.enable_instruction_proof();
    let mut counts = InstructionProofCounts {
        initial_state_compared: true,
        ..InstructionProofCounts::default()
    };
    let mut report = replay(
        session,
        reference,
        candidate,
        diagnose_frame,
        true,
        |reference, candidate, step| {
            let expected = reference.instruction_proof();
            let actual = candidate.instruction_proof();
            if let Some(step) = step {
                counts.scheduler_steps_compared += 1;
                counts.instructions_compared += u64::from(step.instruction_pc.is_some());
                counts.bus_accesses_compared += expected.accesses.len() as u64;
                counts.ram_writes_compared += expected.ram_writes.len() as u64;
            }
            compare_instruction_proofs(&expected, &actual)
        },
    )?;
    report.instruction_proof = Some(counts);
    Ok(report)
}

fn divergence(
    frame: u64,
    step: u64,
    granularity: Granularity,
    positions: (Position, Position),
    steps: (Step, Step),
    catalogue: &Catalogue,
    difference: Difference,
) -> Divergence {
    let (reference, candidate) = positions;
    let instruction_pc = steps.0.instruction_pc;
    let candidate_instruction_pc = steps.1.instruction_pc;
    Divergence {
        frame,
        step,
        granularity,
        reference,
        candidate,
        instruction_pc,
        function: catalogue
            .name(instruction_pc.unwrap_or(reference.pc))
            .into(),
        candidate_instruction_pc,
        candidate_function: catalogue
            .name(candidate_instruction_pc.unwrap_or(candidate.pc))
            .into(),
        difference,
    }
}

fn replay<R: Machine, C: Machine>(
    session: &Session,
    reference: &mut R,
    candidate: &mut C,
    diagnose_frame: Option<u64>,
    compare_initial_state: bool,
    mut compare_step: impl FnMut(&R, &C, Option<Step>) -> Option<Difference>,
) -> Result<ReplayReport> {
    session.validate()?;
    ensure!(
        session.frames > 0,
        "verification requires a nonempty session"
    );
    let revision = session.revision.into();
    let catalogue = Catalogue::committed(revision);
    let mut report = ReplayReport {
        format: REPORT_FORMAT,
        revision: revision.name().into(),
        reference_backend: reference.backend_name().into(),
        candidate_backend: candidate.backend_name().into(),
        requested_frames: session.frames,
        frames_compared: 0,
        scheduler_steps: 0,
        divergence: None,
        coverage: CoverageReport::default(),
        instruction_proof: None,
    };
    let mut coverage = CoverageCollector::default();
    if compare_initial_state {
        let expected = reference.capture()?;
        let actual = candidate.capture()?;
        if let Some(difference) =
            compare_frames(&expected, &actual).or_else(|| compare_step(reference, candidate, None))
        {
            let no_instruction = Step {
                frame_complete: false,
                instruction_pc: None,
            };
            report.divergence = Some(divergence(
                0,
                0,
                Granularity::OriginalInstructionBoundary,
                (reference.position(), candidate.position()),
                (no_instruction, no_instruction),
                &catalogue,
                difference,
            ));
            report.coverage = coverage.finish(&catalogue);
            return Ok(report);
        }
    }
    'frames: for (frame, pads) in session.inputs().enumerate() {
        let frame = frame as u64;
        reference.begin_frame();
        candidate.begin_frame();
        for write in session
            .writes()
            .iter()
            .filter(|write| write.before_frame == frame)
        {
            reference
                .write_ram(write)
                .context("applying reference RAM recipe")?;
            candidate
                .write_ram(write)
                .context("applying candidate RAM recipe")?;
        }
        let mut step = 0;
        loop {
            let expected = reference.step(pads).with_context(|| {
                format!(
                    "reference frame {frame} step {step} at {:?}",
                    reference.position()
                )
            })?;
            let actual = candidate.step(pads).with_context(|| {
                format!(
                    "candidate frame {frame} step {step} at {:?}",
                    candidate.position()
                )
            })?;
            report.scheduler_steps += 1;
            if let Some(pc) = actual.instruction_pc {
                coverage.record(pc, frame);
            }
            let positions = (reference.position(), candidate.position());
            let boundary_difference = if expected != actual {
                Some(Difference {
                    domain: "scheduler_step".into(),
                    index: None,
                    reference: format!("{expected:?}"),
                    candidate: format!("{actual:?}"),
                })
            } else if positions.0 != positions.1 {
                Some(Difference {
                    domain: "position".into(),
                    index: None,
                    reference: format!("{:?}", positions.0),
                    candidate: format!("{:?}", positions.1),
                })
            } else {
                None
            };
            if let Some(difference) = boundary_difference {
                report.divergence = Some(divergence(
                    frame,
                    step,
                    Granularity::OriginalInstructionBoundary,
                    positions,
                    (expected, actual),
                    &catalogue,
                    difference,
                ));
                break 'frames;
            }
            if let Some(difference) = compare_step(reference, candidate, Some(expected)) {
                report.divergence = Some(divergence(
                    frame,
                    step,
                    Granularity::OriginalInstructionBoundary,
                    positions,
                    (expected, actual),
                    &catalogue,
                    difference,
                ));
                break 'frames;
            }
            if expected.frame_complete || diagnose_frame == Some(frame) {
                let reference_state = reference.capture()?;
                let candidate_state = candidate.capture()?;
                if let Some(difference) = compare_frames(&reference_state, &candidate_state) {
                    let granularity = if diagnose_frame == Some(frame) {
                        Granularity::OriginalInstructionBoundary
                    } else {
                        Granularity::FrameBoundary
                    };
                    report.divergence = Some(divergence(
                        frame,
                        step,
                        granularity,
                        positions,
                        (expected, actual),
                        &catalogue,
                        difference,
                    ));
                    break 'frames;
                }
            }
            if expected.frame_complete {
                report.frames_compared += 1;
                break;
            }
            step += 1;
        }
    }
    report.coverage = coverage.finish(&catalogue);
    Ok(report)
}
