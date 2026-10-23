//! Borrowed realtime output, without the reference runner's state snapshots.

use crate::{Host, HostError, Mode, Result, catalogue, sinks::Inputs};
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
                "play_frame requires reference or replacement mode; check mode uses run_frame".into(),
            ));
        }
        self.audio.samples.clear();
        let mut trace = self.emulator.hybrid_bus_mut().hybrid_writes.take();
        if let Some(trace) = &mut trace {
            trace.clear();
        }
        let result = self.play_until_frame(inputs);
        self.emulator.hybrid_bus_mut().hybrid_writes = trace;
        result?;
        self.frame += 1;
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
                // Bounded milestones remain useful to the frontends without
                // collecting the reference runner's per-frame diagnostics.
                match pc {
                    catalogue::INTRO => {
                        self.measurements.intro_frame.get_or_insert(self.frame);
                    }
                    0x027A10 => {
                        self.measurements.title_frame.get_or_insert(self.frame);
                    }
                    catalogue::NEW_GAME_MENU => {
                        self.measurements.menu_frame.get_or_insert(self.frame);
                    }
                    catalogue::MAIN_LOOP => {
                        self.measurements.game_frame.get_or_insert(self.frame);
                        self.measurements.logic_ticks += 1;
                    }
                    _ => {}
                }
                if self.mode == Mode::Replace
                    && let Some(hook) = self.registry.at(pc).cloned()
                {
                    self.replace_hook(&hook)?;
                    continue;
                }
            }
            if self.tick(&mut inputs)? == TickEffect::FrameRendered {
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
            self.audio.samples.reserve(capacity - self.audio.samples.len());
        }
        self.audio.samples.clear();
        Ok(())
    }

    /// Hard reset with the same ROM, mode, registered hooks and sample rate.
    pub fn reset(&mut self) {
        self.emulator.hard_reset(&mut self.saves);
        self.frame = 0;
        self.main_idle = false;
        self.emulator
            .update_audio_output_frequency(u64::from(self.audio_sample_rate));
        self.audio.samples.clear();
        self.checks = Default::default();
        self.measurements = Default::default();
        self.emulator.hybrid_bus_mut().hybrid_writes = Some(Vec::new());
    }
}
