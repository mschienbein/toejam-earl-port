//! Portable, local-only playtest reports. This schema deliberately has no machine
//! snapshot, ROM, generated-code, audio, or arbitrary byte attachment field.
use crate::{Run, Session};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use tje_rom::Revision;

pub const REPORT_VERSION: u32 = 1;
/// 216,000 console frames: about one hour at 60 Hz, 864,000 input bytes.
/// The oldest inputs are evicted, never silently relabelled as a power-on replay.
pub const RECORDING_CAPACITY: usize = 216_000;
pub const BUILD_COMMIT: &str = env!("TJE_BUILD_COMMIT");
pub const BUILD_TARGET: &str = env!("TJE_BUILD_TARGET");

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Build {
    pub version: String,
    pub commit: String,
    pub target: String,
    pub frontend: String,
    pub engine: String,
}
impl Build {
    pub fn current(frontend: &str, engine: &str) -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").into(),
            commit: BUILD_COMMIT.into(),
            target: BUILD_TARGET.into(),
            frontend: frontend.into(),
            engine: engine.into(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Environment {
    pub os: String,
    pub architecture: String,
    pub browser: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Cartridge {
    pub revision: crate::SessionRevision,
    pub sha1: String,
}
impl From<Revision> for Cartridge {
    fn from(revision: Revision) -> Self {
        Self {
            revision: revision.into(),
            sha1: revision.sha1().into(),
        }
    }
}

/// Last *completed* console frame. No CPU register/ROM/VRAM/RAM dumps are saved.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EngineState {
    pub frame: u64,
    pub pc: u32,
    // A string is lossless across the browser's JSON number boundary.
    pub master_clock: String,
    pub players: [[i16; 3]; 2],
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FrontendState {
    pub phase: String,
    pub paused: bool,
    pub focused: bool,
    pub ticks: u64,
    pub backend: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Crash,
    Bug,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Failure {
    pub source: String,
    pub message: String,
    pub backtrace: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Screenshot {
    /// Only ever true when the user opted in *before* these pixels were captured.
    pub consent: bool,
    pub frame: u64,
    pub width: u32,
    pub height: u32,
    pub png: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Recording {
    pub capacity_frames: usize,
    pub first_frame: u64,
    pub completed_frames: u64,
    /// Input to the failing/unfinished frame, not a fabricated completed frame.
    pub in_flight: Option<[u16; 2]>,
    /// Present only if all input since power-on fits in the ring. An unfinished
    /// frame's input is the last frame of this session, so it can provoke a fault.
    pub session: Option<Session>,
    /// Overflow evidence only. Never passed off as a replay from power-on.
    pub input_tail: Vec<Run>,
}
impl Recording {
    pub fn replay_session(&self) -> Result<&Session, crate::Error> {
        let session = self.session.as_ref().ok_or_else(|| {
            crate::Error::Invalid(
                "recording ring overflowed: input_tail is not a deterministic power-on replay"
                    .into(),
            )
        })?;
        session.validate()?;
        Ok(session)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Bundle {
    pub format: u32,
    pub id: String,
    pub created_utc: String,
    pub kind: Kind,
    pub build: Build,
    pub environment: Environment,
    pub rom: Option<Cartridge>,
    pub settings: serde_json::Value,
    pub failure: Failure,
    pub note: String,
    pub engine: Option<EngineState>,
    pub frontend: FrontendState,
    pub recording: Option<Recording>,
    pub screenshot: Option<Screenshot>,
}
impl Bundle {
    pub fn from_json(json: &str) -> Result<Self, crate::Error> {
        let bundle: Self = serde_json::from_str(json)?;
        bundle.validate()?;
        Ok(bundle)
    }
    pub fn validate(&self) -> Result<(), crate::Error> {
        let invalid = |s: &str| crate::Error::Invalid(s.into());
        if self.format != REPORT_VERSION {
            return Err(invalid("unsupported report format"));
        }
        if let Some(rom) = &self.rom {
            let revision: Revision = rom.revision.into();
            if rom.sha1 != revision.sha1() {
                return Err(invalid("report cartridge hash does not match its revision"));
            }
        }
        if let Some(recording) = &self.recording {
            if recording.capacity_frames == 0 || recording.first_frame > recording.completed_frames
            {
                return Err(invalid("invalid report recording bounds"));
            }
            if let Some(pads) = recording.in_flight {
                crate::validate_pads(pads)?;
            }
            if let Some(session) = &recording.session {
                session.validate()?;
                let expected_frames = recording
                    .completed_frames
                    .checked_add(u64::from(recording.in_flight.is_some()))
                    .ok_or_else(|| invalid("report frame count overflow"))?;
                if recording.first_frame != 0
                    || !recording.input_tail.is_empty()
                    || session.frames != expected_frames
                    || self.rom.as_ref().map(|rom| rom.revision) != Some(session.revision)
                {
                    return Err(invalid(
                        "report session does not match its power-on timeline",
                    ));
                }
            } else if recording.first_frame == 0 {
                return Err(invalid("a complete recording must contain its session"));
            }
        }
        if let Some(screenshot) = &self.screenshot
            && (!screenshot.consent || !screenshot.png.starts_with(b"\x89PNG\r\n\x1a\n"))
        {
            return Err(invalid("report screenshot requires consent and a PNG"));
        }
        Ok(())
    }
    pub fn to_json(&self) -> Result<String, crate::Error> {
        self.validate()?;
        Ok(serde_json::to_string(self)? + "\n")
    }
    /// Exporters are deliberately independent of submission. A future upload
    /// transport can consume this bundle only after a separate explicit gesture.
    pub fn issue_url(&self) -> String {
        let title = format!(
            "[{}] {}",
            self.build.frontend,
            self.failure
                .message
                .lines()
                .next()
                .unwrap_or("Playtest report")
                .chars()
                .take(100)
                .collect::<String>()
        );
        let body = format!(
            "Playtest {} report `{}`\n\nBuild: {} / commit {}\nTarget: {}\nOS: {} / {}\nBrowser: {}\nROM: {}\nLast completed frame: {}\nSource: {}\n\n```text\n{}\n```\n\n{}\n\nNothing has been uploaded. Download/locate `{}.tje-report.json`, review its note/settings and optional consented game screenshot, then attach it here using GitHub's attachment control (ZIP it first if GitHub does not accept JSON). Never attach your ROM or compiled-code cache. The bundle includes controller-only replay inputs; an overflow is explicitly marked.\n",
            if self.kind == Kind::Crash {
                "crash"
            } else {
                "bug"
            },
            self.id,
            self.build.version,
            self.build.commit,
            self.build.target,
            self.environment.os,
            self.environment.architecture,
            self.environment
                .browser
                .as_deref()
                .unwrap_or("not applicable"),
            self.rom.as_ref().map_or_else(
                || "not loaded".into(),
                |rom| format!("{:?}, SHA-1 {}", rom.revision, rom.sha1)
            ),
            self.engine.as_ref().map_or(0, |engine| engine.frame),
            self.failure.source,
            self.failure.message.chars().take(2000).collect::<String>(),
            self.note.chars().take(1000).collect::<String>(),
            self.id,
        );
        format!(
            "https://github.com/mschienbein/toejam-earl-port/issues/new?title={}&body={}",
            percent_encode(&title),
            percent_encode(&body)
        )
    }
}

fn percent_encode(text: &str) -> String {
    use std::fmt::Write;
    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

/// Fixed-memory input ring; no allocation/copy on steady-state frame append.
pub struct InputRing {
    revision: Revision,
    start: crate::Start,
    pads: VecDeque<[u16; 2]>,
    capacity: usize,
    completed: u64,
    in_flight: Option<[u16; 2]>,
}
impl InputRing {
    pub fn new(revision: Revision) -> Self {
        Self::with_capacity(revision, RECORDING_CAPACITY)
    }
    fn with_capacity(revision: Revision, capacity: usize) -> Self {
        Self {
            revision,
            start: crate::Start::PowerOn,
            pads: VecDeque::with_capacity(capacity),
            capacity,
            completed: 0,
            in_flight: None,
        }
    }
    /// Preserve only declared setup from a developer replay, never machine RAM.
    pub fn set_start(&mut self, start: crate::Start) {
        self.start = start;
    }
    pub fn reset(&mut self) {
        self.pads.clear();
        self.completed = 0;
        self.in_flight = None;
    }
    pub fn begin(&mut self, pads: [u16; 2]) {
        self.in_flight = Some(pads);
    }
    pub fn complete(&mut self) {
        if let Some(pads) = self.in_flight.take() {
            if self.pads.len() == self.capacity {
                self.pads.pop_front();
            }
            self.pads.push_back(pads);
            self.completed += 1;
        }
    }
    pub fn snapshot(&self) -> Result<Recording, crate::Error> {
        let first_frame = self.completed - self.pads.len() as u64;
        let mut session = Session::new(
            self.revision,
            "Automatic playtest inputs since power-on; no machine snapshot or ROM data",
        );
        for pads in self.pads.iter().copied().chain(self.in_flight) {
            session.record(pads)?;
        }
        if let crate::Start::Recipe {
            description,
            writes,
        } = &self.start
        {
            let writes: Vec<_> = writes
                .iter()
                .filter(|write| write.before_frame < session.frames)
                .cloned()
                .collect();
            if !writes.is_empty() {
                session.start = crate::Start::Recipe {
                    description: description.clone(),
                    writes,
                };
            }
        }
        let (session, input_tail) = if first_frame == 0 {
            (Some(session), Vec::new())
        } else {
            (None, session.runs)
        };
        Ok(Recording {
            capacity_frames: self.capacity,
            first_frame,
            completed_frames: self.completed,
            in_flight: self.in_flight,
            session,
            input_tail,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CONNECTED;
    #[test]
    fn bounded_ring_preserves_power_on_until_overflow_and_reset() {
        let mut ring = InputRing::with_capacity(Revision::Rev02, 3);
        for _ in 0..3 {
            ring.begin([CONNECTED, 0]);
            ring.complete();
        }
        let full = ring.snapshot().unwrap();
        assert_eq!(full.replay_session().unwrap().frames, 3);
        ring.begin([CONNECTED | 1, 0]);
        let pending = ring.snapshot().unwrap();
        assert_eq!(pending.completed_frames, 3);
        assert_eq!(pending.replay_session().unwrap().frames, 4);
        ring.complete();
        let tail = ring.snapshot().unwrap();
        assert_eq!(tail.first_frame, 1);
        assert!(tail.replay_session().is_err());
        assert_eq!(
            tail.input_tail,
            [Run(2, CONNECTED, 0), Run(1, CONNECTED | 1, 0)]
        );
        ring.reset();
        assert_eq!(ring.snapshot().unwrap().replay_session().unwrap().frames, 0);
    }
    #[test]
    fn developer_recipe_keeps_only_the_setup_already_applied() {
        let mut ring = InputRing::with_capacity(Revision::Rev02, 3);
        ring.set_start(crate::Start::Recipe {
            description: "declared setup".into(),
            writes: vec![crate::RamWrite {
                before_frame: 1,
                address: 0xFF8000,
                width: 2,
                value: 7,
                reason: "declared developer setup".into(),
            }],
        });
        ring.begin([CONNECTED, 0]);
        ring.complete();
        assert!(matches!(
            ring.snapshot().unwrap().replay_session().unwrap().start,
            crate::Start::PowerOn
        ));
        ring.begin([CONNECTED, 0]);
        ring.complete();
        assert_eq!(
            ring.snapshot()
                .unwrap()
                .replay_session()
                .unwrap()
                .writes()
                .len(),
            1
        );
    }
    #[test]
    fn schema_has_only_identity_and_inputs_not_asset_or_snapshot_fields() {
        let mut ring = InputRing::with_capacity(Revision::Rev00, 2);
        ring.begin([CONNECTED | 8, 0]);
        ring.complete();
        let bundle = Bundle {
            format: REPORT_VERSION,
            id: "test".into(),
            created_utc: "0".into(),
            kind: Kind::Crash,
            build: Build::current("test", "test"),
            environment: Environment::default(),
            rom: Some(Revision::Rev00.into()),
            settings: serde_json::json!({"volume": 0.7}),
            failure: Failure {
                source: "test".into(),
                message: "forced".into(),
                backtrace: None,
            },
            note: String::new(),
            engine: None,
            frontend: FrontendState::default(),
            recording: Some(ring.snapshot().unwrap()),
            screenshot: None,
        };
        let text = bundle.to_json().unwrap();
        assert!(Bundle::from_json(&text).is_ok());
        let mut json: serde_json::Value = serde_json::from_str(&text).unwrap();
        json["rom"]["bytes"] = serde_json::json!([1, 2, 3]);
        assert!(Bundle::from_json(&json.to_string()).is_err());
        assert!(
            bundle
                .issue_url()
                .starts_with("https://github.com/mschienbein/toejam-earl-port/issues/new?title=")
        );
        assert!(!bundle.issue_url().contains(' '));
    }
}
