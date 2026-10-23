//! Source-backed exemplar, G startup_and_main_loop.c:142 and its MCC68K output.
//! This remains an explicit proof integration point, not production promotion.
//! Eleven reviewed source roles preserve the real D0/D1/SR/SP intermediate
//! states. Runtime-derived PC identities never become a committed ROM listing.
use super::*;
use tje_engine_core::{
    compile_instruction_with_divider,
    decode::{AddressingMode as Ea, BranchCondition as Cc, Instruction as I, OpSize as S},
};
use tje_engine_runtime::timing_region::{RegionEdge, ReviewedRegion, TimingProgram};
use tje_game::core::timed_wait::{WaitBranch, WaitRegisters, WaitRole as Role, WaitStep};
use tje_mem::{Revision, ram};

const ROLES: [Role; 11] = Role::ALL;

pub struct WaitPlan {
    timing: TimingProgram,
    pcs: [u32; 11],
    revision: Revision,
    return_pc: u32,
}
impl WaitPlan {
    pub fn timing(&self) -> &TimingProgram {
        &self.timing
    }
    pub fn compile(rom: &[u8], revision: Revision, return_pc: u32, divider: u32) -> Result<Self> {
        let function = tje_game::ids::WAIT_FOR_NEXT_V_BLANK.metadata();
        let start = function.address(revision);
        let end = start + function.size(revision);
        let mut instructions = Vec::new();
        let mut pc = start;
        while pc < end {
            let ir = compile_instruction_with_divider(rom, pc, divider)
                .map_err(|e| HostError::Metadata(e.to_string()))?;
            pc = ir.next_pc;
            instructions.push(ir);
        }
        if pc != end || instructions.len() != ROLES.len() {
            return Err(HostError::Metadata(
                "WaitForNextVBlank source region extent changed".into(),
            ));
        }
        let mut pcs = [0; 11];
        for (dst, ir) in pcs.iter_mut().zip(&instructions) {
            *dst = ir.pc;
        }
        let display = ram::g_mode2_reg_softcopy(revision).address();
        let counter = ram::g_frame_counter(revision).address();
        for (&role, ir) in ROLES.iter().zip(&instructions) {
            let i = tje_engine_core::decode::decode(ir.opcode);
            let valid = match (role, i) {
                (
                    Role::LoadCount,
                    I::Move {
                        size: S::Word,
                        source: Ea::AddressIndirectDisplacement(a),
                        dest: Ea::DataDirect(d),
                    },
                ) => a.0 == 7 && d.0 == 0 && word(rom, ir.pc + 2)? == 6,
                (
                    Role::TestDisplay,
                    I::BitTest {
                        source: Ea::Immediate,
                        dest,
                    },
                ) => word(rom, ir.pc + 2)? == 6 && absolute(rom, ir.pc + 4, dest)? == Some(display),
                (Role::DisplayBranch, I::Branch(Cc::Equal, d)) => {
                    branch_target(rom, ir.pc, d)? == pcs[10]
                }
                (Role::ToCountTest, I::Branch(Cc::True, d)) => {
                    branch_target(rom, ir.pc, d)? == pcs[8]
                }
                (
                    Role::CaptureCounter,
                    I::Move {
                        size: S::Word,
                        source,
                        dest: Ea::DataDirect(d),
                    },
                ) => d.0 == 1 && absolute(rom, ir.pc + 2, source)? == Some(counter),
                (
                    Role::CompareCounter,
                    I::Compare {
                        size: S::Word,
                        source,
                        dest: Ea::DataDirect(d),
                    },
                ) => d.0 == 1 && absolute(rom, ir.pc + 2, source)? == Some(counter),
                (Role::PollBranch, I::Branch(Cc::Equal, d)) => {
                    branch_target(rom, ir.pc, d)? == pcs[5]
                }
                (
                    Role::DecrementCount,
                    I::Subtract {
                        size: S::Word,
                        source: Ea::Quick(1),
                        dest: Ea::DataDirect(d),
                        with_extend: false,
                    },
                ) => d.0 == 0,
                (Role::TestCount, I::Test(S::Word, Ea::DataDirect(d))) => d.0 == 0,
                (Role::CountBranch, I::Branch(Cc::GreaterThan, d)) => {
                    branch_target(rom, ir.pc, d)? == pcs[4]
                }
                (Role::Return, I::Return { restore_ccr: false }) => true,
                _ => false,
            };
            if !valid {
                return Err(HostError::Metadata(format!(
                    "unreviewed WaitForNextVBlank source role {role:?}"
                )));
            }
        }
        let edges = vec![
            RegionEdge {
                from: pcs[0],
                to: pcs[1],
            },
            RegionEdge {
                from: pcs[1],
                to: pcs[2],
            },
            RegionEdge {
                from: pcs[2],
                to: pcs[3],
            },
            RegionEdge {
                from: pcs[2],
                to: pcs[10],
            },
            RegionEdge {
                from: pcs[3],
                to: pcs[8],
            },
            RegionEdge {
                from: pcs[4],
                to: pcs[5],
            },
            RegionEdge {
                from: pcs[5],
                to: pcs[6],
            },
            RegionEdge {
                from: pcs[6],
                to: pcs[5],
            },
            RegionEdge {
                from: pcs[6],
                to: pcs[7],
            },
            RegionEdge {
                from: pcs[7],
                to: pcs[8],
            },
            RegionEdge {
                from: pcs[8],
                to: pcs[9],
            },
            RegionEdge {
                from: pcs[9],
                to: pcs[4],
            },
            RegionEdge {
                from: pcs[9],
                to: pcs[10],
            },
            RegionEdge {
                from: pcs[10],
                to: return_pc,
            },
        ];
        let timing = TimingProgram::compile(
            rom,
            ReviewedRegion {
                name: "WaitForNextVBlank",
                source: "src/core/startup_and_main_loop.c:142; compiled source roles",
                entry: start,
                instruction_pcs: pcs.to_vec(),
                edges,
                return_pc,
                maximum_steps: 1_000_000,
                required_interrupts: &[4, 6],
            },
            divider,
        )
        .map_err(timing_error)?;
        Ok(Self {
            timing,
            pcs,
            revision,
            return_pc,
        })
    }
    pub fn source_step(&self, entry: CpuState) -> Result<WaitSourceStep<'_>> {
        let index = self
            .pcs
            .iter()
            .position(|&pc| pc == entry.pc)
            .ok_or_else(|| HostError::Metadata("Wait source escaped reviewed role".into()))?;
        let source = WaitStep::begin(
            self.revision,
            ROLES[index],
            WaitRegisters {
                d0: entry.d[0],
                d1: entry.d[1],
                sr: entry.sr,
                sp: entry.read_address(7),
            },
        );
        Ok(WaitSourceStep {
            plan: self,
            entry,
            source: Some(source),
        })
    }
}
fn word(rom: &[u8], pc: u32) -> Result<u16> {
    let bytes = rom
        .get(pc as usize..pc as usize + 2)
        .ok_or_else(|| HostError::Metadata("truncated reviewed wait region".into()))?;
    Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
}
fn branch_target(rom: &[u8], pc: u32, displacement: i8) -> Result<u32> {
    Ok(pc.wrapping_add(2).wrapping_add(if displacement == 0 {
        word(rom, pc + 2)? as i16 as u32
    } else {
        displacement as u32
    }))
}
fn absolute(rom: &[u8], pc: u32, ea: Ea) -> Result<Option<u32>> {
    Ok(match ea {
        Ea::AbsoluteShort => Some(word(rom, pc)? as i16 as u32),
        Ea::AbsoluteLong => Some((u32::from(word(rom, pc)?) << 16) | u32::from(word(rom, pc + 2)?)),
        _ => None,
    }
    .map(|v| v & 0xFFFFFF))
}

pub struct WaitSourceStep<'a> {
    plan: &'a WaitPlan,
    entry: CpuState,
    source: Option<WaitStep>,
}
impl RustInstruction for WaitSourceStep<'_> {
    fn entry_state(&self) -> &CpuState {
        &self.entry
    }
    fn effect(
        &mut self,
        emulator: &mut GenesisEmulator,
        expected: ExpectedAccess,
    ) -> Result<BusObservation> {
        // Prefetch is timing infrastructure, not a game read. Semantic effects
        // are requested and consumed by the actual readable Rust source body.
        if expected.kind == AccessKind::Read(ReadKind::Prefetch) {
            return rust_bus_effect(emulator, expected, None);
        }
        let source = self.source.as_mut().ok_or_else(|| {
            HostError::Metadata("Wait source instruction already finished".into())
        })?;
        let read = source
            .expected_read()
            .map_err(|e| HostError::Metadata(e.to_string()))?;
        let width = if read.word { Width::Word } else { Width::Byte };
        if expected.address & 0xFFFFFF != read.address & 0xFFFFFF
            || expected.width != width
            || expected.kind != AccessKind::Read(ReadKind::Semantic)
        {
            return Err(HostError::Metadata(
                "Wait source effect identity changed".into(),
            ));
        }
        let observation = rust_bus_effect(emulator, expected, None)?;
        source
            .acknowledge_read(read, observation.value)
            .map_err(|e| HostError::Metadata(e.to_string()))?;
        Ok(observation)
    }
    fn finish(&mut self, emulator: &mut GenesisEmulator) -> Result<RustPathChoice> {
        let source = self.source.take().ok_or_else(|| {
            HostError::Metadata("Wait source instruction already finished".into())
        })?;
        let effect = source
            .finish()
            .map_err(|e| HostError::Metadata(e.to_string()))?;
        let next_pc = match effect.next {
            WaitBranch::Role(role) => self.plan.pcs[role.index()],
            WaitBranch::Return(pc) if pc == self.plan.return_pc => pc,
            WaitBranch::Return(_) => {
                return Err(HostError::Metadata("Wait RTS return changed".into()));
            }
        };
        let cpu = emulator.hybrid_parts().0;
        let mut d = cpu.data_registers();
        d[0] = effect.registers.d0;
        d[1] = effect.registers.d1;
        cpu.set_data_registers(d);
        cpu.set_status_register(effect.registers.sr as u16);
        crate::abi::set_sp(cpu, effect.registers.sp);
        Ok(RustPathChoice { next_pc })
    }
}

#[cfg(test)]
mod tests;
