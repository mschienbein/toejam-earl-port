//! Rust-owned effect/state roles for the reviewed WaitForNextVBlank region.
//!
//! Source: startup_and_main_loop.c:142 and matching MCC68K source output. These
//! roles contain game/ABI semantics only, not clocks, decoded opcodes or a CPU
//! interpreter. A platform must admit each role through the timing-only engine
//! and execute its requested real read once. This is not auto-registration.
use std::fmt;
use tje_mem::{Revision, ram};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum WaitRole {
    LoadCount,
    TestDisplay,
    DisplayBranch,
    ToCountTest,
    CaptureCounter,
    CompareCounter,
    PollBranch,
    DecrementCount,
    TestCount,
    CountBranch,
    Return,
}
impl WaitRole {
    pub const ALL: [Self; 11] = [
        Self::LoadCount,
        Self::TestDisplay,
        Self::DisplayBranch,
        Self::ToCountTest,
        Self::CaptureCounter,
        Self::CompareCounter,
        Self::PollBranch,
        Self::DecrementCount,
        Self::TestCount,
        Self::CountBranch,
        Self::Return,
    ];
    pub const fn index(self) -> usize {
        self as usize
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaitRegisters {
    pub d0: u32,
    pub d1: u32,
    pub sr: u32,
    pub sp: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceRead {
    pub address: u32,
    pub word: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitBranch {
    Role(WaitRole),
    Return(u32),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaitEffect {
    pub registers: WaitRegisters,
    pub next: WaitBranch,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitSourceError {
    ExtraRead,
    MissingRead,
    WrongRead,
}
impl fmt::Display for WaitSourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "WaitForNextVBlank source effect: {self:?}")
    }
}
impl std::error::Error for WaitSourceError {}

pub struct WaitStep {
    revision: Revision,
    role: WaitRole,
    entry: WaitRegisters,
    reads: [u32; 2],
    count: usize,
}
impl WaitStep {
    pub fn begin(revision: Revision, role: WaitRole, entry: WaitRegisters) -> Self {
        Self {
            revision,
            role,
            entry,
            reads: [0; 2],
            count: 0,
        }
    }
    pub fn expected_read(&self) -> Result<SourceRead, WaitSourceError> {
        let read = match self.role {
            WaitRole::LoadCount if self.count == 0 => SourceRead {
                address: self.entry.sp.wrapping_add(6),
                word: true,
            },
            WaitRole::TestDisplay if self.count == 0 => SourceRead {
                address: ram::g_mode2_reg_softcopy(self.revision).address(),
                word: false,
            },
            WaitRole::CaptureCounter | WaitRole::CompareCounter if self.count == 0 => SourceRead {
                address: ram::g_frame_counter(self.revision).address(),
                word: true,
            },
            WaitRole::Return if self.count < 2 => SourceRead {
                address: self.entry.sp.wrapping_add(2 * self.count as u32),
                word: true,
            },
            _ => return Err(WaitSourceError::ExtraRead),
        };
        Ok(read)
    }
    pub fn acknowledge_read(
        &mut self,
        read: SourceRead,
        value: u32,
    ) -> Result<(), WaitSourceError> {
        if self.expected_read()? != read || value > if read.word { 65535 } else { 255 } {
            return Err(WaitSourceError::WrongRead);
        }
        self.reads[self.count] = value;
        self.count += 1;
        Ok(())
    }
    pub fn finish(self) -> Result<WaitEffect, WaitSourceError> {
        use WaitRole::*;
        let required = match self.role {
            LoadCount | TestDisplay | CaptureCounter | CompareCounter => 1,
            Return => 2,
            _ => 0,
        };
        if self.count != required {
            return Err(WaitSourceError::MissingRead);
        }
        let mut r = self.entry;
        let mut next = WaitBranch::Role(match self.role {
            LoadCount => TestDisplay,
            TestDisplay => DisplayBranch,
            DisplayBranch => ToCountTest,
            ToCountTest => TestCount,
            CaptureCounter => CompareCounter,
            CompareCounter => PollBranch,
            PollBranch => DecrementCount,
            DecrementCount => TestCount,
            TestCount => CountBranch,
            CountBranch | Return => Return,
        });
        match self.role {
            LoadCount => {
                r.d0 = (r.d0 & !65535) | self.reads[0];
                r.sr = nz(r.sr, self.reads[0]);
            }
            TestDisplay => r.sr = (r.sr & !4) | (u32::from(self.reads[0] & 0x40 == 0) << 2),
            DisplayBranch => {
                if r.sr & 4 != 0 {
                    next = WaitBranch::Role(Return);
                }
            }
            ToCountTest => {}
            CaptureCounter => {
                r.d1 = (r.d1 & !65535) | self.reads[0];
                r.sr = nz(r.sr, self.reads[0]);
            }
            CompareCounter => r.sr = subtract_flags(r.sr, r.d1 as u16, self.reads[0] as u16, false),
            PollBranch => {
                if r.sr & 4 != 0 {
                    next = WaitBranch::Role(CompareCounter);
                }
            }
            DecrementCount => {
                r.sr = subtract_flags(r.sr, r.d0 as u16, 1, true);
                r.d0 = (r.d0 & !65535) | u32::from((r.d0 as u16).wrapping_sub(1));
            }
            TestCount => r.sr = nz(r.sr, r.d0 & 65535),
            CountBranch => {
                if r.sr & 4 == 0 && ((r.sr >> 3) ^ (r.sr >> 1)) & 1 == 0 {
                    next = WaitBranch::Role(CaptureCounter);
                }
            }
            Return => {
                next = WaitBranch::Return((self.reads[0] << 16) | self.reads[1]);
                r.sp = r.sp.wrapping_add(4);
            }
        }
        Ok(WaitEffect { registers: r, next })
    }
}
fn nz(sr: u32, word: u32) -> u32 {
    (sr & !15) | (u32::from(word & 65535 == 0) << 2) | (u32::from(word & 0x8000 != 0) << 3)
}
fn subtract_flags(sr: u32, left: u16, right: u16, set_x: bool) -> u32 {
    let result = left.wrapping_sub(right);
    let carry = left < right;
    let overflow = ((left ^ right) & (left ^ result) & 0x8000) != 0;
    let flags = u32::from(carry)
        | (u32::from(overflow) << 1)
        | (u32::from(result == 0) << 2)
        | (u32::from(result & 0x8000 != 0) << 3);
    (sr & if set_x { !31 } else { !15 }) | flags | if set_x { u32::from(carry) << 4 } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_effects_preserve_low_word_flags_and_require_real_reads() {
        for count in [0_u32, 1, 0x7fff, 0x8000, 0xffff] {
            let r = WaitRegisters {
                d0: 0x12340000 | count,
                d1: 0x5678ffff,
                sr: 0x271f,
                sp: 0xfff000,
            };
            let step = WaitStep::begin(Revision::Rev02, WaitRole::DecrementCount, r);
            let effect = step.finish().unwrap();
            assert_eq!(
                effect.registers.d0,
                0x12340000 | u32::from((count as u16).wrapping_sub(1))
            );
            assert_eq!(effect.registers.sr & 2 != 0, count == 0x8000);
            assert_eq!(effect.registers.sr & 16 != 0, count == 0);
        }
        let r = WaitRegisters {
            d0: 0,
            d1: 0,
            sr: 0x271f,
            sp: 0xfff000,
        };
        let mut step = WaitStep::begin(Revision::Rev00, WaitRole::TestDisplay, r);
        let read = step.expected_read().unwrap();
        step.acknowledge_read(read, 0x40).unwrap();
        assert_eq!(step.finish().unwrap().registers.sr, r.sr & !4);
        assert!(
            WaitStep::begin(Revision::Rev02, WaitRole::LoadCount, r)
                .finish()
                .is_err()
        );
    }
}
