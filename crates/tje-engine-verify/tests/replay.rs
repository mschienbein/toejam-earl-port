//! Authored machine timelines test the harness, not game behavior.
use anyhow::Result;
use tje_engine_verify::{
    FrameState, Granularity, Machine, Position, Step, verify_session, verify_session_with,
};
use tje_rom::Revision;
use tje_session::{CONNECTED, RamWrite, Session, Start};

struct AuthoredTimeline {
    tick: u64,
    work: Vec<u8>,
    corrupt_at: Option<u64>,
    samples: Vec<[u64; 2]>,
}

impl AuthoredTimeline {
    fn new(corrupt_at: Option<u64>) -> Self {
        Self {
            tick: 0,
            work: vec![0; 16],
            corrupt_at,
            samples: Vec::new(),
        }
    }
}

impl Machine for AuthoredTimeline {
    fn backend_name(&self) -> &str {
        "authored-self-test"
    }
    fn position(&self) -> Position {
        Position {
            pc: 0x100 + 2 * (self.tick as u32 % 4),
            master_clock: self.tick * 28,
            scanline: 0,
            line_master_clock: self.tick * 28,
        }
    }
    fn begin_frame(&mut self) {
        self.samples.clear();
    }
    fn write_ram(&mut self, write: &RamWrite) -> Result<()> {
        let offset = (write.address & 0xffff) as usize;
        let bytes = write.value.to_be_bytes();
        self.work[offset..offset + usize::from(write.width)]
            .copy_from_slice(&bytes[4 - usize::from(write.width)..]);
        Ok(())
    }
    fn step(&mut self, pads: [u16; 2]) -> Result<Step> {
        let pc = self.position().pc;
        self.work[0] = pads[0] as u8;
        self.work[1] = pads[1] as u8;
        if self.corrupt_at == Some(self.tick) {
            self.work[7] ^= 1;
        }
        self.samples.push([self.tick, self.tick + 1]);
        self.tick += 1;
        Ok(Step {
            frame_complete: self.tick.is_multiple_of(4),
            instruction_pc: (!self.tick.is_multiple_of(3)).then_some(pc),
        })
    }
    fn capture(&self) -> Result<FrameState> {
        Ok(FrameState {
            position: self.position(),
            work_ram: self.work.clone(),
            audio_bits: self.samples.clone(),
            ..FrameState::default()
        })
    }
}

fn session() -> Session {
    let mut session = Session::new(Revision::Rev02, "authored harness timeline");
    session.record([CONNECTED, 0]).unwrap();
    session.record([CONNECTED | 4, CONNECTED]).unwrap();
    session.record([CONNECTED | 8, CONNECTED | 2]).unwrap();
    session
}

#[test]
fn independent_identical_timelines_compare_every_frame() {
    let report = verify_session(
        &session(),
        &mut AuthoredTimeline::new(None),
        &mut AuthoredTimeline::new(None),
    )
    .unwrap();
    assert!(report.passed());
    assert_eq!(report.frames_compared, 3);
    assert_eq!(report.scheduler_steps, 12);
    assert_eq!(report.coverage.instructions_executed, 8);
    assert_eq!(
        report
            .coverage
            .instructions
            .iter()
            .map(|entry| entry.executions)
            .sum::<u64>(),
        8
    );
}

#[test]
fn first_bad_frame_is_replayed_to_first_bad_original_instruction() {
    let report = verify_session_with(&session(), || {
        Ok((AuthoredTimeline::new(None), AuthoredTimeline::new(Some(6))))
    })
    .unwrap();
    assert!(!report.passed());
    assert_eq!(report.frames_compared, 1);
    let difference = report.divergence.unwrap();
    assert_eq!(difference.frame, 1);
    assert_eq!(difference.step, 2);
    assert_eq!(
        difference.granularity,
        Granularity::OriginalInstructionBoundary
    );
    assert_eq!(difference.reference.master_clock, 196);
    assert_eq!(difference.reference.scanline, 0);
    assert_eq!(difference.reference.line_master_clock, 196);
    assert_eq!(difference.instruction_pc, Some(0x104));
    assert_eq!(difference.difference.domain, "work_ram");
    assert_eq!(difference.difference.index, Some(7));
}

#[test]
fn documented_recipes_are_applied_to_both_independent_machines() {
    let mut session = session();
    session.start = Start::Recipe {
        description: "synthetic harness mutation".into(),
        writes: vec![RamWrite {
            before_frame: 1,
            address: 0xff000a,
            width: 2,
            value: 0x1234,
            reason: "test paired big-endian recipe".into(),
        }],
    };
    let (mut reference, mut candidate) = (AuthoredTimeline::new(None), AuthoredTimeline::new(None));
    let report = verify_session(&session, &mut reference, &mut candidate).unwrap();
    assert!(report.passed());
    assert_eq!(&candidate.work[10..12], &[0x12, 0x34]);
    assert_eq!(reference.work, candidate.work);
}
