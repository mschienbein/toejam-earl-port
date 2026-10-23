//! Per-instruction source facade: no provider-owned CPU pipeline or clock, no
//! raw bus escape, and no fabricated acknowledgement values.
use super::*;
use crate::{Bus, ReadKind};
use tje_engine_core::{
    ir::Width,
    timing_region::{AccessKind, BusObservation, EventToken, ExpectedAccess},
};

#[derive(Default)]
pub(super) struct SourceEvidence {
    pub owner: Option<CursorIdentity>,
    pub bus_effects: u32,
    pub phase_updates: u32,
    pub failed: bool,
}
pub struct IsrSourceContext<'a> {
    cpu: &'a mut CpuState,
    bus: &'a mut dyn Bus,
    program: &'a NativeIsrProgram,
    scratch: &'a mut TimingScratch,
    evidence: &'a mut SourceEvidence,
}
impl<'a> IsrSourceContext<'a> {
    pub(super) fn new(
        cpu: &'a mut CpuState,
        bus: &'a mut dyn Bus,
        program: &'a NativeIsrProgram,
        scratch: &'a mut TimingScratch,
        evidence: &'a mut SourceEvidence,
    ) -> Self {
        Self {
            cpu,
            bus,
            program,
            scratch,
            evidence,
        }
    }
    pub fn state(&self) -> &CpuState {
        self.cpu
    }
    /// Splits disjoint borrows so the caller can drive its real bus effects
    /// while Core retains an immutable actual-entry snapshot. Exactly one
    /// admission per callback; no seeking a different plan after an error.
    pub fn admit_source<'b>(
        &'b mut self,
        entry: &'b CpuState,
    ) -> Result<(TimingCursor<'b>, IsrEffects<'b>), TimingError> {
        if self.evidence.owner.is_some() || entry != &*self.cpu || entry.address_error != 0 {
            self.evidence.failed = true;
            return Err(TimingError::WrongEntry);
        }
        let instruction = self
            .program
            .instructions
            .get(&entry.pc)
            .ok_or(TimingError::MissingIsrCapability)?;
        if instruction.opcode() == 0x4e73 {
            self.evidence.failed = true;
            return Err(TimingError::WrongToken);
        }
        let cursor = TimingCursor::begin(instruction, entry, self.scratch)?;
        self.evidence.owner = Some(cursor.identity());
        Ok((
            cursor,
            IsrEffects {
                cpu: &mut *self.cpu,
                bus: &mut *self.bus,
                evidence: &mut *self.evidence,
            },
        ))
    }
}

pub struct IsrEffects<'a> {
    cpu: &'a mut CpuState,
    bus: &'a mut dyn Bus,
    evidence: &'a mut SourceEvidence,
}
impl IsrEffects<'_> {
    fn fail<T>(&mut self, error: TimingError) -> Result<T, TimingError> {
        self.evidence.failed = true;
        Err(error)
    }
    fn access(
        &mut self,
        cursor: &TimingCursor<'_>,
        token: &EventToken,
    ) -> Result<ExpectedAccess, TimingError> {
        if self.evidence.failed || self.evidence.owner != Some(cursor.identity()) {
            return self.fail(TimingError::WrongToken);
        }
        match cursor.pending_access(token) {
            Ok(expected) => Ok(expected),
            Err(e) => self.fail(e),
        }
    }
    fn acknowledge(
        &mut self,
        cursor: &mut TimingCursor<'_>,
        token: EventToken,
        observation: BusObservation,
    ) -> Result<(), TimingError> {
        if let Err(error) = cursor.acknowledge_bus(token, observation) {
            return self.fail(error);
        }
        self.evidence.bus_effects = self
            .evidence
            .bus_effects
            .checked_add(1)
            .ok_or(TimingError::Exhausted)?;
        Ok(())
    }
    pub fn read(
        &mut self,
        cursor: &mut TimingCursor<'_>,
        token: EventToken,
        address: u32,
        width: Width,
        kind: ReadKind,
    ) -> Result<u32, TimingError> {
        let expected = self.access(cursor, &token)?;
        if expected.kind != AccessKind::Read(kind)
            || expected.width != width
            || expected.address & 0xffffff != address & 0xffffff
        {
            return self.fail(TimingError::WrongObservation);
        }
        let value = match width {
            Width::Byte => u32::from(self.bus.read8(expected.address, kind)),
            Width::Word => u32::from(self.bus.read16(expected.address, kind)),
        };
        self.acknowledge(
            cursor,
            token,
            BusObservation {
                address: expected.address,
                width,
                kind: expected.kind,
                value,
            },
        )?;
        Ok(value)
    }
    /// Pipeline infrastructure, not a source semantic value. The actual fetched
    /// word is fed to Core automatically and cannot be replaced by the provider.
    pub fn prefetch(
        &mut self,
        cursor: &mut TimingCursor<'_>,
        token: EventToken,
    ) -> Result<(), TimingError> {
        let expected = self.access(cursor, &token)?;
        if expected.kind != AccessKind::Read(ReadKind::Prefetch) || expected.width != Width::Word {
            return self.fail(TimingError::WrongObservation);
        }
        let value = u32::from(self.bus.read16(expected.address, ReadKind::Prefetch));
        self.acknowledge(
            cursor,
            token,
            BusObservation {
                address: expected.address,
                width: Width::Word,
                kind: expected.kind,
                value,
            },
        )
    }
    pub fn write(
        &mut self,
        cursor: &mut TimingCursor<'_>,
        token: EventToken,
        address: u32,
        width: Width,
        value: u32,
    ) -> Result<(), TimingError> {
        let expected = self.access(cursor, &token)?;
        if expected.kind != AccessKind::Write
            || expected.width != width
            || expected.address & 0xffffff != address & 0xffffff
            || value
                > match width {
                    Width::Byte => 255,
                    Width::Word => 65535,
                }
        {
            return self.fail(TimingError::WrongObservation);
        }
        match width {
            Width::Byte => self.bus.write8(expected.address, value as u8),
            Width::Word => self.bus.write16(expected.address, value as u16),
        }
        self.acknowledge(
            cursor,
            token,
            BusObservation {
                address: expected.address,
                width,
                kind: AccessKind::Write,
                value,
            },
        )
    }
    pub fn read_offset(
        &mut self,
        cursor: &mut TimingCursor<'_>,
        token: EventToken,
    ) -> Result<(), TimingError> {
        if self.evidence.failed || self.evidence.owner != Some(cursor.identity()) {
            return self.fail(TimingError::WrongToken);
        }
        let clocks = match cursor.pending_read_offset(&token) {
            Ok(clocks) => clocks,
            Err(e) => return self.fail(e),
        };
        self.bus.set_read_offset_mclk(clocks);
        if let Err(e) = cursor.acknowledge_offset(token) {
            return self.fail(e);
        }
        self.evidence.phase_updates = self
            .evidence
            .phase_updates
            .checked_add(1)
            .ok_or(TimingError::Exhausted)?;
        Ok(())
    }
    pub fn set_data(&mut self, register: u8, value: u32) -> Result<(), TimingError> {
        if register >= 8 {
            return self.fail(TimingError::WrongObservation);
        }
        self.cpu.d[usize::from(register)] = value;
        Ok(())
    }
    pub fn set_address(&mut self, register: u8, value: u32) -> Result<(), TimingError> {
        if register >= 8 {
            return self.fail(TimingError::WrongObservation);
        }
        self.cpu.write_address(register, value);
        Ok(())
    }
    pub fn set_status(&mut self, value: u32) {
        self.cpu.set_sr(value);
    }
    pub fn set_user_stack(&mut self, value: u32) {
        self.cpu.usp = value;
    }
    pub fn set_supervisor_stack(&mut self, value: u32) {
        self.cpu.ssp = value;
    }
}
