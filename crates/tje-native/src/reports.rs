//! Crash-safe ownership boundary: hooks read a small cached snapshot and input
//! ring, never the engine, GPU, audio callback, or a live App borrow.
use anyhow::{Context, Result, anyhow, ensure};
use serde::{Deserialize, Serialize};
use std::{
    backtrace::Backtrace,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tje_native::backend::{Status, VideoFrame};
use tje_rom::Revision;
use tje_session::report::{
    Build, Bundle, EngineState, Environment, Failure, FrontendState, InputRing, Kind,
    REPORT_VERSION, Screenshot,
};

#[derive(Default, Deserialize, Serialize)]
struct Preferences {
    include_screenshot: bool,
}

struct State {
    rom: Option<Revision>,
    settings: serde_json::Value,
    engine: Option<Status>,
    frontend: FrontendState,
    ring: Option<InputRing>,
    consent: bool,
    pixels: Vec<u8>,
    size: [u32; 2],
    screenshot_frame: u64,
}
struct Inner {
    directory: PathBuf,
    environment: Environment,
    state: Mutex<State>,
    capturing: AtomicBool,
    crash_saved: AtomicBool,
}
#[derive(Clone)]
pub struct Reporter(Arc<Inner>);

struct CaptureGuard<'a>(&'a AtomicBool);
impl Drop for CaptureGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
fn enter(flag: &AtomicBool) -> Option<CaptureGuard<'_>> {
    flag.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .ok()
        .map(|_| CaptureGuard(flag))
}

#[derive(Clone)]
pub struct Offer {
    pub path: PathBuf,
    pub issue_url: String,
    pub summary: String,
}
impl Offer {
    fn new(path: PathBuf, bundle: &Bundle) -> Self {
        Self {
            path,
            issue_url: bundle.issue_url(),
            summary: format!(
                "{} (last completed frame {})",
                bundle
                    .failure
                    .message
                    .lines()
                    .next()
                    .unwrap_or("Playtest report"),
                bundle.engine.as_ref().map_or(0, |engine| engine.frame)
            ),
        }
    }
}

impl Reporter {
    pub fn new(data_directory: &Path) -> Self {
        let directory = data_directory.join("reports");
        // Invalid preferences must never imply consent.
        let preferences: Preferences = fs::read(directory.join("preferences.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let os = if cfg!(unix) {
            Command::new("uname")
                .arg("-srvm")
                .output()
                .ok()
                .filter(|out| out.status.success())
                .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
                .unwrap_or_else(|| std::env::consts::OS.into())
        } else {
            std::env::var("OS").unwrap_or_else(|_| std::env::consts::OS.into())
        };
        Self(Arc::new(Inner {
            directory,
            environment: Environment {
                os,
                architecture: std::env::consts::ARCH.into(),
                browser: None,
            },
            state: Mutex::new(State {
                rom: None,
                settings: serde_json::Value::Null,
                engine: None,
                frontend: FrontendState {
                    phase: "startup".into(),
                    backend: "native".into(),
                    ..FrontendState::default()
                },
                ring: None,
                consent: preferences.include_screenshot,
                pixels: Vec::new(),
                size: [0, 0],
                screenshot_frame: 0,
            }),
            capturing: AtomicBool::new(false),
            crash_saved: AtomicBool::new(false),
        }))
    }
    pub fn install_hook(&self) {
        let reporter = self.clone();
        std::panic::set_hook(Box::new(move |info| {
            // No previous/user-supplied hook is called: it could panic recursively.
            // A reentrant call returns before touching locks, formatting or I/O.
            let Some(_guard) = enter(&reporter.0.capturing) else {
                return;
            };
            let message = info.to_string();
            let result = reporter.persist(
                Kind::Crash,
                Failure {
                    source: "rust_panic".into(),
                    message: message.clone(),
                    backtrace: Some(Backtrace::force_capture().to_string()),
                },
                String::new(),
            );
            let mut stderr = std::io::stderr().lock();
            let _ = writeln!(stderr, "{message}");
            match result {
                Ok(offer) => {
                    reporter.0.crash_saved.store(true, Ordering::Release);
                    let _ = writeln!(stderr, "CRASH_REPORT {}", offer.path.display());
                }
                Err(error) => {
                    let _ = writeln!(stderr, "Could not persist crash report: {error:#}");
                }
            }
        }));
    }
    pub fn consent(&self) -> bool {
        self.0
            .state
            .lock()
            .map(|state| state.consent)
            .unwrap_or(false)
    }
    pub fn set_consent(&self, consent: bool) -> Result<()> {
        {
            let mut state = self
                .0
                .state
                .lock()
                .map_err(|_| anyhow!("report snapshot lock poisoned"))?;
            state.consent = consent;
            if !consent {
                state.pixels.clear();
            }
        }
        fs::create_dir_all(&self.0.directory)?;
        fs::write(
            self.0.directory.join("preferences.json"),
            serde_json::to_vec(&Preferences {
                include_screenshot: consent,
            })?,
        )?;
        Ok(())
    }
    pub fn settings(&self, settings: &impl Serialize) -> Result<()> {
        let value = serde_json::to_value(settings)?;
        if let Ok(mut state) = self.0.state.lock() {
            state.settings = value;
        }
        Ok(())
    }
    pub fn start(&self, revision: Revision, start: Option<&tje_session::Start>) {
        if let Ok(mut state) = self.0.state.lock() {
            state.rom = Some(revision);
            state.engine = None;
            state.pixels.clear();
            let mut ring = InputRing::new(revision);
            if let Some(start) = start {
                ring.set_start(start.clone());
            }
            state.ring = Some(ring);
        }
    }
    pub fn reset(&self) {
        if let Ok(mut state) = self.0.state.lock() {
            if let Some(ring) = &mut state.ring {
                ring.reset();
            } else if let Some(revision) = state.rom {
                state.ring = Some(InputRing::new(revision));
            }
            state.engine = None;
            state.pixels.clear();
        }
    }
    /// A restored/rewound timeline cannot be replayed from power-on using only
    /// subsequent inputs. Keep diagnostics, but never export a false replay.
    pub fn invalidate_timeline(&self) {
        if let Ok(mut state) = self.0.state.lock() {
            state.ring = None;
            state.pixels.clear();
        }
    }
    pub fn unload(&self) {
        if let Ok(mut state) = self.0.state.lock() {
            state.rom = None;
            state.ring = None;
            state.engine = None;
            state.pixels.clear();
        }
    }
    pub fn begin_frame(&self, pads: [u16; 2]) {
        if let Ok(mut state) = self.0.state.lock()
            && let Some(ring) = &mut state.ring
        {
            ring.begin(pads);
        }
    }
    pub fn complete_frame(&self, status: &Status, frame: VideoFrame<'_>) {
        if let Ok(mut state) = self.0.state.lock() {
            if let Some(ring) = &mut state.ring {
                ring.complete();
            }
            // The master clock is encoded only on report capture, below.
            state.engine = Some(*status);
            cache_pixels(&mut state, frame, status.frame);
        }
    }
    pub fn remember_frame(&self, frame: VideoFrame<'_>, number: u64) {
        if let Ok(mut state) = self.0.state.lock() {
            cache_pixels(&mut state, frame, number);
        }
    }
    pub fn frontend(&self, paused: bool, focused: bool, ticks: u64, loading: bool, has_game: bool) {
        if let Ok(mut state) = self.0.state.lock() {
            let phase = if loading {
                "compiling"
            } else if !has_game {
                "empty"
            } else if paused {
                "paused"
            } else {
                "running"
            };
            if state.frontend.phase != phase {
                phase.clone_into(&mut state.frontend.phase);
            }
            state.frontend.paused = paused;
            state.frontend.focused = focused;
            state.frontend.ticks = ticks;
        }
    }
    pub fn error(&self, source: &str, error: &anyhow::Error) -> Result<Option<Offer>> {
        if self.0.crash_saved.load(Ordering::Acquire) {
            return Ok(None);
        }
        let Some(_guard) = enter(&self.0.capturing) else {
            return Ok(None);
        };
        let offer = self.persist(
            Kind::Crash,
            Failure {
                source: source.into(),
                message: format!("{error:#}"),
                backtrace: Some(Backtrace::force_capture().to_string()),
            },
            String::new(),
        )?;
        self.0.crash_saved.store(true, Ordering::Release);
        println!("CRASH_REPORT {}", offer.path.display());
        Ok(Some(offer))
    }
    pub fn bug(&self, note: String) -> Result<Offer> {
        let _guard = enter(&self.0.capturing).context("a report is already being captured")?;
        let offer = self.persist(
            Kind::Bug,
            Failure {
                source: "report_a_bug".into(),
                message: "User-reported playtest bug".into(),
                backtrace: None,
            },
            note,
        )?;
        println!("BUG_REPORT {}", offer.path.display());
        Ok(offer)
    }
    fn persist(&self, kind: Kind, failure: Failure, note: String) -> Result<Offer> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?;
        let id = format!("report-{}-{}", now.as_nanos(), std::process::id());
        let mut build = Build::current("native", tje_engine_cranelift::ENGINE_VERSION);
        build.version = env!("CARGO_PKG_VERSION").into();
        // Never block a hook on a lock the panicking thread may own. If the
        // cache itself is unavailable, still persist the original failure.
        let state = self.0.state.try_lock().ok();
        let screenshot = if let Some(state) = &state
            && state.consent
            && !state.pixels.is_empty()
        {
            Some(Screenshot {
                consent: true,
                frame: state.screenshot_frame,
                width: state.size[0],
                height: state.size[1],
                png: encode_png(&state.pixels, state.size)?,
            })
        } else {
            None
        };
        let bundle = Bundle {
            format: REPORT_VERSION,
            id: id.clone(),
            created_utc: format!("unix:{}", now.as_secs()),
            kind,
            build,
            environment: self.0.environment.clone(),
            rom: state.as_ref().and_then(|state| state.rom).map(Into::into),
            settings: state
                .as_ref()
                .map_or(serde_json::Value::Null, |state| state.settings.clone()),
            failure,
            note,
            engine: state
                .as_ref()
                .and_then(|state| state.engine)
                .map(|status| EngineState {
                    frame: status.frame,
                    pc: status.pc,
                    master_clock: status.master_clock.to_string(),
                    players: status.players,
                }),
            frontend: state.as_ref().map_or_else(
                || FrontendState {
                    phase: "snapshot unavailable: report lock busy or poisoned".into(),
                    ..FrontendState::default()
                },
                |state| state.frontend.clone(),
            ),
            recording: state
                .as_ref()
                .and_then(|state| state.ring.as_ref())
                .map(InputRing::snapshot)
                .transpose()?,
            screenshot,
        };
        drop(state);
        let json = bundle.to_json()?;
        fs::create_dir_all(&self.0.directory).context("create local report directory")?;
        let path = self.0.directory.join(format!("{id}.tje-report.json"));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .context("create local report bundle")?;
        file.write_all(json.as_bytes())?;
        file.sync_all()?;
        if bundle.kind == Kind::Crash {
            fs::write(
                self.0.directory.join("pending-crash.txt"),
                format!("{id}.tje-report.json"),
            )?;
        }
        Ok(Offer::new(path, &bundle))
    }
    pub fn pending(&self) -> Result<Option<Offer>> {
        let pointer = self.0.directory.join("pending-crash.txt");
        if !pointer.exists() {
            return Ok(None);
        }
        let filename = fs::read_to_string(pointer)?;
        ensure!(
            Path::new(&filename).components().count() == 1
                && filename.ends_with(".tje-report.json"),
            "invalid pending report filename"
        );
        let path = self.0.directory.join(filename);
        let bundle = Bundle::from_json(&fs::read_to_string(&path)?)?;
        Ok(Some(Offer::new(path, &bundle)))
    }
    pub fn dismiss_pending(&self) -> Result<()> {
        match fs::remove_file(self.0.directory.join("pending-crash.txt")) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}
fn cache_pixels(state: &mut State, frame: VideoFrame<'_>, number: u64) {
    if state.consent {
        state.pixels.clear();
        state.pixels.extend_from_slice(frame.rgba);
        state.size = [frame.width, frame.height];
        state.screenshot_frame = number;
    }
}
fn encode_png(pixels: &[u8], size: [u32; 2]) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, size[0], size[1]);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let rgb: Vec<_> = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|pixel| pixel[..3].iter().copied())
        .collect();
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&rgb)?;
    writer.finish()?;
    Ok(bytes)
}

/// Explicit gestures only. No opener or HTTP operation runs during capture/startup.
pub fn open(path_or_url: &std::ffi::OsStr) -> Result<()> {
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut cmd = Command::new("rundll32.exe");
        cmd.arg("url.dll,FileProtocolHandler");
        cmd
    };
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let mut command = Command::new("xdg-open");
    command
        .arg(path_or_url)
        .spawn()
        .context("open the report folder or GitHub in the default browser")?;
    Ok(())
}

#[derive(Default)]
pub struct Actions {
    pub capture: Option<String>,
    pub consent: Option<bool>,
    pub issue: bool,
    pub folder: bool,
    pub dismiss: bool,
}
pub struct Ui {
    pub offer: Option<Offer>,
    pub consent: bool,
    note: String,
    writing: bool,
}
impl Ui {
    pub fn new() -> Self {
        Self {
            offer: None,
            consent: false,
            note: String::new(),
            writing: false,
        }
    }
    pub fn visible(&self) -> bool {
        self.offer.is_some() || self.writing
    }
    pub fn controls(&mut self, ui: &mut egui::Ui) {
        if ui.button("Report a bug…").clicked() {
            self.writing = true;
        }
    }
    pub fn windows(&mut self, context: &egui::Context, actions: &mut Actions) {
        if self.writing {
            let mut open = true;
            egui::Window::new("Report a bug — saved only on this computer").open(&mut open).default_width(560.0).show(context, |ui| {
                ui.label("Collects build/OS, ROM revision and SHA-1 (not ROM bytes), settings, the last completed frame/state and controller-only inputs since reset (one-hour ring). No upload occurs.");
                ui.label("What happened? Include reproduction steps; review personal details before sharing.");
                ui.add(egui::TextEdit::multiline(&mut self.note).desired_rows(5).desired_width(f32::INFINITY).char_limit(16_384));
                if ui.checkbox(&mut self.consent, "Include last game screenshot in reports (copyrighted game graphics; optional)").changed() { actions.consent = Some(self.consent); }
                ui.small("This choice is remembered for future crash reports. Turn it off to stop collecting pixels. Screenshots are never collected without this consent.");
                if ui.button("Save report bundle locally").clicked() { actions.capture = Some(self.note.clone()); self.writing = false; }
            });
            if !open {
                self.writing = false;
            }
        }
        if let Some(offer) = &self.offer {
            egui::Window::new("Playtest report available — nothing was sent").default_width(580.0).collapsible(false).show(context, |ui| {
                ui.label(&offer.summary);
                ui.label(format!("Saved bundle: {}", offer.path.display()));
                ui.label("Review the bundle before sharing. GitHub receives only the prefilled issue text when you click below. Attach the report file yourself (ZIP it if needed); never attach a ROM or compiled cache.");
                if ui.button("Show report folder").clicked() { actions.folder = true; }
                if ui.button("Open prefilled GitHub issue (network, only on click)").clicked() { actions.issue = true; }
                if ui.button("Dismiss offer (keep saved bundle)").clicked() { actions.dismiss = true; }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capture_guard_cannot_recurse_and_releases_after_failure() {
        let flag = AtomicBool::new(false);
        let guard = enter(&flag).unwrap();
        assert!(enter(&flag).is_none());
        drop(guard);
        assert!(enter(&flag).is_some());
    }
    #[test]
    fn invalidated_enhanced_timelines_never_export_controller_only_recordings() {
        let directory =
            std::env::temp_dir().join(format!("tje-enhanced-report-test-{}", std::process::id()));
        let reporter = Reporter::new(&directory);
        reporter.start(Revision::Rev00, None);
        for _ in 0..2 {
            // Enhanced admission and every reset reassert the same boundary.
            reporter.invalidate_timeline();
            reporter.begin_frame([tje_session::CONNECTED, 0]);
            let offer = reporter
                .bug("enhanced diagnostic, not faithful replay".into())
                .unwrap();
            let bundle = Bundle::from_json(&fs::read_to_string(offer.path).unwrap()).unwrap();
            assert!(bundle.recording.is_none());
            reporter.reset();
        }
        fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn busy_snapshot_still_saves_the_original_failure_without_deadlocking() {
        let directory =
            std::env::temp_dir().join(format!("tje-busy-report-test-{}", std::process::id()));
        let reporter = Reporter::new(&directory);
        let guard = reporter.0.state.lock().unwrap();
        let offer = reporter.bug("busy snapshot test".into()).unwrap();
        let bundle = Bundle::from_json(&fs::read_to_string(offer.path).unwrap()).unwrap();
        assert_eq!(bundle.failure.message, "User-reported playtest bug");
        assert!(bundle.frontend.phase.contains("snapshot unavailable"));
        assert!(bundle.recording.is_none());
        drop(guard);
        fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn panic_hook_persists_once_without_recursing() {
        // Hooks are process-global; exercise the real hook in a separate test process.
        const CHILD: &str = "TJE_PANIC_REPORT_TEST_CHILD";
        if let Some(directory) = std::env::var_os(CHILD) {
            let reporter = Reporter::new(Path::new(&directory));
            reporter.start(Revision::Rev02, None);
            reporter.begin_frame([tje_session::CONNECTED, 0]);
            reporter.install_hook();
            let result = std::panic::catch_unwind(|| panic!("forced hook test"));
            assert!(result.is_err());
            let offer = reporter.pending().unwrap().unwrap();
            let bundle = Bundle::from_json(&fs::read_to_string(offer.path).unwrap()).unwrap();
            assert_eq!(bundle.failure.source, "rust_panic");
            assert_eq!(
                bundle.recording.unwrap().replay_session().unwrap().frames,
                1
            );
            assert!(bundle.screenshot.is_none());
            return;
        }
        let directory =
            std::env::temp_dir().join(format!("tje-panic-report-test-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "reports::tests::panic_hook_persists_once_without_recursing",
                "--nocapture",
            ])
            .env(CHILD, &directory)
            .output()
            .unwrap();
        let _ = fs::remove_dir_all(directory);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
