//! Test-only interpreter backend; never a fallback for generated native code.
use crate::{Catalogue, interpreter::Interpreter};
use anyhow::{Context, Result, ensure};
use std::collections::HashMap;
use tje_engine_core::{Bus, CpuState, InstructionIr, StepResult, compile_instruction};
use tje_engine_runtime::Backend;
use tje_rom::Rom;

pub struct IrBackend {
    rom: Vec<u8>,
    catalogue: Catalogue,
    instructions: HashMap<u32, InstructionIr>,
    interpreter: Interpreter,
}

impl IrBackend {
    pub fn new(rom: &Rom) -> Self {
        Self {
            rom: rom.bytes().to_vec(),
            catalogue: Catalogue::committed(rom.revision()),
            instructions: HashMap::new(),
            interpreter: Interpreter::default(),
        }
    }
}

impl Backend for IrBackend {
    type Error = anyhow::Error;
    fn execute(&mut self, cpu: &mut CpuState, bus: &mut dyn Bus) -> Result<StepResult> {
        let pc = cpu.pc;
        if let std::collections::hash_map::Entry::Vacant(entry) = self.instructions.entry(pc) {
            ensure!(
                self.catalogue.function_at(pc).is_some(),
                "IR oracle PC {pc:06X} is outside committed function metadata"
            );
            let instruction = compile_instruction(&self.rom, pc).with_context(|| {
                format!(
                    "lowering IR oracle PC {pc:06X} ({})",
                    self.catalogue.name(pc)
                )
            })?;
            entry.insert(instruction);
        }
        self.interpreter.evaluate(&self.instructions[&pc], cpu, bus)
    }
}
