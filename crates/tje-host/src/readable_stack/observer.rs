use crate::{HostError, Result};
use std::sync::Arc;
use tje_game::stack_read::{GUARD_CAPACITY, Guard, Namespace};

pub(crate) struct Observer {
    pub(crate) guard: Arc<Guard>,
}
impl std::fmt::Debug for Observer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ReadableStackObserver")
    }
}
pub(crate) struct Pool {
    pub(crate) namespace: Arc<Namespace>,
    slots: [Arc<Observer>; GUARD_CAPACITY],
}
impl Pool {
    pub(crate) fn new() -> Self {
        let namespace = Arc::new(Namespace::default());
        let slots = std::array::from_fn(|_| {
            Arc::new(Observer {
                guard: Arc::new(Guard::with_namespace(Arc::clone(&namespace))),
            })
        });
        Self { namespace, slots }
    }
    pub(crate) fn acquire(
        &self,
        root: tje_game::FunctionId,
        entry_sp: u32,
        analysis: bool,
    ) -> Result<Arc<Observer>> {
        if analysis && self.slots.iter().all(|slot| Arc::strong_count(slot) == 1) {
            self.namespace.reset_failure();
        }
        if let Some(error) = self.namespace.failure() {
            return Err(HostError::Translation(error.into()));
        }
        self.slots
            .iter()
            .find(|slot| Arc::strong_count(slot) == 1)
            .map(Arc::clone)
            .ok_or_else(|| {
                HostError::Translation(self.namespace.capacity_failure(root, entry_sp).into())
            })
    }
    pub(crate) fn finish_proof(&self) -> Result<()> {
        for slot in &self.slots {
            slot.guard
                .finish_proof()
                .map_err(|error| HostError::Translation(error.into()))?;
        }
        Ok(())
    }
}

#[cfg(feature = "ram-dma-observer")]
impl genesis_core::ram_dma_observer::Observer for Observer {
    fn ram_committed(
        &self,
        address: u32,
        word: bool,
        actor: genesis_core::ram_dma_observer::Actor,
    ) {
        self.guard.commit(
            if word { address & !1 } else { address },
            if word { 2 } else { 1 },
            actor == genesis_core::ram_dma_observer::Actor::M68k,
        );
    }
    fn before_semantic_ram_read(&self, address: u32, word: bool) {
        self.guard
            .semantic_ram_read(address, if word { 2 } else { 1 });
    }
    fn command_ownership(
        &self,
        actor: genesis_core::ram_dma_observer::Actor,
    ) -> genesis_core::ram_dma_observer::CommandOwnership {
        use genesis_core::ram_dma_observer::{Actor, CommandOwnership};
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
    fn dma_retired(
        &self,
        epoch: u64,
        generation: u64,
        end: genesis_core::ram_dma_observer::TransferEnd,
    ) {
        use genesis_core::ram_dma_observer::TransferEnd;
        use tje_game::stack_read::DmaRetirement;
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
