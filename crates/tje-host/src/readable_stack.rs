//! Readable-root stack guard, distinct from the original independence audit.
use crate::{Host, HostError, Result};
use serde::Serialize;
use std::sync::Arc;
use tje_game::stack_read::{Unsupported, UnsupportedReason};
mod admission;
pub use admission::DeclaredStackInput;
pub(crate) use admission::{AdmittedRead, pair, revalidate};
mod observer;
pub(crate) use observer::{Observer, Pool};
mod metrics;
pub use metrics::ReadableGuardMetrics;
mod original_root;
pub(crate) use original_root::{OriginalRootScope, OriginalRootWitness};
#[cfg(all(test, feature = "ram-dma-observer", panic = "unwind"))]
mod tests;

#[derive(Debug, Clone)]
pub(crate) struct ExplicitRead {
    pub declaration: tje_game::stack_read::UninitializedLocalRead,
    pub mask: u8,
    pub beats: [u16; 2],
    pub first_read: usize,
    pub ordinals: [usize; 2],
    pub beat_count: usize,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct UnsupportedStackRead {
    pub root: &'static str,
    pub entry_sp: u32,
    pub raw_address: u32,
    pub address: u32,
    pub width: u8,
    pub reader: &'static str,
    pub reason: &'static str,
}
impl From<Unsupported> for UnsupportedStackRead {
    fn from(error: Unsupported) -> Self {
        Self {
            root: error.root.metadata().name,
            entry_sp: error.entry_sp,
            raw_address: error.raw_address,
            address: error.address,
            width: error.width,
            reader: error.reader.as_str(),
            reason: error.reason.as_str(),
        }
    }
}
impl HostError {
    pub(crate) fn unsupported_stack_read(&self) -> Option<UnsupportedStackRead> {
        self.typed_stack_read().map(Into::into)
    }
    /// Return the original typed readable failure, including through a probe.
    pub fn typed_stack_read(&self) -> Option<Unsupported> {
        match self {
            Self::Translation(error) => error.unsupported_stack_read(),
            Self::DependencyProbe { cause } => cause.typed_stack_read(),
            _ => None,
        }
    }
}
impl Host {
    pub(crate) fn record_readable_stack_metrics(&mut self) {
        if let Some(pool) = &self.readable_stack {
            self.checks.readable_stack_guard = pool.namespace.metrics().into();
        }
    }
    pub(crate) fn readable_stack_for_root(
        &mut self,
        root: tje_game::FunctionId,
        entry_sp: u32,
    ) -> Result<Arc<Observer>> {
        if !cfg!(feature = "ram-dma-observer") {
            return Err(HostError::Translation(
                Unsupported::capability(root, entry_sp, UnsupportedReason::ObserverUnavailable)
                    .into(),
            ));
        }
        if !cfg!(panic = "unwind") {
            return Err(HostError::Translation(
                Unsupported::capability(root, entry_sp, UnsupportedReason::UnwindUnavailable)
                    .into(),
            ));
        }
        self.readable_stack
            .as_ref()
            .ok_or_else(|| {
                HostError::Translation(
                    Unsupported::capability(root, entry_sp, UnsupportedReason::ObserverUnavailable)
                        .into(),
                )
            })?
            .acquire(root, entry_sp, self.mode == crate::Mode::Check)
    }
}
