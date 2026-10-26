use super::{
    EnhancedInputs, EnhancedMode, MAX_DEPTH, context,
    resumable::{Action, Invocation},
    sidecar,
};
use crate::{Backend, Engine, MachineSnapshot, SnapshotError};
use bincode::{Decode, Encode};
use tje_game::exits::ExitContract;

// Schema 2 adds a deferred, snapshot-safe before-only original-entry bypass.
const STATE_VERSION: u32 = 2;
#[derive(Encode, Decode)]
struct State {
    version: u32,
    inputs: EnhancedInputs,
    interrupts: u64,
    pending: Vec<Invocation>,
    sidecars: Vec<sidecar::Image>,
}

fn invalid(error: impl std::fmt::Display) -> SnapshotError {
    SnapshotError::EnhancedState(error.to_string())
}

impl EnhancedMode {
    pub fn snapshot<B: Backend>(
        &self,
        engine: &Engine<B>,
    ) -> Result<MachineSnapshot, SnapshotError> {
        let mut snapshot = MachineSnapshot::default();
        self.snapshot_into(engine, &mut snapshot)?;
        Ok(snapshot)
    }

    /// Capture at a completed scheduler step. Reuses the machine-image buffer.
    pub fn snapshot_into<B: Backend>(
        &self,
        engine: &Engine<B>,
        snapshot: &mut MachineSnapshot,
    ) -> Result<(), SnapshotError> {
        if self.driver.profile.is_none() {
            return engine.snapshot_into(snapshot);
        }
        if self.failed {
            return Err(invalid(
                "reset or restore before capturing failed enhanced execution",
            ));
        }
        if Some(engine.devices().memory().revision()) != self.driver.revision {
            return Err(SnapshotError::RomMismatch);
        }
        // Borrow scheduler data: no per-frame clone of activation state.
        let state = (
            STATE_VERSION,
            self.inputs,
            self.interrupts,
            &self.pending,
            self.sidecars.images().map_err(invalid)?,
        );
        let payload = bincode::encode_to_vec(state, sidecar::codec()).map_err(invalid)?;
        if payload.len() > sidecar::MAX_STATE_BYTES {
            return Err(invalid("enhanced snapshot exceeds its size limit"));
        }
        engine.snapshot_into_with_profile(snapshot, self.profile_id())?;
        snapshot.seal_enhanced(&payload)
    }

    /// Decode both halves before any state mutation. A wrong profile, course,
    /// schema, ROM or malformed scheduler never partially restores the machine.
    pub fn restore<B: Backend>(
        &mut self,
        engine: &mut Engine<B>,
        snapshot: &MachineSnapshot,
    ) -> Result<(), SnapshotError> {
        if self.driver.profile.is_none() {
            return engine.restore(snapshot);
        }
        if Some(engine.devices().memory().revision()) != self.driver.revision {
            return Err(SnapshotError::RomMismatch);
        }
        let payload = snapshot.enhanced_payload()?;
        let (state, used): (State, _) =
            bincode::decode_from_slice(payload, sidecar::codec()).map_err(invalid)?;
        if used != payload.len() || state.version != STATE_VERSION {
            return Err(invalid(
                "enhanced snapshot payload version or extent does not match",
            ));
        }
        if state.pending.len() > usize::from(MAX_DEPTH) {
            return Err(invalid(
                "enhanced snapshot exceeds the activation depth limit",
            ));
        }
        for invocation in &state.pending {
            if self
                .driver
                .resumable
                .binary_search_by_key(&invocation.pc, |entry| entry.0)
                .is_err()
                || invocation.sp & 1 != 0
                || !(0xe00000..=0xfffffc).contains(&invocation.sp)
            {
                return Err(invalid("enhanced snapshot has an invalid resumable entry"));
            }
            context::validate_return(invocation.return_pc).map_err(invalid)?;
            match invocation.action {
                Action::Original { pc, sp, .. } => {
                    let id =
                        tje_game::function_at(self.driver.revision.expect("selected revision"), pc)
                            .ok_or_else(|| {
                                invalid("enhanced snapshot has an unknown original entry")
                            })?;
                    let bytes = 4 * (u32::from(id.metadata().abi.argument_count) + 1);
                    if tje_game::exits::contract(id) != ExitContract::Rts
                        || sp < 0xe00000
                        || sp.checked_add(bytes) != Some(invocation.sp)
                    {
                        return Err(invalid(
                            "enhanced snapshot has an invalid original call frame",
                        ));
                    }
                }
                Action::Interrupt(after) if after > state.interrupts => {
                    return Err(invalid("enhanced snapshot has a future interrupt epoch"));
                }
                _ => {}
            }
        }
        let sidecars = self.sidecars.prepare(state.sidecars).map_err(invalid)?;
        engine.restore_enhanced_checked(snapshot, self.profile_id(), |hardware| {
            self.sidecars
                .validate_restore(&sidecars, &hardware)
                .map_err(|error| error.to_string())
        })?;
        self.sidecars.commit(sidecars);
        self.inputs = state.inputs;
        self.interrupts = state.interrupts;
        self.pending = state.pending;
        self.failed = false;
        self.started = true;
        Ok(())
    }
}
