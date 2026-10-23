use crate::input::Controllers;
use anyhow::Result;
use genesis_config::{GenesisButton, GenesisController, GenesisControllerType, GenesisInputs};
use tje_host::{Host, Mode, hooks::HookRegistry};
use tje_rom::Rom;

pub struct VideoFrame<'a> {
    pub rgba: &'a [u8],
    pub width: u32,
    pub height: u32,
}

#[derive(Debug)]
pub struct Status {
    pub frame: u64,
    pub stage: &'static str,
    pub title_frame: Option<u64>,
    pub menu_frame: Option<u64>,
    pub gameplay_frame: Option<u64>,
    pub translated_calls: u64,
    pub players: [[i16; 3]; 2],
}

/// Platform code knows only buttons, borrowed pixels and stereo PCM. A future
/// native game implements this interface without changing window/audio/input.
pub trait GameBackend {
    fn advance(&mut self, controllers: Controllers) -> Result<&[[f64; 2]]>;
    fn video(&self) -> VideoFrame<'_>;
    fn fps(&self) -> f64;
    fn reset(&mut self);
    fn status(&mut self) -> Status;
}

pub struct HybridBackend {
    host: Host,
    revision: tje_rom::Revision,
}

impl HybridBackend {
    pub fn new(rom: &Rom, sample_rate: u32) -> Result<Self> {
        let hooks = HookRegistry::translated(rom.revision())?;
        println!(
            "BACKEND hybrid mode=replace revision={} translated_functions={}",
            rom.revision(),
            hooks.len()
        );
        let mut host = Host::new(rom.bytes(), Mode::Replace, hooks)?;
        host.set_audio_sample_rate(sample_rate)?;
        Ok(Self {
            host,
            revision: rom.revision(),
        })
    }
}

impl GameBackend for HybridBackend {
    fn advance(&mut self, controllers: Controllers) -> Result<&[[f64; 2]]> {
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
        Ok(self.host.play_frame(GenesisInputs {
            p1: pads[0],
            p2: pads[1],
        })?)
    }

    fn video(&self) -> VideoFrame<'_> {
        let (pixels, size) = self.host.frame_buffer();
        VideoFrame {
            rgba: bytemuck::cast_slice(&pixels[..size.len() as usize]),
            width: size.width,
            height: size.height,
        }
    }

    fn fps(&self) -> f64 {
        self.host.target_fps()
    }

    fn reset(&mut self) {
        self.host.reset();
    }

    fn status(&mut self) -> Status {
        let players = {
            let revision = self.revision;
            let context = self.host.context();
            std::array::from_fn(|index| {
                let player = tje_mem::ram::g_player_info(revision).at(index as i32);
                [
                    context.read_u16(player.x().address()) as i16,
                    context.read_u16(player.y().address()) as i16,
                    context.read_u16(player.height().address()) as i16,
                ]
            })
        };
        let measurements = &self.host.measurements;
        Status {
            frame: self.host.frame_count(),
            stage: if measurements.game_frame.is_some() {
                "gameplay"
            } else if measurements.menu_frame.is_some() {
                "menu"
            } else if measurements.title_frame.is_some() {
                "title"
            } else {
                "intro"
            },
            title_frame: measurements.title_frame,
            menu_frame: measurements.menu_frame,
            gameplay_frame: measurements.game_frame,
            translated_calls: self
                .host
                .checks
                .by_function
                .values()
                .map(|coverage| coverage.calls_replaced)
                .sum(),
            players,
        }
    }
}
