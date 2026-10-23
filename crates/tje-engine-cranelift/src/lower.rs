//! Direct SSA lowering. No decoded instruction or semantic executor is called by
//! generated code: the only calls are ordered bus operations through the ABI table.
use crate::{
    Error, Result,
    cache::{Artifact, CODE_ALIGNMENT, Entry},
};
use cranelift_codegen::{
    Context,
    ir::{self, AbiParam, InstBuilder, MemFlagsData, Signature, types},
    isa::TargetIsa,
};
use cranelift_control::ControlPlane;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use std::mem::offset_of;
use tje_engine_core::{
    InstructionIr, Program,
    ir::{BinaryOp, CompareOp, ExceptionExit, Op, Value, Width},
};
use tje_engine_runtime::abi::BusCallbacks as Callbacks;

pub(crate) fn compile(
    program: &Program,
    isa: &dyn TargetIsa,
    mut progress: impl FnMut(usize, usize),
) -> Result<Artifact> {
    let mut context = Context::new();
    let mut frontend = FunctionBuilderContext::new();
    let mut control = ControlPlane::default();
    let mut entries = Vec::with_capacity(program.instructions.len());
    let mut code = Vec::new();
    progress(0, program.instructions.len());
    for (index, instruction) in program.instructions.iter().enumerate() {
        context.clear();
        context.func.signature = signature(isa);
        build(instruction, &mut context.func, &mut frontend, isa)?;
        let compiled = context
            .compile(isa, &mut control)
            .map_err(|error| Error::Compile {
                pc: instruction.pc,
                message: format!("{:?}", error.inner),
            })?;
        if !compiled.buffer.relocs().is_empty() {
            return Err(Error::Compile {
                pc: instruction.pc,
                message: "continuation unexpectedly requires external relocations".into(),
            });
        }
        let aligned = code.len().next_multiple_of(CODE_ALIGNMENT);
        code.resize(aligned, 0);
        entries.push(Entry {
            pc: instruction.pc,
            offset: u32::try_from(code.len()).map_err(|_| Error::CodeTooLarge)?,
            length: u32::try_from(compiled.buffer.data().len()).map_err(|_| Error::CodeTooLarge)?,
        });
        code.extend_from_slice(compiled.buffer.data());
        if (index + 1).is_multiple_of(128) || index + 1 == program.instructions.len() {
            progress(index + 1, program.instructions.len());
        }
    }
    Ok(Artifact { entries, code })
}

fn signature(isa: &dyn TargetIsa) -> Signature {
    let pointer = isa.pointer_type();
    let mut signature = Signature::new(isa.default_call_conv());
    // state, opaque bus context, callback table, exception detail output
    signature.params.extend([AbiParam::new(pointer); 4]);
    signature.returns.push(AbiParam::new(types::I64));
    signature
}

fn build(
    instruction: &InstructionIr,
    function: &mut ir::Function,
    frontend: &mut FunctionBuilderContext,
    isa: &dyn TargetIsa,
) -> Result<()> {
    if instruction.value_count as usize > instruction.ops.len() {
        return Err(Error::InvalidIr {
            pc: instruction.pc,
            message: "value_count exceeds operation count",
        });
    }
    let pointer_type = isa.pointer_type();
    let mut builder = FunctionBuilder::new(function, frontend);
    let entry = builder.create_block();
    builder.append_block_params_for_function_params(entry);
    builder.switch_to_block(entry);
    builder.seal_block(entry);
    let arguments = builder.block_params(entry).to_vec();
    let mut lowering = Lowering {
        builder,
        values: vec![None; instruction.value_count as usize],
        state: arguments[0],
        bus: arguments[1],
        callbacks: arguments[2],
        exception: arguments[3],
        pointer_type,
        call_conv: isa.default_call_conv(),
        pc: instruction.pc,
    };
    let mut finished = false;
    for operation in &instruction.ops {
        if finished {
            return Err(lowering.invalid("operations after Finish"));
        }
        match *operation {
            Op::Constant { dst, value } => {
                let result = lowering.builder.ins().iconst(types::I32, i64::from(value));
                lowering.define(dst, result)?;
            }
            Op::LoadState { dst, field } => {
                let offset = state_offset(field).ok_or_else(|| lowering.invalid("state field"))?;
                let result = lowering.builder.ins().load(
                    types::I32,
                    MemFlagsData::trusted(),
                    lowering.state,
                    offset,
                );
                lowering.define(dst, result)?;
            }
            Op::StoreState { field, value } => {
                let value = lowering.value(value)?;
                let offset = state_offset(field).ok_or_else(|| lowering.invalid("state field"))?;
                lowering.builder.ins().store(
                    MemFlagsData::trusted(),
                    value,
                    lowering.state,
                    offset,
                );
            }
            Op::Binary { dst, op, lhs, rhs } => {
                let lhs = lowering.value(lhs)?;
                let rhs = lowering.value(rhs)?;
                let result = lowering.binary(op, lhs, rhs);
                lowering.define(dst, result)?;
            }
            Op::Compare { dst, op, lhs, rhs } => {
                let lhs = lowering.value(lhs)?;
                let rhs = lowering.value(rhs)?;
                let condition = match op {
                    CompareOp::Equal => ir::condcodes::IntCC::Equal,
                    CompareOp::NotEqual => ir::condcodes::IntCC::NotEqual,
                    CompareOp::UnsignedLess => ir::condcodes::IntCC::UnsignedLessThan,
                    CompareOp::UnsignedLessEqual => ir::condcodes::IntCC::UnsignedLessThanOrEqual,
                    CompareOp::SignedLess => ir::condcodes::IntCC::SignedLessThan,
                    CompareOp::SignedLessEqual => ir::condcodes::IntCC::SignedLessThanOrEqual,
                };
                let result = lowering.builder.ins().icmp(condition, lhs, rhs);
                let result = lowering.builder.ins().uextend(types::I32, result);
                lowering.define(dst, result)?;
            }
            Op::Select {
                dst,
                condition,
                if_true,
                if_false,
            } => {
                let condition = lowering.value(condition)?;
                let if_true = lowering.value(if_true)?;
                let if_false = lowering.value(if_false)?;
                let result = lowering.builder.ins().select(condition, if_true, if_false);
                lowering.define(dst, result)?;
            }
            Op::ReadMemory {
                dst,
                address,
                width,
                kind,
            } => {
                let address = lowering.value(address)?;
                let kind = lowering.builder.ins().iconst(types::I32, kind as i64);
                let offset = match width {
                    Width::Byte => offset_of!(Callbacks, read8),
                    Width::Word => offset_of!(Callbacks, read16),
                };
                let result = lowering
                    .call(offset, &[address, kind], true)
                    .expect("read returns value");
                lowering.define(dst, result)?;
            }
            Op::ReadMemoryIf {
                dst,
                condition,
                address,
                width,
                kind,
            } => {
                let condition = lowering.value(condition)?;
                let address = lowering.value(address)?;
                let read_block = lowering.builder.create_block();
                let merge_block = lowering.builder.create_block();
                lowering.builder.append_block_param(merge_block, types::I32);
                let zero = lowering.builder.ins().iconst(types::I32, 0);
                lowering.builder.ins().brif(
                    condition,
                    read_block,
                    &[],
                    merge_block,
                    &[zero.into()],
                );
                lowering.builder.switch_to_block(read_block);
                lowering.builder.seal_block(read_block);
                let kind = lowering.builder.ins().iconst(types::I32, kind as i64);
                let offset = match width {
                    Width::Byte => offset_of!(Callbacks, read8),
                    Width::Word => offset_of!(Callbacks, read16),
                };
                let value = lowering
                    .call(offset, &[address, kind], true)
                    .expect("read returns value");
                lowering.builder.ins().jump(merge_block, &[value.into()]);
                lowering.builder.switch_to_block(merge_block);
                lowering.builder.seal_block(merge_block);
                let result = lowering.builder.block_params(merge_block)[0];
                lowering.define(dst, result)?;
            }
            Op::WriteMemory {
                address,
                value,
                width,
            } => {
                let address = lowering.value(address)?;
                let value = lowering.value(value)?;
                let offset = match width {
                    Width::Byte => offset_of!(Callbacks, write8),
                    Width::Word => offset_of!(Callbacks, write16),
                };
                lowering.call(offset, &[address, value], false);
            }
            Op::SetReadOffset { cycles } => {
                let cycles = lowering.value(cycles)?;
                lowering.call(
                    offset_of!(Callbacks, set_read_offset_mclk),
                    &[cycles],
                    false,
                );
            }
            Op::ExitIf { condition, exit } => {
                let condition = lowering.value(condition)?;
                let exception_block = lowering.builder.create_block();
                let continuation = lowering.builder.create_block();
                lowering
                    .builder
                    .ins()
                    .brif(condition, exception_block, &[], continuation, &[]);
                lowering.builder.switch_to_block(exception_block);
                lowering.builder.seal_block(exception_block);
                lowering.exit(exit)?;
                lowering.builder.switch_to_block(continuation);
                lowering.builder.seal_block(continuation);
            }
            Op::Finish { cycles } => {
                let cycles = lowering.value(cycles)?;
                let result = lowering.builder.ins().uextend(types::I64, cycles);
                lowering.builder.ins().return_(&[result]);
                finished = true;
            }
        }
    }
    if !finished {
        return Err(lowering.invalid("missing Finish"));
    }
    lowering.builder.finalize(isa.frontend_config());
    Ok(())
}

fn state_offset(field: tje_engine_core::ir::StateField) -> Option<i32> {
    use tje_engine_core::ir::StateField;
    if matches!(field, StateField::Data(n) if n >= 8)
        || matches!(field, StateField::Address(n) if n >= 7)
    {
        return None;
    }
    Some(field.offset() as i32)
}

struct Lowering<'a> {
    builder: FunctionBuilder<'a>,
    values: Vec<Option<ir::Value>>,
    state: ir::Value,
    bus: ir::Value,
    callbacks: ir::Value,
    exception: ir::Value,
    pointer_type: ir::Type,
    call_conv: cranelift_codegen::isa::CallConv,
    pc: u32,
}

impl Lowering<'_> {
    fn invalid(&self, message: &'static str) -> Error {
        Error::InvalidIr {
            pc: self.pc,
            message,
        }
    }

    fn value(&self, value: Value) -> Result<ir::Value> {
        self.values
            .get(value.0 as usize)
            .copied()
            .flatten()
            .ok_or_else(|| self.invalid("SSA use before definition or outside value_count"))
    }

    fn define(&mut self, value: Value, result: ir::Value) -> Result<()> {
        let Some(slot) = self.values.get_mut(value.0 as usize) else {
            return Err(self.invalid("SSA definition outside value_count"));
        };
        if slot.replace(result).is_some() {
            return Err(self.invalid("duplicate SSA definition"));
        }
        Ok(())
    }

    fn call(&mut self, offset: usize, operands: &[ir::Value], returns: bool) -> Option<ir::Value> {
        let callee = self.builder.ins().load(
            self.pointer_type,
            MemFlagsData::trusted(),
            self.callbacks,
            offset as i32,
        );
        let mut signature = Signature::new(self.call_conv);
        signature.params.push(AbiParam::new(self.pointer_type));
        signature
            .params
            .extend(operands.iter().map(|_| AbiParam::new(types::I32)));
        if returns {
            signature.returns.push(AbiParam::new(types::I32));
        }
        let signature = self.builder.import_signature(signature);
        // There are at most two u32 operands. Keep the hot callback path allocation-free.
        let mut arguments = [self.bus; 3];
        arguments[1..1 + operands.len()].copy_from_slice(operands);
        let call =
            self.builder
                .ins()
                .call_indirect(signature, callee, &arguments[..1 + operands.len()]);
        returns.then(|| self.builder.inst_results(call)[0])
    }

    fn binary(&mut self, op: BinaryOp, lhs: ir::Value, rhs: ir::Value) -> ir::Value {
        match op {
            BinaryOp::Add => self.builder.ins().iadd(lhs, rhs),
            BinaryOp::Subtract => self.builder.ins().isub(lhs, rhs),
            BinaryOp::Multiply => self.builder.ins().imul(lhs, rhs),
            BinaryOp::And => self.builder.ins().band(lhs, rhs),
            BinaryOp::Or => self.builder.ins().bor(lhs, rhs),
            BinaryOp::Xor => self.builder.ins().bxor(lhs, rhs),
            BinaryOp::ShiftLeft | BinaryOp::ShiftRight | BinaryOp::ArithmeticShiftRight => {
                let count = self.builder.ins().band_imm_u(rhs, 31);
                match op {
                    BinaryOp::ShiftLeft => self.builder.ins().ishl(lhs, count),
                    BinaryOp::ShiftRight => self.builder.ins().ushr(lhs, count),
                    BinaryOp::ArithmeticShiftRight => self.builder.ins().sshr(lhs, count),
                    _ => unreachable!(),
                }
            }
            BinaryOp::DivideUnsigned => self.builder.ins().udiv(lhs, rhs),
            BinaryOp::RemainderUnsigned => self.builder.ins().urem(lhs, rhs),
            BinaryOp::DivideSigned | BinaryOp::RemainderSigned => {
                // CLIF signed division traps at MIN/-1. The IR deliberately wraps,
                // so make the hardware operation safe before selecting the result.
                let minimum = self.builder.ins().icmp_imm_s(
                    ir::condcodes::IntCC::Equal,
                    lhs,
                    i64::from(i32::MIN),
                );
                let minus_one = self
                    .builder
                    .ins()
                    .icmp_imm_s(ir::condcodes::IntCC::Equal, rhs, -1);
                let overflow = self.builder.ins().band(minimum, minus_one);
                let zero = self.builder.ins().iconst(types::I32, 0);
                let safe_lhs = self.builder.ins().select(overflow, zero, lhs);
                if op == BinaryOp::DivideSigned {
                    let quotient = self.builder.ins().sdiv(safe_lhs, rhs);
                    let wrapped = self.builder.ins().iconst(types::I32, i64::from(i32::MIN));
                    self.builder.ins().select(overflow, wrapped, quotient)
                } else {
                    self.builder.ins().srem(safe_lhs, rhs)
                }
            }
        }
    }

    fn exit(&mut self, exit: ExceptionExit) -> Result<()> {
        let cycles = self.value(exit.cycles)?;
        for (offset, value) in [
            (0, exit.saved_pc),
            (4, exit.fault_address),
            (8, exit.fault_status),
        ] {
            let value = self.value(value)?;
            self.builder
                .ins()
                .store(MemFlagsData::trusted(), value, self.exception, offset);
        }
        let cycles = self.builder.ins().uextend(types::I64, cycles);
        let result = self
            .builder
            .ins()
            .bor_imm_u(cycles, i64::from(exit.vector) << 32);
        self.builder.ins().return_(&[result]);
        Ok(())
    }
}
