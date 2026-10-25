//! TJE neutral passive RAM/DMA observers, shared with retained device adapters.
//! No fixture, reset, original-read oracle or CPU authority is enabled here.
pub use genesis_components::ram_dma_observer::{
    Actor, CommandOrigin, CommandOwnership, Observer, ObserverSlot, TransferEnd,
};
