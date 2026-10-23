use anyhow::{Context, Result, anyhow};
use std::{
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, TryRecvError},
    },
    thread,
    time::Instant,
};
use tje_engine_cranelift::{CacheConfig, CompileProgress};
use tje_native::backend::NativeBackend;
use tje_rom::{Revision, Rom};

pub struct Loading {
    result: Receiver<Result<Box<NativeBackend>>>,
    progress: Arc<Mutex<CompileProgress>>,
    pub shown: CompileProgress,
    pub store: bool,
    pub revision: Revision,
    pub started: Instant,
}

impl Loading {
    pub fn start(rom: Rom, cache: CacheConfig, store: bool) -> Result<Self> {
        let revision = rom.revision();
        let progress = Arc::new(Mutex::new(CompileProgress::Lifting));
        let shared = progress.clone();
        let (sender, result) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("native-game-compiler".into())
            .spawn(move || {
                let mut last = None;
                let result = NativeBackend::load(rom, &cache, |update| {
                    // Lowering reports every instruction. Publish only observable
                    // percentage changes so the compiler does not flood the UI.
                    let key = progress_key(update);
                    if last != Some(key) {
                        if let Ok(mut value) = shared.lock() {
                            *value = update;
                        }
                        last = Some(key);
                    }
                })
                .map(Box::new);
                let _ = sender.send(result);
            })
            .context("start native compilation worker")?;
        Ok(Self {
            result,
            progress,
            shown: CompileProgress::Lifting,
            store,
            revision,
            started: Instant::now(),
        })
    }
    pub fn progress_changed(&mut self) -> bool {
        let Ok(value) = self.progress.lock() else {
            return false;
        };
        if self.shown == *value {
            return false;
        }
        self.shown = *value;
        true
    }
    pub fn poll(&self) -> Option<Result<Box<NativeBackend>>> {
        match self.result.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(anyhow!(
                "native compilation worker stopped before returning a game"
            ))),
        }
    }
}

fn progress_key(progress: CompileProgress) -> (u8, usize) {
    match progress {
        CompileProgress::Lifting => (0, 0),
        CompileProgress::CheckingCache => (1, 0),
        CompileProgress::Compiling { completed, total } => (
            2,
            completed
                .saturating_mul(100)
                .checked_div(total)
                .unwrap_or(0),
        ),
        CompileProgress::WritingCache => (3, 0),
        CompileProgress::Ready { .. } => (4, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compiler_progress_is_bounded_to_percent_changes() {
        assert_eq!(
            progress_key(CompileProgress::Compiling {
                completed: 1,
                total: 100_000
            }),
            (2, 0)
        );
        assert_eq!(
            progress_key(CompileProgress::Compiling {
                completed: 999,
                total: 100_000
            }),
            (2, 0)
        );
        assert_eq!(
            progress_key(CompileProgress::Compiling {
                completed: 100_000,
                total: 100_000
            }),
            (2, 100)
        );
        assert_eq!(
            progress_key(CompileProgress::Compiling {
                completed: 0,
                total: 0
            }),
            (2, 0)
        );
    }
}
