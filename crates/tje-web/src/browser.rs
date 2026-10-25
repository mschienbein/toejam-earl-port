use crate::{
    input,
    video::{Video, VideoOptions},
};
use genesis_config::GenesisEmulatorConfig;
use js_sys::{Function, Object, Reflect};
use std::sync::atomic::{AtomicBool, Ordering};
use tje_engine_core::metadata::{compile_program_with_divider, game_metadata};
use tje_engine_runtime::{Engine, MachineSnapshot, RewindBuffer};
use tje_engine_wasmgen::{BrowserBackend, ENGINE_VERSION};
use tje_mem::{AddressSpace, ram};
use tje_native_platform::DeviceRuntime;
use tje_rom::{Revision, Rom};
use tje_session::report::{Build, EngineState};
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

mod netplay;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = globalThis, js_name = tjeCaptureWasmPanic, catch)]
    fn capture_wasm_panic(message: &str) -> Result<(), JsValue>;
}

fn install_panic_hook() {
    static INSTALLED: std::sync::Once = std::sync::Once::new();
    static CAPTURING: AtomicBool = AtomicBool::new(false);
    INSTALLED.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            if CAPTURING.swap(true, Ordering::AcqRel) {
                return;
            }
            // The JS callback uses only cached state, never a borrowed WebGame.
            let _ = capture_wasm_panic(&info.to_string());
            console_error_panic_hook::hook(info);
        }))
    });
}

#[wasm_bindgen]
pub fn report_build() -> Result<String, JsValue> {
    install_panic_hook();
    let mut build = Build::current("web", ENGINE_VERSION);
    build.version = env!("CARGO_PKG_VERSION").into();
    serde_json::to_string(&build).map_err(js_error)
}

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
        })
    }

    /// Immutable load-time selection. Saves/rewind carry the exact fix mask;
    /// changing it requires a new cartridge, never a live sidecar mutation.
    pub fn set_fixed(&mut self, enabled: bool, fixes: u32) -> Result<(), JsValue> {
        #[cfg(feature = "enhanced-mode")]
        {
            if !enabled {
                return Ok(());
            }
            if self.engine.frame_count() != 0
                || self.enhanced.profile_id() != "faithful"
                || self.extended_width != 0
                || self.jam_settings.enabled
            {
                return Err(js_error(
                    "select Fixed before play, without debug, extended view or Jam Out",
                ));
            }
            let candidate = tje_extras::fixed::new_mode(
                self.revision,
                tje_extras::fixed::Settings { enabled, fixes },
            )
            .map_err(js_error)?;
            self.enhanced = candidate;
            self.enhanced.reset(&mut self.engine);
            Ok(())
        }
        #[cfg(not(feature = "enhanced-mode"))]
        {
            let _ = fixes;
            if enabled {
                Err(js_error(
                    "this build has no enhanced-mode feature; rebuild with TJE_ENHANCED=1",
                ))
            } else {
                Ok(())
            }
        }
    }
    /// Explicit load-time flag; unchanged false never resets faithful state.
    /// Validate before installing so an unavailable feature preserves the old game.
    pub fn set_enhanced_debug(&mut self, enabled: bool) -> Result<(), JsValue> {
        #[cfg(feature = "enhanced-mode")]
        {
            use tje_engine_runtime::enhanced::{DEBUG_PROFILE, EnhancedMode};
            if enabled {
                self.enhanced =
                    EnhancedMode::new(DEBUG_PROFILE, self.revision).map_err(js_error)?;
                self.enhanced.reset(&mut self.engine);
                self.presenter = None;
                self.jam_capable = false;
            } else if self.enhanced.profile_id() != "faithful" {
                self.enhanced = EnhancedMode::default();
                self.enhanced.reset(&mut self.engine);
                self.presenter = None;
                self.jam_capable = false;
            }
            Ok(())
        }
        #[cfg(not(feature = "enhanced-mode"))]
        if enabled {
            Err(js_error(
                "this build has no enhanced-mode feature; rebuild with TJE_ENHANCED=1",
            ))
        } else {
            Ok(())
        }
    }

    pub fn set_extended_view(&mut self, width: u16) -> Result<(), JsValue> {
        #[cfg(feature = "enhanced-mode")]
        {
            if width == 0 {
                return Ok(());
            }
            if self.engine.frame_count() != 0 {
                return Err(js_error(
                    "select extended view before installing the cartridge",
                ));
            }
            if !(width == 400 || ((408..=512).contains(&width) && width.is_multiple_of(8))) {
                return Err(js_error(
                    "extended view must be 400, or experimental 408..512 in steps of 8",
                ));
            }
            if self.enhanced.profile_id() != "faithful" && self.presenter.is_none() {
                return Err(js_error(
                    "extended view cannot be combined with enhanced debug",
                ));
            }
            self.enhanced =
                tje_extras::new_mode(self.revision, width, self.jam_settings).map_err(js_error)?;
            self.extended_width = width;
            self.presenter = Some(tje_extended_view::presentation::Presenter::new(
                tje_extras::view_mode(width).map_err(js_error)?,
            ));
            self.enhanced.reset(&mut self.engine);
            self.jam_capable = self.jam_settings.enabled;
            Ok(())
        }
        #[cfg(not(feature = "enhanced-mode"))]
        if width == 0 {
            Ok(())
        } else {
            Err(js_error(
                "this build has no enhanced-mode feature; rebuild with TJE_ENHANCED=1",
            ))
        }
    }

    pub fn set_jam_out(
        &mut self,
        enabled: bool,
        sounds: bool,
        busy: bool,
        chord: bool,
        backing: u8,
    ) -> Result<bool, JsValue> {
        if backing > 6 {
            return Err(js_error("Jam backing must be 0..=6"));
        }
        #[cfg(feature = "enhanced-mode")]
        {
            let settings = tje_jam_out::Settings {
                enabled,
                performance_sounds: sounds,
                sounds_while_busy: busy,
                chord,
                backing,
            };
            let mut reset = false;
            if self.jam_capable {
                tje_jam_out::configure(&mut self.enhanced, settings).map_err(js_error)?;
            } else if enabled {
                if self.enhanced.profile_id() != "faithful" && self.presenter.is_none() {
                    return Err(js_error("Jam Out cannot be combined with enhanced debug"));
                }
                self.enhanced = tje_extras::new_mode(self.revision, self.extended_width, settings)
                    .map_err(js_error)?;
                self.presenter = Some(tje_extended_view::presentation::Presenter::new(
                    tje_extras::view_mode(self.extended_width).map_err(js_error)?,
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
            let _ = (sounds, busy, chord);
            if enabled {
                Err(js_error(
                    "this build has no enhanced-mode feature; rebuild with TJE_ENHANCED=1",
                ))
            } else {
                Ok(false)
            }
        }
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

impl CompiledCartridge {
    #[cfg(feature = "enhanced-mode")]
    fn refresh_presentation(&mut self) -> Result<(), JsValue> {
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
        presenter
            .compose(
                memory,
                vram,
                cram,
                original,
                self.enhanced
                    .sidecars()
                    .get::<PublishedView>()
                    .map_err(js_error)?,
                &overrides[..count],
            )
            .map_err(js_error)
    }

    fn video(
        &self,
    ) -> (
        &[jgenesis_common::frontend::Color],
        jgenesis_common::frontend::FrameSize,
    ) {
        #[cfg(feature = "enhanced-mode")]
        if let Some(presenter) = &self.presenter {
            return presenter.frame();
        }
        self.engine.frame_buffer()
    }
    fn profile_id(&self) -> &'static str {
        #[cfg(feature = "enhanced-mode")]
        return self.enhanced.profile_id();
        #[cfg(not(feature = "enhanced-mode"))]
        "faithful"
    }

    fn snapshot_state(&self) -> Result<MachineSnapshot, JsValue> {
        #[cfg(feature = "enhanced-mode")]
        return self.enhanced.snapshot(&self.engine).map_err(js_error);
        #[cfg(not(feature = "enhanced-mode"))]
        self.engine.snapshot().map_err(js_error)
    }

    fn restore_state(&mut self, image: &MachineSnapshot) -> Result<(), JsValue> {
        #[cfg(feature = "enhanced-mode")]
        self.enhanced
            .restore(&mut self.engine, image)
            .map_err(js_error)?;
        #[cfg(feature = "enhanced-mode")]
        if self.jam_capable {
            self.jam_settings = self
                .enhanced
                .sidecars()
                .get::<tje_jam_out::State>()
                .map_err(js_error)?
                .requested_settings();
        }
        #[cfg(not(feature = "enhanced-mode"))]
        self.engine.restore(image).map_err(js_error)?;
        Ok(())
    }
}

#[wasm_bindgen]
#[derive(Clone, Copy)]
pub enum VideoFilter {
    Sharp = 0,
    Nearest = 1,
    Linear = 2,
}

/// One browser session, with no global emulator or DOM ownership.
#[wasm_bindgen]
pub struct WebGame {
    video: Video,
    cartridge: Option<CompiledCartridge>,
    audio: Vec<f32>,
    sample_rate: u32,
    rewind: Option<RewindBuffer>,
    netplay: Option<tje_netplay::Peer<tje_engine_runtime::MachineSnapshot>>,
    #[cfg(feature = "enhanced-mode")]
    jam_input: u8,
}

#[wasm_bindgen]
impl WebGame {
    /// Automatic backend fallback belongs to the page: a fresh canvas is needed
    /// when WebGPU initialization has already selected its canvas context type.
    pub async fn create(canvas: HtmlCanvasElement, backend: &str) -> Result<WebGame, JsValue> {
        install_panic_hook();
        Ok(Self {
            video: Video::new(canvas, backend).await.map_err(js_error)?,
            cartridge: None,
            audio: Vec::with_capacity(4096),
            sample_rate: 48_000,
            rewind: None,
            netplay: None,
            #[cfg(feature = "enhanced-mode")]
            jam_input: 0,
        })
    }

    pub fn has_enhanced_mode(&self) -> bool {
        cfg!(feature = "enhanced-mode")
    }

    pub fn set_jam_input(&mut self, mask: u8) {
        #[cfg(feature = "enhanced-mode")]
        {
            self.jam_input = mask & 3;
        }
        #[cfg(not(feature = "enhanced-mode"))]
        let _ = mask;
    }

    pub fn set_jam_out(
        &mut self,
        enabled: bool,
        sounds: bool,
        busy: bool,
        chord: bool,
        backing: u8,
    ) -> Result<bool, JsValue> {
        if self.netplay.is_some() && enabled {
            return Err(js_error("disconnect netplay before enabling Jam Out"));
        }
        if let Some(cartridge) = &mut self.cartridge {
            let reset = cartridge.set_jam_out(enabled, sounds, busy, chord, backing)?;
            if let Some(rewind) = &mut self.rewind {
                rewind.clear();
            }
            self.audio.clear();
            Ok(reset)
        } else if enabled {
            Err(js_error("import a ROM first"))
        } else {
            Ok(false)
        }
    }

    pub fn install(&mut self, cartridge: CompiledCartridge) -> String {
        // A cartridge/profile change ends the peer's simulation identity.
        self.netplay = None;
        let revision = cartridge.revision.name().to_owned();
        self.sample_rate = cartridge.sample_rate;
        self.cartridge = Some(cartridge);
        self.audio.clear();
        if let Some(rewind) = &mut self.rewind {
            rewind.clear();
        }
        revision
    }

    pub fn profile_id(&self) -> String {
        self.cartridge
            .as_ref()
            .map_or("faithful", CompiledCartridge::profile_id)
            .into()
    }

    /// Release the generated instance/table entry before asynchronous cache
    /// removal. No self borrow is retained by the subsequent forgetting future.
    pub fn unload(&mut self) -> Option<String> {
        let identity = self
            .cartridge
            .as_ref()
            .map(|cartridge| cartridge.revision.sha1().to_owned());
        self.netplay = None;
        self.cartridge = None;
        self.audio.clear();
        if let Some(rewind) = &mut self.rewind {
            rewind.clear();
        }
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
        if self.netplay.is_some() {
            return Err(js_error(
                "disconnect netplay before changing the audio sample rate",
            ));
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

    /// Private ROM-derived bytes, never an input recording or bug-report field.
    pub fn snapshot(&self) -> Result<Vec<u8>, JsValue> {
        if self.netplay.is_some() {
            return Err(js_error("disconnect netplay before saving a local state"));
        }
        let cartridge = self
            .cartridge
            .as_ref()
            .ok_or_else(|| js_error("import a ROM first"))?;
        Ok(cartridge.snapshot_state()?.into_bytes())
    }

    pub fn restore(&mut self, bytes: Vec<u8>) -> Result<(), JsValue> {
        if self.netplay.is_some() {
            return Err(js_error("disconnect netplay before loading a local state"));
        }
        let image = MachineSnapshot::from_bytes(bytes).map_err(js_error)?;
        let rate = self.sample_rate;
        let cartridge = self.cartridge_mut()?;
        cartridge.restore_state(&image)?;
        if cartridge.engine.devices().audio_sample_rate() != rate {
            cartridge
                .engine
                .set_audio_sample_rate(rate)
                .map_err(js_error)?;
        }
        self.audio.clear();
        if let Some(rewind) = &mut self.rewind {
            rewind.clear();
        }
        Ok(())
    }

    pub fn state_hash(&self) -> Result<Vec<u8>, JsValue> {
        let cartridge = self
            .cartridge
            .as_ref()
            .ok_or_else(|| js_error("import a ROM first"))?;
        Ok(cartridge.snapshot_state()?.state_hash().to_vec())
    }

    pub fn set_rewind(&mut self, enabled: bool) {
        if enabled && self.netplay.is_none() {
            self.rewind
                .get_or_insert_with(|| RewindBuffer::new(64 * 1024 * 1024));
        } else {
            self.rewind = None;
        }
    }

    pub fn rewind(&mut self) -> Result<bool, JsValue> {
        if self.netplay.is_some() {
            return Err(js_error("rewind is unavailable during netplay"));
        }
        let Some(cartridge) = &mut self.cartridge else {
            return Ok(false);
        };
        let Some(image) = self
            .rewind
            .as_mut()
            .and_then(|ring| ring.pop_before(cartridge.engine.frame_count()))
        else {
            return Ok(false);
        };
        cartridge.restore_state(&image)?;
        if cartridge.engine.devices().audio_sample_rate() != self.sample_rate {
            cartridge
                .engine
                .set_audio_sample_rate(self.sample_rate)
                .map_err(js_error)?;
        }
        self.audio.clear();
        Ok(true)
    }

    pub fn reset(&mut self) -> Result<(), JsValue> {
        if self.netplay.is_some() {
            return Err(js_error(
                "disconnect netplay before resetting the cartridge",
            ));
        }
        let cartridge = self.cartridge_mut()?;
        #[cfg(feature = "enhanced-mode")]
        {
            if cartridge.jam_capable {
                cartridge.jam_settings = cartridge
                    .enhanced
                    .sidecars()
                    .get::<tje_jam_out::State>()
                    .map_err(js_error)?
                    .requested_settings();
            }
            cartridge.enhanced.reset(&mut cartridge.engine);
            if cartridge.jam_capable {
                tje_jam_out::configure(&mut cartridge.enhanced, cartridge.jam_settings)
                    .map_err(js_error)?;
            }
        }
        #[cfg(not(feature = "enhanced-mode"))]
        cartridge.engine.reset();
        #[cfg(feature = "replay-observer")]
        self.cartridge_mut()?.engine.devices_mut().enable_trace();
        self.audio.clear();
        if let Some(rewind) = &mut self.rewind {
            rewind.clear();
        }
        Ok(())
    }

    /// One real video frame from generated Wasm continuations, with reusable
    /// interleaved f32 output for the AudioWorklet. Exact f64 samples stay in the
    /// engine and are observed without conversion in replay-observer builds.
    pub fn step(&mut self, p1: u8, p2: u8, p2_connected: bool) -> Result<(), JsValue> {
        if self.netplay.is_some() {
            return Err(js_error("use netplay_step while a peer session is active"));
        }
        let cartridge = self
            .cartridge
            .as_mut()
            .ok_or_else(|| js_error("import a ROM first"))?;
        #[cfg(feature = "replay-observer")]
        cartridge.engine.devices_mut().clear_trace();
        #[cfg(feature = "enhanced-mode")]
        let inputs = {
            use genesis_config::GenesisController;
            let mut inputs = tje_engine_runtime::enhanced::EnhancedInputs::from(input::inputs(
                p1,
                p2,
                p2_connected,
            ));
            for (index, pad) in inputs.pads[..2].iter_mut().enumerate() {
                if let GenesisController::ThreeButton(buttons)
                | GenesisController::SixButton(buttons) = pad
                {
                    buttons.mode = self.jam_input & (1 << index) != 0;
                }
            }
            inputs
        };
        #[cfg(feature = "enhanced-mode")]
        let samples = cartridge
            .enhanced
            .play_frame(&mut cartridge.engine, inputs)
            .map_err(js_error)?;
        #[cfg(not(feature = "enhanced-mode"))]
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
        if let Some(rewind) = &mut self.rewind
            && cartridge.engine.frame_count().is_multiple_of(3)
        {
            rewind.push(cartridge.snapshot_state()?);
        }
        Ok(())
    }

    pub fn render(
        &mut self,
        width: u32,
        height: u32,
        native_pixels: bool,
        integer_scale: bool,
        filter: VideoFilter,
        display_shader: u32,
    ) -> Result<bool, JsValue> {
        let display_shader = tje_display::DisplayShader::from_id(display_shader)
            .ok_or_else(|| js_error("unknown display shader preset"))?;
        let Some(cartridge) = &mut self.cartridge else {
            return Ok(false);
        };
        #[cfg(feature = "enhanced-mode")]
        cartridge.refresh_presentation()?;
        let (pixels, size) = cartridge.video();
        self.video
            .render(
                pixels,
                size,
                jgenesis_common::frontend::FrameSize { width, height },
                VideoOptions {
                    native_pixels,
                    integer_scale,
                    filter,
                    display_shader,
                },
            )
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
    /// ROM identity only: no raw cartridge, generated code or machine bytes.
    pub fn report_identity(&self) -> Result<String, JsValue> {
        let cartridge = self
            .cartridge
            .as_ref()
            .ok_or_else(|| js_error("import a ROM first"))?;
        serde_json::to_string(&tje_session::report::Cartridge::from(cartridge.revision))
            .map_err(js_error)
    }

    pub fn report_state(&self) -> Result<String, JsValue> {
        let cartridge = self
            .cartridge
            .as_ref()
            .ok_or_else(|| js_error("import a ROM first"))?;
        let devices = cartridge.engine.devices();
        let memory = devices.memory();
        let players = std::array::from_fn(|index| {
            let player = ram::g_player_info(cartridge.revision).at(index as i32);
            [
                player.x().read(memory),
                player.y().read(memory),
                player.height().read(memory),
            ]
        });
        serde_json::to_string(&EngineState {
            frame: cartridge.engine.frame_count(),
            pc: cartridge.engine.cpu().pc,
            master_clock: devices.clock().to_string(),
            players,
        })
        .map_err(js_error)
    }

    /// Called only after explicit screenshot consent. The JS cache survives a
    /// subsequent panic without reentering this wasm-bindgen object.
    pub fn report_pixels_ptr(&self) -> *const u8 {
        self.cartridge
            .as_ref()
            .map_or(std::ptr::null(), |cartridge| {
                cartridge.engine.frame_buffer().0.as_ptr().cast()
            })
    }
    pub fn report_width(&self) -> u32 {
        self.cartridge
            .as_ref()
            .map_or(0, |cartridge| cartridge.engine.frame_buffer().1.width)
    }
    pub fn report_height(&self) -> u32 {
        self.cartridge
            .as_ref()
            .map_or(0, |cartridge| cartridge.engine.frame_buffer().1.height)
    }

    #[cfg(debug_assertions)]
    pub fn debug_crash(&self) {
        panic!("Forced browser playtest crash");
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
