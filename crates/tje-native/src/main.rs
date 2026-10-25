mod audio;
mod convenience;
mod loading;
mod overlay;
mod recording;
mod reports;
mod video;

use anyhow::{Context, Result, ensure};
use audio::Audio;
use clap::Parser;
use config::{DisplayShader, Filter, PixelShape, Scale, Settings, Storage};
use input::{Input, InputScript};
use loading::Loading;
use overlay::{Info, Overlay};
use recording::Recorder;
use std::{
    fs,
    io::BufWriter,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tje_engine_cranelift::{CacheConfig, ENGINE_VERSION};
use tje_native::{
    backend::{GameBackend, VideoFrame},
    config, input,
    replay::Replay,
};
use tje_rom::Rom;
use video::Video;
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::{ElementState, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Fullscreen, Window, WindowId},
};

#[derive(Parser)]
#[command(
    name = "toejam-earl",
    about = "Faithful ToeJam & Earl native port. Supply your own cartridge on first launch.",
    after_help = "First launch: enter a cartridge path in the window, drop the file, or pass --rom PATH.\n\
Your cartridge is remembered after import. Later launches use its compiled engine cache automatically.\n\
Data/config/cache directories are shown at startup and in Settings; --forget-rom removes the saved game/cache.\n\
Open Settings in the window (F1) to change keyboard/gamepad bindings, output and video settings live.\n\n\
Keyboard P1: arrows, Z/X/C = A/B/C, Enter = Start.\n\
Keyboard P2: W/S/A/D, G/H/J = A/B/C, Space = Start.\n\
Gamepads: D-pad or left stick, west/south/east = A/B/C, Start; connection slots 0/1.\n\
Hotkeys: Esc pause, F5 hard reset, F11 fullscreen, F6 integer/fit,\n\
F7 sharp-bilinear/nearest/linear, F8 vsync, F9 record (hard-resets on start), PageUp/PageDown volume.\n\
F1 opens in-app settings; settings pause the game without adding recorded frames.\n\
Recording stops on F9/reset/import/exit and saves controller-only sessions under user data.\n\
--replay PATH replays controller sessions through the native engine; F5 restarts, F9 is disabled during replay.\n\
Keyboard script example: {\"holds\":[{\"start\":300,\"end\":308,\"keys\":[\"Enter\"]}]}.\n\
Intervals are half-open console scheduler ticks; pause still advances the script clock.\n\
Captures contain copyrighted game images: keep --capture-dir outside the public tree."
)]
struct Args {
    /// Import a user's .bin/.md/.gen/.smd/.zip cartridge; later launches use the stored copy.
    #[arg(long)]
    rom: Option<PathBuf>,
    /// Override the OS-specific per-user data directory (also useful for isolated test sessions).
    #[arg(long)]
    data_dir: Option<PathBuf>,
    /// Forget the remembered cartridge and private compiled cache; keep controls/settings.
    #[arg(long, conflicts_with_all = ["rom", "replay", "replay_report"])]
    forget_rom: bool,
    /// Pixel shape: original Genesis NTSC aspect (default), or square pixels.
    #[arg(long, value_enum)]
    pixel_shape: Option<PixelShape>,
    /// Largest whole source-line multiple (default), or fit the available area.
    #[arg(long, value_enum)]
    scale: Option<Scale>,
    /// Sharp bilinear (default), nearest, or linear pixel filtering.
    #[arg(long, value_enum)]
    filter: Option<Filter>,
    /// Optional GPU display preset; off preserves the original display.
    #[arg(long, value_enum)]
    display_shader: Option<DisplayShader>,
    #[arg(long)]
    window_scale: Option<u32>,
    #[arg(long, conflicts_with = "windowed")]
    fullscreen: bool,
    #[arg(long)]
    windowed: bool,
    #[arg(long)]
    no_vsync: bool,
    #[arg(long)]
    volume: Option<f32>,
    #[arg(long, default_value = "auto", value_parser = ["auto", "vulkan", "gl", "dx12", "metal"])]
    gpu_backend: String,
    /// Inject physical keyboard holds through the normal bindings (JSON; see --help).
    #[arg(long, conflicts_with = "replay")]
    input_script: Option<PathBuf>,
    /// Replay a versioned input-only session through the shipped native engine.
    #[arg(long)]
    replay: Option<PathBuf>,
    /// Replay a downloaded playtest report; rejects an overflowed input tail.
    #[arg(long, conflicts_with_all = ["replay", "input_script"])]
    replay_report: Option<PathBuf>,
    /// Exit after this many console frames, only after presenting the last frame.
    #[arg(long)]
    run_frames: Option<u64>,
    /// Exit after this many scheduler ticks (including paused/first-launch UI ticks).
    #[arg(long)]
    run_ticks: Option<u64>,
    /// Open in-app settings immediately.
    #[arg(long)]
    settings: bool,
    /// Save explicitly requested VDP frames as PNGs here. Never publish game captures.
    #[arg(long)]
    capture_dir: Option<PathBuf>,
    #[arg(long, value_delimiter = ',', requires = "capture_dir")]
    capture_frame: Vec<u64>,
}

struct App {
    args: Args,
    storage: Storage,
    settings: Settings,
    input: Input,
    script: Option<InputScript>,
    changes: Vec<(KeyCode, bool)>,
    window: Option<Arc<Window>>,
    video: Option<Video>,
    audio: Option<Audio>,
    backend: Option<Box<dyn GameBackend>>,
    loading: Option<Loading>,
    replay: Option<Replay>,
    paused: bool,
    tick: u64,
    total_frames: u64,
    deadline: Instant,
    started: Instant,
    completed: bool,
    error: Option<anyhow::Error>,
    recorder: Recorder,
    convenience: convenience::Convenience,
    revision: Option<tje_rom::Revision>,
    overlay: Overlay,
    devices: Vec<String>,
    focused: bool,
    effective_paused: bool,
    applied: LiveSettings,
    ui_repaint: Instant,
    preserve_config: bool,
    reports: reports::Reporter,
    expected_report_state: Option<tje_session::report::EngineState>,
    #[cfg(debug_assertions)]
    debug_crash_frame: Option<u64>,
}

#[derive(Clone, Copy)]
struct LiveSettings {
    filter: Filter,
    display_shader: DisplayShader,
    vsync: bool,
    fullscreen: bool,
    window_scale: u32,
}
impl From<&Settings> for LiveSettings {
    fn from(settings: &Settings) -> Self {
        Self {
            filter: settings.filter,
            display_shader: settings.display_shader,
            vsync: settings.vsync,
            fullscreen: settings.fullscreen,
            window_scale: settings.window_scale,
        }
    }
}

fn effective_pause(
    manual: bool,
    overlay: bool,
    focused: bool,
    pause_on_focus_loss: bool,
    scripted: bool,
) -> bool {
    manual || overlay || (!focused && pause_on_focus_loss && !scripted)
}

impl App {
    fn new(args: Args, reports: reports::Reporter) -> Result<Self> {
        ensure!(
            args.run_frames != Some(0),
            "--run-frames must be greater than zero"
        );
        ensure!(
            args.run_ticks != Some(0),
            "--run-ticks must be greater than zero"
        );
        let storage = Storage::new(args.data_dir.clone())?;
        if args.forget_rom {
            storage.forget_rom()?;
        }
        let (mut settings, config_error) = match storage.load() {
            Ok(settings) => (settings, None),
            Err(error) => (
                Settings::default(),
                Some(format!(
                    "Could not load settings: {error:#}. The original file is unchanged. Restore defaults or change a setting to save a valid configuration."
                )),
            ),
        };
        if let Some(pixel_shape) = args.pixel_shape {
            settings.pixel_shape = pixel_shape;
        }
        if let Some(scale) = args.scale {
            settings.scale = scale;
        }
        if let Some(filter) = args.filter {
            settings.filter = filter;
        }
        if let Some(display_shader) = args.display_shader {
            settings.display_shader = display_shader;
        }
        if let Some(scale) = args.window_scale {
            settings.window_scale = scale;
        }
        if args.fullscreen {
            settings.fullscreen = true;
        }
        if args.windowed {
            settings.fullscreen = false;
        }
        if args.no_vsync {
            settings.vsync = false;
        }
        if let Some(volume) = args.volume {
            settings.volume = volume;
        }
        settings.validate()?;
        let input = Input::new(&settings.gamepad_devices)?;
        settings.gamepad_devices = input.saved_devices();
        if config_error.is_none() {
            storage.save(&settings)?;
        }
        let script = args
            .input_script
            .as_deref()
            .map(InputScript::load)
            .transpose()?;
        let (replay, expected_report_state) = if let Some(path) = &args.replay_report {
            let bundle = tje_session::report::Bundle::from_json(&fs::read_to_string(path)?)?;
            let recording = bundle
                .recording
                .context("report has no cartridge/input recording")?;
            let session = recording.replay_session()?.clone();
            let expected = if recording.in_flight.is_none() {
                bundle.engine
            } else {
                None
            };
            (Some(Replay::new(session)?), expected)
        } else {
            (args.replay.as_deref().map(Replay::load).transpose()?, None)
        };
        if let (Some(replay), Some(frames)) = (&replay, args.run_frames) {
            ensure!(
                frames <= replay.frames(),
                "--run-frames cannot exceed the replay length"
            );
        }
        if let Some(path) = &args.capture_dir {
            fs::create_dir_all(path)?;
        }
        println!("STORAGE {}", storage.directory.display());
        println!("CONFIG {}", storage.config_directory.display());
        println!("CACHE {}", storage.cache_directory.display());
        println!(
            "SETTINGS pixel_shape={:?} scale={:?} filter={:?} display_shader={:?} fullscreen={} vsync={} volume={}",
            settings.pixel_shape,
            settings.scale,
            settings.filter,
            settings.display_shader,
            settings.fullscreen,
            settings.vsync,
            settings.volume
        );
        let now = Instant::now();
        let applied = LiveSettings::from(&settings);
        let preserve_config = config_error.is_some();
        let mut overlay = Overlay::new();
        overlay.open = args.settings || config_error.is_some();
        overlay.message = config_error;
        reports.settings(&settings)?;
        overlay.reports.consent = reports.consent();
        match reports.pending() {
            Ok(Some(offer)) => {
                overlay.reports.offer = Some(offer);
                overlay.open |= args.replay_report.is_none();
            }
            Ok(None) => {}
            Err(error) => {
                overlay.show_error(format!("Could not read saved crash report: {error:#}"))
            }
        }
        #[cfg(debug_assertions)]
        let debug_crash_frame = std::env::var("TJE_DEBUG_CRASH_FRAME")
            .ok()
            .map(|value| {
                value
                    .parse::<u64>()
                    .context("TJE_DEBUG_CRASH_FRAME must be a frame number")
            })
            .transpose()?;
        Ok(Self {
            args,
            storage,
            settings,
            input,
            script,
            changes: Vec::with_capacity(32),
            window: None,
            video: None,
            audio: None,
            backend: None,
            loading: None,
            replay,
            paused: false,
            tick: 0,
            total_frames: 0,
            deadline: now,
            started: now,
            completed: false,
            error: None,
            recorder: Recorder::default(),
            convenience: convenience::Convenience::default(),
            revision: None,
            overlay,
            devices: Vec::new(),
            focused: true,
            effective_paused: false,
            applied,
            ui_repaint: now,
            preserve_config,
            reports,
            expected_report_state,
            #[cfg(debug_assertions)]
            debug_crash_frame,
        })
    }

    fn initialize(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        let window = Arc::new(
            event_loop.create_window(
                Window::default_attributes()
                    .with_title("ToeJam & Earl — drop your cartridge here, or use --rom PATH")
                    .with_inner_size(LogicalSize::new(
                        320 * self.settings.window_scale,
                        224 * self.settings.window_scale,
                    ))
                    .with_min_inner_size(LogicalSize::new(320, 224)),
            )?,
        );
        if self.settings.fullscreen {
            window.set_fullscreen(Some(Fullscreen::Borderless(None)));
        }
        let video = pollster::block_on(Video::new(
            window.clone(),
            event_loop,
            &self.args.gpu_backend,
            self.settings.filter,
            self.settings.display_shader,
            self.settings.vsync,
        ))?;
        self.overlay.initialize(&window);
        self.window = Some(window);
        self.video = Some(video);
        self.refresh_devices();
        if let Some((path, remember)) = self.storage.startup_rom(self.args.rom.as_deref()) {
            if let Err(error) = self.import(&path, remember) {
                self.import_error(error);
            }
        } else {
            println!(
                "IMPORT waiting: no saved cartridge; choose one once using the path field, drop, or --rom PATH"
            );
        }
        self.started = Instant::now();
        self.deadline = self.started;
        self.window.as_ref().unwrap().request_redraw();
        Ok(())
    }

    fn import(&mut self, path: &Path, store: bool) -> Result<()> {
        ensure!(
            self.loading.is_none(),
            "native game preparation is already running"
        );
        let bytes = fs::read(path).with_context(|| format!("read cartridge {}", path.display()))?;
        let rom = Rom::import(&bytes).context("identify cartridge")?;
        if let Some(replay) = &self.replay {
            ensure!(
                replay.revision() == rom.revision(),
                "this replay needs {}, not {}",
                replay.revision(),
                rom.revision()
            );
        }
        self.stop_recording()?;
        println!(
            "IMPORT verified revision={} normalized_bytes={} remember={store}",
            rom.revision(),
            rom.bytes().len()
        );
        self.reports.start(
            rom.revision(),
            self.replay.as_ref().map(|replay| &replay.session().start),
        );
        self.loading = Some(Loading::start(
            rom,
            CacheConfig {
                data_dir: self.storage.cache_directory.clone(),
            },
            store,
        )?);
        self.sync_pause();
        if let Some(window) = &self.window {
            window.request_redraw();
        }
        Ok(())
    }

    fn poll_loading(&mut self) -> Result<()> {
        let Some(loading) = &mut self.loading else {
            return Ok(());
        };
        if loading.progress_changed() {
            println!("COMPILE {:?}", loading.shown);
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
        let Some(result) = loading.poll() else {
            return Ok(());
        };
        let loading = self.loading.take().unwrap();
        match result {
            Ok(mut backend) => {
                if loading.store {
                    self.storage.store_rom(backend.rom_bytes())?;
                }
                if self.audio.is_none() {
                    self.audio = Some(Audio::new(
                        self.settings.volume,
                        self.settings.audio_device.clone(),
                    ));
                }
                let audio = self.audio.as_mut().unwrap();
                backend.set_audio_sample_rate(audio.sample_rate)?;
                audio.flush();
                let stats = backend.compile_stats();
                println!(
                    "BACKEND native version={ENGINE_VERSION} revision={} instructions={} code_bytes={} cache={:?} compile_seconds={:.6} load_seconds={:.6} cache_rejection={:?}",
                    backend.revision(),
                    stats.instruction_count,
                    stats.code_bytes,
                    stats.cache_status,
                    stats.compilation_time.as_secs_f64(),
                    loading.started.elapsed().as_secs_f64(),
                    stats.cache_rejection
                );
                println!(
                    "IMPORT {} revision={} normalized_bytes={} stored={}",
                    if loading.store { "fresh" } else { "cached" },
                    backend.revision(),
                    backend.rom_bytes().len(),
                    self.storage.rom_path().display()
                );
                println!("CLOCK fps={:.10} source=cycle-exact-native", backend.fps());
                self.revision = Some(backend.revision());
                self.reports
                    .complete_frame(&backend.status(), backend.video());
                self.backend = Some(backend);
                self.convenience.clear();
                if let Some(replay) = &mut self.replay {
                    replay.restart();
                }
                self.paused = false;
                self.total_frames = 0;
                self.tick = 0;
                self.started = Instant::now();
                self.deadline = self.started;
                self.sync_pause();
                self.update_title();
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            Err(error) => {
                self.import_error(error.context("prepare native game from the verified cartridge"))
            }
        }
        Ok(())
    }

    fn forget_rom(&mut self) -> Result<()> {
        ensure!(
            self.loading.is_none(),
            "wait for native game preparation before forgetting the cartridge"
        );
        self.stop_recording()?;
        self.backend = None;
        self.convenience.clear();
        self.replay = None;
        self.revision = None;
        self.reports.unload();
        self.audio = None;
        self.storage.forget_rom()?;
        self.overlay.open = false;
        self.overlay.message = Some("Saved cartridge and compiled cache forgotten. Your controls/settings were kept; choose a cartridge to play again.".into());
        self.paused = false;
        self.sync_pause();
        if let Some(window) = &self.window {
            window.set_title("ToeJam & Earl — choose a cartridge");
            window.request_redraw();
        }
        println!("IMPORT forgotten");
        Ok(())
    }

    fn update_title(&self) {
        if let Some(window) = &self.window
            && self.backend.is_some()
        {
            window.set_title(&format!(
                "ToeJam & Earl — {:?} settings · {:?} {} · {:?} reset · {:?} fullscreen",
                self.settings.hotkeys.settings,
                self.settings.hotkeys.pause,
                if self.is_paused() { "resume" } else { "pause" },
                self.settings.hotkeys.reset,
                self.settings.hotkeys.fullscreen,
            ));
        }
    }

    fn is_paused(&self) -> bool {
        effective_pause(
            self.paused,
            self.overlay.open || self.loading.is_some(),
            self.focused,
            self.settings.pause_on_focus_loss,
            self.script.is_some(),
        )
    }

    fn sync_pause(&mut self) {
        let paused = self.is_paused();
        self.reports.frontend(
            paused,
            self.focused,
            self.tick,
            self.loading.is_some(),
            self.backend.is_some(),
        );
        if paused != self.effective_paused {
            self.effective_paused = paused;
            if let Some(audio) = &mut self.audio {
                audio.set_paused(paused);
            }
            println!(
                "PAUSE effective={paused} manual={} focus={} settings={}",
                self.paused, self.focused, self.overlay.open
            );
            self.update_title();
        }
    }

    fn refresh_devices(&mut self) {
        match Audio::devices() {
            Ok(devices) => self.devices = devices,
            Err(error) => self
                .overlay
                .show_error(format!("Could not enumerate audio devices: {error:#}")),
        }
    }

    fn import_error(&mut self, error: anyhow::Error) {
        let message = format!(
            "Cartridge import failed: {error:#}. Your saved cartridge was not replaced. Select an original ToeJam & Earl REV 02/REV 00 dump and retry."
        );
        eprintln!("{message}");
        self.overlay.show_error(message);
        if self.backend.is_some() {
            self.overlay.open = true;
        }
        self.sync_pause();
    }

    fn apply_settings(&mut self) -> Result<()> {
        self.settings.validate()?;
        self.convenience.configure(&self.settings.convenience);
        let next = LiveSettings::from(&self.settings);
        if let Some(window) = &self.window {
            if next.fullscreen != self.applied.fullscreen {
                window.set_fullscreen(next.fullscreen.then_some(Fullscreen::Borderless(None)));
            }
            if !next.fullscreen
                && (next.window_scale != self.applied.window_scale || self.applied.fullscreen)
            {
                let _ = window.request_inner_size(LogicalSize::new(
                    320 * next.window_scale,
                    224 * next.window_scale,
                ));
            }
        }
        if let Some(video) = &mut self.video {
            if next.filter != self.applied.filter
                || next.display_shader != self.applied.display_shader
            {
                video.set_presentation(next.filter, next.display_shader);
                if let Some(backend) = &self.backend {
                    video.upload(backend.video());
                }
            }
            if next.vsync != self.applied.vsync {
                video.set_vsync(next.vsync);
            }
        }
        if let Some(audio) = &mut self.audio {
            audio.set_volume(self.settings.volume);
            audio.set_device(self.settings.audio_device.clone());
        }
        self.applied = next;
        println!(
            "SETTINGS applied pixel_shape={:?} scale={:?} filter={:?} display_shader={:?} vsync={} fullscreen={} window_scale={} volume={} focus_pause={}",
            self.settings.pixel_shape,
            self.settings.scale,
            next.filter,
            next.display_shader,
            next.vsync,
            next.fullscreen,
            next.window_scale,
            self.settings.volume,
            self.settings.pause_on_focus_loss
        );
        self.sync_pause();
        self.reports.settings(&self.settings)?;
        self.storage.save(&self.settings)
    }

    fn save_settings(&mut self) {
        self.preserve_config = false;
        if let Err(error) = self.apply_settings() {
            eprintln!("Settings error: {error:#}");
            self.overlay
                .show_error(format!("Could not apply/save settings: {error:#}"));
            self.overlay.open = true;
            self.sync_pause();
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
    fn stop_recording(&mut self) -> Result<()> {
        if let Some(path) = self.recorder.stop(&self.storage.directory)? {
            println!("RECORD saved {}", path.display());
        }
        Ok(())
    }

    fn rewind_once(&mut self) -> Result<()> {
        ensure!(
            self.replay.is_none() && !self.recorder.active(),
            "rewind is unavailable during input replay or recording"
        );
        if let Some(backend) = &mut self.backend
            && self.convenience.rewind(backend.as_mut())?
        {
            self.reports.invalidate_timeline();
            self.reports
                .complete_frame(&backend.status(), backend.video());
            self.video.as_mut().unwrap().upload(backend.video());
            if let Some(audio) = &self.audio {
                audio.flush();
            }
        }
        Ok(())
    }

    fn convenience_command(&mut self, command: convenience::Command) {
        use convenience::Command;
        let result = (|| -> Result<()> {
            ensure!(
                self.replay.is_none(),
                "convenience controls are unavailable during input replay"
            );
            match command {
                Command::NextSlot => {
                    self.settings.convenience.slot = (self.settings.convenience.slot + 1) % 5;
                    self.save_settings();
                }
                Command::ToggleSpeed => self.convenience.toggle_speed(),
                Command::Rewind => self.rewind_once()?,
                Command::Save | Command::Load => {
                    let backend = self.backend.as_mut().context("import a cartridge first")?;
                    let revision = self.revision.context("import a cartridge first")?;
                    let slot = self.settings.convenience.slot;
                    if matches!(command, Command::Save) {
                        convenience::save(
                            &self.storage.directory,
                            revision,
                            slot,
                            &backend.snapshot()?,
                        )?;
                        self.overlay.message =
                            Some(format!("Saved local state in slot {}.", slot + 1));
                    } else {
                        ensure!(
                            !self.recorder.active(),
                            "loading a state is unavailable while recording"
                        );
                        let image = convenience::load(&self.storage.directory, revision, slot)?;
                        backend.restore(&image)?;
                        self.convenience.clear();
                        self.reports.invalidate_timeline();
                        self.reports
                            .complete_frame(&backend.status(), backend.video());
                        self.video.as_mut().unwrap().upload(backend.video());
                        if let Some(audio) = &self.audio {
                            audio.flush();
                        }
                        self.overlay.message = Some(format!(
                            "Loaded local slot {}. Input-only reports resume after a hard reset.",
                            slot + 1
                        ));
                    }
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.overlay
                .show_error(format!("Convenience action rejected: {error:#}"));
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn key(&mut self, key: KeyCode, pressed: bool) -> Result<()> {
        if !self.input.key(key, pressed) {
            return Ok(());
        }
        if self.overlay.open && self.overlay.capturing() {
            if key == KeyCode::Escape {
                self.overlay.cancel();
            } else if self.overlay.capture_key(&mut self.settings, key) {
                self.save_settings();
            }
            self.input.clear_keys();
            if let Some(window) = &self.window {
                window.request_redraw();
            }
            return Ok(());
        }
        if key == self.settings.hotkeys.settings {
            self.overlay.toggle();
            self.input.clear_keys();
            if self.overlay.open {
                self.refresh_devices();
            }
            self.sync_pause();
            if let Some(window) = &self.window {
                window.request_redraw();
            }
            println!("HOTKEY settings={}", self.overlay.open);
            return Ok(());
        }
        if self.overlay.open {
            if key == KeyCode::Escape {
                self.overlay.cancel();
                self.input.clear_keys();
                self.sync_pause();
            }
            return Ok(());
        }
        if self.backend.is_none() || self.loading.is_some() {
            return Ok(());
        }
        if let Some(command) = convenience::shortcut(&self.settings, key) {
            self.convenience_command(command);
            return Ok(());
        }
        let hotkeys = &self.settings.hotkeys;
        let settings_changed = [
            hotkeys.fullscreen,
            hotkeys.scale,
            hotkeys.filter,
            hotkeys.vsync,
            hotkeys.volume_up,
            hotkeys.volume_down,
        ]
        .contains(&key);
        if key == hotkeys.pause {
            self.paused = !self.paused;
            self.sync_pause();
            println!("HOTKEY pause={}", self.paused);
        } else if key == hotkeys.reset {
            if let Some(replay) = &mut self.replay {
                replay.restart();
            }
            self.stop_recording()?;
            self.reports.reset();
            self.convenience.clear();
            if let Some(backend) = &mut self.backend {
                backend.reset();
                self.reports
                    .complete_frame(&backend.status(), backend.video());
            }
            if let Some(audio) = &self.audio {
                audio.flush();
            }
            println!("HOTKEY reset");
        } else if key == hotkeys.record {
            if self.replay.is_some() {
                self.overlay.show_error("Recording is unavailable during replay. Import a cartridge without --replay to record live input.");
                return Ok(());
            }
            if self.recorder.active() {
                self.stop_recording()?;
            } else if let (Some(backend), Some(revision)) = (&mut self.backend, self.revision) {
                self.reports.reset();
                self.convenience.clear();
                backend.reset();
                self.reports
                    .complete_frame(&backend.status(), backend.video());
                self.recorder.start(revision);
                if let Some(audio) = &self.audio {
                    audio.flush();
                }
                println!("RECORD started from power-on (hard reset)");
            }
        } else if key == hotkeys.fullscreen {
            self.settings.fullscreen = !self.settings.fullscreen;
            println!("HOTKEY fullscreen={}", self.settings.fullscreen);
        } else if key == hotkeys.scale {
            self.settings.scale = match self.settings.scale {
                Scale::Integer => Scale::Fit,
                Scale::Fit => Scale::Integer,
            };
            println!("HOTKEY scale={:?}", self.settings.scale);
        } else if key == hotkeys.filter {
            self.settings.filter = match self.settings.filter {
                Filter::Sharp => Filter::Nearest,
                Filter::Nearest => Filter::Linear,
                Filter::Linear => Filter::Sharp,
            };
            println!("HOTKEY filter={:?}", self.settings.filter);
        } else if key == hotkeys.vsync {
            self.settings.vsync = !self.settings.vsync;
            println!("HOTKEY vsync={}", self.settings.vsync);
        } else if key == hotkeys.volume_up || key == hotkeys.volume_down {
            let direction = if key == hotkeys.volume_up {
                0.05
            } else {
                -0.05
            };
            self.settings.volume = (self.settings.volume + direction).clamp(0.0, 1.0);
            println!("HOTKEY volume={:.2}", self.settings.volume);
        } else {
            return Ok(());
        }
        if settings_changed {
            self.save_settings();
        } else if let Some(window) = &self.window {
            window.request_redraw();
        }
        Ok(())
    }

    fn step(&mut self) -> Result<()> {
        if self.loading.is_some() {
            self.tick += 1;
            self.completed |= self.args.run_ticks.is_some_and(|ticks| self.tick >= ticks);
            if self.completed
                && let Some(window) = &self.window
            {
                window.request_redraw();
            }
            return Ok(());
        }
        if let Some(script) = &self.script {
            script.changes(self.tick, &mut self.changes);
        }
        // Reuse the vector while permitting hotkeys to mutate application state.
        for index in 0..self.changes.len() {
            let (key, pressed) = self.changes[index];
            println!("INJECT tick={} key={key:?} pressed={pressed}", self.tick);
            self.key(key, pressed)?;
        }
        self.changes.clear();
        self.tick += 1;
        self.reports.frontend(
            self.is_paused(),
            self.focused,
            self.tick,
            self.loading.is_some(),
            self.backend.is_some(),
        );
        let rewinding = !self.is_paused()
            && self.replay.is_none()
            && !self.recorder.active()
            && self.convenience.rewinding(&self.settings, &self.input);
        if rewinding {
            self.rewind_once()?;
        }
        let fast = self.replay.is_none() && self.convenience.fast(&self.settings, &self.input);
        if (fast || rewinding)
            && let Some(audio) = &self.audio
        {
            audio.flush();
        }
        for _ in 0..if fast { 4 } else { 1 } {
            if self.completed || rewinding {
                break;
            }
            if !self.is_paused()
                && let Some(backend) = &mut self.backend
            {
                let controllers = if let Some(replay) = &self.replay {
                    replay
                        .controllers()
                        .context("input replay ended before its advertised frame count")?
                } else {
                    self.input.controllers(&self.settings.players)
                };
                self.reports.begin_frame(std::array::from_fn(|index| {
                    tje_session::encode_pad(
                        controllers.connected[index],
                        controllers.buttons[index],
                    )
                }));
                let samples = if let Some(replay) = &mut self.replay {
                    replay
                        .advance(backend.as_mut())?
                        .context("input replay ended before its advertised frame count")?
                        .audio
                } else {
                    backend.advance(controllers)?
                };
                if !fast {
                    self.audio.as_ref().unwrap().push(samples);
                }
                self.recorder.frame(controllers)?;
                self.total_frames += 1;
                self.video.as_mut().unwrap().upload(backend.video());
                self.reports
                    .complete_frame(&backend.status(), backend.video());
                if self.replay.is_none() && !self.recorder.active() {
                    self.convenience
                        .capture(backend.as_ref(), &self.settings.convenience)?;
                }
                #[cfg(debug_assertions)]
                if self.debug_crash_frame == Some(backend.status().frame) {
                    panic!(
                        "Forced native playtest crash at frame {}",
                        backend.status().frame
                    );
                }
                if self.total_frames == 1
                    || self.total_frames.is_multiple_of(300)
                    || self.args.capture_frame.contains(&self.total_frames)
                {
                    let status = backend.status();
                    println!(
                        "FRAME total={} backend={} pc={:06X} master_clock={} players={:?} elapsed={:.3}",
                        self.total_frames,
                        status.frame,
                        status.pc,
                        status.master_clock,
                        status.players,
                        self.started.elapsed().as_secs_f64()
                    );
                }
                if self.args.capture_frame.contains(&self.total_frames) {
                    let path = self
                        .args
                        .capture_dir
                        .as_ref()
                        .unwrap()
                        .join(format!("frame-{:06}.png", self.total_frames));
                    capture(&path, backend.video())?;
                    println!("CAPTURE {}", path.display());
                }
                self.completed = self
                    .args
                    .run_frames
                    .is_some_and(|frames| self.total_frames >= frames);
                self.completed |= self.replay.as_ref().is_some_and(Replay::finished);
            }
        }
        self.completed |= self.args.run_ticks.is_some_and(|ticks| self.tick >= ticks);
        if let Some(window) = &self.window
            && ((!self.is_paused() && self.backend.is_some())
                || self.completed
                || self.script.is_some())
        {
            window.request_redraw();
        }
        Ok(())
    }

    fn redraw(&mut self) -> Result<()> {
        let was_open = self.overlay.open;
        let paused = self.is_paused();
        let (frame, actions) = self.overlay.draw(
            self.window.as_ref().unwrap(),
            &mut self.settings,
            Info {
                has_game: self.backend.is_some(),
                paused,
                recording: self.recorder.active(),
                loading: self
                    .loading
                    .as_ref()
                    .map(|loading| (loading.revision, loading.shown)),
                replay: self
                    .replay
                    .as_ref()
                    .map(|replay| (replay.frame(), replay.frames())),
                audio: self
                    .audio
                    .as_ref()
                    .map_or("Audio starts after cartridge import", Audio::status),
                storage: &self.storage,
                input: &self.input,
                devices: &self.devices,
            },
        );
        if actions.restore_defaults {
            self.settings = Settings {
                gamepad_devices: self.input.saved_devices(),
                ..Settings::default()
            };
            self.input.clear_keys();
        }
        if actions.settings_changed || actions.restore_defaults {
            self.save_settings();
        }
        if let Some(command) = actions.convenience {
            self.convenience_command(command);
        }
        if actions.refresh_devices {
            self.refresh_devices();
        }
        if actions.dismiss {
            self.overlay.message = None;
        }
        if actions.resume {
            self.paused = false;
            self.overlay.open = false;
        }
        if let Some(path) = actions.import {
            match self.import(&path, true) {
                Ok(()) => {
                    self.overlay.open = false;
                    self.overlay.message = None;
                    self.input.clear_keys();
                }
                Err(error) => self.import_error(error),
            }
        }
        if actions.forget_rom
            && let Err(error) = self.forget_rom()
        {
            self.overlay
                .show_error(format!("Could not forget cartridge: {error:#}"));
        }
        if let Some(consent) = actions.reporting.consent {
            if let Err(error) = self.reports.set_consent(consent) {
                self.overlay
                    .show_error(format!("Could not save screenshot preference: {error:#}"));
            }
            if let Some(backend) = &self.backend {
                self.reports
                    .remember_frame(backend.video(), backend.status().frame);
            }
        }
        if let Some(note) = actions.reporting.capture {
            match self.reports.bug(note) {
                Ok(offer) => self.overlay.reports.offer = Some(offer),
                Err(error) => self
                    .overlay
                    .show_error(format!("Could not save report bundle: {error:#}")),
            }
        }
        if let Some(offer) = &self.overlay.reports.offer {
            let result = if actions.reporting.issue {
                reports::open(std::ffi::OsStr::new(&offer.issue_url))
            } else if actions.reporting.folder {
                reports::open(
                    offer
                        .path
                        .parent()
                        .unwrap_or(&self.storage.directory)
                        .as_os_str(),
                )
            } else {
                Ok(())
            };
            if let Err(error) = result {
                self.overlay.show_error(format!("{error:#}"));
            }
        }
        if actions.reporting.dismiss {
            self.reports.dismiss_pending()?;
            self.overlay.reports.offer = None;
        }
        if was_open != self.overlay.open {
            self.input.clear_keys();
            if self.overlay.open {
                self.refresh_devices();
            }
        }
        self.sync_pause();
        let now = Instant::now();
        self.ui_repaint = now
            .checked_add(frame.repaint_after)
            .unwrap_or(now + Duration::from_secs(3600));
        self.video.as_mut().unwrap().present(
            self.window.as_ref().unwrap(),
            self.settings.pixel_shape,
            self.settings.scale,
            self.backend.is_some(),
            frame,
        )
    }

    fn finish(&mut self) {
        if let Err(error) = self.stop_recording() {
            self.error.get_or_insert(error);
        }
        let presented = self.video.as_ref().map_or(0, |video| video.presented);
        let seconds = self.started.elapsed().as_secs_f64();
        println!(
            "RUN frames={} ticks={} presented={presented} seconds={seconds:.3} fps={:.6} replay_frame={:?}",
            self.total_frames,
            self.tick,
            if seconds > 0.0 {
                self.total_frames as f64 / seconds
            } else {
                0.0
            },
            self.replay.as_ref().map(Replay::frame)
        );
        if let Some(backend) = &mut self.backend {
            if let Some(expected) = &self.expected_report_state {
                let actual = backend.status();
                if actual.frame != expected.frame
                    || actual.pc != expected.pc
                    || actual.master_clock.to_string() != expected.master_clock
                    || actual.players != expected.players
                {
                    self.error.get_or_insert_with(|| anyhow::anyhow!("captured report replay state differs: expected {expected:?}, actual {actual:?}"));
                } else {
                    println!(
                        "REPORT_REPLAY_MATCH frame={} pc={:06X} master_clock={} players={:?}",
                        actual.frame, actual.pc, actual.master_clock, actual.players
                    );
                }
            }
            println!("BACKEND final={:?}", backend.status());
        }
        if let Some(audio) = &self.audio {
            let stats = audio.stats();
            println!(
                "AUDIO callbacks={} device_frames={} nonzero_frames={} underrun_frames={} overflows={} peak={:.6} queued={}",
                stats.callbacks,
                stats.frames,
                stats.nonzero_frames,
                stats.underrun_frames,
                stats.overflows,
                stats.peak,
                stats.queued
            );
        }
        if let Some(error) = &self.error
            && let Err(capture_error) = self.reports.error("frontend_exit", error)
        {
            eprintln!("Could not persist error report: {capture_error:#}");
        }
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: anyhow::Error) {
        if let Err(capture_error) = self.reports.error("frontend_error", &error) {
            eprintln!("Could not persist error report: {capture_error:#}");
        }
        self.error = Some(error);
        event_loop.exit();
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none()
            && let Err(error) = self.initialize(event_loop)
        {
            self.fail(event_loop, error);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        if let Some(window) = &self.window
            && self.overlay.event(window, &event)
        {
            window.request_redraw();
        }
        let result = match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
                Ok(())
            }
            WindowEvent::Resized(size) => {
                if let Some(video) = &mut self.video {
                    video.resize(size);
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
                Ok(())
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                if let (Some(video), Some(window)) = (&mut self.video, &self.window) {
                    video.resize(window.inner_size());
                    window.request_redraw();
                    println!(
                        "DPI scale={} physical={:?}",
                        window.scale_factor(),
                        window.inner_size()
                    );
                }
                Ok(())
            }
            WindowEvent::Focused(focused) => {
                self.focused = focused;
                if !focused && self.script.is_none() {
                    self.input.clear_keys();
                }
                self.sync_pause();
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
                Ok(())
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(key) = event.physical_key
                    && (!event.repeat || event.state == ElementState::Released)
                {
                    self.key(key, event.state == ElementState::Pressed)
                } else {
                    Ok(())
                }
            }
            WindowEvent::DroppedFile(path) => {
                match self.import(&path, true) {
                    Ok(()) => {
                        self.overlay.open = false;
                        self.overlay.message = None;
                        self.input.clear_keys();
                        self.sync_pause();
                    }
                    Err(error) => self.import_error(error),
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
                Ok(())
            }
            WindowEvent::RedrawRequested => {
                let result = self.redraw();
                if self.completed && result.is_ok() {
                    event_loop.exit();
                }
                result
            }
            _ => Ok(()),
        };
        if let Err(error) = result {
            self.fail(event_loop, error);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.error.is_some() {
            // fail() already requested exit; callbacks the platform still delivers must not
            // advance the console past the frame the crash report captured.
            event_loop.exit();
            return;
        }
        if let Err(error) = self.poll_loading() {
            self.import_error(error);
        }
        let pads = self.input.poll_gamepads();
        if self.overlay.open
            && let Some(press) = pads.pressed
            && self.overlay.capture_pad(&mut self.settings, press)
        {
            self.save_settings();
        }
        if pads.devices_changed {
            self.settings.gamepad_devices = self.input.saved_devices();
            if !self.preserve_config {
                self.save_settings();
            }
        }
        let now = Instant::now();
        if let Some(audio) = &mut self.audio
            && let Some(rate) = audio.maintain(now)
            && let Some(backend) = &mut self.backend
            && let Err(error) = backend.set_audio_sample_rate(rate)
        {
            self.fail(event_loop, error);
            return;
        }
        if self.completed {
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        }
        // Keep the injected-key clock running through pause and first launch.
        // The game advances only on console deadlines; idle UI repaints follow
        // egui's requested cadence instead of regenerating meshes every tick.
        let fps = self.backend.as_ref().map_or(60.0, |backend| backend.fps());
        let period = Duration::from_secs_f64(1.0 / fps);
        if now.saturating_duration_since(self.deadline) > period * 4 {
            self.deadline = now;
        }
        let mut count = 0;
        while Instant::now() >= self.deadline && count < 4 && !self.completed {
            self.deadline += period;
            if let Err(error) = self.step() {
                self.fail(event_loop, error);
                return;
            }
            count += 1;
        }
        if now >= self.ui_repaint {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
            self.ui_repaint = now + Duration::from_secs(1);
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.deadline.min(self.ui_repaint)));
    }

    fn exiting(&mut self, _: &ActiveEventLoop) {
        self.finish();
    }
}

fn capture(path: &Path, frame: VideoFrame<'_>) -> Result<()> {
    let mut encoder = png::Encoder::new(
        BufWriter::new(fs::File::create(path)?),
        frame.width,
        frame.height,
    );
    // The frame's fourth byte is not opacity (the presentation shader ignores it), so
    // captures are written as opaque RGB.
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let (pixels, _) = frame.rgba.as_chunks::<4>();
    let rgb: Vec<u8> = pixels
        .iter()
        .flat_map(|&[red, green, blue, _]| [red, green, blue])
        .collect();
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&rgb)?;
    writer.finish()?;
    Ok(())
}

fn main() -> Result<()> {
    let args = Args::parse();
    let storage = Storage::new(args.data_dir.clone())?;
    let reporter = reports::Reporter::new(&storage.directory);
    reporter.install_hook();
    let result = (|| {
        let mut app = App::new(args, reporter.clone())?;
        EventLoop::new()?.run_app(&mut app)?;
        if let Some(error) = app.error {
            return Err(error);
        }
        Ok(())
    })();
    if let Err(error) = &result
        && let Err(capture_error) = reporter.error("main_exit", error)
    {
        eprintln!("Could not persist error report: {capture_error:#}");
    }
    result
}

#[cfg(test)]
mod pause_tests {
    use super::effective_pause;

    #[test]
    fn focus_and_settings_pause_do_not_override_a_manual_pause() {
        assert!(effective_pause(false, false, false, true, false));
        assert!(!effective_pause(false, false, true, true, false));
        assert!(effective_pause(true, false, true, true, false));
        assert!(effective_pause(false, true, true, false, false));
        assert!(!effective_pause(false, false, false, false, false));
        assert!(!effective_pause(false, false, false, true, true));
    }
}
