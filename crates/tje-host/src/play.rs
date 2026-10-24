//! Borrowed realtime output, without the reference runner's state snapshots.

use crate::{Host, HostError, Mode, Result, sinks::Inputs};
use genesis_config::GenesisInputs;
use jgenesis_common::frontend::{EmulatorTrait, TickEffect};

impl Host {
    /// Advance one video frame, returning stereo samples at the configured rate.
    ///
    /// The slice remains valid until the next mutable host operation. The audio
    /// allocation is retained between frames; RAM/VDP snapshots, hardware-write
    /// recording and unbounded per-frame measurement vectors are not produced.
    /// Use `run_frame` for reference/check recordings instead. `frame_buffer`
    /// borrows the corresponding RGBA image without copying it.
    pub fn play_frame(&mut self, inputs: GenesisInputs) -> Result<&[[f64; 2]]> {
        if self.mode == Mode::Check {
            return Err(HostError::Metadata(
                "play_frame requires reference or replacement mode; check mode uses run_frame"
                    .into(),
            ));
        }
        self.wait_clock.frame = self.frame;
        self.wait_clock.inputs = inputs;
        self.audio.samples.clear();
        let mut trace = self.emulator.hybrid_bus_mut().hybrid_writes.take();
        if let Some(trace) = &mut trace {
            trace.clear();
        }
        let result = self.play_until_frame(inputs);
        self.emulator.hybrid_bus_mut().hybrid_writes = trace;
        result?;
        self.frame += 1;
        self.frame_progress = Default::default();
        self.wait_clock.frame = self.frame;
        Ok(&self.audio.samples)
    }

    fn play_until_frame(&mut self, inputs: GenesisInputs) -> Result<()> {
        let mut inputs = Inputs(inputs);
        loop {
            let pc = self.emulator.hybrid_cpu().pc();
            let bus = self.emulator.hybrid_bus();
            let ready = bus.cycles.m68k_wait_cpu_cycles == 0
                && self.emulator.hybrid_cpu().hybrid_instruction_ready(bus);
            if ready {
                if self.session_trace.is_some() {
                    self.trace_instruction(pc);
                }
                // Bounded milestones remain useful to the frontends without
                // collecting the reference runner's per-frame diagnostics.
                let symbols = self.symbols;
                if pc == symbols.intro {
                    self.measurements.intro_frame.get_or_insert(self.frame);
                } else if pc == symbols.title {
                    self.measurements.title_frame.get_or_insert(self.frame);
                } else if pc == symbols.new_game_menu {
                    self.measurements.menu_frame.get_or_insert(self.frame);
                } else if pc == symbols.main_loop {
                    self.measurements.game_frame.get_or_insert(self.frame);
                    self.measurements.logic_ticks += 1;
                }
                if self.mode == Mode::Replace
                    && let Some(hook) = self.registry.at(pc).cloned()
                    && self.terminal_entry != Some(hook.symbol.id)
                {
                    self.replace_hook(&hook)?;
                    inputs.0 = self.wait_clock.inputs;
                    continue;
                }
            }
            let effect = if self.session_trace.is_some() {
                self.tick_live_original(&mut inputs)?
            } else {
                self.tick(&mut inputs)?
            };
            if effect == TickEffect::FrameRendered {
                return Ok(());
            }
        }
    }

    /// Exact console refresh rate, including the current VDP interlace mode.
    pub fn target_fps(&self) -> f64 {
        self.emulator.target_fps()
    }

    /// Change the hardware resampler's output rate before the next frame.
    /// Existing returned samples retain their old rate. A frontend must flush
    /// its output queue when changing rates or resetting the console.
    pub fn set_audio_sample_rate(&mut self, sample_rate: u32) -> Result<()> {
        if !(8_000..=192_000).contains(&sample_rate) {
            return Err(HostError::Metadata(
                "audio sample rate must be between 8000 and 192000 Hz".into(),
            ));
        }
        self.emulator
            .update_audio_output_frequency(u64::from(sample_rate));
        // Allocate outside the realtime loop even for a 192 kHz device.
        self.audio_sample_rate = sample_rate;
        let capacity = (f64::from(sample_rate) / self.target_fps()).ceil() as usize + 16;
        if self.audio.samples.capacity() < capacity {
            self.audio
                .samples
                .reserve(capacity - self.audio.samples.len());
        }
        self.audio.samples.clear();
        Ok(())
    }

    /// Hard reset with the same ROM, mode, registered hooks and sample rate.
    pub fn reset(&mut self) {
        let provenance = self.reset_future_frame_provenance();
        self.emulator.hard_reset(&mut self.saves);
        self.frame = 0;
        self.wait_clock.frame = 0;
        self.wait_clock.inputs = GenesisInputs::default();
        self.main_idle = false;
        self.frame_progress = Default::default();
        self.terminal_entry = None;
        self.disabled_check_roots.clear();
        self.session_trace = None;
        self.emulator
            .update_audio_output_frequency(u64::from(self.audio_sample_rate));
        self.audio.samples.clear();
        self.checks = crate::CheckReport::registered(&self.registry);
        self.checks.future_frame_provenance = provenance;
        self.checks.failure_policy = self.check_failure_policy;
        self.measurements = Default::default();
        self.emulator.hybrid_bus_mut().hybrid_writes = Some(Vec::new());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::HookRegistry;

    #[test]
    #[ignore = "requires the user's private REV 02 ROM"]
    fn playback_reuses_audio_and_preserves_rate_on_reset() {
        let path = std::env::var_os("TJE_REV02_ROM")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| "../../roms/rev02.bin".into());
        let bytes = std::fs::read(path).unwrap();
        let mut host = Host::new(&bytes, Mode::Reference, HookRegistry::default()).unwrap();
        assert!((host.target_fps() - 59.9227).abs() < 0.001);
        assert!(host.set_audio_sample_rate(0).is_err());
        host.set_audio_sample_rate(44_100).unwrap();
        // The first presentation starts part-way through power-on scanout.
        // Measure a steady-state interval, not that partial startup frame.
        host.play_frame(GenesisInputs::default()).unwrap();
        let mut samples = 0;
        let capacity = host.audio.samples.capacity();
        for _ in 0..120 {
            samples += host.play_frame(GenesisInputs::default()).unwrap().len();
            assert_eq!(host.audio.samples.capacity(), capacity);
            assert!(
                host.emulator
                    .hybrid_bus()
                    .hybrid_writes
                    .as_ref()
                    .unwrap()
                    .is_empty()
            );
        }
        let expected = 120.0 * 44_100.0 / host.target_fps();
        assert!(
            (samples as f64 - expected).abs() < 5.0,
            "samples={samples}, expected={expected}"
        );
        assert!(host.measurements.idle_draws_per_frame.is_empty());
        assert!(host.measurements.lag_events.is_empty());
        host.reset();
        assert_eq!(host.frame_count(), 0);
        assert_eq!(host.audio_sample_rate, 44_100);
        host.play_frame(GenesisInputs::default()).unwrap();
        let mut reset_samples = 0;
        for _ in 0..120 {
            reset_samples += host.play_frame(GenesisInputs::default()).unwrap().len();
        }
        assert_eq!(reset_samples, samples);
        assert_eq!(host.audio.samples.capacity(), capacity);
    }
}
