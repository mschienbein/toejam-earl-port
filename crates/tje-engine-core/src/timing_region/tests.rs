use super::*;
use crate::compile_instruction;

fn plan(words: &[u16]) -> (TimingInstruction, CpuState) {
    let rom: Vec<_> = words
        .iter()
        .flat_map(|w| w.to_be_bytes())
        .chain([0; 12])
        .collect();
    let ir = compile_instruction(&rom, 0).unwrap();
    (
        TimingInstruction::compile(&ir).unwrap(),
        CpuState {
            prefetch: u32::from(words[0]),
            sr: 0x2700,
            a: [0x1000; 7],
            ssp: 0x2000,
            ..CpuState::default()
        },
    )
}
fn next_bus(cursor: &mut TimingCursor<'_>) -> (EventToken, ExpectedAccess) {
    loop {
        match cursor.next_event().unwrap() {
            TimingEvent::Bus { token, expected } => return (token, expected),
            TimingEvent::SetReadOffset { token, .. } => cursor.acknowledge_offset(token).unwrap(),
            other => panic!("expected bus, got {other:?}"),
        }
    }
}
fn ack(cursor: &mut TimingCursor<'_>, token: EventToken, expected: ExpectedAccess, value: u32) {
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

#[test]
fn read_ack_controls_divide_timing_without_extra_read_or_game_store() {
    const DIVU_MEMORY_D0: u16 = 0x80D0;
    let (instruction, mut cpu) = plan(&[DIVU_MEMORY_D0]);
    cpu.d[0] = 0x12345;
    for divisor in [1, 3, 7, 255, 0xffff] {
        let unchanged = cpu.clone();
        let mut scratch = TimingScratch::default();
        let mut cursor = TimingCursor::begin(&instruction, &cpu, &mut scratch).unwrap();
        let (token, fetch) = next_bus(&mut cursor);
        assert_eq!(
            (fetch.address, fetch.kind),
            (2, AccessKind::Read(ReadKind::Prefetch))
        );
        ack(&mut cursor, token, fetch, 0);
        let (token, operand) = next_bus(&mut cursor);
        assert_eq!(operand.address, 0x1000);
        ack(&mut cursor, token, operand, divisor);
        let TimingEvent::InstructionComplete {
            token,
            cycles,
            pipeline,
        } = cursor.next_event().unwrap()
        else {
            panic!("expected completion");
        };
        assert_eq!(
            cycles,
            crate::timing::divu_cycles(cpu.d[0], divisor as u16) + 4
        );
        let grant = cursor
            .complete_instruction(token, RustPathChoice { next_pc: 2 })
            .unwrap();
        assert_eq!(grant.pipeline(), pipeline);
        assert_eq!(cpu, unchanged);
    }
}

#[test]
fn long_predecrement_retains_two_source_owned_word_transfers() {
    const MOVE_LONG_D0_PREDEC_A0: u16 = 0x2100;
    let (instruction, cpu) = plan(&[MOVE_LONG_D0_PREDEC_A0]);
    let unchanged = cpu.clone();
    let mut scratch = TimingScratch::default();
    let mut cursor = TimingCursor::begin(&instruction, &cpu, &mut scratch).unwrap();
    let (token, fetch) = next_bus(&mut cursor);
    ack(&mut cursor, token, fetch, 0);
    let (token, low) = next_bus(&mut cursor);
    assert_eq!(
        (low.address, low.width, low.kind),
        (0xffe, Width::Word, AccessKind::Write)
    );
    ack(&mut cursor, token, low, 0x6789);
    let (token, high) = next_bus(&mut cursor);
    assert_eq!(
        (high.address, high.width, high.kind),
        (0xffc, Width::Word, AccessKind::Write)
    );
    ack(&mut cursor, token, high, 0x1234);
    let TimingEvent::InstructionComplete { token, cycles, .. } = cursor.next_event().unwrap()
    else {
        panic!("completion");
    };
    assert_eq!(cycles, 12);
    cursor
        .complete_instruction(token, RustPathChoice { next_pc: 2 })
        .unwrap();
    assert_eq!(cpu, unchanged); // no private architectural A0 update is published
}

#[test]
fn conditional_prefetch_requires_rust_path_and_does_not_speculate() {
    const BNE_SHORT: u16 = 0x6602;
    for (sr, next, reads, cycles) in [(0x2700, 4, 2, 10), (0x2704, 2, 1, 8)] {
        let (instruction, mut cpu) = plan(&[BNE_SHORT]);
        cpu.sr = sr;
        let mut scratch = TimingScratch::default();
        let mut cursor = TimingCursor::begin(&instruction, &cpu, &mut scratch).unwrap();
        let mut observed = 0;
        loop {
            match cursor.next_event().unwrap() {
                TimingEvent::SetReadOffset { token, .. } => {
                    cursor.acknowledge_offset(token).unwrap()
                }
                TimingEvent::Bus { token, expected } => {
                    observed += 1;
                    ack(&mut cursor, token, expected, 0);
                }
                TimingEvent::InstructionComplete {
                    token,
                    cycles: actual,
                    ..
                } => {
                    assert_eq!((observed, actual), (reads, cycles));
                    cursor
                        .complete_instruction(token, RustPathChoice { next_pc: next })
                        .unwrap();
                    break;
                }
                other => panic!("{other:?}"),
            }
        }
    }
    let (instruction, cpu) = plan(&[BNE_SHORT]);
    let mut scratch = TimingScratch::default();
    let mut cursor = TimingCursor::begin(&instruction, &cpu, &mut scratch).unwrap();
    loop {
        match cursor.next_event().unwrap() {
            TimingEvent::SetReadOffset { token, .. } => cursor.acknowledge_offset(token).unwrap(),
            TimingEvent::Bus { token, expected } => ack(&mut cursor, token, expected, 0),
            TimingEvent::InstructionComplete { token, .. } => {
                assert!(matches!(
                    cursor.complete_instruction(token, RustPathChoice { next_pc: 2 }),
                    Err(TimingError::WrongPath { .. })
                ));
                break;
            }
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn dropped_reordered_cross_cursor_tokens_and_bad_observations_fail_closed() {
    const MOVE_WORD_A0_D0: u16 = 0x3010;
    let (instruction, cpu) = plan(&[MOVE_WORD_A0_D0]);
    let mut scratch_a = TimingScratch::default();
    let mut scratch_b = TimingScratch::default();
    let mut a = TimingCursor::begin(&instruction, &cpu, &mut scratch_a).unwrap();
    let mut b = TimingCursor::begin(&instruction, &cpu, &mut scratch_b).unwrap();
    let (token_a, expected) = next_bus(&mut a);
    let (token_b, _) = next_bus(&mut b);
    assert!(matches!(
        a.next_event(),
        Err(TimingError::PendingAcknowledgement)
    ));
    let observation = BusObservation {
        address: expected.address,
        width: expected.width,
        kind: expected.kind,
        value: 0,
    };
    assert_eq!(
        a.acknowledge_bus(token_b, observation),
        Err(TimingError::WrongToken)
    );
    assert_eq!(
        a.acknowledge_bus(token_a, observation),
        Err(TimingError::WrongToken)
    );
    let mut scratch_c = TimingScratch::default();
    let mut c = TimingCursor::begin(&instruction, &cpu, &mut scratch_c).unwrap();
    let (token, _) = next_bus(&mut c);
    assert_eq!(
        c.acknowledge_bus(
            token,
            BusObservation {
                kind: AccessKind::Read(ReadKind::Semantic),
                ..observation
            }
        ),
        Err(TimingError::WrongObservation)
    );
}

#[test]
fn privilege_and_divide_zero_are_explicit_typed_exits() {
    const STOP: u16 = 0x4e72;
    let (instruction, mut cpu) = plan(&[STOP, 0x2700]);
    cpu.sr = 0;
    let mut scratch = TimingScratch::default();
    let mut cursor = TimingCursor::begin(&instruction, &cpu, &mut scratch).unwrap();
    let (token, fetch) = next_bus(&mut cursor);
    ack(&mut cursor, token, fetch, 0x2700);
    let TimingEvent::Exception { token, result } = cursor.next_event().unwrap() else {
        panic!("privilege exit");
    };
    assert_eq!((result.vector, result.cycles, result.saved_pc), (8, 34, 0));
    cursor.acknowledge_exception(token).unwrap();
    assert!(matches!(cursor.next_event(), Err(TimingError::Exhausted)));
    const DIVU_MEMORY_D0: u16 = 0x80d0;
    let (instruction, cpu) = plan(&[DIVU_MEMORY_D0]);
    let mut scratch = TimingScratch::default();
    let mut cursor = TimingCursor::begin(&instruction, &cpu, &mut scratch).unwrap();
    let (token, fetch) = next_bus(&mut cursor);
    ack(&mut cursor, token, fetch, 0);
    let (token, divisor) = next_bus(&mut cursor);
    ack(&mut cursor, token, divisor, 0);
    let TimingEvent::Exception { token, result } = cursor.next_event().unwrap() else {
        panic!("zero-divide exit");
    };
    assert_eq!((result.vector, result.cycles, result.saved_pc), (5, 42, 2));
    cursor.acknowledge_exception(token).unwrap();
}
