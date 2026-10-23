use crate::{input, video::Video};
use tje_host::{Host, Mode, hooks::HookRegistry};
use tje_rom::Rom;
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

fn js_error(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}

/// Replacement-mode playback with every translated function registered. Keeping host
/// construction here makes the hook-registry policy independent of the web UI.
fn create_playback(rom: &Rom, sample_rate: u32) -> tje_host::Result<Host> {
    let mut host = Host::new(
        rom.bytes(),
        Mode::Replace,
        HookRegistry::translated(rom.revision())?,
    )?;
    host.set_audio_sample_rate(sample_rate)?;
    Ok(host)
}

/// One browser session, with no global emulator or DOM ownership.
#[wasm_bindgen]
pub struct WebGame {
    video: Video,
    host: Option<Host>,
    audio: Vec<f32>,
    sample_rate: u32,
}

#[wasm_bindgen]
impl WebGame {
    /// Choose an explicit backend. The page retries WebGL2 on a fresh canvas if
    /// WebGPU adapter/device/surface initialization fails in automatic mode.
    pub async fn create(canvas: HtmlCanvasElement, backend: &str) -> Result<WebGame, JsValue> {
        console_error_panic_hook::set_once();
        let video = Video::new(canvas, backend).await.map_err(js_error)?;
        Ok(Self {
            video,
            host: None,
            audio: Vec::with_capacity(4096),
            sample_rate: 48_000,
        })
    }

    /// Import/identify before replacing the current cartridge. The JS caller
    /// persists the accepted input in IndexedDB, never over a network.
    pub fn load_rom(&mut self, bytes: &[u8]) -> Result<String, JsValue> {
        let rom = Rom::import(bytes).map_err(js_error)?;
        let revision = rom.revision();
        self.host = Some(create_playback(&rom, self.sample_rate).map_err(js_error)?);
        self.audio.clear();
        Ok(revision.name().to_owned())
    }

    pub fn set_sample_rate(&mut self, sample_rate: u32) -> Result<(), JsValue> {
        if !(8000..=192_000).contains(&sample_rate) {
            return Err(js_error(
                "audio sample rate must be between 8000 and 192000 Hz",
            ));
        }
        if sample_rate == self.sample_rate {
            return Ok(());
        }
        if let Some(host) = &mut self.host {
            host.set_audio_sample_rate(sample_rate).map_err(js_error)?;
        }
        self.sample_rate = sample_rate;
        Ok(())
    }

    pub fn reset(&mut self) -> Result<(), JsValue> {
        self.host_mut()?.reset();
        self.audio.clear();
        Ok(())
    }

    /// Emulate one real video frame, retaining reusable interleaved f32 audio.
    /// JavaScript copies this view straight into a transferable worklet block.
    pub fn step(&mut self, p1: u8, p2: u8, p2_connected: bool) -> Result<(), JsValue> {
        let host = self
            .host
            .as_mut()
            .ok_or_else(|| js_error("import a ROM first"))?;
        let samples = host
            .play_frame(input::inputs(p1, p2, p2_connected))
            .map_err(js_error)?;
        self.audio.clear();
        self.audio.reserve(samples.len() * 2);
        for &[left, right] in samples {
            self.audio.push(left as f32);
            self.audio.push(right as f32);
        }
        Ok(())
    }

    pub fn render(
        &mut self,
        width: u32,
        height: u32,
        integer_scale: bool,
    ) -> Result<bool, JsValue> {
        let Some(host) = &self.host else {
            return Ok(false);
        };
        let (pixels, size) = host.frame_buffer();
        self.video
            .render(pixels, size, width, height, integer_scale)
            .map_err(js_error)
    }

    pub fn audio_ptr(&self) -> *const f32 {
        self.audio.as_ptr()
    }

    pub fn audio_len(&self) -> usize {
        self.audio.len()
    }

    pub fn frames(&self) -> f64 {
        self.host
            .as_ref()
            .map_or(0.0, |host| host.frame_count() as f64)
    }

    pub fn target_fps(&self) -> f64 {
        self.host.as_ref().map_or(60.0, Host::target_fps)
    }

    pub fn title_frame(&self) -> Option<f64> {
        self.host
            .as_ref()?
            .measurements
            .title_frame
            .map(|frame| frame as f64)
    }

    pub fn game_frame(&self) -> Option<f64> {
        self.host
            .as_ref()?
            .measurements
            .game_frame
            .map(|frame| frame as f64)
    }

    pub fn logic_ticks(&self) -> f64 {
        self.host
            .as_ref()
            .map_or(0.0, |host| host.measurements.logic_ticks as f64)
    }

    pub fn backend(&self) -> String {
        self.video.backend().to_owned()
    }

    pub fn adapter(&self) -> String {
        self.video.adapter().to_owned()
    }
}

impl WebGame {
    fn host_mut(&mut self) -> Result<&mut Host, JsValue> {
        self.host
            .as_mut()
            .ok_or_else(|| js_error("import a ROM first"))
    }
}
