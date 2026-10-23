mod audio;
mod backend;
mod config;
mod input;
mod video;

use anyhow::{Context, Result, ensure};
use audio::Audio;
use backend::{GameBackend, HybridBackend, VideoFrame};
use clap::Parser;
use config::{Filter, Scale, Settings, Storage};
use input::{Input, InputScript};
use std::{
    fs,
    io::BufWriter,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tje_rom::Rom;
use video::Video;
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::{ElementState, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Fullscreen, Window, WindowId},
};

#[derive(Parser)]
#[command(
    name = "toejam-earl",
    about = "Faithful ToeJam & Earl native port. Supply your own cartridge on first launch.",
    after_help = "First launch: drop your cartridge on the window, or pass --rom PATH.\n\
Settings and the normalized cartridge live in the OS user-data directory shown at startup.\n\
Edit config.toml there to change keyboard/gamepad bindings; restart to reload it.\n\n\
Keyboard P1: arrows, Z/X/C = A/B/C, Enter = Start.\n\
Keyboard P2: W/S/A/D, G/H/J = A/B/C, Space = Start.\n\
Gamepads: D-pad or left stick, west/south/east = A/B/C, Start; connection slots 0/1.\n\
Hotkeys: Esc pause, F5 hard reset, F11 fullscreen, F6 integer/NTSC fit,\n\
F7 nearest/linear, F8 vsync, PageUp/PageDown volume. All are configurable.\n\n\
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
    #[arg(long, value_enum)]
    scale: Option<Scale>,
    #[arg(long, value_enum)]
    filter: Option<Filter>,
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
    #[arg(long)]
    input_script: Option<PathBuf>,
    /// Exit after this many emulated frames, only after presenting the last frame.
    #[arg(long)]
    run_frames: Option<u64>,
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
    paused: bool,
    tick: u64,
    total_frames: u64,
    deadline: Instant,
    started: Instant,
    completed: bool,
    error: Option<anyhow::Error>,
    last_stage: &'static str,
}

impl App {
    fn new(args: Args) -> Result<Self> {
        ensure!(
            args.run_frames != Some(0),
            "--run-frames must be greater than zero"
        );
        let storage = Storage::new(args.data_dir.clone())?;
        let mut settings = storage.load()?;
        if let Some(scale) = args.scale {
            settings.scale = scale;
        }
        if let Some(filter) = args.filter {
            settings.filter = filter;
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
        storage.save(&settings)?;
        let script = args
            .input_script
            .as_deref()
            .map(InputScript::load)
            .transpose()?;
        if let Some(path) = &args.capture_dir {
            fs::create_dir_all(path)?;
        }
        println!("STORAGE {}", storage.directory.display());
        println!(
            "SETTINGS scale={:?} filter={:?} fullscreen={} vsync={} volume={}",
            settings.scale, settings.filter, settings.fullscreen, settings.vsync, settings.volume
        );
        let now = Instant::now();
        Ok(Self {
            args,
            storage,
            settings,
            input: Input::new()?,
            script,
            changes: Vec::with_capacity(32),
            window: None,
            video: None,
            audio: None,
            backend: None,
            paused: false,
            tick: 0,
            total_frames: 0,
            deadline: now,
            started: now,
            completed: false,
            error: None,
            last_stage: "",
        })
    }

    fn initialize(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        let window = Arc::new(
            event_loop.create_window(
                Window::default_attributes()
                    .with_title("ToeJam & Earl — drop your cartridge here, or use --rom PATH")
                    .with_inner_size(PhysicalSize::new(
                        320 * self.settings.window_scale,
                        224 * self.settings.window_scale,
                    ))
                    .with_min_inner_size(PhysicalSize::new(160, 112)),
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
            self.settings.vsync,
        ))?;
        self.window = Some(window);
        self.video = Some(video);
        let explicit = self.args.rom.clone();
        if let Some(path) = explicit {
            self.import(&path, true)?;
        } else if self.storage.rom_path().is_file() {
            self.import(&self.storage.rom_path(), false)?;
        } else {
            println!("IMPORT waiting: drop a cartridge on the window, or restart with --rom PATH");
        }
        self.started = Instant::now();
        self.deadline = self.started;
        self.window.as_ref().unwrap().request_redraw();
        Ok(())
    }

    fn import(&mut self, path: &Path, store: bool) -> Result<()> {
        let bytes = fs::read(path).with_context(|| format!("read cartridge {}", path.display()))?;
        let rom = Rom::import(&bytes).context("identify cartridge")?;
        if self.audio.is_none() {
            self.audio = Some(Audio::new(self.settings.volume)?);
        }
        let audio = self.audio.as_ref().unwrap();
        let backend = HybridBackend::new(&rom, audio.sample_rate)?;
        if store {
            fs::write(self.storage.rom_path(), rom.bytes()).context("store imported cartridge")?;
        }
        println!(
            "IMPORT {} revision={} normalized_bytes={} stored={}",
            if store { "fresh" } else { "cached" },
            rom.revision(),
            rom.bytes().len(),
            self.storage.rom_path().display()
        );
        println!("CLOCK fps={:.10} source=console", backend.fps());
        audio.flush();
        audio.set_paused(false);
        self.backend = Some(Box::new(backend));
        self.paused = false;
        self.total_frames = 0;
        self.tick = 0;
        self.last_stage = "";
        self.deadline = Instant::now();
        self.update_title();
        Ok(())
    }

    fn update_title(&self) {
        if let Some(window) = &self.window
            && self.backend.is_some()
        {
            window.set_title(&format!(
                "ToeJam & Earl — {:?} {} · {:?} reset · {:?} fullscreen",
                self.settings.hotkeys.pause,
                if self.paused { "resume" } else { "pause" },
                self.settings.hotkeys.reset,
                self.settings.hotkeys.fullscreen,
            ));
        }
    }

    fn key(&mut self, key: KeyCode, pressed: bool) -> Result<()> {
        if !self.input.key(key, pressed) {
            return Ok(());
        }
        let hotkeys = &self.settings.hotkeys;
        if key == hotkeys.pause {
            self.paused = !self.paused;
            if let Some(audio) = &self.audio {
                audio.set_paused(self.paused);
            }
            println!("HOTKEY pause={}", self.paused);
            self.update_title();
        } else if key == hotkeys.reset {
            if let Some(backend) = &mut self.backend {
                backend.reset();
            }
            if let Some(audio) = &self.audio {
                audio.flush();
            }
            self.last_stage = "";
            println!("HOTKEY reset");
        } else if key == hotkeys.fullscreen {
            self.settings.fullscreen = !self.settings.fullscreen;
            if let Some(window) = &self.window {
                window.set_fullscreen(
                    self.settings
                        .fullscreen
                        .then_some(Fullscreen::Borderless(None)),
                );
            }
            println!("HOTKEY fullscreen={}", self.settings.fullscreen);
        } else if key == hotkeys.scale {
            self.settings.scale = match self.settings.scale {
                Scale::Integer => Scale::Fit,
                Scale::Fit => Scale::Integer,
            };
            println!("HOTKEY scale={:?}", self.settings.scale);
        } else if key == hotkeys.filter {
            self.settings.filter = match self.settings.filter {
                Filter::Nearest => Filter::Linear,
                Filter::Linear => Filter::Nearest,
            };
            if let Some(video) = &mut self.video {
                video.set_filter(self.settings.filter);
            }
            println!("HOTKEY filter={:?}", self.settings.filter);
        } else if key == hotkeys.vsync {
            self.settings.vsync = !self.settings.vsync;
            if let Some(video) = &mut self.video {
                video.set_vsync(self.settings.vsync);
            }
            println!("HOTKEY vsync={}", self.settings.vsync);
        } else if key == hotkeys.volume_up || key == hotkeys.volume_down {
            let direction = if key == hotkeys.volume_up {
                0.05
            } else {
                -0.05
            };
            self.settings.volume = (self.settings.volume + direction).clamp(0.0, 1.0);
            if let Some(audio) = &self.audio {
                audio.set_volume(self.settings.volume);
            }
            println!("HOTKEY volume={:.2}", self.settings.volume);
        } else {
            return Ok(());
        }
        self.storage.save(&self.settings)?;
        if let Some(window) = &self.window {
            window.request_redraw();
        }
        Ok(())
    }

    fn step(&mut self) -> Result<()> {
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
        if let Some(audio) = &self.audio {
            audio.check()?;
        }
        if self.paused {
            return Ok(());
        }
        if let Some(backend) = &mut self.backend {
            let controllers = self.input.controllers(&self.settings.players);
            let samples = backend.advance(controllers)?;
            self.audio.as_ref().unwrap().push(samples);
            self.total_frames += 1;
            self.video.as_mut().unwrap().upload(backend.video());
            if self.total_frames == 1
                || self.total_frames.is_multiple_of(60)
                || self.args.capture_frame.contains(&self.total_frames)
            {
                let status = backend.status();
                if status.stage != self.last_stage {
                    println!(
                        "STAGE {} frame={} title={:?} menu={:?} gameplay={:?}",
                        status.stage,
                        status.frame,
                        status.title_frame,
                        status.menu_frame,
                        status.gameplay_frame
                    );
                    self.last_stage = status.stage;
                }
                if self.total_frames.is_multiple_of(300)
                    || self.args.capture_frame.contains(&self.total_frames)
                {
                    println!(
                        "FRAME total={} backend={} translated_calls={} players={:?} elapsed={:.3}",
                        self.total_frames,
                        status.frame,
                        status.translated_calls,
                        status.players,
                        self.started.elapsed().as_secs_f64()
                    );
                }
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
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
        Ok(())
    }

    fn finish(&mut self) {
        let presented = self.video.as_ref().map_or(0, |video| video.presented);
        println!(
            "RUN frames={} ticks={} presented={presented} seconds={:.3} stage={}",
            self.total_frames,
            self.tick,
            self.started.elapsed().as_secs_f64(),
            self.last_stage
        );
        if let Some(backend) = &mut self.backend {
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
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: anyhow::Error) {
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
            WindowEvent::Focused(false) if self.script.is_none() => {
                self.input.clear_keys();
                Ok(())
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(key) = event.physical_key {
                    self.key(key, event.state == ElementState::Pressed)
                } else {
                    Ok(())
                }
            }
            WindowEvent::DroppedFile(path) => {
                if let Err(error) = self.import(&path, true) {
                    eprintln!("IMPORT rejected: {error:#}");
                    if let Some(window) = &self.window {
                        window.set_title(&format!(
                            "Cartridge rejected: {error} — drop a valid ToeJam & Earl ROM"
                        ));
                    }
                }
                Ok(())
            }
            WindowEvent::RedrawRequested => {
                let result = self.video.as_mut().unwrap().present(
                    self.window.as_ref().unwrap(),
                    self.settings.scale,
                    self.backend.is_some(),
                );
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
        self.input.poll_gamepads();
        if self.completed {
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        }
        let now = Instant::now();
        if self.backend.is_some() {
            // Vsync controls presentation, not game speed. A monotonic console
            // clock advances every frame, with bounded catch-up after a stall.
            let fps = self.backend.as_ref().unwrap().fps();
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
            event_loop.set_control_flow(ControlFlow::WaitUntil(self.deadline));
        } else {
            event_loop.set_control_flow(ControlFlow::WaitUntil(now + Duration::from_millis(100)));
        }
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
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(frame.rgba)?;
    writer.finish()?;
    Ok(())
}

fn main() -> Result<()> {
    let mut app = App::new(Args::parse())?;
    EventLoop::new()?.run_app(&mut app)?;
    if let Some(error) = app.error {
        return Err(error);
    }
    Ok(())
}
