use crate::input::Controllers;
use anyhow::{Result, ensure};
use genesis_config::{
    GenesisButton, GenesisController, GenesisControllerType, GenesisEmulatorConfig, GenesisInputs,
};
use tje_engine_cranelift::{CacheConfig, CompileProgress, CompileStats, Engine, load_engine};
use tje_engine_runtime::MachineSnapshot;
use tje_mem::Bus;
use tje_rom::{Revision, Rom};
use tje_session::RamWrite;

pub struct VideoFrame<'a> {
    pub rgba: &'a [u8],
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
    pub frame: u64,
    pub pc: u32,
    pub master_clock: u64,
    pub players: [[i16; 3]; 2],
}

/// Platform code sees controllers, borrowed RGBA/PCM, reset and timing. The
/// development proof package drives this same implementation, never a fallback.
pub trait GameBackend {
    fn advance(&mut self, controllers: Controllers) -> Result<&[[f64; 2]]>;
    fn video(&self) -> VideoFrame<'_>;
    fn fps(&self) -> f64;
    fn set_audio_sample_rate(&mut self, sample_rate: u32) -> Result<()>;
    fn reset(&mut self);
    fn status(&self) -> Status;
    /// Documented input-session recipes can write work RAM before a frame.
    fn write_ram(&mut self, write: &RamWrite) -> Result<()>;
    fn snapshot(&self) -> Result<MachineSnapshot>;
    fn restore(&mut self, snapshot: &MachineSnapshot) -> Result<()>;
    /// Simulation profile is part of the peer/save identity, not a video preset.
    fn profile_id(&self) -> &'static str {
        "faithful"
    }
}

pub struct NativeBackend {
    engine: Engine,
    revision: Revision,
    sample_rate: u32,
    #[cfg(feature = "enhanced-mode")]
    enhanced: tje_engine_runtime::enhanced::EnhancedMode,
}

impl NativeBackend {
    pub fn load(
        rom: Rom,
        cache: &CacheConfig,
        progress: impl FnMut(CompileProgress),
    ) -> Result<Self> {
        let revision = rom.revision();
        let engine = load_engine(rom, &GenesisEmulatorConfig::default(), cache, progress)?;
        Ok(Self {
            engine,
            revision,
            sample_rate: 48_000,
            #[cfg(feature = "enhanced-mode")]
            enhanced: Default::default(),
        })
    }

    pub fn revision(&self) -> Revision {
        self.revision
    }
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    pub fn profile_id(&self) -> &'static str {
        #[cfg(feature = "enhanced-mode")]
        return self.enhanced.profile_id();
        #[cfg(not(feature = "enhanced-mode"))]
        "faithful"
    }
    /// Select only at cartridge load/reset, never halfway through an invocation.
    #[cfg(feature = "enhanced-mode")]
    pub fn set_enhanced_debug(&mut self, enabled: bool) -> Result<()> {
        use tje_engine_runtime::enhanced::{DEBUG_PROFILE, EnhancedMode};
        self.enhanced = if enabled {
            EnhancedMode::new(DEBUG_PROFILE, self.revision)?
        } else {
            EnhancedMode::default()
        };
        self.enhanced.reset(&mut self.engine);
        Ok(())
    }
    pub fn rom_bytes(&self) -> &[u8] {
        self.engine.devices().memory().rom()
    }
    pub fn compile_stats(&self) -> &CompileStats {
        self.engine.backend().stats()
    }

    /// Borrowed diagnostics for the separate proof executable. Normal gameplay
    /// leaves all device/instruction observers disabled.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }
    /// Mutable access for opt-in rollback/state adapters, never the offline loop.
    pub fn engine_mut(&mut self) -> &mut Engine {
        &mut self.engine
    }
    pub fn set_trace_enabled(&mut self, enabled: bool) {
        if enabled {
            self.engine.devices_mut().enable_trace();
        } else {
            self.engine.devices_mut().disable_trace();
        }
    }
}

pub fn genesis_inputs(controllers: Controllers) -> GenesisInputs {
    use GenesisButton::*;
    const BUTTONS: [GenesisButton; 8] = [Up, Down, Left, Right, A, B, C, Start];
    let pads: [GenesisController; 2] = std::array::from_fn(|index| {
        let mut controller = if controllers.connected[index] {
            GenesisController::new(GenesisControllerType::ThreeButton)
        } else {
            GenesisController::None
        };
        for (button, pressed) in BUTTONS.into_iter().zip(controllers.buttons[index]) {
            controller.set_field(button, pressed);
        }
        controller
    });
    GenesisInputs {
        p1: pads[0],
        p2: pads[1],
    }
}

impl GameBackend for NativeBackend {
    fn profile_id(&self) -> &'static str {
        NativeBackend::profile_id(self)
    }
    fn snapshot(&self) -> Result<MachineSnapshot> {
        #[cfg(feature = "enhanced-mode")]
        ensure!(
            !self.enhanced.is_failed(),
            "reset or restore a valid save before capturing failed enhanced execution"
        );
        Ok(self.engine.snapshot_with_profile(self.profile_id())?)
    }
    fn restore(&mut self, snapshot: &MachineSnapshot) -> Result<()> {
        let profile = self.profile_id();
        self.engine.restore_with_profile(snapshot, profile)?;
        if self.engine.devices().audio_sample_rate() != self.sample_rate {
            self.engine.set_audio_sample_rate(self.sample_rate)?;
        }
        #[cfg(feature = "enhanced-mode")]
        self.enhanced.after_restore();
        Ok(())
    }
    fn advance(&mut self, controllers: Controllers) -> Result<&[[f64; 2]]> {
        self.engine.devices_mut().clear_trace();
        #[cfg(feature = "enhanced-mode")]
        return Ok(self
            .enhanced
            .play_frame(&mut self.engine, genesis_inputs(controllers))?);
        #[cfg(not(feature = "enhanced-mode"))]
        Ok(self.engine.play_frame(genesis_inputs(controllers))?)
    }

    fn video(&self) -> VideoFrame<'_> {
        let (pixels, size) = self.engine.frame_buffer();
        VideoFrame {
            rgba: bytemuck::cast_slice(&pixels[..size.len() as usize]),
            width: size.width,
            height: size.height,
        }
    }
    fn fps(&self) -> f64 {
        self.engine.target_fps()
    }
    fn set_audio_sample_rate(&mut self, sample_rate: u32) -> Result<()> {
        self.engine.set_audio_sample_rate(sample_rate)?;
        self.sample_rate = sample_rate;
        Ok(())
    }
    fn reset(&mut self) {
        #[cfg(feature = "enhanced-mode")]
        self.enhanced.reset(&mut self.engine);
        #[cfg(not(feature = "enhanced-mode"))]
        self.engine.reset();
    }
    fn status(&self) -> Status {
        let memory = self.engine.devices().memory();
        let players = std::array::from_fn(|index| {
            let player = tje_mem::ram::g_player_info(self.revision).at(index as i32);
            [
                player.x().read(memory),
                player.y().read(memory),
                player.height().read(memory),
            ]
        });
        Status {
            frame: self.engine.frame_count(),
            pc: self.engine.cpu().pc,
            master_clock: self.engine.devices().clock(),
            players,
        }
    }
    fn write_ram(&mut self, write: &RamWrite) -> Result<()> {
        ensure!(
            matches!(write.width, 1 | 2 | 4)
                && write.address >= 0xFF0000
                && write.address <= 0x1000000 - u32::from(write.width),
            "replay writes must stay inside work RAM"
        );
        let bytes = write.value.to_be_bytes();
        for (offset, &value) in bytes[4 - usize::from(write.width)..].iter().enumerate() {
            self.engine
                .devices_mut()
                .memory_mut()
                .write_u8(write.address + offset as u32, value);
        }
        Ok(())
    }
}
