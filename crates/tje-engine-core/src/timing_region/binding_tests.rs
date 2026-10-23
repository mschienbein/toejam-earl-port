use super::*;

#[test]
fn reviewed_prefetch_rejects_changed_extension_value_without_second_read() {
    const MOVE_IMMEDIATE_WORD_D0: u16 = 0x303c;
    const IMMEDIATE: u16 = 7;
    let words = [MOVE_IMMEDIATE_WORD_D0, IMMEDIATE, 0x4e71];
    let rom: Vec<_> = words.iter().flat_map(|w| w.to_be_bytes()).collect();
    let ir = crate::compile_instruction(&rom, 0).unwrap();
    let mut instruction = TimingInstruction::compile(&ir).unwrap();
    let binding = std::sync::Arc::new(std::collections::BTreeMap::from([
        (0, MOVE_IMMEDIATE_WORD_D0),
        (2, IMMEDIATE),
        (4, 0x4e71),
    ]));
    instruction.bind_prefetch_words(binding).unwrap();
    let cpu = CpuState {
        prefetch: u32::from(MOVE_IMMEDIATE_WORD_D0),
        ..CpuState::default()
    };
    let mut scratch = TimingScratch::default();
    let mut cursor = TimingCursor::begin(&instruction, &cpu, &mut scratch).unwrap();
    let TimingEvent::SetReadOffset { token, .. } = cursor.next_event().unwrap() else {
        panic!("phase");
    };
    cursor.acknowledge_offset(token).unwrap();
    let TimingEvent::Bus { token, expected } = cursor.next_event().unwrap() else {
        panic!("prefetch");
    };
    assert_eq!(expected.address, 2);
    assert_eq!(
        cursor.acknowledge_bus(
            token,
            BusObservation {
                address: 2,
                width: Width::Word,
                kind: AccessKind::Read(ReadKind::Prefetch),
                value: 8
            }
        ),
        Err(TimingError::WrongObservation)
    );
    assert!(matches!(
        cursor.next_event(),
        Err(TimingError::PendingAcknowledgement)
    ));
}
