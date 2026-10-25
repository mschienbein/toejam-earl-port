//! Positive provenance for a read in the actual original root invocation.
//! Once control may have entered another frame, no parent restoration is inferred.
use m68000_emu::M68000;
use std::sync::atomic::{AtomicBool, Ordering};
use tje_game::FunctionId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OriginalRootWitness {
    pub(crate) function: FunctionId,
    pub(crate) entry_pc: u32,
    pub(crate) entry_sp: u32,
}
#[derive(Debug)]
pub(crate) struct OriginalRootScope {
    witness: OriginalRootWitness,
    exception_depth: u32,
    supervisor: bool,
    valid: AtomicBool,
}
impl OriginalRootScope {
    pub(crate) fn new(cpu: &M68000, function: FunctionId, entry_pc: u32, entry_sp: u32) -> Self {
        Self {
            witness: OriginalRootWitness {
                function,
                entry_pc,
                entry_sp,
            },
            exception_depth: cpu.hybrid_exception_depth(),
            supervisor: cpu.status_register() & 0x2000 != 0,
            valid: AtomicBool::new(cpu.pc() == entry_pc && cpu.stack_pointer() == entry_sp),
        }
    }
    pub(crate) fn before_step(&self, cpu: &M68000, ready: bool) {
        self.after_step(cpu);
        if !ready || !self.valid.load(Ordering::Relaxed) {
            return;
        }
        let opcode = cpu.next_opcode();
        if opcode & 0xffc0 == 0x4e80 // JSR, every addressing mode
            || opcode & 0xffc0 == 0x4ec0 // JMP, including tail/shared entries
            || opcode & 0xff00 == 0x6100 // BSR, both displacements
            || opcode & 0xfff0 == 0x4e40 // TRAP
            || matches!(opcode, 0x4e70 | 0x4e72..=0x4e77 | 0x4afc)
            || matches!(opcode & 0xf000, 0xa000 | 0xf000)
        {
            self.valid.store(false, Ordering::Relaxed);
        }
    }
    pub(crate) fn after_step(&self, cpu: &M68000) {
        if cpu.hybrid_exception_depth() != self.exception_depth
            || (cpu.status_register() & 0x2000 != 0) != self.supervisor
        {
            self.valid.store(false, Ordering::Relaxed);
        }
    }
    pub(crate) fn witness(&self) -> Option<OriginalRootWitness> {
        self.valid.load(Ordering::Relaxed).then_some(self.witness)
    }
}
