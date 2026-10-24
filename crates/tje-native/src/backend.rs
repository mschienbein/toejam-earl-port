use crate::input::Controllers;
use anyhow::{Result, ensure};
use genesis_config::{
    GenesisButton, GenesisController, GenesisControllerType, GenesisEmulatorConfig, GenesisInputs,
};
use tje_engine_cranelift::{CacheConfig, CompileProgress, CompileStats, Engine, load_engine};
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
}

pub struct NativeBackend {
    engine: Engine,
    revision: Revision,
}

impl NativeBackend {
    pub fn load(
        rom: Rom,
        cache: &CacheConfig,
        progress: impl FnMut(CompileProgress),
    ) -> Result<Self> {
        let revision = rom.revision();
        let engine = load_engine(rom, &GenesisEmulatorConfig::default(), cache, progress)?;
        Ok(Self { engine, revision })
    }

    pub fn revision(&self) -> Revision {
        self.revision
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
    fn advance(&mut self, controllers: Controllers) -> Result<&[[f64; 2]]> {
        self.engine.devices_mut().clear_trace();
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
        Ok(self.engine.set_audio_sample_rate(sample_rate)?)
    }
    fn reset(&mut self) {
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
