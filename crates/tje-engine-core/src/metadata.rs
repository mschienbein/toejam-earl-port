//! Reachability is constrained by committed source metadata, never a ROM scan.
//! Table locations and shapes are metadata; their offsets/targets are read only
//! from the user's ROM. No decoded target or instruction table is distributed.
mod switches;
use crate::{
    Program,
    decode::{AddressingMode as Ea, BranchCondition, Instruction as I, decode},
    lower::{CompileError, compile_instruction_with_divider},
};
use std::collections::{BTreeMap, BTreeSet};
use tje_mem::Revision;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodeRange {
    pub start: u32,
    pub end: u32,
}
impl CodeRange {
    pub fn contains(self, pc: u32) -> bool {
        self.start <= pc && pc < self.end
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
    pub name: &'static str,
    pub range: CodeRange,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JumpTable {
    pub jump_pc: u32,
    pub address: u32,
    pub count: u16,
    pub relative_base: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProgramMetadata {
    pub functions: Vec<Function>,
    pub code_ranges: Vec<CodeRange>,
    pub entry_points: Vec<u32>,
    pub jump_tables: Vec<JumpTable>,
}

pub fn game_metadata(revision: Revision) -> ProgramMetadata {
    let functions: Vec<_> = tje_game::functions()
        .map(|f| Function {
            name: f.name,
            range: CodeRange {
                start: f.address(revision),
                end: f.address(revision) + f.size(revision),
            },
        })
        .collect();
    let jump_tables = switches::tables(revision);
    let entry_points = functions.iter().map(|f| f.range.start).collect();
    let mut code_ranges = Vec::new();
    for f in &functions {
        let mut pieces = vec![f.range];
        for table in &jump_tables {
            let data = CodeRange {
                start: table.address,
                end: table.address + 2 * u32::from(table.count),
            };
            let mut next = Vec::new();
            for range in pieces {
                if data.end <= range.start || data.start >= range.end {
                    next.push(range);
                    continue;
                }
                if range.start < data.start {
                    next.push(CodeRange {
                        start: range.start,
                        end: data.start,
                    });
                }
                if data.end < range.end {
                    next.push(CodeRange {
                        start: data.end,
                        end: range.end,
                    });
                }
            }
            pieces = next;
        }
        code_ranges.extend(pieces);
    }
    code_ranges.sort_by_key(|r| r.start);
    ProgramMetadata {
        functions,
        code_ranges,
        entry_points,
        jump_tables,
    }
}

fn word(rom: &[u8], address: u32) -> Result<u16, CompileError> {
    let bytes = rom
        .get(address as usize..address as usize + 2)
        .ok_or(CompileError::Truncated(address))?;
    Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
}
fn control_target(rom: &[u8], pc: u32, ea: Ea) -> Result<Option<u32>, CompileError> {
    Ok(match ea {
        Ea::AbsoluteShort => Some(word(rom, pc + 2)? as i16 as u32),
        Ea::AbsoluteLong => {
            Some((u32::from(word(rom, pc + 2)?) << 16) | u32::from(word(rom, pc + 4)?))
        }
        Ea::PcRelativeDisplacement => Some((pc + 2).wrapping_add(word(rom, pc + 2)? as i16 as u32)),
        _ => None,
    })
}

pub fn compile_program(rom: &[u8], metadata: &ProgramMetadata) -> Result<Program, CompileError> {
    compile_program_with_divider(rom, metadata, 7)
}

pub fn compile_program_with_divider(
    rom: &[u8],
    metadata: &ProgramMetadata,
    m68k_divider: u32,
) -> Result<Program, CompileError> {
    let valid = |pc: u32| metadata.code_ranges.iter().any(|range| range.contains(pc));
    for range in &metadata.code_ranges {
        if range.start & 1 != 0
            || range.end & 1 != 0
            || range.start >= range.end
            || range.end as usize > rom.len()
        {
            return Err(CompileError::InvalidMetadata(
                "invalid executable range".into(),
            ));
        }
    }
    let mut pending: BTreeSet<u32> = metadata.entry_points.iter().copied().collect();
    for table in &metadata.jump_tables {
        if !valid(table.jump_pc) || table.address & 1 != 0 {
            return Err(CompileError::InvalidMetadata(
                "invalid jump table metadata".into(),
            ));
        }
        for i in 0..table.count {
            pending.insert(
                table
                    .relative_base
                    .wrapping_add(word(rom, table.address + 2 * u32::from(i))? as i16 as u32),
            );
        }
    }
    let mut decoded = BTreeMap::new();
    let mut occupied = BTreeMap::new();
    while let Some(pc) = pending.pop_first() {
        if decoded.contains_key(&pc) {
            continue;
        }
        if !valid(pc) {
            return Err(CompileError::InvalidMetadata(format!(
                "control flow leaves executable metadata at {pc:08x}"
            )));
        }
        let ir = compile_instruction_with_divider(rom, pc, m68k_divider)?;
        for address in (pc..ir.next_pc).step_by(2) {
            if !valid(address) {
                return Err(CompileError::InvalidMetadata(format!(
                    "instruction crosses code/data boundary at {address:08x}"
                )));
            }
            if occupied
                .insert(address, pc)
                .is_some_and(|owner| owner != pc)
            {
                return Err(CompileError::InvalidMetadata(format!(
                    "overlapping instruction at {address:08x}"
                )));
            }
        }
        let instruction = decode(ir.opcode);
        let mut fallthrough = true;
        match instruction {
            I::Branch(condition, disp) => {
                let disp = if disp == 0 {
                    word(rom, pc + 2)? as i16 as u32
                } else {
                    disp as u32
                };
                pending.insert((pc + 2).wrapping_add(disp));
                if condition == BranchCondition::True {
                    fallthrough = false;
                }
            }
            I::BranchToSubroutine(disp) => {
                let disp = if disp == 0 {
                    word(rom, pc + 2)? as i16 as u32
                } else {
                    disp as u32
                };
                pending.insert((pc + 2).wrapping_add(disp));
            }
            I::BranchDecrement(..) => {
                pending.insert((pc + 2).wrapping_add(word(rom, pc + 2)? as i16 as u32));
            }
            I::Jump(ea) => {
                if let Some(target) = control_target(rom, pc, ea)? {
                    pending.insert(target);
                }
                fallthrough = false;
            }
            I::JumpToSubroutine(ea) => {
                if let Some(target) = control_target(rom, pc, ea)? {
                    pending.insert(target);
                }
            }
            I::Return { .. } | I::ReturnFromException | I::Illegal { .. } => fallthrough = false,
            _ => {}
        }
        if fallthrough {
            pending.insert(ir.next_pc);
        }
        decoded.insert(pc, ir);
    }
    Ok(Program {
        instructions: decoded.into_values().collect(),
    })
}
