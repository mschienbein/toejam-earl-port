//! Versioned controller-only recordings. No machine snapshots or game assets.
//!
//! A run is `{\"frames\": count, \"pads\": [pad1, pad2]}`. Pad bits 0..7 are Up, Down, Left,
//! Right, A, B, C, Start; bit 8 means connected. All pads are three-button.
//! Frame zero is the first video frame after power-on, not a game RAM counter.

use serde::{Deserialize, Serialize};
use tje_rom::Revision;

pub mod report;

pub const FORMAT_VERSION: u32 = 1;
pub const TOOL_VERSION: &str = concat!("tje-session/", env!("CARGO_PKG_VERSION"));
pub const CONNECTED: u16 = 1 << 8;
pub const UP: u16 = 1;
pub const DOWN: u16 = 2;
pub const LEFT: u16 = 4;
pub const RIGHT: u16 = 8;
pub const A: u16 = 16;
pub const B: u16 = 32;
pub const C: u16 = 64;
pub const START: u16 = 128;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid session: {0}")]
    Invalid(String),
    #[error("session JSON: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SessionRevision {
    Rev02,
    Rev00,
}
impl From<Revision> for SessionRevision {
    fn from(revision: Revision) -> Self {
        match revision {
            Revision::Rev02 => Self::Rev02,
            Revision::Rev00 => Self::Rev00,
        }
    }
}
impl From<SessionRevision> for Revision {
    fn from(revision: SessionRevision) -> Self {
        match revision {
            SessionRevision::Rev02 => Self::Rev02,
            SessionRevision::Rev00 => Self::Rev00,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Start {
    PowerOn,
    /// The inputs still begin at power-on. Writes happen before the indicated
    /// video frame on *every* runner. There is never an opaque saved state.
    Recipe {
        description: String,
        writes: Vec<RamWrite>,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RamWrite {
    pub before_frame: u64,
    /// Canonical FF0000..FFFFFF work RAM only; no ROM or device writes.
    pub address: u32,
    /// Exactly 1, 2, or 4 bytes; big-endian, just like the original machine.
    pub width: u8,
    pub value: u32,
    pub reason: String,
}

/// A coupled duration and two pad masks. Named wire fields keep each input
/// record self-describing without repeating redundant per-frame states.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(from = "RunFields", into = "RunFields")]
pub struct Run(pub u64, pub u16, pub u16);

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RunFields {
    frames: u64,
    pads: [u16; 2],
}
impl From<Run> for RunFields {
    fn from(Run(frames, p1, p2): Run) -> Self {
        Self {
            frames,
            pads: [p1, p2],
        }
    }
}
impl From<RunFields> for Run {
    fn from(run: RunFields) -> Self {
        Self(run.frames, run.pads[0], run.pads[1])
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Session {
    pub format: u32,
    pub revision: SessionRevision,
    pub start: Start,
    pub frames: u64,
    pub tool: String,
    pub description: String,
    pub runs: Vec<Run>,
}

impl Session {
    pub fn new(revision: Revision, description: impl Into<String>) -> Self {
        Self {
            format: FORMAT_VERSION,
            revision: revision.into(),
            start: Start::PowerOn,
            frames: 0,
            tool: TOOL_VERSION.into(),
            description: description.into(),
            runs: Vec::new(),
        }
    }

    /// Append one frame without allocating unless the controller state changes.
    pub fn record(&mut self, pads: [u16; 2]) -> Result<(), Error> {
        validate_pads(pads)?;
        let frames = self
            .frames
            .checked_add(1)
            .ok_or_else(|| invalid("frame overflow"))?;
        if let Some(Run(count, p1, p2)) = self.runs.last_mut()
            && [*p1, *p2] == pads
        {
            *count += 1;
        } else {
            self.runs.push(Run(1, pads[0], pads[1]));
        }
        self.frames = frames;
        Ok(())
    }

    pub fn validate(&self) -> Result<(), Error> {
        if self.format != FORMAT_VERSION {
            return Err(invalid("unsupported format version"));
        }
        if self.tool.trim().is_empty() {
            return Err(invalid("missing tool version"));
        }
        let mut sum = 0_u64;
        let mut previous = None;
        for &Run(frames, p1, p2) in &self.runs {
            if frames == 0 {
                return Err(invalid("zero-length run"));
            }
            validate_pads([p1, p2])?;
            if previous == Some([p1, p2]) {
                return Err(invalid("adjacent identical runs must be combined"));
            }
            previous = Some([p1, p2]);
            sum = sum
                .checked_add(frames)
                .ok_or_else(|| invalid("frame overflow"))?;
        }
        if sum != self.frames {
            return Err(invalid("header frame count does not equal run lengths"));
        }
        if let Start::Recipe {
            description,
            writes,
        } = &self.start
        {
            if description.trim().is_empty() || writes.is_empty() {
                return Err(invalid("a recipe must document its RAM writes"));
            }
            let mut previous = 0;
            for write in writes {
                if !matches!(write.width, 1 | 2 | 4)
                    || write.address < 0xFF0000
                    || write.address > 0x1000000 - u32::from(write.width)
                    || (write.width > 1 && write.address & 1 != 0)
                    || (write.width < 4 && write.value >= 1 << (8 * write.width))
                    || write.before_frame >= self.frames
                    || write.before_frame < previous
                    || write.reason.trim().is_empty()
                {
                    return Err(invalid("invalid, undocumented, or unsorted recipe write"));
                }
                previous = write.before_frame;
            }
        }
        Ok(())
    }

    pub fn from_json(json: &str) -> Result<Self, Error> {
        let session: Self = serde_json::from_str(json)?;
        session.validate()?;
        Ok(session)
    }

    pub fn to_json(&self) -> Result<String, Error> {
        self.validate()?;
        Ok(serde_json::to_string(self)? + "\n")
    }

    pub fn inputs(&self) -> impl Iterator<Item = [u16; 2]> + '_ {
        self.runs
            .iter()
            .flat_map(|&Run(count, p1, p2)| (0..count).map(move |_| [p1, p2]))
    }

    pub fn writes(&self) -> &[RamWrite] {
        match &self.start {
            Start::PowerOn => &[],
            Start::Recipe { writes, .. } => writes,
        }
    }
}

pub fn encode_pad(connected: bool, buttons: [bool; 8]) -> u16 {
    if !connected {
        return 0;
    }
    buttons
        .into_iter()
        .enumerate()
        .fold(CONNECTED, |state, (bit, pressed)| {
            state | (u16::from(pressed) << bit)
        })
}

fn validate_pads(pads: [u16; 2]) -> Result<(), Error> {
    if pads
        .into_iter()
        .any(|pad| pad & !0x1FF != 0 || (pad & CONNECTED == 0 && pad != 0))
    {
        return Err(invalid(
            "pad must be disconnected (0) or a three-button mask with bit 8 set",
        ));
    }
    Ok(())
}
fn invalid(message: &str) -> Error {
    Error::Invalid(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_round_trip_and_exact_frame_expansion() {
        let mut session = Session::new(Revision::Rev02, "synthetic inputs");
        for _ in 0..300 {
            session.record([CONNECTED, 0]).unwrap();
        }
        session.record([CONNECTED | START, CONNECTED | A]).unwrap();
        session.record([CONNECTED, 0]).unwrap();
        assert_eq!(session.runs.len(), 3);
        assert_eq!(session.frames, 302);
        let restored = Session::from_json(&session.to_json().unwrap()).unwrap();
        assert_eq!(restored, session);
        let mut wire: serde_json::Value =
            serde_json::from_str(&session.to_json().unwrap()).unwrap();
        assert_eq!(wire["runs"][0]["frames"], 300);
        assert_eq!(wire["runs"][0]["pads"], serde_json::json!([CONNECTED, 0]));
        wire["runs"][0]["snapshot"] = serde_json::json!(true);
        assert!(Session::from_json(&wire.to_string()).is_err());
        assert_eq!(
            restored.inputs().nth(300),
            Some([CONNECTED | START, CONNECTED | A])
        );
        assert_eq!(restored.inputs().count(), 302);
        assert_eq!(encode_pad(false, [true; 8]), 0);
        assert_eq!(encode_pad(true, [true; 8]), 0x1FF);
    }

    #[test]
    fn rejects_corruption_and_noncanonical_runs() {
        let mut session = Session::new(Revision::Rev00, "synthetic");
        session.record([CONNECTED, CONNECTED]).unwrap();
        session.frames += 1;
        assert!(session.validate().is_err());
        session.runs.push(Run(1, CONNECTED, CONNECTED));
        assert!(session.validate().is_err());
        assert!(session.record([1, 0]).is_err());
        assert!(session.record([0x200, 0]).is_err());
        let json = session.to_json();
        assert!(json.is_err());
    }

    #[test]
    fn recipe_is_bounded_documented_and_not_a_snapshot() {
        let mut session = Session::new(Revision::Rev00, "synthetic recipe");
        session.record([CONNECTED, 0]).unwrap();
        session.start = Start::Recipe {
            description: "exercise a synthetic RAM value".into(),
            writes: vec![RamWrite {
                before_frame: 0,
                address: 0xFF8000,
                width: 2,
                value: 7,
                reason: "synthetic test only".into(),
            }],
        };
        assert!(session.validate().is_ok());
        if let Start::Recipe { writes, .. } = &mut session.start {
            writes[0].address = 0xA04000;
        }
        assert!(session.validate().is_err());
    }
}
