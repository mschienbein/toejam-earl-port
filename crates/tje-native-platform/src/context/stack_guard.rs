use genesis_core::ram_dma_observer::{Actor, CommandOwnership, Observer, TransferEnd};
use std::sync::Arc;
use tje_game::stack_read::{DmaRetirement, GUARD_CAPACITY, Guard, Namespace};

pub(super) struct StackObserver {
    pub(super) guard: Arc<Guard>,
}
impl std::fmt::Debug for StackObserver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ReadableStackObserver")
    }
}
pub(super) struct Pool {
    pub(super) namespace: Arc<Namespace>,
    slots: [Arc<StackObserver>; GUARD_CAPACITY],
}
impl Pool {
    pub(super) fn new() -> Self {
        let namespace = Arc::new(Namespace::default());
        let slots = std::array::from_fn(|_| {
            Arc::new(StackObserver {
                guard: Arc::new(Guard::with_namespace(Arc::clone(&namespace))),
            })
        });
        Self { namespace, slots }
    }
    pub(super) fn acquire(
        &self,
        root: tje_game::FunctionId,
        entry_sp: u32,
    ) -> tje_game::Result<Arc<StackObserver>> {
        if let Some(error) = self.namespace.failure() {
            return Err(error.into());
        }
        self.slots
            .iter()
            .find(|slot| Arc::strong_count(slot) == 1)
            .map(Arc::clone)
            .ok_or_else(|| self.namespace.capacity_failure(root, entry_sp).into())
    }
    pub(super) fn finish_proof(&self) -> tje_game::Result<()> {
        for slot in &self.slots {
            slot.guard.finish_proof()?;
        }
        Ok(())
    }
}
impl Observer for StackObserver {
    fn ram_committed(&self, address: u32, word: bool, actor: Actor) {
        self.guard.commit(
            if word { address & !1 } else { address },
            if word { 2 } else { 1 },
            actor == Actor::M68k,
        );
    }
    fn before_semantic_ram_read(&self, address: u32, word: bool) {
        self.guard
            .semantic_ram_read(address, if word { 2 } else { 1 });
    }
    fn command_ownership(&self, actor: Actor) -> CommandOwnership {
        match self
            .guard
            .owns_dma_command(actor == Actor::Z80, actor == Actor::Unknown)
        {
            Some(true) => CommandOwnership::Owned,
            Some(false) => CommandOwnership::Unknown,
            None => CommandOwnership::Foreign,
        }
    }
    fn scope_epoch(&self) -> u64 {
        self.guard.epoch()
    }
    fn command_queued(&self, epoch: u64) {
        self.guard.command_queued(epoch);
    }
    fn command_retired(&self, epoch: u64, discarded: bool) {
        self.guard.command_retired(epoch, discarded);
    }
    fn missing_command_origin(&self, _epoch: u64) {
        self.guard.missing_dma_origin();
    }
    fn dma_started(&self, epoch: u64, generation: u64, uncertain: bool) {
        self.guard.dma_started(epoch, generation, uncertain);
    }
    fn before_dma_ram_read(&self, epoch: u64, address: u32, generation: u64, uncertain: bool) {
        self.guard.dma_read(epoch, generation, address, uncertain);
    }
    fn dma_retired(&self, epoch: u64, generation: u64, end: TransferEnd) {
        self.guard.dma_retired(
            epoch,
            generation,
            match end {
                TransferEnd::Completed => DmaRetirement::Completed,
                TransferEnd::Cancelled => DmaRetirement::Cancelled,
                TransferEnd::ResetInvalidated => DmaRetirement::ResetInvalidated,
                TransferEnd::Discarded => DmaRetirement::Discarded,
            },
        );
    }
}
