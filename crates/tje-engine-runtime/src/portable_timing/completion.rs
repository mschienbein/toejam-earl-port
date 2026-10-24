//! Platform-owned, affine completion authority. Source values are expectations.
use super::{BoundProgram, CpuState, TimingError, game};
use crate::timing_region::{BoundaryAction, BoundarySignals, TimingRegion, classify_boundary};
use std::sync::atomic::{AtomicBool, Ordering};
use tje_engine_core::timing_region::{CursorIdentity, ScratchIdentity, TimingScratch};
use tje_game::FunctionId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Boundary {
    pc: u32,
    prefetch: u32,
    user: u32,
    supervisor: u32,
    status: u32,
    d0: u32,
    pending: u32,
    stopped: u32,
    frozen: u32,
    fault: u32,
}
impl Boundary {
    fn capture(cpu: &CpuState) -> Self {
        Self {
            pc: cpu.pc,
            prefetch: cpu.prefetch,
            user: cpu.usp,
            supervisor: cpu.ssp,
            status: cpu.sr,
            d0: cpu.d[0],
            pending: cpu.pending_interrupt,
            stopped: cpu.stopped,
            frozen: cpu.frozen,
            fault: cpu.address_error,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InvocationKey {
    ledger: ScratchIdentity,
    scope: u64,
    region: CursorIdentity,
    entry: FunctionId,
    entry_sp: u32,
}
#[derive(Debug)]
pub struct Invocation {
    key: InvocationKey,
}
/// Minted only after actual return, post-return hardware admission and exact
/// quiescent boundary validation for the invocation branded BEFORE its effects.
#[derive(Debug)]
pub struct CertifiedReturn {
    key: InvocationKey,
    exit: game::TimedExit,
    boundary: Boundary,
}
pub(super) fn certify_original_return(
    invocation: &Invocation,
    owner: CursorIdentity,
    return_pc: u32,
    cpu: &CpuState,
    signals: BoundarySignals,
) -> Result<CertifiedReturn, TimingError> {
    if invocation.key.region != owner
        || cpu.pc != return_pc
        || cpu.read_address(7) != invocation.key.entry_sp.wrapping_add(4)
        || classify_boundary(cpu, signals) != BoundaryAction::Ready
        || cpu.address_error != 0
        || cpu.frozen != 0
    {
        return Err(TimingError::WrongToken);
    }
    let exit = game::TimedExit {
        entry: invocation.key.entry,
        program_counter: cpu.pc,
        stack_pointer: cpu.read_address(7),
        value: invocation
            .key
            .entry
            .metadata()
            .abi
            .returns_value
            .then_some(cpu.d[0]),
    };
    Ok(CertifiedReturn {
        key: invocation.key,
        exit,
        boundary: Boundary::capture(cpu),
    })
}
impl BoundProgram {
    pub fn certify_return(
        &self,
        region: &TimingRegion<'_>,
        invocation: &Invocation,
        cpu: &CpuState,
        exit: game::TimedExit,
        signals: BoundarySignals,
    ) -> Result<CertifiedReturn, TimingError> {
        let usage = region.usage();
        if invocation.key.region != region.identity()
            || invocation.key.entry != self.source.entry.function
            || !region.belongs_to(&self.timing)
            || !region.is_complete()
            || classify_boundary(cpu, signals) != BoundaryAction::Ready
            || usage.instructions == 0
            || usage.scheduler_steps == 0
            || usage.master_clocks == 0
            || exit.entry != invocation.key.entry
            || exit.program_counter != region.return_target().pc
            || cpu.pc != exit.program_counter
            || cpu.read_address(7) != invocation.key.entry_sp.wrapping_add(4)
            || cpu.read_address(7) != exit.stack_pointer
            || cpu.address_error != 0
            || cpu.frozen != 0
            || exit.value.is_some_and(|value| value != cpu.d[0])
        {
            return Err(TimingError::WrongToken);
        }
        Ok(CertifiedReturn {
            key: invocation.key,
            exit,
            boundary: Boundary::capture(cpu),
        })
    }
}

/// Lazy unique scalar ledger identity, checked local scope serial, and actual
/// region brand. No per-entry allocation/global lock, cloning, or snapshot repair.
#[derive(Debug, Default)]
pub struct CompletionLedger {
    identity: Option<ScratchIdentity>,
    scope: u64,
    inflight: Option<InvocationKey>,
    pending: Option<CertifiedReturn>,
    consumed_scope: Option<u64>,
    admitted_entry: Option<FunctionId>,
    failed: AtomicBool,
}
impl CompletionLedger {
    pub fn change_scope(&mut self) -> Result<(), TimingError> {
        if self.failed.load(Ordering::Relaxed) || self.inflight.is_some() || self.pending.is_some()
        {
            self.failed.store(true, Ordering::Relaxed);
            return Err(TimingError::WrongToken);
        }
        self.scope = self.scope.checked_add(1).ok_or(TimingError::Exhausted)?;
        self.admitted_entry = None;
        Ok(())
    }
    /// Only an admitted outstanding/failed invocation protects a live frame.
    /// A successfully consumed invocation cannot taint unrelated ordinary errors.
    pub fn protected_entry(&self) -> Option<FunctionId> {
        if self.failed.load(Ordering::Relaxed) || self.inflight.is_some() || self.pending.is_some()
        {
            self.admitted_entry
        } else {
            None
        }
    }
    pub(super) fn admits_child(
        &self,
        invocation: &Invocation,
        token: &crate::timing_region::CallToken,
    ) -> bool {
        !self.failed.load(Ordering::Relaxed)
            && self.inflight == Some(invocation.key)
            && self.identity == Some(invocation.key.ledger)
            && self.scope == invocation.key.scope
            && invocation.key.region == token.identity().0
            && self.pending.is_none()
    }
    pub(super) fn was_consumed(&self) -> bool {
        !self.failed.load(Ordering::Relaxed)
            && self.inflight.is_none()
            && self.pending.is_none()
            && self.consumed_scope == Some(self.scope)
    }
    pub fn begin(
        &mut self,
        entry: FunctionId,
        entry_sp: u32,
        region: &TimingRegion<'_>,
    ) -> Result<Invocation, TimingError> {
        if region.usage().scheduler_steps != 0
            || region.usage().instructions != 0
            || region.entry_stack() != entry_sp
        {
            self.failed.store(true, Ordering::Relaxed);
            return Err(TimingError::WrongToken);
        }
        self.begin_identity(entry, entry_sp, region.identity())
    }
    pub(super) fn begin_identity(
        &mut self,
        entry: FunctionId,
        entry_sp: u32,
        region: CursorIdentity,
    ) -> Result<Invocation, TimingError> {
        if self.failed.load(Ordering::Relaxed)
            || self.inflight.is_some()
            || self.pending.is_some()
            || self.consumed_scope == Some(self.scope)
        {
            self.failed.store(true, Ordering::Relaxed);
            return Err(TimingError::WrongToken);
        }
        let ledger = match self.identity {
            Some(identity) => identity,
            None => {
                // Empty scratch constructs only a unique scalar, not a buffer.
                let identity = TimingScratch::default().identity()?;
                self.identity = Some(identity);
                identity
            }
        };
        let key = InvocationKey {
            ledger,
            scope: self.scope,
            region,
            entry,
            entry_sp,
        };
        self.inflight = Some(key);
        self.admitted_entry = Some(entry);
        Ok(Invocation { key })
    }
    pub fn fail(&self) {
        self.failed.store(true, Ordering::Relaxed);
    }
    pub fn complete(
        &mut self,
        invocation: Invocation,
        completed: CertifiedReturn,
    ) -> Result<game::TimedExit, TimingError> {
        let key = invocation.key;
        if self.failed.load(Ordering::Relaxed)
            || self.identity != Some(key.ledger)
            || self.scope != key.scope
            || self.inflight != Some(key)
            || self.pending.is_some()
            || completed.key != key
        {
            self.failed.store(true, Ordering::Relaxed);
            return Err(TimingError::WrongToken);
        }
        let exit = completed.exit;
        self.inflight = None;
        self.pending = Some(completed);
        Ok(exit)
    }
    pub fn consume(
        &mut self,
        request: game::CompletionRequest,
        cpu: &CpuState,
    ) -> Result<game::Completion, TimingError> {
        if self.failed.load(Ordering::Relaxed) || self.inflight.is_some() {
            self.failed.store(true, Ordering::Relaxed);
            return Err(TimingError::WrongToken);
        }
        let Some(pending) = self.pending.as_ref() else {
            return if self.consumed_scope == Some(self.scope) {
                self.failed.store(true, Ordering::Relaxed);
                Err(TimingError::WrongToken)
            } else {
                Ok(game::Completion::Absent)
            };
        };
        if self.identity != Some(pending.key.ledger)
            || pending.key.scope != self.scope
            || pending.key.entry != request.entry
            || pending.key.entry_sp != request.entry_sp
            || request.kind != game::CompletionKind::Rts
            || request
                .expected_pc
                .is_some_and(|pc| pc != pending.exit.program_counter)
            || request.value != pending.exit.value
            || Boundary::capture(cpu) != pending.boundary
        {
            self.failed.store(true, Ordering::Relaxed);
            return Err(TimingError::WrongToken);
        }
        self.pending = None;
        self.consumed_scope = Some(self.scope);
        Ok(game::Completion::Consumed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timing_region::{RegionStorage, ReturnTarget, ReviewedRegion, TimingProgram};
    use tje_game::ids;

    // Tests of ledger acceptance use private certificate fixtures, NOT a claim
    // that a snapshot can mint production authority. Integration proves minting.
    fn pending() -> (CompletionLedger, Invocation, CertifiedReturn, CpuState) {
        let mut rom = vec![0; 0x1002];
        rom[0x200..0x202].copy_from_slice(&0x4e71_u16.to_be_bytes());
        rom[0x1000..0x1002].copy_from_slice(&0x4e71_u16.to_be_bytes());
        let program = TimingProgram::compile(
            &rom,
            ReviewedRegion {
                calls: Vec::new(),
                name: "authored ledger fixture",
                source: "synthetic NOP, ledger protocol only",
                entry: 0x200,
                instruction_pcs: vec![0x200],
                edges: vec![],
                return_pc: None,
                maximum_steps: u64::MAX,
                required_interrupts: &[],
            },
            7,
        )
        .unwrap();
        let mut storage = RegionStorage::new(&program);
        let mut cpu = CpuState {
            pc: 0x200,
            prefetch: 0x4e71,
            sr: 0x2000,
            ssp: 0xff3ffc,
            usp: 0xff2800,
            ..CpuState::default()
        };
        let target = ReturnTarget::from_rom(&rom, 0x1000).unwrap();
        let region = TimingRegion::begin_oracle(&program, &cpu, target, &mut storage).unwrap();
        let mut ledger = CompletionLedger::default();
        let invocation = ledger
            .begin(ids::WAIT_FOR_NEXT_V_BLANK, cpu.ssp, &region)
            .unwrap();
        cpu.pc = target.pc;
        cpu.ssp += 4;
        let completed = CertifiedReturn {
            key: invocation.key,
            exit: game::TimedExit {
                entry: ids::WAIT_FOR_NEXT_V_BLANK,
                program_counter: cpu.pc,
                stack_pointer: cpu.ssp,
                value: None,
            },
            boundary: Boundary::capture(&cpu),
        };
        (ledger, invocation, completed, cpu)
    }
    fn request(cpu: &CpuState) -> game::CompletionRequest {
        game::CompletionRequest {
            entry: ids::WAIT_FOR_NEXT_V_BLANK,
            entry_sp: cpu.read_address(7).wrapping_sub(4),
            kind: game::CompletionKind::Rts,
            expected_pc: Some(cpu.pc),
            value: None,
        }
    }
    fn ready() -> (CompletionLedger, CpuState) {
        let (mut ledger, invocation, completed, cpu) = pending();
        ledger.complete(invocation, completed).unwrap();
        (ledger, cpu)
    }
    #[test]
    fn foreign_ledger_region_and_cross_machine_certificate_swaps_never_publish_pending() {
        let (_a, invocation_a, certificate_a, _) = pending();
        let (mut b, _invocation_b, _certificate_b, _) = pending();
        assert!(b.complete(invocation_a, certificate_a).is_err());
        assert!(b.pending.is_none());
        let (_a, _invocation_a, certificate_a, _) = pending();
        let (mut b, invocation_b, _certificate_b, _) = pending();
        assert!(b.complete(invocation_b, certificate_a).is_err());
        assert!(b.pending.is_none());
        let (mut same_ledger, invocation, mut wrong_region, _) = pending();
        let mut other_storage = TimingScratch::default();
        wrong_region.key.region = other_storage.claim_identity().unwrap();
        assert!(same_ledger.complete(invocation, wrong_region).is_err());
        assert!(same_ledger.pending.is_none());
    }
    #[test]
    fn ordinary_absence_is_not_authority_and_a_receipt_is_single_use() {
        let (mut ledger, cpu) = ready();
        assert_eq!(ledger.protected_entry(), Some(ids::WAIT_FOR_NEXT_V_BLANK));
        let mut empty = CompletionLedger::default();
        assert_eq!(
            empty.consume(request(&cpu), &cpu).unwrap(),
            game::Completion::Absent
        );
        assert_eq!(empty.protected_entry(), None);
        assert_eq!(
            ledger.consume(request(&cpu), &cpu).unwrap(),
            game::Completion::Consumed
        );
        assert_eq!(
            ledger.protected_entry(),
            None,
            "consumption ends active protection"
        );
        assert!(ledger.consume(request(&cpu), &cpu).is_err());
        assert_eq!(ledger.protected_entry(), Some(ids::WAIT_FOR_NEXT_V_BLANK));
    }
    #[test]
    fn stale_owner_same_id_scope_and_foreign_entry_kind_sp_pc_result_reject() {
        for variant in 0..6 {
            let (mut ledger, cpu) = ready();
            let mut expected = request(&cpu);
            match variant {
                0 => expected.entry = ids::HANDLE_V_BLANK,
                1 => expected.kind = game::CompletionKind::ExceptionReturn,
                2 => expected.entry_sp = expected.entry_sp.wrapping_sub(4),
                3 => expected.expected_pc = Some(cpu.pc + 2),
                4 => expected.value = Some(0),
                _ => assert!(ledger.change_scope().is_err()),
            }
            assert!(ledger.consume(expected, &cpu).is_err());
        }
    }
    #[test]
    fn reused_owner_keeps_identity_and_never_erases_pending_or_poison() {
        let (mut owner, cpu) = ready();
        let identity = owner.identity;
        let old_scope = owner.scope;
        assert_eq!(
            owner.consume(request(&cpu), &cpu).unwrap(),
            game::Completion::Consumed
        );
        owner.change_scope().unwrap();
        assert_eq!(owner.protected_entry(), None);
        assert_eq!(
            owner.admitted_entry, None,
            "clean new scopes discard no live authority"
        );
        assert_eq!(owner.identity, identity);
        assert_eq!(owner.scope, old_scope + 1);
        assert!(!owner.failed.load(Ordering::Relaxed));
        let (mut pending_owner, cpu) = ready();
        assert!(pending_owner.change_scope().is_err());
        assert!(
            pending_owner.pending.is_some(),
            "failed rebinding cannot erase authority"
        );
        assert!(pending_owner.consume(request(&cpu), &cpu).is_err());
        assert!(
            pending_owner.change_scope().is_err(),
            "poison survives owner reuse"
        );
    }
    #[test]
    fn changed_live_pipeline_banks_sr_d0_cannot_be_repaired() {
        for variant in 0..10 {
            let (mut ledger, cpu) = ready();
            let mut wrong = cpu.clone();
            match variant {
                0 => wrong.pc ^= 2,
                1 => wrong.prefetch ^= 1,
                2 => wrong.usp ^= 2,
                3 => wrong.ssp ^= 2,
                4 => wrong.sr ^= 1,
                5 => wrong.d[0] ^= 1,
                6 => wrong.pending_interrupt = 6,
                7 => wrong.stopped = 1,
                8 => wrong.frozen = 1,
                _ => wrong.address_error = 1,
            }
            let before = wrong.clone();
            assert!(ledger.consume(request(&cpu), &wrong).is_err());
            assert_eq!(wrong, before);
        }
    }
}
