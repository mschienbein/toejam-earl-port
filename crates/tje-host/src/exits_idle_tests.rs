//! Independently authored idle-loop shape; not cartridge instruction bytes.
use super::*;

const COUNTER: u32 = 0xff8010;
const BEGIN: u32 = CODE + 24;
const DRAW: u32 = CODE + 40;

fn idle_machine(revision: Revision, counter_changed: bool) -> GenesisEmulator {
    // CMP.W counter,D3; BNE.S begin; JSR draw; BRA.S compare.
    let mut emulator = machine(
        revision,
        &[
            0xb679,
            (COUNTER >> 16) as u16,
            COUNTER as u16,
            0x6610,
            0x4eb9,
            (DRAW >> 16) as u16,
            DRAW as u16,
            0x60f0,
        ],
    );
    let mut context = HookContext::new(&mut emulator, revision, Dispatcher::hybrid_with(&[]));
    context.write_u16(COUNTER, if counter_changed { 8 } else { 7 });
    context.set_data_register(3, 7);
    context.write_u16(BEGIN, 0x4e71);
    context.write_u16(BEGIN + 2, 0x4e71);
    // The independently authored draw increments D0's low word and returns.
    context.write_u16(DRAW, 0x5240);
    context.write_u16(DRAW + 2, 0x4e75);
    emulator
}

#[test]
fn a_recurrent_idle_checkpoint_requires_one_comparison_and_one_draw_not_a_whole_work_iteration() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut emulator = idle_machine(revision, false);
        let before = emulator.hybrid_cpu().data_registers()[0];
        let mut boundary = ReferenceBoundary::returned(CODE, STACK);
        boundary.min_instructions = 1;
        let audit = run_original_atomic_until(&mut emulator, boundary, true).unwrap();
        assert_eq!(audit.instructions, 6);
        assert!(audit.uninitialized_reads.is_empty());
        assert_eq!(emulator.hybrid_cpu().pc(), CODE);
        assert_eq!(emulator.hybrid_cpu().stack_pointer(), STACK);
        assert_eq!(emulator.hybrid_cpu().data_registers()[0], before + 1);
    }
}

#[test]
fn changed_counter_leaves_idle_without_drawing_a_random_number() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut emulator = idle_machine(revision, true);
        let before = emulator.hybrid_cpu().data_registers()[0];
        let mut boundary = ReferenceBoundary::returned(BEGIN, STACK);
        boundary.min_instructions = 1;
        let audit = run_original_atomic_until(&mut emulator, boundary, true).unwrap();
        assert_eq!(audit.instructions, 2);
        assert_eq!(emulator.hybrid_cpu().pc(), BEGIN);
        assert_eq!(emulator.hybrid_cpu().data_registers()[0], before);
        assert_eq!(emulator.hybrid_cpu().stack_pointer(), STACK);
    }
}
