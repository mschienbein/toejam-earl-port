//! Development-only IR oracle. This is not linked into either shipped frontend.
//! Every memory operation stays ordered, including prefetch and discarded reads.

use anyhow::{Result, bail};
use tje_engine_core::{
    Bus, CpuState, InstructionIr, StepResult,
    ir::{BinaryOp, CompareOp, Op, StateField, Value, Width},
};

/// Reuses the SSA value buffer across instructions; no per-step allocation after
/// the largest previously evaluated instruction.
#[derive(Debug, Default)]
pub struct Interpreter {
    values: Vec<u32>,
}

impl Interpreter {
    pub fn evaluate(
        &mut self,
        instruction: &InstructionIr,
        state: &mut CpuState,
        bus: &mut (impl Bus + ?Sized),
    ) -> Result<StepResult> {
        self.values.resize(instruction.value_count as usize, 0);
        for op in &instruction.ops {
            let get = |value: Value| self.values[value.0 as usize];
            let (dst, result) = match *op {
                Op::Constant { dst, value } => (dst, value),
                Op::LoadState { dst, field } => (dst, load(state, field)),
                Op::StoreState { field, value } => {
                    store(state, field, get(value));
                    continue;
                }
                Op::Binary { dst, op, lhs, rhs } => {
                    let (lhs, rhs) = (get(lhs), get(rhs));
                    let result = match op {
                        BinaryOp::Add => lhs.wrapping_add(rhs),
                        BinaryOp::Subtract => lhs.wrapping_sub(rhs),
                        BinaryOp::Multiply => lhs.wrapping_mul(rhs),
                        BinaryOp::And => lhs & rhs,
                        BinaryOp::Or => lhs | rhs,
                        BinaryOp::Xor => lhs ^ rhs,
                        BinaryOp::ShiftLeft => lhs.wrapping_shl(rhs),
                        BinaryOp::ShiftRight => lhs.wrapping_shr(rhs),
                        BinaryOp::ArithmeticShiftRight => (lhs as i32).wrapping_shr(rhs) as u32,
                        BinaryOp::DivideUnsigned
                        | BinaryOp::RemainderUnsigned
                        | BinaryOp::DivideSigned
                        | BinaryOp::RemainderSigned
                            if rhs == 0 =>
                        {
                            bail!("unguarded IR division by zero at {:06X}", instruction.pc);
                        }
                        BinaryOp::DivideUnsigned => lhs / rhs,
                        BinaryOp::DivideSigned => (lhs as i32).wrapping_div(rhs as i32) as u32,
                        BinaryOp::RemainderUnsigned => lhs % rhs,
                        BinaryOp::RemainderSigned => (lhs as i32).wrapping_rem(rhs as i32) as u32,
                    };
                    (dst, result)
                }
                Op::Compare { dst, op, lhs, rhs } => {
                    let (lhs, rhs) = (get(lhs), get(rhs));
                    let result = match op {
                        CompareOp::Equal => lhs == rhs,
                        CompareOp::NotEqual => lhs != rhs,
                        CompareOp::UnsignedLess => lhs < rhs,
                        CompareOp::UnsignedLessEqual => lhs <= rhs,
                        CompareOp::SignedLess => (lhs as i32) < (rhs as i32),
                        CompareOp::SignedLessEqual => (lhs as i32) <= (rhs as i32),
                    };
                    (dst, u32::from(result))
                }
                Op::Select {
                    dst,
                    condition,
                    if_true,
                    if_false,
                } => (
                    dst,
                    get(if get(condition) != 0 {
                        if_true
                    } else {
                        if_false
                    }),
                ),
                Op::ReadMemory {
                    dst,
                    address,
                    width,
                    kind,
                } => {
                    let address = get(address);
                    let result = match width {
                        Width::Byte => u32::from(bus.read8(address, kind)),
                        Width::Word => u32::from(bus.read16(address, kind)),
                    };
                    (dst, result)
                }
                Op::ReadMemoryIf {
                    dst,
                    condition,
                    address,
                    width,
                    kind,
                } => {
                    let result = if get(condition) == 0 {
                        0
                    } else {
                        let address = get(address);
                        match width {
                            Width::Byte => u32::from(bus.read8(address, kind)),
                            Width::Word => u32::from(bus.read16(address, kind)),
                        }
                    };
                    (dst, result)
                }
                Op::WriteMemory {
                    address,
                    value,
                    width,
                } => {
                    match width {
                        Width::Byte => bus.write8(get(address), get(value) as u8),
                        Width::Word => bus.write16(get(address), get(value) as u16),
                    }
                    continue;
                }
                Op::SetReadOffset { cycles } => {
                    bus.set_read_offset_mclk(get(cycles));
                    continue;
                }
                Op::ExitIf { condition, exit } => {
                    if get(condition) != 0 {
                        return Ok(StepResult {
                            cycles: get(exit.cycles),
                            vector: exit.vector,
                            saved_pc: get(exit.saved_pc),
                            fault_address: get(exit.fault_address),
                            fault_status: get(exit.fault_status),
                        });
                    }
                    continue;
                }
                Op::Finish { cycles } => return Ok(StepResult::normal(get(cycles))),
            };
            self.values[dst.0 as usize] = result;
        }
        bail!(
            "IR instruction at {:06X} has no executed exit",
            instruction.pc
        )
    }
}

fn load(state: &CpuState, field: StateField) -> u32 {
    match field {
        StateField::Data(n) => state.d[usize::from(n)],
        StateField::Address(n) => state.a[usize::from(n)],
        StateField::Usp => state.usp,
        StateField::Ssp => state.ssp,
        StateField::Pc => state.pc,
        StateField::Sr => state.sr,
        StateField::Prefetch => state.prefetch,
        StateField::PendingInterrupt => state.pending_interrupt,
        StateField::Stopped => state.stopped,
        StateField::Frozen => state.frozen,
        StateField::AddressError => state.address_error,
        StateField::LastMulDiv => state.last_muldiv,
    }
}

fn store(state: &mut CpuState, field: StateField, value: u32) {
    match field {
        StateField::Data(n) => state.d[usize::from(n)] = value,
        StateField::Address(n) => state.a[usize::from(n)] = value,
        StateField::Usp => state.usp = value,
        StateField::Ssp => state.ssp = value,
        StateField::Pc => state.pc = value,
        StateField::Sr => state.sr = value,
        StateField::Prefetch => state.prefetch = value,
        StateField::PendingInterrupt => state.pending_interrupt = value,
        StateField::Stopped => state.stopped = value,
        StateField::Frozen => state.frozen = value,
        StateField::AddressError => state.address_error = value,
        StateField::LastMulDiv => state.last_muldiv = value,
    }
}
