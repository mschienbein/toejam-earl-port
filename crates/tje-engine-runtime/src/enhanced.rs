//! Deliberately non-cycle-exact, opt-in gameplay replacements.
//!
//! This registry is separate from every proven/readable timing selection. The
//! ordinary `Machine::step` and `Machine::play_frame` paths never consult it.

mod context;
mod resumable;
mod sidecar;
mod snapshot;
pub use resumable::{
    Context, Continuation, EnhancedInputs, OverrideStep, ResumableTranslation, Wait,
};
pub use sidecar::{Sidecar, Sidecars};
#[cfg(test)]
mod tests;

use crate::{Backend, CpuState, DeviceStep, Devices, Engine, RuntimeStep, StepKind};
use genesis_config::GenesisInputs;
use std::fmt;
use tje_game::{Dispatcher, FramedTranslation, SelectedEntry, Translation};
use tje_mem::Revision;

/// A versioned, built-in mod selection. Change `id` whenever behaviour changes;
/// snapshots, replays and peers must agree on this identity as well as the ROM.
/// Only these explicit entries run as Rust; the proven tables are never included.
#[derive(Clone, Copy)]
pub struct EnhancedProfile {
    pub id: &'static str,
    pub translations: &'static [Translation],
    pub framed_translations: &'static [FramedTranslation],
    pub resumable_translations: &'static [ResumableTranslation],
    pub resumable_sets: &'static [&'static [ResumableTranslation]],
}

impl EnhancedProfile {
    pub const fn new(
        id: &'static str,
        translations: &'static [Translation],
        framed_translations: &'static [FramedTranslation],
    ) -> Self {
        Self {
            id,
            translations,
            framed_translations,
            resumable_translations: &[],
            resumable_sets: &[],
        }
    }

    pub const fn with_resumable(mut self, entries: &'static [ResumableTranslation]) -> Self {
        self.resumable_translations = entries;
        self
    }

    pub const fn with_resumable_sets(
        mut self,
        sets: &'static [&'static [ResumableTranslation]],
    ) -> Self {
        self.resumable_sets = sets;
        self
    }

    fn dispatcher(self) -> Dispatcher {
        Dispatcher::hybrid_with_frames(self.translations, self.framed_translations)
    }
}

#[derive(Debug)]
pub enum Error<E> {
    Backend(E),
    Override(tje_game::Error),
    Failed,
}
impl<E: fmt::Display> fmt::Display for Error<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Backend(error) => error.fmt(f),
            Self::Override(error) => write!(f, "enhanced override: {error}"),
            Self::Failed => f.write_str(
                "enhanced execution failed; reset or restore a valid save before continuing",
            ),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for Error<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Backend(error) => Some(error),
            Self::Override(error) => Some(error),
            Self::Failed => None,
        }
    }
}

/// Shared limits for a synchronous callback and its nested compiled calls.
/// TimeAware suspension instead uses explicit serializable scheduler activations.
const MAX_ORIGINAL_STEPS: u32 = 1_000_000;
const MAX_DEPTH: u8 = 32;
struct Budget {
    steps: u32,
    depth: u8,
    progress: DeviceStep,
}
impl Default for Budget {
    fn default() -> Self {
        Self {
            steps: MAX_ORIGINAL_STEPS,
            depth: 0,
            progress: DeviceStep::default(),
        }
    }
}
impl Budget {
    fn record(&mut self, step: DeviceStep) {
        self.progress.cpu_cycles = self.progress.cpu_cycles.saturating_add(step.cpu_cycles);
        self.progress.master_clocks += step.master_clocks;
    }
}

/// Explicit extra-owned state around the independent faithful machine.
#[derive(Default)]
pub struct EnhancedMode {
    driver: Driver,
    sidecars: Sidecars,
    inputs: EnhancedInputs,
    pending: Vec<resumable::Invocation>,
    interrupts: u64,
    failed: bool,
}

#[derive(Default)]
struct Driver {
    profile: Option<EnhancedProfile>,
    revision: Option<Revision>,
    entries: Vec<(u32, SelectedEntry)>,
    resumable: Vec<(u32, ResumableTranslation)>,
}

impl EnhancedMode {
    /// Validate immutable selection once, outside the instruction hot path.
    pub fn new(profile: EnhancedProfile, revision: Revision) -> tje_game::Result<Self> {
        use tje_game::{execution::ExecutionClass, exits::ExitContract};
        if profile.id.is_empty() || profile.id == "faithful" {
            return Err(context::error(
                "an enhanced profile needs a distinct versioned identity",
            ));
        }
        let dispatcher = profile.dispatcher();
        let mut entries =
            Vec::with_capacity(profile.translations.len() + profile.framed_translations.len());
        for id in profile
            .translations
            .iter()
            .map(|entry| entry.id)
            .chain(profile.framed_translations.iter().map(|entry| entry.id))
        {
            if tje_game::exits::contract(id) != ExitContract::Rts
                || !matches!(
                    tje_game::execution_class(id),
                    Some(ExecutionClass::Atomic | ExecutionClass::Functional)
                )
            {
                return Err(context::error(
                    "enhanced overrides require an Atomic/Functional RTS entry",
                ));
            }
            let entry = dispatcher
                .resolve_entry(id)?
                .ok_or(tje_game::Error::Untranslated(id))?;
            entries.push((id.metadata().address(revision), entry));
        }
        entries.sort_unstable_by_key(|entry| entry.0);
        if entries.windows(2).any(|pair| pair[0].0 == pair[1].0) {
            return Err(context::error(
                "enhanced profile has duplicate entry addresses",
            ));
        }
        let mut resumable = Vec::new();
        for entry in profile.resumable_translations.iter().chain(
            profile
                .resumable_sets
                .iter()
                .flat_map(|entries| entries.iter()),
        ) {
            if tje_game::exits::contract(entry.id) != ExitContract::Rts
                || tje_game::execution_class(entry.id).is_none()
            {
                return Err(context::error(
                    "resumable overrides require a classified RTS entry",
                ));
            }
            resumable.push((entry.id.metadata().address(revision), *entry));
        }
        resumable.sort_unstable_by_key(|entry| entry.0);
        if resumable.windows(2).any(|pair| pair[0].0 == pair[1].0)
            || resumable.iter().any(|entry| {
                entries
                    .binary_search_by_key(&entry.0, |entry| entry.0)
                    .is_ok()
            })
        {
            return Err(context::error(
                "enhanced profile has duplicate entry addresses",
            ));
        }
        if entries.is_empty() && resumable.is_empty() {
            return Err(context::error(
                "an enhanced profile must select at least one override",
            ));
        }
        Ok(Self {
            driver: Driver {
                profile: Some(profile),
                revision: Some(revision),
                entries,
                resumable,
            },
            ..Self::default()
        })
    }

    pub fn profile_id(&self) -> &'static str {
        self.driver.profile.map_or("faithful", |profile| profile.id)
    }

    pub fn is_failed(&self) -> bool {
        self.failed
    }

    pub fn sidecars(&self) -> &Sidecars {
        &self.sidecars
    }

    pub fn sidecars_mut(&mut self) -> &mut Sidecars {
        &mut self.sidecars
    }

    pub fn reset<B: Backend>(&mut self, engine: &mut Engine<B>) {
        engine.reset();
        self.failed = false;
        self.pending.clear();
        self.interrupts = 0;
        self.inputs = EnhancedInputs::default();
        self.sidecars.reset();
    }

    pub fn play_frame<'a, B: Backend>(
        &mut self,
        engine: &'a mut Engine<B>,
        inputs: impl Into<EnhancedInputs>,
    ) -> Result<&'a [[f64; 2]], Error<B::Error>>
    where
        B::Error: fmt::Display,
    {
        let inputs = inputs.into();
        if self.driver.profile.is_none() {
            return engine.play_frame(inputs.original()).map_err(Error::Backend);
        }
        engine.devices_mut().clear_audio();
        loop {
            if self.step(engine, inputs)?.devices.frame_complete {
                return Ok(engine.devices().audio_samples());
            }
        }
    }

    pub fn step<B: Backend>(
        &mut self,
        engine: &mut Engine<B>,
        inputs: impl Into<EnhancedInputs>,
    ) -> Result<RuntimeStep, Error<B::Error>>
    where
        B::Error: fmt::Display,
    {
        let inputs = inputs.into();
        if self.driver.profile.is_none() {
            return engine.step(inputs.original()).map_err(Error::Backend);
        }
        if self.failed {
            return Err(Error::Failed);
        }
        if Some(engine.devices().memory().revision()) != self.driver.revision {
            self.failed = true;
            return Err(Error::Override(context::error(
                "enhanced profile belongs to a different cartridge revision",
            )));
        }
        self.inputs = inputs;
        let result = self.scheduled_step(engine);
        if result.is_err() {
            self.failed = true;
        }
        result
    }
}

impl Driver {
    fn step_inner<B: Backend>(
        &self,
        engine: &mut Engine<B>,
        inputs: GenesisInputs,
        budget: &mut Budget,
    ) -> Result<RuntimeStep, Error<B::Error>>
    where
        B::Error: fmt::Display,
    {
        let selected = if ready(engine) {
            if self
                .resumable
                .binary_search_by_key(&engine.cpu().pc, |entry| entry.0)
                .is_ok()
            {
                return Err(Error::Override(context::error(
                    "a synchronous callback cannot enter a resumable override; use CallOriginal",
                )));
            }
            self.entries
                .binary_search_by_key(&engine.cpu().pc, |entry| entry.0)
                .ok()
                .map(|index| self.entries[index].1)
        } else {
            None
        };
        if let Some(entry) = selected {
            let before = budget.progress;
            let before_frame = engine.frame_count();
            self.invoke(engine, inputs, entry, budget)
                .map_err(Error::Override)?;
            return Ok(RuntimeStep {
                devices: DeviceStep {
                    cpu_cycles: budget.progress.cpu_cycles - before.cpu_cycles,
                    master_clocks: budget.progress.master_clocks - before.master_clocks,
                    // A nested original call may have completed the frame before
                    // the replacement's return. Do not lose that pacing event.
                    frame_complete: engine.frame_count() != before_frame,
                },
                kind: StepKind::EnhancedOverride {
                    function: entry.id(),
                },
                instruction_pc: None,
            });
        }
        if budget.steps == 0 {
            return Err(Error::Override(context::error(
                "enhanced original-call step limit exhausted",
            )));
        }
        budget.steps -= 1;
        let step = engine.step(inputs).map_err(Error::Backend)?;
        budget.record(step.devices);
        Ok(step)
    }

    fn invoke<B: Backend>(
        &self,
        engine: &mut Engine<B>,
        inputs: GenesisInputs,
        entry: SelectedEntry,
        budget: &mut Budget,
    ) -> tje_game::Result<()>
    where
        B::Error: fmt::Display,
    {
        if budget.depth >= MAX_DEPTH {
            return Err(context::error("enhanced callback nesting limit exhausted"));
        }
        let profile = self.profile.expect("selected enhanced entry has a profile");
        let sp = engine.cpu().read_address(7);
        context::validate_stack(engine, sp)?;
        let return_pc = tje_mem::Bus::read_u32(engine.devices().memory(), sp);
        context::validate_return(return_pc)?;
        let pc = engine.cpu().pc;
        engine.devices_mut().set_inputs(inputs);
        engine.cpu_mut().address_error = 0;
        engine.cpu_mut().last_muldiv = 0;
        budget.depth += 1;
        let result = {
            let mut context =
                context::Context::new(self, engine, inputs, profile.dispatcher(), sp, budget);
            entry.invoke_prepared(&mut context)
        };
        budget.depth -= 1;
        let value = result?;
        if let Some(value) = value {
            engine.cpu_mut().d[0] = value;
        }
        if engine.cpu().read_address(7) != sp {
            return Err(context::error(
                "enhanced callback did not restore its entry stack",
            ));
        }
        if tje_mem::Bus::read_u32(engine.devices().memory(), sp) != return_pc {
            return Err(context::error(
                "enhanced callback changed its caller return address",
            ));
        }
        // This is deliberately a synthetic return, not a source timing receipt.
        // A positive RTS-sized charge keeps even all-Rust loops clock-progressing.
        engine.devices_mut().begin_instruction(pc, 0x4e75);
        engine.cpu_mut().write_address(7, sp.wrapping_add(4));
        context::set_pc(engine, return_pc);
        let progress = engine.devices_mut().finish_instruction(pc, 16);
        engine.complete(
            progress,
            StepKind::EnhancedOverride {
                function: entry.id(),
            },
            None,
        );
        budget.record(progress);
        Ok(())
    }
}

fn ready<B: Backend>(engine: &Engine<B>) -> bool {
    let cpu: &CpuState = engine.cpu();
    let devices = engine.devices();
    cpu.pc & 1 == 0
        && cpu.pending_interrupt == 0
        && cpu.stopped == 0
        && cpu.frozen == 0
        && !devices.wait_pending()
        && !devices.reset_asserted()
        && !devices.cpu_halted()
        && devices.pending_interrupt() & 7 <= ((cpu.sr >> 8) & 7) as u8
}

/// Visible, harmless demonstration: retain the readable UI helper's behaviour,
/// then replace its text-panel background with a newly chosen magenta colour.
/// No palette data is extracted or checked into the repository.
fn debug_text_background(
    context: &mut dyn tje_game::Context,
    arguments: &[u32],
) -> tje_game::Result<Option<u32>> {
    tje_game::ui::force_text_background_color_to_black(context, arguments)?;
    context.write_vdp_command(0xc01e_0000);
    context.write_vdp_data(0x0e0e);
    Ok(None)
}
static DEBUG_TRANSLATIONS: &[Translation] = &[Translation {
    id: tje_game::ids::FORCE_TEXT_BACKGROUND_COLOR_TO_BLACK,
    function: debug_text_background,
}];
pub const DEBUG_PROFILE: EnhancedProfile =
    EnhancedProfile::new("debug-text-background-v1", DEBUG_TRANSLATIONS, &[]);
