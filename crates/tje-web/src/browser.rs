use crate::{input, video::Video};
use genesis_config::GenesisEmulatorConfig;
use js_sys::{Function, Object, Reflect};
use tje_engine_core::metadata::{compile_program_with_divider, game_metadata};
use tje_engine_runtime::Engine;
use tje_engine_wasmgen::{BrowserBackend, ENGINE_VERSION};
use tje_mem::{AddressSpace, ram};
use tje_native_platform::DeviceRuntime;
use tje_rom::{Revision, Rom};
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

fn js_error(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}

/// An asynchronously compiled cartridge, separate from the live frontend so no
/// WebGame borrow survives an IndexedDB/compilation await. Failed imports leave
/// the previous cartridge playable. No interpreter or development oracle links.
#[wasm_bindgen]
pub struct CompiledCartridge {
    engine: Engine<BrowserBackend>,
    revision: Revision,
    sample_rate: u32,
}

#[wasm_bindgen]
impl CompiledCartridge {
    /// Identify first, verify the private code cache, and generate only on a
    /// cache miss. The exact same clamped divider binds compilation and devices.
    pub async fn load(
        bytes: Vec<u8>,
        sample_rate: u32,
        progress: Function,
    ) -> Result<CompiledCartridge, JsValue> {
        let rom = Rom::import(&bytes)
            .map_err(|error| js_error(format!("ROM identification failed: {error}")))?;
        drop(bytes);
        let revision = rom.revision();
        let config = GenesisEmulatorConfig::default();
        let divider = u32::try_from(config.clamped_m68k_divider().get()).map_err(js_error)?;
        progress.call1(&JsValue::NULL, &JsValue::from_str("cache"))?;
        let backend = BrowserBackend::load(revision.sha1(), divider, || {
            progress
                .call1(&JsValue::NULL, &JsValue::from_str("generate"))
                .map_err(|error| format!("compilation status callback: {error:?}"))?;
            compile_program_with_divider(rom.bytes(), &game_metadata(revision), divider)
                .map_err(|error| error.to_string())
        })
        .await
        .map_err(js_error)?;
        let mut engine = Engine::from_devices(
            DeviceRuntime::new(AddressSpace::new(rom.into_bytes(), revision), &config),
            backend,
        );
        engine
            .set_audio_sample_rate(sample_rate)
            .map_err(js_error)?;
        #[cfg(feature = "replay-observer")]
        engine.devices_mut().enable_trace();
        progress.call1(&JsValue::NULL, &JsValue::from_str("ready"))?;
        Ok(Self {
            engine,
            revision,
            sample_rate,
        })
    }

    /// Forget every compiled configuration/version for the accepted cartridge.
    /// None is only for a corrupt legacy record whose identity cannot be read;
    /// in that case both supported cartridge identities are removed.
    pub async fn forget_cached(sha1: Option<String>) -> Result<(), JsValue> {
        if let Some(identity) = &sha1
            && !Revision::ALL
                .iter()
                .any(|revision| revision.sha1() == identity)
        {
            return Err(js_error(
                "saved cartridge identity is invalid; re-import a verified ROM",
            ));
        }
        for revision in Revision::ALL {
            if sha1
                .as_ref()
                .is_none_or(|identity| identity == revision.sha1())
            {
                BrowserBackend::forget_rom(revision.sha1())
                    .await
                    .map_err(js_error)?;
            }
        }
        Ok(())
    }
}

/// One browser session, with no global emulator or DOM ownership.
#[wasm_bindgen]
pub struct WebGame {
    video: Video,
    cartridge: Option<CompiledCartridge>,
    audio: Vec<f32>,
    sample_rate: u32,
}

#[wasm_bindgen]
impl WebGame {
    /// Automatic backend fallback belongs to the page: a fresh canvas is needed
    /// when WebGPU initialization has already selected its canvas context type.
    pub async fn create(canvas: HtmlCanvasElement, backend: &str) -> Result<WebGame, JsValue> {
        console_error_panic_hook::set_once();
        Ok(Self {
            video: Video::new(canvas, backend).await.map_err(js_error)?,
            cartridge: None,
            audio: Vec::with_capacity(4096),
            sample_rate: 48_000,
        })
    }

    pub fn install(&mut self, cartridge: CompiledCartridge) -> String {
        let revision = cartridge.revision.name().to_owned();
        self.sample_rate = cartridge.sample_rate;
        self.cartridge = Some(cartridge);
        self.audio.clear();
        revision
    }

    /// Release the generated instance/table entry before asynchronous cache
    /// removal. No self borrow is retained by the subsequent forgetting future.
    pub fn unload(&mut self) -> Option<String> {
        let identity = self
            .cartridge
            .as_ref()
            .map(|cartridge| cartridge.revision.sha1().to_owned());
        self.cartridge = None;
        self.audio.clear();
        identity
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
        if let Some(cartridge) = &mut self.cartridge {
            cartridge
                .engine
                .set_audio_sample_rate(sample_rate)
                .map_err(js_error)?;
            cartridge.sample_rate = sample_rate;
        }
        self.sample_rate = sample_rate;
        Ok(())
    }

    pub fn reset(&mut self) -> Result<(), JsValue> {
        self.cartridge_mut()?.engine.reset();
        #[cfg(feature = "replay-observer")]
        self.cartridge_mut()?.engine.devices_mut().enable_trace();
        self.audio.clear();
        Ok(())
    }

    /// One real video frame from generated Wasm continuations, with reusable
    /// interleaved f32 output for the AudioWorklet. Exact f64 samples stay in the
    /// engine and are observed without conversion in replay-observer builds.
    pub fn step(&mut self, p1: u8, p2: u8, p2_connected: bool) -> Result<(), JsValue> {
        let cartridge = self
            .cartridge
            .as_mut()
            .ok_or_else(|| js_error("import a ROM first"))?;
        #[cfg(feature = "replay-observer")]
        cartridge.engine.devices_mut().clear_trace();
        let samples = cartridge
            .engine
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
        linear_filter: bool,
    ) -> Result<bool, JsValue> {
        let Some(cartridge) = &self.cartridge else {
            return Ok(false);
        };
        let (pixels, size) = cartridge.engine.frame_buffer();
        self.video
            .render(pixels, size, width, height, integer_scale, linear_filter)
            .map_err(js_error)
    }

    pub fn audio_ptr(&self) -> *const f32 {
        self.audio.as_ptr()
    }
    pub fn audio_len(&self) -> usize {
        self.audio.len()
    }
    pub fn frames(&self) -> f64 {
        self.cartridge
            .as_ref()
            .map_or(0.0, |cartridge| cartridge.engine.frame_count() as f64)
    }
    pub fn target_fps(&self) -> f64 {
        self.cartridge
            .as_ref()
            .map_or(60.0, |cartridge| cartridge.engine.target_fps())
    }

    /// A live, named RAM observation, not a guessed routine-entry milestone.
    pub fn player_level(&self) -> Option<i8> {
        let cartridge = self.cartridge.as_ref()?;
        Some(ram::g_toejam_level(cartridge.revision).read(cartridge.engine.devices().memory()))
    }

    pub fn execution_engine(&self) -> String {
        format!("cycle-exact WebAssembly ({ENGINE_VERSION})")
    }
    pub fn backend(&self) -> String {
        self.video.backend().to_owned()
    }
    pub fn adapter(&self) -> String {
        self.video.adapter().to_owned()
    }

    pub fn compilation(&self) -> Result<JsValue, JsValue> {
        let Some(cartridge) = &self.cartridge else {
            return Ok(JsValue::NULL);
        };
        let metrics = cartridge.engine.backend().metrics();
        let value = Object::new();
        for (key, field) in [
            ("cacheHit", JsValue::from_bool(metrics.cache_hit)),
            ("generationMs", JsValue::from_f64(metrics.generation_ms)),
            ("cacheReadMs", JsValue::from_f64(metrics.cache_read_ms)),
            ("cacheWriteMs", JsValue::from_f64(metrics.cache_write_ms)),
            ("instantiateMs", JsValue::from_f64(metrics.instantiate_ms)),
            (
                "moduleBytes",
                JsValue::from_f64(metrics.module_bytes as f64),
            ),
            (
                "storageError",
                metrics
                    .storage_error
                    .as_ref()
                    .map_or(JsValue::NULL, |error| JsValue::from_str(error)),
            ),
            ("version", JsValue::from_str(ENGINE_VERSION)),
        ] {
            Reflect::set(&value, &JsValue::from_str(key), &field)?;
        }
        Ok(value.into())
    }

    #[cfg(feature = "replay-observer")]
    pub fn observation(&self) -> Result<Vec<u8>, JsValue> {
        let cartridge = self
            .cartridge
            .as_ref()
            .ok_or_else(|| js_error("import a ROM first"))?;
        crate::observation::capture(&cartridge.engine).map_err(js_error)
    }
}

impl WebGame {
    fn cartridge_mut(&mut self) -> Result<&mut CompiledCartridge, JsValue> {
        self.cartridge
            .as_mut()
            .ok_or_else(|| js_error("import a ROM first"))
    }
}
