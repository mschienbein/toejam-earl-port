//! Immutable user-ROM binding for a complete reviewed native ISR/callee closure.
use super::*;
use tje_engine_core::decode::{AddressingMode as Ea, BranchCondition, Instruction, decode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterruptEntry {
    pub level: u8,
    pub pc: u32,
}
#[derive(Debug, Clone)]
pub struct NativeIsrContract {
    pub name: &'static str,
    pub source: &'static str,
    pub entries: Vec<InterruptEntry>,
    pub instruction_pcs: Vec<u32>,
    pub edges: Vec<RegionEdge>,
}
pub struct NativeIsrProgram {
    pub(super) instructions: BTreeMap<u32, TimingInstruction>,
    pub(super) edges: BTreeSet<(u32, u32)>,
    entries: BTreeMap<u8, u32>,
    pub(super) divider: u32,
}
impl NativeIsrProgram {
    pub fn compile(
        rom: &[u8],
        contract: NativeIsrContract,
        divider: u32,
    ) -> Result<Self, TimingError> {
        if contract.name.is_empty() || contract.source.is_empty() || contract.entries.is_empty() {
            return Err(TimingError::MissingIsrCapability);
        }
        let instructions = compile_schedules(rom, &contract.instruction_pcs, &[], divider)?;
        let edges: BTreeSet<_> = contract.edges.iter().map(|e| (e.from, e.to)).collect();
        if edges.len() != contract.edges.len()
            || edges.iter().any(|&(from, to)| {
                !instructions.contains_key(&from) || !instructions.contains_key(&to)
            })
        {
            return Err(TimingError::InvalidIr(
                "incomplete native ISR/callee edge closure",
            ));
        }
        let mut entries = BTreeMap::new();
        for entry in &contract.entries {
            if !(1..=7).contains(&entry.level)
                || !instructions.contains_key(&entry.pc)
                || entries.insert(entry.level, entry.pc).is_some()
                || long(rom, 0x60 + 4 * u32::from(entry.level))? != entry.pc
            {
                return Err(TimingError::MissingIsrCapability);
            }
        }
        for (&pc, timing) in &instructions {
            let mut targets = Vec::with_capacity(2);
            let mut fallthrough = true;
            match decode(timing.opcode()) {
                Instruction::ReturnFromException => {
                    fallthrough = false;
                    if edges.iter().any(|&(from, _)| from == pc) {
                        return Err(TimingError::InvalidIr(
                            "RTE is controller-owned, not a source edge",
                        ));
                    }
                }
                Instruction::Branch(condition, displacement) => {
                    targets.push(branch(rom, pc, displacement)?);
                    fallthrough = condition != BranchCondition::True;
                }
                Instruction::BranchToSubroutine(displacement) => {
                    targets.push(branch(rom, pc, displacement)?)
                }
                Instruction::BranchDecrement(..) => {
                    targets.push((pc + 2).wrapping_add(word(rom, pc + 2)? as i16 as u32))
                }
                Instruction::Jump(ea) | Instruction::JumpToSubroutine(ea) => {
                    if let Some(target) = control(rom, pc, ea)? {
                        targets.push(target);
                    } else if !edges.iter().any(|&(from, _)| from == pc) {
                        return Err(TimingError::InvalidIr(
                            "unreviewed native indirect transfer",
                        ));
                    }
                    fallthrough =
                        matches!(decode(timing.opcode()), Instruction::JumpToSubroutine(_));
                }
                Instruction::Return { .. } => {
                    fallthrough = false;
                    if !edges.iter().any(|&(from, _)| from == pc) {
                        return Err(TimingError::InvalidIr(
                            "native callee return has no reviewed callers",
                        ));
                    }
                }
                Instruction::Illegal { .. }
                | Instruction::Trap(_)
                | Instruction::TrapOnOverflow
                | Instruction::Stop => {
                    return Err(TimingError::InvalidIr(
                        "native ISR has an undeclared exception/terminal exit",
                    ));
                }
                _ => {}
            }
            if fallthrough {
                targets.push(timing.next_pc());
            }
            for target in targets {
                if !instructions.contains_key(&target) || !edges.contains(&(pc, target)) {
                    return Err(TimingError::InvalidIr(
                        "missing native ISR/callee source continuation",
                    ));
                }
            }
        }
        Ok(Self {
            instructions,
            edges,
            entries,
            divider,
        })
    }
    pub(super) fn validate_requirements(
        &self,
        reviewed: &ReviewedRegion,
    ) -> Result<(), TimingError> {
        if reviewed
            .required_interrupts
            .iter()
            .any(|level| !self.entries.contains_key(level))
        {
            Err(TimingError::MissingIsrCapability)
        } else {
            Ok(())
        }
    }
}
fn word(rom: &[u8], pc: u32) -> Result<u16, TimingError> {
    let bytes = rom
        .get(pc as usize..pc as usize + 2)
        .ok_or(TimingError::InvalidIr(
            "native ISR metadata leaves immutable ROM",
        ))?;
    Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
}
fn long(rom: &[u8], pc: u32) -> Result<u32, TimingError> {
    Ok((u32::from(word(rom, pc)?) << 16) | u32::from(word(rom, pc + 2)?))
}
fn branch(rom: &[u8], pc: u32, displacement: i8) -> Result<u32, TimingError> {
    Ok((pc + 2).wrapping_add(if displacement == 0 {
        word(rom, pc + 2)? as i16 as u32
    } else {
        displacement as u32
    }))
}
fn control(rom: &[u8], pc: u32, ea: Ea) -> Result<Option<u32>, TimingError> {
    Ok(match ea {
        Ea::AbsoluteShort => Some(word(rom, pc + 2)? as i16 as u32),
        Ea::AbsoluteLong => Some(long(rom, pc + 2)?),
        Ea::PcRelativeDisplacement => Some((pc + 2).wrapping_add(word(rom, pc + 2)? as i16 as u32)),
        _ => None,
    })
}
