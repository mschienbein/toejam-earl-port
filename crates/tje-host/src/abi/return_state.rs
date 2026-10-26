//! ABI-visible values only; these immutable observations cannot restore a CPU.
use m68000_emu::M68000;

#[derive(Debug, Clone, Copy)]
pub(crate) struct ReturnState {
    pub data: [u32; 8],
    pub address: [u32; 7],
    pub usp: u32,
    pub ssp: u32,
    pub pc: u32,
    pub status: u16,
    pub exception_depth: u32,
}
impl ReturnState {
    pub fn capture(cpu: &M68000) -> Self {
        Self {
            data: cpu.data_registers(),
            address: cpu.address_registers(),
            usp: cpu.user_stack_pointer(),
            ssp: cpu.supervisor_stack_pointer(),
            pc: cpu.pc(),
            status: cpu.status_register(),
            exception_depth: cpu.hybrid_exception_depth(),
        }
    }
    pub fn sp(&self) -> u32 {
        if self.status & 0x2000 == 0 {
            self.usp
        } else {
            self.ssp
        }
    }
    pub fn compare(&self, actual: &Self, abi: &tje_game::Abi) -> Result<(), String> {
        if self.pc != actual.pc || self.sp() != actual.sp() {
            return Err(format!(
                "return PC/SP: original={:08X}/{:08X}, Rust={:08X}/{:08X}",
                self.pc,
                self.sp(),
                actual.pc,
                actual.sp()
            ));
        }
        if self.usp != actual.usp || self.ssp != actual.ssp {
            return Err("banked USP/SSP state differs".into());
        }
        if self.exception_depth != actual.exception_depth {
            return Err("accepted exception nesting depth differs".into());
        }
        if abi.returns_value && self.data[0] != actual.data[0] {
            return Err(format!(
                "D0: original={:08X}, Rust={:08X}",
                self.data[0], actual.data[0]
            ));
        }
        for index in 0..8 {
            if (abi.strict_registers || abi.preserved_data & (1 << index) != 0)
                && self.data[index] != actual.data[index]
            {
                return Err(format!(
                    "ABI-preserved D{index}: original={:08X}, Rust={:08X}",
                    self.data[index], actual.data[index]
                ));
            }
        }
        for index in 0..7 {
            if (abi.strict_registers || abi.preserved_address & (1 << index) != 0)
                && self.address[index] != actual.address[index]
            {
                return Err(format!("ABI-preserved A{index} mismatch"));
            }
        }
        let sr_mask = if abi.strict_registers { 0xffff } else { 0xffe0 };
        if self.status & sr_mask != actual.status & sr_mask {
            return Err(format!(
                "SR control bits: original={:04X}, Rust={:04X}",
                self.status, actual.status
            ));
        }
        Ok(())
    }
}
