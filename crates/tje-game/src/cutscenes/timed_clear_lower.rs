//! Source-owned clear-plane roles, including the real call and caller cleanup.
//! G/src/cutscenes/title_and_transition.c:99 and its matching MCC68K output.
use crate::{Context, Error, Result, ids, timing::*};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
enum Role {
    SaveLoopRegister,
    PushWaitCount,
    CallWait,
    DropWaitSlot,
    ResetRow,
    LoadRow,
    ExtendRow,
    SetPlaneShift,
    ScaleRow,
    AddPlaneBase,
    PublishPlaneCommand,
    ResetColumn,
    PublishTile,
    IncrementColumn,
    CompareColumns,
    BranchColumns,
    IncrementRow,
    CompareRows,
    BranchRows,
    RestoreLoopRegister,
    Return,
}
const fn location(role: Role) -> Location {
    Location {
        function: ids::CLEAR_LOWER_VRAM_TILEMAP,
        role: role as u16,
    }
}
const fn edge(from: Role, to: Role) -> Edge {
    Edge {
        from: location(from),
        to: location(to),
    }
}
static PROGRAM: Program = Program {
    name: "ClearLowerVramTilemap",
    source: "G/src/cutscenes/title_and_transition.c:99; matching MCC68K source roles",
    entry: location(Role::SaveLoopRegister),
    functions: &[FunctionRegion {
        function: ids::CLEAR_LOWER_VRAM_TILEMAP,
        roles: Role::Return as u16 + 1,
    }],
    edges: &[
        edge(Role::SaveLoopRegister, Role::PushWaitCount),
        edge(Role::PushWaitCount, Role::CallWait),
        edge(Role::DropWaitSlot, Role::ResetRow),
        edge(Role::ResetRow, Role::LoadRow),
        edge(Role::LoadRow, Role::ExtendRow),
        edge(Role::ExtendRow, Role::SetPlaneShift),
        edge(Role::SetPlaneShift, Role::ScaleRow),
        edge(Role::ScaleRow, Role::AddPlaneBase),
        edge(Role::AddPlaneBase, Role::PublishPlaneCommand),
        edge(Role::PublishPlaneCommand, Role::ResetColumn),
        edge(Role::ResetColumn, Role::PublishTile),
        edge(Role::PublishTile, Role::IncrementColumn),
        edge(Role::IncrementColumn, Role::CompareColumns),
        edge(Role::CompareColumns, Role::BranchColumns),
        edge(Role::BranchColumns, Role::PublishTile),
        edge(Role::BranchColumns, Role::IncrementRow),
        edge(Role::IncrementRow, Role::CompareRows),
        edge(Role::CompareRows, Role::BranchRows),
        edge(Role::BranchRows, Role::LoadRow),
        edge(Role::BranchRows, Role::RestoreLoopRegister),
        edge(Role::RestoreLoopRegister, Role::Return),
    ],
    calls: &[CallSite {
        at: location(Role::CallWait),
        targets: &[ids::WAIT_FOR_NEXT_V_BLANK],
        resume: location(Role::DropWaitSlot),
    }],
};
struct Work {
    role: Role,
    registers: Registers,
    effects: [Option<Effect>; 2],
    acknowledged: usize,
    pending: Option<Effect>,
    reads: [u16; 2],
}
/// One invocation's source state. Reentrant callers instantiate distinct values;
/// no architectural result is restored from an old private register snapshot.
pub struct ClearLowerSource {
    expected: Option<Role>,
    work: Option<Work>,
    failed: bool,
}
impl Default for ClearLowerSource {
    fn default() -> Self {
        Self::new()
    }
}
impl ClearLowerSource {
    pub const fn definition() -> &'static Program {
        &PROGRAM
    }
    pub const fn new() -> Self {
        Self {
            expected: Some(Role::SaveLoopRegister),
            work: None,
            failed: false,
        }
    }
    fn error(&mut self, message: &'static str) -> Error {
        self.failed = true;
        Error::Platform(message.into())
    }
}
fn word(address: u32, kind: EffectKind) -> Option<Effect> {
    Some(Effect {
        address,
        width: Width::Word,
        kind,
    })
}
fn push_words(sp: u32, value: u32, low_first: bool) -> [Option<Effect>; 2] {
    let high = word(sp, EffectKind::Write((value >> 16) as u16));
    let low = word(sp.wrapping_add(2), EffectKind::Write(value as u16));
    if low_first { [low, high] } else { [high, low] }
}
fn nzvc(registers: &mut Registers, negative: bool, zero: bool, overflow: bool, carry: bool) {
    registers.status = (registers.status & !0x000f)
        | (u16::from(negative) << 3)
        | (u16::from(zero) << 2)
        | (u16::from(overflow) << 1)
        | u16::from(carry);
}
fn moved(registers: &mut Registers, value: u32, long: bool) {
    let value = if long { value } else { value & 0xffff };
    nzvc(
        registers,
        value & if long { 0x8000_0000 } else { 0x8000 } != 0,
        value == 0,
        false,
        false,
    );
}
fn extend_carry(registers: &mut Registers, carry: bool) {
    registers.status = (registers.status & !0x10) | (u16::from(carry) << 4);
}
fn increment_word(registers: &mut Registers, register: usize) {
    let before = registers.data[register] as u16;
    let (value, carry) = before.overflowing_add(1);
    registers.data[register] = (registers.data[register] & 0xffff0000) | u32::from(value);
    nzvc(
        registers,
        value & 0x8000 != 0,
        value == 0,
        before == 0x7fff,
        carry,
    );
    extend_carry(registers, carry);
}
fn compare_word(registers: &mut Registers, register: usize, bound: u16) {
    let before = registers.data[register] as u16;
    let value = before.wrapping_sub(bound);
    nzvc(
        registers,
        value & 0x8000 != 0,
        value == 0,
        ((before ^ bound) & (before ^ value) & 0x8000) != 0,
        before < bound,
    );
}
fn signed_less(status: u16) -> bool {
    (status & 8 != 0) != (status & 2 != 0)
}
impl TimingSource for ClearLowerSource {
    fn program(&self) -> &'static Program {
        &PROGRAM
    }
    fn begin_instruction(&mut self, site: InstructionSite, registers: Registers) -> Result<()> {
        let Some(role) = self.expected else {
            return Err(self.error("clear-plane source already returned"));
        };
        if self.failed || self.work.is_some() || site.location != location(role) {
            return Err(self.error("clear-plane source role or invocation mismatch"));
        }
        let sp = registers.stack_pointer();
        let effects = match role {
            Role::SaveLoopRegister => push_words(sp.wrapping_sub(4), registers.data[2], true),
            Role::PushWaitCount => push_words(sp.wrapping_sub(4), 1, false),
            Role::CallWait => push_words(sp.wrapping_sub(4), site.fallthrough, false),
            Role::PublishPlaneCommand => push_words(0xc00004, registers.data[0], false),
            Role::PublishTile => [word(0xc00000, EffectKind::Write(0x8000)), None],
            Role::RestoreLoopRegister | Role::Return => [
                word(sp, EffectKind::Read(ReadPurpose::Semantic)),
                word(sp.wrapping_add(2), EffectKind::Read(ReadPurpose::Semantic)),
            ],
            _ => [None, None],
        };
        self.work = Some(Work {
            role,
            registers,
            effects,
            acknowledged: 0,
            pending: None,
            reads: [0; 2],
        });
        Ok(())
    }
    fn next_effect(&mut self) -> Result<Option<Effect>> {
        if self.failed {
            return Err(self.error("clear-plane source is failed"));
        }
        let Some(work) = &mut self.work else {
            return Err(self.error("clear-plane source instruction has not begun"));
        };
        if work.pending.is_some() {
            return Err(self.error("clear-plane source effect is still unacknowledged"));
        }
        let effect = work.effects.get(work.acknowledged).copied().flatten();
        work.pending = effect;
        Ok(effect)
    }
    fn acknowledge(&mut self, actual: Observation) -> Result<()> {
        if self.failed {
            return Err(self.error("clear-plane source is failed"));
        }
        let Some(work) = &mut self.work else {
            return Err(self.error("clear-plane source has no active instruction"));
        };
        if work.pending != Some(actual.effect)
            || work.acknowledged >= work.effects.len()
            || matches!(actual.effect.kind, EffectKind::Write(value) if value != actual.value)
        {
            return Err(self.error("clear-plane source acknowledgement mismatch"));
        }
        work.reads[work.acknowledged] = actual.value;
        work.acknowledged += 1;
        work.pending = None;
        Ok(())
    }
    fn finish_instruction(&mut self) -> Result<Commit> {
        if self.failed {
            return Err(self.error("clear-plane source is failed"));
        }
        let Some(work) = self.work.take() else {
            return Err(self.error("clear-plane source instruction already finished"));
        };
        if work.pending.is_some()
            || work
                .effects
                .get(work.acknowledged)
                .copied()
                .flatten()
                .is_some()
        {
            return Err(self.error("clear-plane source has missing bus acknowledgements"));
        }
        let mut registers = work.registers;
        let next = match work.role {
            Role::SaveLoopRegister => {
                registers.set_stack_pointer(registers.stack_pointer().wrapping_sub(4));
                let value = registers.data[2];
                moved(&mut registers, value, true);
                Role::PushWaitCount
            }
            Role::PushWaitCount => {
                registers.set_stack_pointer(registers.stack_pointer().wrapping_sub(4));
                Role::CallWait
            }
            Role::CallWait => {
                registers.set_stack_pointer(registers.stack_pointer().wrapping_sub(4));
                self.expected = Some(Role::DropWaitSlot);
                return Ok(Commit {
                    registers,
                    path: Path::Call {
                        target: ids::WAIT_FOR_NEXT_V_BLANK,
                        return_to: location(Role::DropWaitSlot),
                    },
                });
            }
            Role::DropWaitSlot => {
                registers.set_stack_pointer(registers.stack_pointer().wrapping_add(4));
                Role::ResetRow
            }
            Role::ResetRow => {
                registers.data[2] &= 0xffff0000;
                moved(&mut registers, 0, false);
                Role::LoadRow
            }
            Role::LoadRow => {
                let value = registers.data[2] & 0xffff;
                registers.data[0] = (registers.data[0] & 0xffff0000) | value;
                moved(&mut registers, value, false);
                Role::ExtendRow
            }
            Role::ExtendRow => {
                let value = registers.data[0] as i16 as i32 as u32;
                registers.data[0] = value;
                moved(&mut registers, value, true);
                Role::SetPlaneShift
            }
            Role::SetPlaneShift => {
                registers.data[1] = 23;
                moved(&mut registers, 23, true);
                Role::ScaleRow
            }
            Role::ScaleRow => {
                let before = registers.data[0];
                let count = registers.data[1] & 63;
                let (value, carry, overflow) = if count == 0 {
                    (before, false, false)
                } else if count < 32 {
                    (
                        before.wrapping_shl(count),
                        before & (1 << (32 - count)) != 0,
                        ((before ^ before.wrapping_shl(1)) & (u32::MAX << (32 - count))) != 0,
                    )
                } else {
                    (0, count == 32 && before & 1 != 0, before != 0)
                };
                registers.data[0] = value;
                nzvc(
                    &mut registers,
                    value & 0x80000000 != 0,
                    value == 0,
                    overflow,
                    carry,
                );
                if count != 0 {
                    extend_carry(&mut registers, carry);
                }
                Role::AddPlaneBase
            }
            Role::AddPlaneBase => {
                let before = registers.data[0];
                let base = 0x60000002;
                let (value, carry) = before.overflowing_add(base);
                registers.data[0] = value;
                nzvc(
                    &mut registers,
                    value & 0x80000000 != 0,
                    value == 0,
                    (!(before ^ base) & (before ^ value) & 0x80000000) != 0,
                    carry,
                );
                extend_carry(&mut registers, carry);
                Role::PublishPlaneCommand
            }
            Role::PublishPlaneCommand => {
                let value = registers.data[0];
                moved(&mut registers, value, true);
                Role::ResetColumn
            }
            Role::ResetColumn => {
                registers.data[0] &= 0xffff0000;
                moved(&mut registers, 0, false);
                Role::PublishTile
            }
            Role::PublishTile => {
                moved(&mut registers, 0x8000, false);
                Role::IncrementColumn
            }
            Role::IncrementColumn => {
                increment_word(&mut registers, 0);
                Role::CompareColumns
            }
            Role::CompareColumns => {
                compare_word(&mut registers, 0, 64);
                Role::BranchColumns
            }
            Role::BranchColumns => {
                if signed_less(registers.status) {
                    Role::PublishTile
                } else {
                    Role::IncrementRow
                }
            }
            Role::IncrementRow => {
                increment_word(&mut registers, 2);
                Role::CompareRows
            }
            Role::CompareRows => {
                compare_word(&mut registers, 2, 32);
                Role::BranchRows
            }
            Role::BranchRows => {
                if signed_less(registers.status) {
                    Role::LoadRow
                } else {
                    Role::RestoreLoopRegister
                }
            }
            Role::RestoreLoopRegister => {
                let value = (u32::from(work.reads[0]) << 16) | u32::from(work.reads[1]);
                registers.data[2] = value;
                registers.set_stack_pointer(registers.stack_pointer().wrapping_add(4));
                moved(&mut registers, value, true);
                Role::Return
            }
            Role::Return => {
                let address = (u32::from(work.reads[0]) << 16) | u32::from(work.reads[1]);
                registers.set_stack_pointer(registers.stack_pointer().wrapping_add(4));
                self.expected = None;
                return Ok(Commit {
                    registers,
                    path: Path::Return {
                        address,
                        value: None,
                    },
                });
            }
        };
        self.expected = Some(next);
        Ok(Commit {
            registers,
            path: Path::Next(location(next)),
        })
    }
}

/// G/src/cutscenes/title_and_transition.c:99 (`ClearLowerVramTilemap`).
/// Complete prepared-entry timing candidate; it does not activate a registry ID.
pub fn clear_lower_vram_tilemap_framed(context: &mut dyn Context) -> Result<Option<u32>> {
    let entry = ids::CLEAR_LOWER_VRAM_TILEMAP;
    let frame = context
        .frame_state()
        .ok_or(crate::call_frame::FrameError::Unavailable)?;
    if !frame.is_prepared_for(entry) || context.stack_pointer()? != frame.entry_sp {
        return Err(crate::call_frame::FrameError::PreparedEntryRequired(entry).into());
    }
    let mut source = ClearLowerSource::new();
    Ok(context.execute_timing_program(entry, &mut source)?.value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_keeps_real_call_slots_lazy_saved_local_reads_and_all_plane_writes() {
        let mut source = ClearLowerSource::new();
        let mut registers = Registers {
            data: [0x98765432; 8],
            address: [0; 7],
            user_stack: 0x3000,
            supervisor_stack: 0x4000,
            status: 0x271f,
        };
        registers.data[2] = 0xdead8001;
        let mut ram = vec![0_u8; 0x5000];
        ram[0x4000..0x4004].copy_from_slice(&0x200_u32.to_be_bytes());
        let mut site = location(Role::SaveLoopRegister);
        let mut commands = Vec::new();
        let mut tiles = 0;
        let mut calls = 0;
        loop {
            source
                .begin_instruction(
                    InstructionSite {
                        location: site,
                        address: 0x100,
                        fallthrough: 0x106,
                    },
                    registers,
                )
                .unwrap();
            while let Some(effect) = source.next_effect().unwrap() {
                let value = match effect.kind {
                    EffectKind::Read(_) => u16::from_be_bytes([
                        ram[effect.address as usize],
                        ram[effect.address as usize + 1],
                    ]),
                    EffectKind::Write(value) => {
                        if effect.address < ram.len() as u32 {
                            ram[effect.address as usize..effect.address as usize + 2]
                                .copy_from_slice(&value.to_be_bytes());
                        } else if effect.address == 0xc00004 || effect.address == 0xc00006 {
                            commands.push(value);
                        } else {
                            assert_eq!(effect.address, 0xc00000);
                            assert_eq!(value, 0x8000);
                            tiles += 1;
                        }
                        value
                    }
                };
                source.acknowledge(Observation { effect, value }).unwrap();
            }
            let commit = source.finish_instruction().unwrap();
            registers = commit.registers;
            match commit.path {
                Path::Next(next) => site = next,
                Path::Call { target, return_to } => {
                    assert_eq!(target, ids::WAIT_FOR_NEXT_V_BLANK);
                    assert_eq!(registers.stack_pointer(), 0x3ff4);
                    assert_eq!(&ram[0x3ff4..0x3ff8], &0x106_u32.to_be_bytes());
                    assert_eq!(&ram[0x3ff8..0x3ffc], &1_u32.to_be_bytes());
                    // Source-only protocol test: this supplies the next real
                    // boundary as an input; it constructs no platform receipt.
                    registers.set_stack_pointer(registers.stack_pointer() + 4);
                    ram[0x3ffc..0x4000].copy_from_slice(&0x1234abcd_u32.to_be_bytes());
                    site = return_to;
                    calls += 1;
                }
                Path::Return { address, value } => {
                    assert_eq!(address, 0x200);
                    assert_eq!(value, None);
                    break;
                }
            }
        }
        assert_eq!(calls, 1);
        assert_eq!(tiles, 32 * 64);
        assert_eq!(commands.len(), 64);
        for row in 0..32_u32 {
            let command = 0x60000002_u32.wrapping_add(row << 23);
            assert_eq!(commands[2 * row as usize], (command >> 16) as u16);
            assert_eq!(commands[2 * row as usize + 1], command as u16);
        }
        assert_eq!(
            registers.data[2], 0x1234abcd,
            "restore must use actual saved-local reads"
        );
        assert_eq!(registers.stack_pointer(), 0x4004);
    }
    #[test]
    fn missing_or_repeated_source_acknowledgements_fail_closed() {
        let registers = Registers {
            data: [0; 8],
            address: [0; 7],
            user_stack: 0x3000,
            supervisor_stack: 0x4000,
            status: 0x2700,
        };
        let mut source = ClearLowerSource::new();
        let site = InstructionSite {
            location: location(Role::SaveLoopRegister),
            address: 0x100,
            fallthrough: 0x102,
        };
        source.begin_instruction(site, registers).unwrap();
        let effect = source.next_effect().unwrap().unwrap();
        source
            .acknowledge(Observation { effect, value: 0 })
            .unwrap();
        assert!(
            source
                .acknowledge(Observation { effect, value: 0 })
                .is_err()
        );
        assert!(source.finish_instruction().is_err());
    }
}
