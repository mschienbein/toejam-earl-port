use super::*;

#[test]
fn cursor_generation_crosses_u32_without_hot_global_identity_or_stale_token_acceptance() {
    const NOP: u16 = 0x4e71;
    let rom = NOP
        .to_be_bytes()
        .into_iter()
        .chain([0; 8])
        .collect::<Vec<_>>();
    let instruction =
        TimingInstruction::compile(&crate::compile_instruction(&rom, 0).unwrap()).unwrap();
    let entry = CpuState {
        prefetch: u32::from(NOP),
        ..CpuState::default()
    };
    let mut scratch = TimingScratch::default();
    let owner = scratch.identity().unwrap();
    scratch.generation = u64::from(u32::MAX) - 1;
    let stale = {
        let mut first = TimingCursor::begin(&instruction, &entry, &mut scratch).unwrap();
        let TimingEvent::SetReadOffset { token, .. } = first.next_event().unwrap() else {
            panic!("phase");
        };
        token
    };
    let mut second = TimingCursor::begin(&instruction, &entry, &mut scratch).unwrap();
    assert_eq!(second.identity().scratch, owner);
    assert_eq!(second.identity().generation, u64::from(u32::MAX) + 1);
    let TimingEvent::SetReadOffset { token: current, .. } = second.next_event().unwrap() else {
        panic!("phase");
    };
    assert_eq!(
        second.acknowledge_offset(stale),
        Err(TimingError::WrongToken)
    );
    assert_eq!(
        second.acknowledge_offset(current),
        Err(TimingError::WrongToken),
        "protocol failure is sticky"
    );
}

#[test]
fn faulted_odd_or_wrong_prefetch_entry_never_issues_a_source_effect() {
    const NOP: u16 = 0x4e71;
    let rom = NOP
        .to_be_bytes()
        .into_iter()
        .chain([0; 8])
        .collect::<Vec<_>>();
    let instruction =
        TimingInstruction::compile(&crate::compile_instruction(&rom, 0).unwrap()).unwrap();
    for (pc, prefetch, address_error, expected) in [
        (0, u32::from(NOP), 1, TimingError::PendingFault),
        (1, u32::from(NOP), 0, TimingError::WrongEntry),
        (0, 0, 0, TimingError::WrongEntry),
    ] {
        let entry = CpuState {
            pc,
            prefetch,
            address_error,
            ..CpuState::default()
        };
        let original = entry.clone();
        let mut scratch = TimingScratch::default();
        assert!(
            matches!(TimingCursor::begin(&instruction, &entry, &mut scratch), Err(error) if error == expected)
        );
        assert_eq!(scratch.generation, 0);
        assert_eq!(entry, original);
    }
}
