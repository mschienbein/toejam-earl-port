use super::*;
use tje_engine_core::timing_region::{BusObservation, RustPathChoice, TimingEvent};

fn plan() -> TimingProgram {
    const NOP: u16 = 0x4e71;
    let rom = NOP
        .to_be_bytes()
        .into_iter()
        .chain([0; 12])
        .collect::<Vec<_>>();
    TimingProgram::compile(
        &rom,
        ReviewedRegion {
            name: "authored single-instruction region",
            source: "synthetic NOP stream",
            entry: 0,
            instruction_pcs: vec![0],
            edges: vec![RegionEdge { from: 0, to: 2 }],
            return_pc: 2,
            maximum_steps: 4,
            required_interrupts: &[6],
        },
        7,
    )
    .unwrap()
}
fn cpu() -> CpuState {
    CpuState {
        prefetch: 0x4e71,
        sr: 0x2000,
        ssp: 0x2000,
        ..CpuState::default()
    }
}

#[test]
fn startup_missing_native_isr_closure_and_wrong_pipeline_fail_closed() {
    let p = plan();
    assert!(TimingRegion::begin_native(&p, &cpu(), None).is_err());
    let mut changed = cpu();
    changed.prefetch ^= 1;
    assert!(TimingRegion::begin_oracle(&p, &changed).is_err());
}

#[test]
fn boundary_priority_matches_wait_reset_halt_pending_irq_mask_stop() {
    let mut c = cpu();
    let mut s = BoundarySignals {
        wait: true,
        reset: true,
        halted: true,
        interrupt_level: 6,
    };
    c.pending_interrupt = 4;
    c.stopped = 1;
    assert_eq!(classify_boundary(&c, s), BoundaryAction::Wait);
    s.wait = false;
    assert_eq!(classify_boundary(&c, s), BoundaryAction::Reset);
    s.reset = false;
    assert_eq!(classify_boundary(&c, s), BoundaryAction::Halted);
    s.halted = false;
    assert_eq!(
        classify_boundary(&c, s),
        BoundaryAction::InterruptAcknowledge { level: 4 }
    );
    c.pending_interrupt = 0;
    assert_eq!(
        classify_boundary(&c, s),
        BoundaryAction::InterruptAdmission { level: 6 }
    );
    c.sr = 0x2700;
    assert_eq!(classify_boundary(&c, s), BoundaryAction::Stopped);
    c.stopped = 0;
    assert_eq!(classify_boundary(&c, s), BoundaryAction::Ready);
}

#[test]
fn interrupt_resumption_cannot_cross_regions_or_repair_state() {
    let p = plan();
    let c = cpu();
    let mut first = TimingRegion::begin_oracle(&p, &c).unwrap();
    let mut second = TimingRegion::begin_oracle(&p, &c).unwrap();
    let action = BoundaryAction::InterruptAdmission { level: 6 };
    let token = first.suspend_interrupt(&c, action).unwrap();
    let _other = second.suspend_interrupt(&c, action).unwrap();
    assert!(second.resume_oracle_interrupt(token, &c, 1, true).is_err());
    let mut region = TimingRegion::begin_oracle(&p, &c).unwrap();
    let token = region.suspend_interrupt(&c, action).unwrap();
    region
        .record_progress(DeviceStep {
            cpu_cycles: 20,
            master_clocks: 140,
            frame_complete: false,
        })
        .unwrap();
    region.record_original_instruction().unwrap();
    let mut wrong = c.clone();
    wrong.ssp += 2;
    assert!(
        region
            .resume_oracle_interrupt(token, &wrong, 1, true)
            .is_err()
    );
    let mut region = TimingRegion::begin_oracle(&p, &c).unwrap();
    let token = region.suspend_interrupt(&c, action).unwrap();
    region
        .record_progress(DeviceStep {
            cpu_cycles: 20,
            master_clocks: 140,
            frame_complete: false,
        })
        .unwrap();
    region.record_original_instruction().unwrap();
    let mut wrong_user = c.clone();
    wrong_user.usp ^= 0x100;
    assert!(
        region
            .resume_oracle_interrupt(token, &wrong_user, 1, true)
            .is_err()
    );
}

#[test]
fn source_edge_completion_and_zero_frame_budget_are_finite() {
    let p = plan();
    let c = cpu();
    let mut region = TimingRegion::begin_oracle(&p, &c).unwrap();
    let mut cursor = region.cursor(&c, BoundaryAction::Ready).unwrap();
    let timing = loop {
        match cursor.next_event().unwrap() {
            TimingEvent::SetReadOffset { token, .. } => cursor.acknowledge_offset(token).unwrap(),
            TimingEvent::Bus { token, expected } => cursor
                .acknowledge_bus(
                    token,
                    BusObservation {
                        address: expected.address,
                        width: expected.width,
                        kind: expected.kind,
                        value: 0,
                    },
                )
                .unwrap(),
            TimingEvent::InstructionComplete { token, .. } => {
                break cursor
                    .complete_instruction(token, RustPathChoice { next_pc: 2 })
                    .unwrap();
            }
            other => panic!("{other:?}"),
        }
    };
    region.accept_instruction(timing).unwrap();
    assert!(region.is_complete());
    for _ in 0..4 {
        region.check_step_budget().unwrap();
        region
            .record_progress(DeviceStep {
                cpu_cycles: 0,
                master_clocks: 0,
                frame_complete: false,
            })
            .unwrap();
    }
    assert!(matches!(
        region.check_step_budget(),
        Err(TimingError::Exhausted)
    ));
}

#[test]
fn address_fault_and_odd_boundary_cannot_be_erased_by_region_admission() {
    let p = plan();
    let mut c = cpu();
    c.address_error = 1;
    assert!(matches!(
        TimingRegion::begin_oracle(&p, &c),
        Err(TimingError::PendingFault)
    ));
    let signals = BoundarySignals {
        wait: false,
        reset: false,
        halted: false,
        interrupt_level: 0,
    };
    assert_eq!(classify_boundary(&c, signals), BoundaryAction::Faulted);
    c.address_error = 0;
    c.pc = 1;
    assert!(TimingRegion::begin_oracle(&p, &c).is_err());
}
