//! Per-depth affine owners. A real child never rebinds its in-flight parent.
use super::{CertifiedReturn, CompletionLedger, CpuState, Invocation, TimingError, game};
use crate::timing_region::{BoundarySignals, CallToken, TimingRegion};
use tje_game::FunctionId;

/// Runtime ownership, independent from the declared execution class and the
/// check-mode proof envelope. Neither hybrid profile grants readable coverage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallProfile {
    HostHybridReference,
    HostCheckOriginalSubstitution,
    NativeStrictReadable,
    NativeCompiledOriginalCallees,
}
impl CallProfile {
    pub const fn label(self) -> &'static str {
        match self {
            Self::HostHybridReference => "HostHybridReference",
            Self::HostCheckOriginalSubstitution => "HostCheckOriginalSubstitution",
            Self::NativeStrictReadable => "NativeStrictReadable",
            Self::NativeCompiledOriginalCallees => "NativeCompiledOriginalCallees",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ChildBinding {
    call: (tje_engine_core::timing_region::CursorIdentity, u64),
    entry: FunctionId,
    entry_sp: u32,
    return_pc: u32,
    profile: CallProfile,
    kind: crate::timing_region::CalleeKind,
}
#[derive(Debug, Default)]
struct ChildOwner {
    ledger: CompletionLedger,
    binding: Option<ChildBinding>,
    original_scratch: tje_engine_core::timing_region::TimingScratch,
}
#[derive(Debug, Default)]
pub struct CompletionOwners {
    root: CompletionLedger,
    children: Vec<ChildOwner>,
    depth: usize,
}
impl CompletionOwners {
    pub fn prepare_depth(&mut self, depth: usize) {
        self.children
            .resize_with(depth.max(self.children.len()), ChildOwner::default);
    }
    fn active(&self) -> &CompletionLedger {
        if self.depth == 0 {
            &self.root
        } else {
            &self.children[self.depth - 1].ledger
        }
    }
    fn active_mut(&mut self) -> &mut CompletionLedger {
        if self.depth == 0 {
            &mut self.root
        } else {
            &mut self.children[self.depth - 1].ledger
        }
    }
    pub fn depth(&self) -> usize {
        self.depth
    }
    pub fn begin_root_scope(&mut self) -> Result<(), TimingError> {
        if self.depth != 0 {
            self.fail();
            return Err(TimingError::WrongToken);
        }
        let result = self.root.change_scope();
        if result.is_err() {
            self.fail();
        }
        result
    }
    /// A public frame change may prepare the active unadmitted owner, never
    /// erase a running/pending one. Controller restoration uses its sealed edge.
    pub fn bind_frame(&mut self) -> Result<(), TimingError> {
        let result = self.active_mut().change_scope();
        if result.is_err() {
            self.fail();
        }
        result
    }
    pub fn protected_entry(&self) -> Option<FunctionId> {
        for child in self.children[..self.depth].iter().rev() {
            if let Some(entry) = child.ledger.protected_entry() {
                return Some(entry);
            }
            // Consuming the child's RTS does not finish its enclosing call:
            // fallible ceiling cleanup still belongs to this active binding.
            if let Some(binding) = child.binding {
                return Some(binding.entry);
            }
        }
        self.root.protected_entry()
    }
    pub fn fail(&self) {
        self.root.fail();
        for child in &self.children[..self.depth] {
            child.ledger.fail();
        }
    }
    /// An admitted RTS-only source chain cannot authorise a typed transfer.
    /// Preserve the first owned cause, including preboxed failures, and poison
    /// every owner before exposing the error to a caller that could catch it.
    pub fn protect_failure(&self, cause: tje_game::Error) -> tje_game::Error {
        match self.protected_entry() {
            Some(entry) => {
                self.fail();
                tje_game::Error::timing_execution_failure(entry, cause)
            }
            None => cause,
        }
    }
    pub fn begin(
        &mut self,
        entry: FunctionId,
        entry_sp: u32,
        region: &TimingRegion<'_>,
    ) -> Result<Invocation, TimingError> {
        if self.depth != 0
            && !self.children[self.depth - 1]
                .binding
                .is_some_and(|binding| {
                    binding.entry == entry
                        && binding.entry_sp == entry_sp
                        && binding.kind == crate::timing_region::CalleeKind::TimedRust
                })
        {
            self.fail();
            return Err(TimingError::WrongToken);
        }
        let result = self.active_mut().begin(entry, entry_sp, region);
        if result.is_err() {
            self.fail();
        }
        result
    }
    pub fn complete(
        &mut self,
        invocation: Invocation,
        completed: CertifiedReturn,
    ) -> Result<game::TimedExit, TimingError> {
        let result = self.active_mut().complete(invocation, completed);
        if result.is_err() {
            self.fail();
        }
        result
    }
    pub fn consume(
        &mut self,
        request: game::CompletionRequest,
        cpu: &CpuState,
    ) -> Result<game::Completion, TimingError> {
        let result = self.active_mut().consume(request, cpu);
        if result.is_err() {
            self.fail();
        }
        result
    }
    pub fn enter_child(
        &mut self,
        parent: &Invocation,
        token: &CallToken,
        entry: FunctionId,
        profile: CallProfile,
        kind: crate::timing_region::CalleeKind,
    ) -> Result<(), TimingError> {
        let allowed = match profile {
            CallProfile::HostHybridReference | CallProfile::HostCheckOriginalSubstitution => {
                matches!(
                    kind,
                    crate::timing_region::CalleeKind::TimedRust
                        | crate::timing_region::CalleeKind::ReferenceOriginalSubstitution
                )
            }
            CallProfile::NativeStrictReadable => {
                kind == crate::timing_region::CalleeKind::TimedRust
            }
            CallProfile::NativeCompiledOriginalCallees => matches!(
                kind,
                crate::timing_region::CalleeKind::TimedRust
                    | crate::timing_region::CalleeKind::CompiledOriginal
            ),
        };
        if !allowed {
            self.fail();
            return Err(TimingError::WrongToken);
        }
        if !self.active().admits_child(parent, token) {
            self.fail();
            return Err(TimingError::WrongToken);
        }
        if self.depth == self.children.len() {
            self.children.push(ChildOwner::default());
        }
        let child = &mut self.children[self.depth];
        if let Err(error) = child.ledger.change_scope() {
            self.fail();
            return Err(error);
        }
        child.binding = Some(ChildBinding {
            call: token.identity(),
            entry,
            entry_sp: token.entry_stack(),
            return_pc: token.return_pc(),
            profile,
            kind,
        });
        self.depth += 1;
        Ok(())
    }
    pub fn finish_child(
        &mut self,
        token: CallToken,
        cpu: &CpuState,
    ) -> Result<ReturnedCall, TimingError> {
        let Some(child) = self
            .depth
            .checked_sub(1)
            .map(|index| &mut self.children[index])
        else {
            self.fail();
            return Err(TimingError::WrongToken);
        };
        if !child.ledger.was_consumed()
            || !child.binding.is_some_and(|binding| {
                binding.call == token.identity()
                    && binding.entry_sp == token.entry_stack()
                    && binding.return_pc == token.return_pc()
            })
            || cpu.pc != token.return_pc()
            || cpu.read_address(7) != token.parent_stack()
            || cpu.prefetch != u32::from(token.return_prefetch())
        {
            self.fail();
            return Err(TimingError::WrongToken);
        }
        let binding = child.binding.expect("validated child binding");
        child.binding = None;
        self.depth -= 1;
        Ok(ReturnedCall {
            token,
            profile: binding.profile,
            kind: binding.kind,
            callee: binding.entry,
        })
    }
    pub fn begin_original(
        &mut self,
        token: &CallToken,
        cpu: &CpuState,
    ) -> Result<super::original::OriginalController, TimingError> {
        let Some(index) = self.depth.checked_sub(1) else {
            self.fail();
            return Err(TimingError::WrongToken);
        };
        let child = &mut self.children[index];
        let Some(binding) = child.binding else {
            self.fail();
            return Err(TimingError::WrongToken);
        };
        if binding.call != token.identity()
            || binding.profile == CallProfile::NativeStrictReadable
            || binding.kind == crate::timing_region::CalleeKind::TimedRust
            || binding.entry_sp != cpu.read_address(7)
            || cpu.pc != token.target()
            || cpu.address_error != 0
            || cpu.frozen != 0
        {
            self.fail();
            return Err(TimingError::WrongEntry);
        }
        let owner = child.original_scratch.claim_identity()?;
        let invocation = child
            .ledger
            .begin_identity(binding.entry, binding.entry_sp, owner)?;
        Ok(super::original::OriginalController::new(
            invocation,
            owner,
            binding.entry_sp,
            binding.return_pc,
            binding.profile,
        ))
    }
    pub fn complete_original(
        &mut self,
        controller: super::original::OriginalController,
        cpu: &CpuState,
        signals: BoundarySignals,
    ) -> Result<game::TimedExit, TimingError> {
        let result = controller.finish(cpu, signals);
        match result {
            Ok((invocation, completed)) => self.complete(invocation, completed),
            Err(cause) => {
                self.fail();
                Err(cause)
            }
        }
    }
}

/// Only the controller can construct this after the child's actual, single-use
/// RTS certificate was consumed. It cannot be cloned or replayed.
#[derive(Debug)]
pub struct ReturnedCall {
    token: CallToken,
    profile: CallProfile,
    kind: crate::timing_region::CalleeKind,
    callee: FunctionId,
}
impl ReturnedCall {
    pub fn profile(&self) -> CallProfile {
        self.profile
    }
    pub fn kind(&self) -> crate::timing_region::CalleeKind {
        self.kind
    }
    pub fn callee(&self) -> FunctionId {
        self.callee
    }
    pub fn separate_callee_proof_required(&self) -> bool {
        self.kind.separate_proof_required()
    }
    pub fn resume(
        self,
        parent: &mut TimingRegion<'_>,
        cpu: &CpuState,
        signals: BoundarySignals,
    ) -> Result<(), TimingError> {
        parent.finish_call(self.token, cpu, signals)
    }
    pub fn resume_original(
        self,
        parent: &mut super::original::OriginalController,
        cpu: &CpuState,
        signals: BoundarySignals,
    ) -> Result<(), TimingError> {
        parent.resume_call(self.token, cpu, signals)
    }
}
