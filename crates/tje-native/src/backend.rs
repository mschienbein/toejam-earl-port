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
    fn set_jam_out(&mut self, settings: tje_jam_out::Settings) -> Result<bool>;
    fn set_fixed(&mut self, settings: tje_extras::fixed::Settings) -> Result<bool>;
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
    #[cfg(feature = "enhanced-mode")]
    presenter: Option<tje_extended_view::presentation::Presenter>,
    #[cfg(feature = "enhanced-mode")]
    extended_width: u16,
    #[cfg(feature = "enhanced-mode")]
    jam_settings: tje_jam_out::Settings,
    #[cfg(feature = "enhanced-mode")]
    jam_capable: bool,
    #[cfg(feature = "enhanced-mode")]
    fixed_settings: tje_extras::fixed::Settings,
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
            #[cfg(feature = "enhanced-mode")]
            presenter: None,
            #[cfg(feature = "enhanced-mode")]
            extended_width: 0,
            #[cfg(feature = "enhanced-mode")]
            jam_settings: Default::default(),
            #[cfg(feature = "enhanced-mode")]
            jam_capable: false,
            #[cfg(feature = "enhanced-mode")]
            fixed_settings: Default::default(),
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
    /// Reuse rollback storage without discarding enhanced scheduler/sidecar state.
    pub fn snapshot_into(&self, state: &mut MachineSnapshot) -> Result<()> {
        #[cfg(feature = "enhanced-mode")]
        return Ok(self.enhanced.snapshot_into(&self.engine, state)?);
        #[cfg(not(feature = "enhanced-mode"))]
        Ok(self.engine.snapshot_into(state)?)
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
        self.presenter = None;
        self.jam_capable = false;
        Ok(())
    }

    /// Explicit, load-time authored course selection. Admission constructs the
    /// candidate first; a wrong catalog/capability never resets the engine.
    #[cfg(feature = "enhanced-mode")]
    pub fn set_maker(&mut self, course: &tje_maker::ValidatedCourse) -> Result<()> {
        ensure!(
            self.engine.frame_count() == 0,
            "select Maker before starting the cartridge"
        );
        ensure!(
            self.profile_id() == "faithful"
                && self.extended_width == 0
                && !self.jam_settings.enabled,
            "Maker cannot be combined with debug, extended view or Jam Out profiles"
        );
        let candidate = tje_maker::enhanced::new_mode(course, self.engine.devices().memory())?;
        self.enhanced = candidate;
        self.presenter = None;
        self.jam_capable = false;
        self.enhanced.reset(&mut self.engine);
        Ok(())
    }

    /// A load-time simulation choice, never a stretch or a live camera mutation.
    #[cfg(feature = "enhanced-mode")]
    pub fn set_extended_view(&mut self, width: u16) -> Result<()> {
        ensure!(
            self.engine.frame_count() == 0,
            "select extended view before starting the cartridge"
        );
        if width == 0 {
            return Ok(());
        }
        ensure!(
            self.profile_id() != tje_maker::enhanced::PROFILE_ID,
            "Maker uses the native-size view and cannot enable extended view"
        );
        ensure!(
            width == 400 || ((408..=512).contains(&width) && width.is_multiple_of(8)),
            "extended view must be 400, or an explicitly experimental 408..512 multiple of 8"
        );
        ensure!(
            self.profile_id() == "faithful" || self.presenter.is_some(),
            "extended view cannot be combined with enhanced debug"
        );
        self.enhanced = tje_extras::new_mode(self.revision, width, self.jam_settings)?;
        self.extended_width = width;
        self.presenter = Some(tje_extended_view::presentation::Presenter::new(
            tje_extras::view_mode(width)?,
        ));
        self.enhanced.reset(&mut self.engine);
        self.jam_capable = self.jam_settings.enabled;
        Ok(())
    }

    #[cfg(feature = "enhanced-mode")]
    fn refresh_presentation(&mut self) -> Result<()> {
        use tje_extended_view::presentation::{FrameOverride, PublishedView};
        let Some(presenter) = &mut self.presenter else {
            return Ok(());
        };
        let devices = self.engine.devices();
        let memory = devices.memory();
        let (vram, cram, _) = devices.vdp().hybrid_memories();
        let (original, _) = self.engine.frame_buffer();
        let mut overrides = [FrameOverride {
            entity: 0,
            frame_address: 0,
            horizontal_flip: false,
            palette_index: None,
        }; 2];
        let mut count = 0;
        let frames = if self.jam_capable {
            tje_jam_out::presentation(&self.enhanced, memory, self.revision)
        } else {
            [None; 2]
        };
        for frame in frames.into_iter().flatten() {
            overrides[count] = FrameOverride {
                entity: frame.entity,
                frame_address: frame.frame_address,
                horizontal_flip: frame.horizontal_flip,
                palette_index: Some(frame.palette_index),
            };
            count += 1;
        }
        presenter.set_shadow_highlight(devices.vdp().hybrid_registers().shadow_highlight_flag);
        presenter.compose(
            memory,
            vram,
            cram,
            original,
            self.enhanced.sidecars().get::<PublishedView>()?,
            &overrides[..count],
        )?;
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
    let pads = std::array::from_fn::<_, 2, _>(|index| {
        controller(controllers.connected[index], controllers.buttons[index])
    });
    GenesisInputs {
        p1: pads[0],
        p2: pads[1],
    }
}

fn controller(connected: bool, buttons: [bool; 8]) -> GenesisController {
    use GenesisButton::*;
    let mut controller = if connected {
        GenesisController::new(GenesisControllerType::ThreeButton)
    } else {
        GenesisController::None
    };
    for (button, pressed) in [Up, Down, Left, Right, A, B, C, Start]
        .into_iter()
        .zip(buttons)
    {
        controller.set_field(button, pressed);
    }
    controller
}

#[cfg(feature = "enhanced-mode")]
pub fn enhanced_inputs(
    local: crate::input::LocalControllers,
) -> tje_engine_runtime::enhanced::EnhancedInputs {
    tje_engine_runtime::enhanced::EnhancedInputs {
        pads: std::array::from_fn(|seat| controller(local.connected[seat], local.buttons[seat])),
    }
}

impl GameBackend for NativeBackend {
    fn set_fixed(&mut self, settings: tje_extras::fixed::Settings) -> Result<bool> {
        ensure!(settings.valid(), "invalid Fixed fix mask");
        #[cfg(feature = "enhanced-mode")]
        {
            if settings == self.fixed_settings || !settings.enabled && !self.fixed_settings.enabled
            {
                self.fixed_settings = settings;
                return Ok(false);
            }
            ensure!(
                !settings.enabled
                    || (self.profile_id() == "faithful"
                        || self.profile_id() == tje_extras::fixed::PROFILE_ID)
                        && self.extended_width == 0
                        && !self.jam_settings.enabled,
                "Fixed is exclusive with debug, Maker, extended view and Jam Out"
            );
            let candidate = tje_extras::fixed::new_mode(self.revision, settings)?;
            self.enhanced = candidate;
            self.fixed_settings = settings;
            self.presenter = None;
            self.jam_capable = false;
            self.enhanced.reset(&mut self.engine);
            Ok(true)
        }
        #[cfg(not(feature = "enhanced-mode"))]
        {
            ensure!(!settings.enabled, "this build has no enhanced-mode feature");
            Ok(false)
        }
    }
    fn set_jam_out(&mut self, settings: tje_jam_out::Settings) -> Result<bool> {
        ensure!(settings.valid(), "Jam backing must be 0..=6");
        #[cfg(feature = "enhanced-mode")]
        {
            ensure!(
                self.profile_id() != tje_maker::enhanced::PROFILE_ID || !settings.enabled,
                "Maker cannot enable the separate Jam Out profile"
            );
            let mut reset = false;
            if self.jam_capable {
                tje_jam_out::configure(&mut self.enhanced, settings)?;
            } else if settings.enabled {
                ensure!(
                    self.profile_id() == "faithful"
                        || self
                            .enhanced
                            .sidecars()
                            .get::<tje_extended_view::presentation::PublishedView>()
                            .is_ok(),
                    "Jam Out cannot be combined with the enhanced debug profile"
                );
                self.enhanced = tje_extras::new_mode(self.revision, self.extended_width, settings)?;
                self.presenter = Some(tje_extended_view::presentation::Presenter::new(
                    tje_extras::view_mode(self.extended_width)?,
                ));
                self.enhanced.reset(&mut self.engine);
                self.jam_capable = true;
                reset = true;
            }
            self.jam_settings = settings;
            Ok(reset)
        }
        #[cfg(not(feature = "enhanced-mode"))]
        {
            ensure!(!settings.enabled, "this build has no enhanced-mode feature");
            Ok(false)
        }
    }
    fn profile_id(&self) -> &'static str {
        NativeBackend::profile_id(self)
    }
    fn snapshot(&self) -> Result<MachineSnapshot> {
        #[cfg(feature = "enhanced-mode")]
        return Ok(self.enhanced.snapshot(&self.engine)?);
        #[cfg(not(feature = "enhanced-mode"))]
        Ok(self.engine.snapshot()?)
    }
    fn restore(&mut self, snapshot: &MachineSnapshot) -> Result<()> {
        #[cfg(feature = "enhanced-mode")]
        self.enhanced.restore(&mut self.engine, snapshot)?;
        #[cfg(feature = "enhanced-mode")]
        if self.jam_capable {
            self.jam_settings = self
                .enhanced
                .sidecars()
                .get::<tje_jam_out::State>()?
                .requested_settings();
        }
        #[cfg(not(feature = "enhanced-mode"))]
        self.engine.restore(snapshot)?;
        if self.engine.devices().audio_sample_rate() != self.sample_rate {
            self.engine.set_audio_sample_rate(self.sample_rate)?;
        }
        #[cfg(feature = "enhanced-mode")]
        self.refresh_presentation()?;
        Ok(())
    }
    fn advance(&mut self, controllers: Controllers) -> Result<&[[f64; 2]]> {
        self.engine.devices_mut().clear_trace();
        #[cfg(feature = "enhanced-mode")]
        {
            let mut inputs =
                tje_engine_runtime::enhanced::EnhancedInputs::from(genesis_inputs(controllers));
            for (pad, jam) in inputs.pads.iter_mut().zip(controllers.jam) {
                if let GenesisController::ThreeButton(buttons)
                | GenesisController::SixButton(buttons) = pad
                {
                    buttons.mode = jam;
                }
            }
            self.enhanced.play_frame(&mut self.engine, inputs)?;
            self.refresh_presentation()?;
            Ok(self.engine.devices().audio_samples())
        }
        #[cfg(not(feature = "enhanced-mode"))]
        Ok(self.engine.play_frame(genesis_inputs(controllers))?)
    }

    fn video(&self) -> VideoFrame<'_> {
        #[cfg(feature = "enhanced-mode")]
        if let Some(presenter) = &self.presenter {
            let (pixels, size) = presenter.frame();
            return VideoFrame {
                rgba: bytemuck::cast_slice(pixels),
                width: size.width,
                height: size.height,
            };
        }
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
        {
            if self.jam_capable {
                self.jam_settings = self
                    .enhanced
                    .sidecars()
                    .get::<tje_jam_out::State>()
                    .expect("selected Jam sidecar")
                    .requested_settings();
            }
            self.enhanced.reset(&mut self.engine);
            if self.jam_capable {
                tje_jam_out::configure(&mut self.enhanced, self.jam_settings)
                    .expect("validated Jam configuration remains accepted after reset");
            }
        }
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
