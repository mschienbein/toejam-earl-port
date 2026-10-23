//! ROM-free sanity: both references independently boot this authored program.
use tje_engine_verify::{
    InstructionMachine, Machine, reference::ReferenceMachine, verify_instruction_session_with,
    verify_session, verify_session_with,
};
use tje_rom::Revision;
use tje_session::{CONNECTED, Session};

fn authored_cartridge() -> Vec<u8> {
    let mut bytes = vec![0; 0x200];
    bytes[..4].copy_from_slice(&0x00ff8000_u32.to_be_bytes());
    bytes[4..8].copy_from_slice(&0x00000100_u32.to_be_bytes());
    // MOVE.B #$90,$C00011; NOP; BRA back to the PSG write. These are
    // independently authored ISA words, not a retail-ROM excerpt.
    let words = [0x13fc_u16, 0x0090, 0x00c0, 0x0011, 0x4e71, 0x60f4];
    for (index, word) in words.into_iter().enumerate() {
        bytes[0x100 + 2 * index..0x102 + 2 * index].copy_from_slice(&word.to_be_bytes());
    }
    bytes
}

#[test]
fn independent_references_match_state_pixels_audio_and_timestamped_writes() {
    let bytes = authored_cartridge();
    let mut session = Session::new(Revision::Rev02, "authored ROM-free reference sanity");
    session.record([CONNECTED, 0]).unwrap();
    session.record([CONNECTED | 0x80, CONNECTED]).unwrap();
    let report = verify_session_with(&session, || {
        Ok((
            ReferenceMachine::from_synthetic_rom(bytes.clone()),
            ReferenceMachine::from_synthetic_rom(bytes.clone()),
        ))
    })
    .unwrap();
    assert!(report.passed());
    assert_eq!(report.frames_compared, 2);
    assert!(report.coverage.instructions_executed > 0);
    assert_eq!(report.coverage.instructions.len(), 3);
}

#[test]
fn reference_snapshot_has_real_nonempty_outputs() {
    let mut reference = ReferenceMachine::from_synthetic_rom(authored_cartridge());
    reference.begin_frame();
    loop {
        if reference.step([CONNECTED, 0]).unwrap().frame_complete {
            break;
        }
    }
    let state = reference.capture().unwrap();
    assert_eq!(state.work_ram.len(), 65536);
    assert_eq!(state.vram.len(), 65536);
    assert_eq!(state.z80_ram.len(), 8192);
    assert!(!state.vdp_registers.is_empty());
    assert!(!state.vdp_state.is_empty());
    assert!(!state.pixels.is_empty());
    assert!(!state.audio_bits.is_empty());
    assert!(!state.chip_writes.is_empty());
    assert!(
        state
            .chip_writes
            .windows(2)
            .all(|pair| pair[0].master_clock <= pair[1].master_clock)
    );
}

#[test]
fn independent_references_match_every_instruction_and_actual_ram_commit() {
    let mut bytes = authored_cartridge();
    // Authored MOVE.W #$1234,$FF0010 followed by PSG write/NOP/BRA.
    let words = [
        0x33fc_u16, 0x1234, 0x00ff, 0x0010, 0x13fc, 0x0090, 0x00c0, 0x0011, 0x4e71, 0x60ec,
    ];
    for (index, word) in words.into_iter().enumerate() {
        bytes[0x100 + 2 * index..0x102 + 2 * index].copy_from_slice(&word.to_be_bytes());
    }
    let mut session = Session::new(
        Revision::Rev02,
        "authored instruction-by-instruction reference sanity",
    );
    session.record([CONNECTED, 0]).unwrap();
    let report = verify_instruction_session_with(&session, || {
        Ok((
            ReferenceMachine::from_synthetic_rom(bytes.clone()),
            ReferenceMachine::from_synthetic_rom(bytes.clone()),
        ))
    })
    .unwrap();
    assert!(report.passed());
    let proof = report.instruction_proof.unwrap();
    assert_eq!(proof.scheduler_steps_compared, report.scheduler_steps);
    assert_eq!(
        proof.instructions_compared,
        report.coverage.instructions_executed
    );
    assert!(proof.initial_state_compared);
    assert!(proof.bus_accesses_compared > proof.instructions_compared);
    assert!(proof.ram_writes_compared > 0);
}

#[test]
fn optional_instruction_observation_does_not_change_reference_outputs() {
    let bytes = authored_cartridge();
    let mut reference = ReferenceMachine::from_synthetic_rom(bytes.clone());
    let mut observed = ReferenceMachine::from_synthetic_rom(bytes);
    observed.enable_instruction_proof();
    let mut session = Session::new(Revision::Rev02, "authored observer-invariance sanity");
    session.record([CONNECTED, 0]).unwrap();
    assert!(
        verify_session(&session, &mut reference, &mut observed)
            .unwrap()
            .passed()
    );
}
