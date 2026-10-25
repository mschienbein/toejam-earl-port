//! TJE neutral passive RAM/DMA ownership metadata; no fixture or oracle.
//! The physical queue remains unchanged. Opaque ids bind metadata to the actual
//! queued control word; clone/decode retain no callbacks or origin registry.
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Actor {
    M68k,
    Z80,
    #[default]
    Unknown,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandOwnership {
    Owned,
    Foreign,
    Unknown,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferEnd {
    Completed,
    Cancelled,
    ResetInvalidated,
    Discarded,
}

pub trait Observer: std::fmt::Debug + Send + Sync {
    fn ram_committed(&self, address: u32, word: bool, actor: Actor);
    fn before_semantic_ram_read(&self, address: u32, word: bool);
    fn command_ownership(&self, actor: Actor) -> CommandOwnership;
    fn scope_epoch(&self) -> u64;
    fn command_queued(&self, epoch: u64);
    fn command_retired(&self, epoch: u64, cancelled: bool);
    fn missing_command_origin(&self, epoch: u64);
    fn dma_started(&self, epoch: u64, generation: u64, uncertain: bool);
    fn before_dma_ram_read(&self, epoch: u64, address: u32, generation: u64, uncertain: bool);
    fn dma_retired(&self, epoch: u64, generation: u64, end: TransferEnd);
}

#[derive(Debug, Default, jgenesis_proc_macros::FakeEncode, jgenesis_proc_macros::FakeDecode)]
pub struct ObserverSlot {
    observer: Option<Arc<dyn Observer>>,
    dispatch_actor: Actor,
}
impl Clone for ObserverSlot {
    fn clone(&self) -> Self {
        Self::default()
    }
}
impl ObserverSlot {
    pub fn replace(&mut self, observer: Option<Arc<dyn Observer>>) -> Self {
        std::mem::replace(self, Self { observer, dispatch_actor: Actor::Unknown })
    }
    pub fn restore(&mut self, previous: Self) {
        *self = previous;
    }
    pub fn set_dispatch_actor(&mut self, actor: Actor) {
        self.dispatch_actor = actor;
    }
    pub fn take_dispatch_actor(&mut self) -> Actor {
        std::mem::take(&mut self.dispatch_actor)
    }
    pub fn ram_committed(&self, address: u32, word: bool, actor: Actor) {
        if let Some(observer) = &self.observer {
            observer.ram_committed(address, word, actor);
        }
    }
    pub fn before_semantic_ram_read(&self, address: u32, word: bool) {
        if let Some(observer) = &self.observer {
            observer.before_semantic_ram_read(address, word);
        }
    }
    pub fn command_origin(&self, actor: Actor) -> CommandOrigin {
        let Some(observer) = &self.observer else { return CommandOrigin::default() };
        match observer.command_ownership(actor) {
            CommandOwnership::Foreign => CommandOrigin::default(),
            ownership => CommandOrigin {
                observer: Some(Arc::clone(observer)),
                epoch: observer.scope_epoch(),
                queued: false,
                uncertain: ownership == CommandOwnership::Unknown,
            },
        }
    }
}

#[derive(Debug, Default, jgenesis_proc_macros::FakeEncode, jgenesis_proc_macros::FakeDecode)]
pub struct CommandOrigin {
    observer: Option<Arc<dyn Observer>>,
    epoch: u64,
    queued: bool,
    uncertain: bool,
}
impl Clone for CommandOrigin {
    fn clone(&self) -> Self {
        Self::default()
    }
}
impl CommandOrigin {
    fn queued(mut self) -> Self {
        if !self.queued
            && let Some(observer) = &self.observer
        {
            observer.command_queued(self.epoch);
            self.queued = true;
        }
        self
    }
    pub(crate) fn consume_queue(&mut self) {
        if self.queued {
            self.observer
                .as_ref()
                .expect("queued command owner")
                .command_retired(self.epoch, false);
            self.queued = false;
        }
    }
    pub(crate) fn start(mut self, generation: u64) -> DmaLease {
        self.consume_queue();
        if let Some(observer) = &self.observer {
            observer.dma_started(self.epoch, generation, self.uncertain);
        }
        DmaLease {
            observer: self.observer.take(),
            epoch: self.epoch,
            generation,
            uncertain: self.uncertain,
        }
    }
}
impl Drop for CommandOrigin {
    fn drop(&mut self) {
        if self.queued {
            self.observer.as_ref().expect("queued command owner").command_retired(self.epoch, true);
        }
    }
}

/// An id without its original registry never authenticates an origin.
#[derive(
    Debug, Default, Clone, Copy, jgenesis_proc_macros::FakeEncode, jgenesis_proc_macros::FakeDecode,
)]
pub(crate) struct OriginTag(u64);
impl PartialEq for OriginTag {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
impl Eq for OriginTag {}
static NEXT_ORIGIN: AtomicU64 = AtomicU64::new(1);
#[derive(Debug, jgenesis_proc_macros::FakeEncode, jgenesis_proc_macros::FakeDecode)]
pub(crate) struct QueuedOrigins {
    entries: Vec<(u64, Option<CommandOrigin>)>,
}
impl Default for QueuedOrigins {
    fn default() -> Self {
        // Match the physical pending-write queue's startup reservation. This
        // is allocation capacity only, never a hardware or source-work bound.
        Self { entries: Vec::with_capacity(16) }
    }
}
impl Clone for QueuedOrigins {
    fn clone(&self) -> Self {
        Self::default()
    }
}
impl QueuedOrigins {
    pub(crate) fn put(&mut self, origin: CommandOrigin) -> OriginTag {
        if origin.observer.is_none() {
            return OriginTag::default();
        }
        let id = match NEXT_ORIGIN
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        {
            Ok(id) => id,
            Err(_) => {
                origin
                    .observer
                    .as_ref()
                    .expect("owned origin")
                    .missing_command_origin(origin.epoch);
                return OriginTag::default();
            }
        };
        let origin = origin.queued();
        if let Some(entry) = self.entries.iter_mut().find(|(_, value)| value.is_none()) {
            *entry = (id, Some(origin));
        } else {
            self.entries.push((id, Some(origin)));
        }
        OriginTag(id)
    }
    pub(crate) fn take(&mut self, tag: OriginTag, current: &CommandOrigin) -> CommandOrigin {
        if tag.0 == 0 {
            return CommandOrigin::default();
        }
        if let Some((_, origin)) = self.entries.iter_mut().find(|(id, _)| *id == tag.0)
            && let Some(origin) = origin.take()
        {
            return origin;
        }
        if let Some(observer) = &current.observer {
            observer.missing_command_origin(current.epoch);
        }
        CommandOrigin::default()
    }
}

#[derive(Debug, Default, jgenesis_proc_macros::FakeEncode, jgenesis_proc_macros::FakeDecode)]
pub(crate) struct DmaLease {
    observer: Option<Arc<dyn Observer>>,
    epoch: u64,
    generation: u64,
    uncertain: bool,
}
impl Clone for DmaLease {
    fn clone(&self) -> Self {
        Self::default()
    }
}
impl DmaLease {
    pub(crate) fn before_ram_read(&self, address: u32) {
        if let Some(observer) = &self.observer {
            observer.before_dma_ram_read(self.epoch, address, self.generation, self.uncertain);
        }
    }
    pub(crate) fn retire(&mut self, end: TransferEnd) {
        if let Some(observer) = self.observer.take() {
            observer.dma_retired(self.epoch, self.generation, end);
        }
    }
}
impl Drop for DmaLease {
    fn drop(&mut self) {
        self.retire(TransferEnd::Discarded);
    }
}

#[derive(Debug, Default, jgenesis_proc_macros::FakeEncode, jgenesis_proc_macros::FakeDecode)]
pub(crate) struct DmaGeneration(u64);
impl Clone for DmaGeneration {
    fn clone(&self) -> Self {
        Self::default()
    }
}
impl DmaGeneration {
    pub(crate) fn started(&mut self) {
        self.0 = self.0.saturating_add(1);
    }
    pub(crate) fn value(&self) -> u64 {
        self.0
    }
}
