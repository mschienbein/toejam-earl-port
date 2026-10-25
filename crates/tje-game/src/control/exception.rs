//! Typed synchronous exception requests. These are source-bound DATA, not
//! exception-entry receipts or permission to execute an arbitrary vector.
use super::{ControlDataError, ControlSite};
use crate::{FunctionId, exits::StackBanks, ids};
use tje_mem::Revision;

/// Standard six-byte 68000 trap frames. Address/bus errors have a different
/// format and are deliberately NOT encoded as a standard trap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SynchronousTrap {
    IllegalInstruction,
    DivideByZero,
    Check,
    Overflow,
    PrivilegeViolation,
    Trace,
    LineA,
    LineF,
    Software(u8),
}
impl SynchronousTrap {
    pub const fn vector(self) -> Option<u8> {
        Some(match self {
            Self::IllegalInstruction => 4,
            Self::DivideByZero => 5,
            Self::Check => 6,
            Self::Overflow => 7,
            Self::PrivilegeViolation => 8,
            Self::Trace => 9,
            Self::LineA => 10,
            Self::LineF => 11,
            Self::Software(number) if number < 16 => 32 + number,
            Self::Software(_) => return None,
        })
    }
}

/// Bind the source-produced state before any exception-entry effect. The
/// changed runtime registers are explicit; untouched D/A remain the context's
/// own values and are compared under the transfer's full-register contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExceptionPreparedState {
    pub data: [u32; 3],
    pub address: [u32; 2],
    pub stack_pointer: u32,
    pub stack_banks: StackBanks,
    pub status: u16,
    pub saved_d2: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExceptionRequest {
    pub site: ControlSite,
    /// Original saved next/fault PC, not a caller-chosen handler destination.
    pub saved_pc: u32,
    pub trap: SynchronousTrap,
    pub prepared: ExceptionPreparedState,
}

/// G src/asm/runtime.s:49,114,213, independently checked in both matching ELFs.
/// All four zero paths fault on DIVU.W D2,D1; these are instruction offsets,
/// never transcribed cartridge data or a generic exception permission table.
pub const fn runtime_divide_fault_offset(entry: FunctionId) -> Option<i32> {
    if entry.0 == ids::LMODT.0 {
        Some(0x24)
    } else if entry.0 == ids::LDIVT.0 {
        Some(0x26)
    } else if entry.0 == ids::ULMODT.0 {
        Some(0x22)
    } else if entry.0 == ids::ULDIVT.0 {
        Some(0x1a)
    } else {
        None
    }
}
impl ExceptionRequest {
    pub fn runtime_divide(
        entry: FunctionId,
        revision: Revision,
        prepared: ExceptionPreparedState,
    ) -> Option<Self> {
        let site = ControlSite {
            entry,
            byte_offset: runtime_divide_fault_offset(entry)?,
        };
        Some(Self {
            site,
            saved_pc: site.address(revision).wrapping_add(2),
            trap: SynchronousTrap::DivideByZero,
            prepared,
        })
    }
    pub fn validate_shape(&self, revision: Revision) -> Result<(), ControlDataError> {
        self.site.validate_shape()?;
        if self.trap.vector().is_none()
            || self.saved_pc & 1 != 0
            || self.prepared.status & !0xa71f != 0
        {
            return Err(ControlDataError::Exception);
        }
        let active = if self.prepared.status & 0x2000 != 0 {
            self.prepared.stack_banks.supervisor
        } else {
            self.prepared.stack_banks.user
        };
        if active != self.prepared.stack_pointer {
            return Err(ControlDataError::Exception);
        }
        // This raw source shape describes ONLY the reviewed four DIVU sites.
        // Other standard kinds cannot borrow it; success is not live authority.
        if self.trap != SynchronousTrap::DivideByZero
            || runtime_divide_fault_offset(self.site.entry) != Some(self.site.byte_offset)
            || self.saved_pc != self.site.address(revision).wrapping_add(2)
            || self.prepared.data[2] != 0
            || self.prepared.data[1] > 0xffff
            || self.prepared.status & 0xf != 0
            || self.prepared.status & 0x8000 != 0
            || self.prepared.address[1] != self.prepared.address[1] as i16 as i32 as u32
        {
            return Err(ControlDataError::Exception);
        }
        let magnitude = (self.prepared.data[1] << 16) | (self.prepared.address[1] & 0xffff);
        if self.site.entry == ids::LMODT || self.site.entry == ids::LDIVT {
            let negative = self.prepared.data[0] == u32::MAX;
            if !matches!(self.prepared.data[0], 0 | u32::MAX)
                || self.prepared.address[0] != self.prepared.data[0]
                || (!negative && magnitude > i32::MAX as u32)
                || (negative
                    && (magnitude == 0
                        || magnitude > 0x80000000
                        || self.prepared.status & 0x10 == 0))
            {
                return Err(ControlDataError::Exception);
            }
        } else if self.prepared.data[0] != magnitude
            || self.prepared.address[0] != u32::from(self.site.entry == ids::ULDIVT)
        {
            return Err(ControlDataError::Exception);
        }
        Ok(())
    }
}
