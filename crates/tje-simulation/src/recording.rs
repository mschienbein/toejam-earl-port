//! Controller/configuration-only enhanced recordings. Faithful tje-session stays
//! exactly two-pad. No RAM writes, core images, ROM resources or function pointers.
use crate::{Error, Result, SeatId, SeatMask, session::SessionConfig};
use bincode::{Decode, Encode};
use sha2::{Digest, Sha256};
use tje_four_player::input::{InputFrame, Router, SourceId};

const MAGIC: &[u8; 8] = b"TJEINPUT";
const VERSION: u16 = 2;
const HEADER_BYTES: usize = 48;
pub const MAX_RECORDING_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_RUNS: usize = 262_144;
pub const MAX_ASSIGNMENTS: usize = 65_536;

fn codec() -> impl bincode::config::Config {
    bincode::config::standard()
        .with_fixed_int_encoding()
        .with_little_endian()
        .with_limit::<MAX_RECORDING_BYTES>()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct Run {
    pub frames: u64,
    pub pads: [u16; 4],
    pub actions: [u16; 4],
}

/// An explicit atomic binding/reconnect/swap event. Disconnecting a source never
/// creates an assignment event; that seat simply records a disconnected pad.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct Assignment {
    pub before_frame: u64,
    pub epoch: u64,
    pub sources: [Option<SourceId>; 4],
}
impl Assignment {
    pub fn apply(&self, router: &mut Router) -> Result<()> {
        if router.assignment_epoch().checked_add(1) != Some(self.epoch) {
            return Err(Error::State(
                "recorded assignment epoch is stale or skipped",
            ));
        }
        let changes =
            std::array::from_fn::<_, 4, _>(|index| (SeatId::ALL[index], self.sources[index]));
        router
            .reassign(&changes)
            .map_err(|_| Error::State("invalid recorded source assignment"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct Recording {
    configuration: SessionConfig,
    configuration_identity: [u8; 32],
    initial_router: Router,
    frames: u64,
    runs: Vec<Run>,
    assignments: Vec<Assignment>,
}
impl Recording {
    pub fn new(mut configuration: SessionConfig, initial_router: Router) -> Result<Self> {
        configuration.validate()?;
        if configuration.roles.director.is_some()
            || configuration.roles.inputs != configuration.roles.heroes
        {
            return Err(Error::Ownership(
                "director role/command recording schema is not admitted",
            ));
        }
        initial_router
            .validate()
            .map_err(|_| Error::State("invalid initial recorded source routing"))?;
        if initial_router.enabled() != configuration.roles.inputs
            || initial_router.swap_ac() != configuration.coop.swap_ac
        {
            return Err(Error::State(
                "recorded router differs from immutable input configuration",
            ));
        }
        let configuration_identity = configuration.identity();
        // A controller recording carries settings and catalogue identity, never
        // the imported ROM animation contracts used by private snapshots.
        configuration.animations = Default::default();
        Ok(Self {
            configuration,
            configuration_identity,
            initial_router,
            frames: 0,
            runs: Vec::new(),
            assignments: Vec::new(),
        })
    }
    pub const fn configuration_identity(&self) -> &[u8; 32] {
        &self.configuration_identity
    }
    /// Assetless normalized settings. Bind the actual catalogue before play.
    pub fn configuration(&self) -> &SessionConfig {
        &self.configuration
    }
    pub fn initial_router(&self) -> &Router {
        &self.initial_router
    }
    pub const fn frames(&self) -> u64 {
        self.frames
    }
    pub fn runs(&self) -> &[Run] {
        &self.runs
    }
    pub fn assignments(&self) -> &[Assignment] {
        &self.assignments
    }
    /// Authenticate against the actual admitted ROM/configuration, not merely
    /// the digest claimed by a file. Generator/roles/caps/resources are included.
    pub fn authenticate(&self, expected: &SessionConfig) -> Result<()> {
        expected.validate()?;
        if self.configuration_identity != expected.identity()
            || self.configuration.settings_identity() != expected.settings_identity()
        {
            return Err(Error::Ownership(
                "recording ROM/profile/generator/role/configuration mismatch",
            ));
        }
        Ok(())
    }
    pub fn record(&mut self, input: InputFrame) -> Result<()> {
        if input.frame != self.frames {
            return Err(Error::State(
                "enhanced recording input frames must be consecutive from zero",
            ));
        }
        let sources = self
            .assignments
            .last()
            .map_or(self.initial_router.sources(), |event| &event.sources);
        validate_input(&input, self.configuration.roles.inputs, sources)?;
        let frames = self
            .frames
            .checked_add(1)
            .ok_or(Error::Capacity("enhanced recording frame count"))?;
        let pads = input.pads.map(|pad| pad.bits());
        if let Some(previous) = self.runs.last_mut()
            && previous.pads == pads
            && previous.actions == input.actions
        {
            previous.frames += 1;
        } else {
            if self.runs.len() >= MAX_RUNS {
                return Err(Error::Capacity("enhanced recording input runs"));
            }
            self.runs.push(Run {
                frames: 1,
                pads,
                actions: input.actions,
            });
        }
        self.frames = frames;
        Ok(())
    }
    /// Call only after an explicit frontend lobby transaction has succeeded.
    /// Captures its real epoch and complete candidate binding map atomically.
    pub fn record_assignment(&mut self, router: &Router) -> Result<()> {
        router
            .validate()
            .map_err(|_| Error::State("invalid recorded lobby routing"))?;
        let previous = self
            .assignments
            .last()
            .map_or(self.initial_router.assignment_epoch(), |event| event.epoch);
        if previous.checked_add(1) != Some(router.assignment_epoch())
            || router.enabled() != self.configuration.roles.inputs
            || router.swap_ac() != self.configuration.coop.swap_ac
        {
            return Err(Error::State(
                "recorded lobby event changes configuration or skips its epoch",
            ));
        }
        if self.assignments.len() >= MAX_ASSIGNMENTS {
            return Err(Error::Capacity("recorded lobby assignments"));
        }
        self.assignments.push(Assignment {
            before_frame: self.frames,
            epoch: router.assignment_epoch(),
            sources: *router.sources(),
        });
        Ok(())
    }
    pub fn validate(&self) -> Result<()> {
        self.configuration.validate_settings()?;
        if self.configuration.roles.director.is_some()
            || self.configuration.roles.inputs != self.configuration.roles.heroes
        {
            return Err(Error::Ownership(
                "director role/command recording schema is not admitted",
            ));
        }
        if self.configuration.animations != Default::default() {
            return Err(Error::State(
                "controller recording contains imported resource programs",
            ));
        }
        self.initial_router
            .validate()
            .map_err(|_| Error::State("invalid initial recorded routing"))?;
        if self.frames == 0
            || self.runs.is_empty()
            || self.runs.len() > MAX_RUNS
            || self.assignments.len() > MAX_ASSIGNMENTS
            || self.initial_router.enabled() != self.configuration.roles.inputs
            || self.initial_router.swap_ac() != self.configuration.coop.swap_ac
        {
            return Err(Error::State("enhanced recording extent/configuration"));
        }
        if self
            .assignments
            .windows(2)
            .any(|pair| pair[0].before_frame > pair[1].before_frame)
            || self
                .assignments
                .last()
                .is_some_and(|event| event.before_frame > self.frames)
        {
            return Err(Error::State("recorded assignments are not in frame order"));
        }
        let mut router = self.initial_router.clone();
        let mut assignments = self.assignments.iter().peekable();
        let mut start = 0_u64;
        let mut previous = None;
        for &run in &self.runs {
            if run.frames == 0 || previous == Some((run.pads, run.actions)) {
                return Err(Error::State(
                    "zero or noncanonical adjacent enhanced input runs",
                ));
            }
            let end = start
                .checked_add(run.frames)
                .ok_or(Error::Capacity("enhanced recording duration"))?;
            while assignments
                .peek()
                .is_some_and(|event| event.before_frame <= start)
            {
                assignments
                    .next()
                    .expect("peeked assignment")
                    .apply(&mut router)?;
            }
            let input = InputFrame::new(start, run.pads, run.actions)
                .map_err(|_| Error::State("invalid enhanced recording pad/action words"))?;
            validate_input(&input, self.configuration.roles.inputs, router.sources())?;
            while assignments
                .peek()
                .is_some_and(|event| event.before_frame < end)
            {
                assignments
                    .next()
                    .expect("peeked assignment")
                    .apply(&mut router)?;
                validate_input(&input, self.configuration.roles.inputs, router.sources())?;
            }
            previous = Some((run.pads, run.actions));
            start = end;
        }
        for event in assignments {
            event.apply(&mut router)?;
        }
        if start != self.frames {
            return Err(Error::State(
                "enhanced recording duration differs from its runs",
            ));
        }
        Ok(())
    }
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        self.validate()?;
        // Encode directly into the final envelope; never allocate/copy a second
        // full recording body just to prepend its header.
        let mut bytes = vec![0; HEADER_BYTES];
        bincode::encode_into_std_write(self, &mut bytes, codec())
            .map_err(|_| Error::State("enhanced recording encoding failed"))?;
        let body_length = bytes.len() - HEADER_BYTES;
        if body_length > MAX_RECORDING_BYTES {
            return Err(Error::Capacity("enhanced recording encoded size"));
        }
        let checksum = Sha256::digest(&bytes[HEADER_BYTES..]);
        bytes[..8].copy_from_slice(MAGIC);
        bytes[8..10].copy_from_slice(&VERSION.to_le_bytes());
        bytes[12..16].copy_from_slice(&(body_length as u32).to_le_bytes());
        bytes[16..HEADER_BYTES].copy_from_slice(&checksum);
        Ok(bytes)
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < HEADER_BYTES
            || &bytes[..8] != MAGIC
            || u16::from_le_bytes(bytes[8..10].try_into().expect("two version bytes")) != VERSION
            || bytes[10..12] != [0; 2]
        {
            return Err(Error::State("unsupported enhanced recording envelope"));
        }
        let length =
            u32::from_le_bytes(bytes[12..16].try_into().expect("four length bytes")) as usize;
        if length > MAX_RECORDING_BYTES || length.checked_add(HEADER_BYTES) != Some(bytes.len()) {
            return Err(Error::Capacity("enhanced recording length/decoding limit"));
        }
        let payload = &bytes[HEADER_BYTES..];
        if Sha256::digest(payload).as_slice() != &bytes[16..HEADER_BYTES] {
            return Err(Error::State("enhanced recording checksum mismatch"));
        }
        let (recording, used): (Self, _) = bincode::decode_from_slice(payload, codec())
            .map_err(|_| Error::State("invalid enhanced recording payload"))?;
        if used != payload.len() {
            return Err(Error::State("enhanced recording has trailing data"));
        }
        recording.validate()?;
        Ok(recording)
    }
    pub fn timeline(&self) -> Result<Timeline<'_>> {
        self.validate()?;
        let mut start = 0;
        let starts = self
            .runs
            .iter()
            .map(|run| {
                let value = start;
                start += run.frames;
                value
            })
            .collect();
        Ok(Timeline {
            recording: self,
            starts,
        })
    }
}

fn validate_input(
    input: &InputFrame,
    seats: SeatMask,
    sources: &[Option<SourceId>; 4],
) -> Result<()> {
    input
        .validate()
        .map_err(|_| Error::State("invalid enhanced input frame"))?;
    for seat in SeatId::ALL {
        let connected = input.pads[seat.index()].connected();
        if connected && (!seats.contains(seat) || sources[seat.index()].is_none()) {
            return Err(Error::State(
                "recorded input names a disabled or unassigned source",
            ));
        }
    }
    Ok(())
}

/// Derivative seek index. After rewind/restore, seek by the restored hardware
/// frame; never serialize a host iterator or silently keep its future position.
pub struct Timeline<'a> {
    recording: &'a Recording,
    starts: Vec<u64>,
}
impl Timeline<'_> {
    pub fn input_at(&self, frame: u64) -> Result<InputFrame> {
        if frame >= self.recording.frames {
            return Err(Error::State(
                "enhanced replay frame is outside the recording",
            ));
        }
        let index = self.starts.partition_point(|&start| start <= frame) - 1;
        let run = self.recording.runs[index];
        InputFrame::new(frame, run.pads, run.actions)
            .map_err(|_| Error::State("invalid indexed enhanced input words"))
    }
    pub fn assignments_before(&self, frame: u64) -> &[Assignment] {
        let events = &self.recording.assignments;
        let first = events.partition_point(|event| event.before_frame < frame);
        let end = events.partition_point(|event| event.before_frame <= frame);
        &events[first..end]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_tests::session;

    fn recording() -> Recording {
        let configuration = session(123).config().clone();
        let mut router = Router::new(SeatMask::ALL, configuration.coop.swap_ac).unwrap();
        let changes = SeatId::ALL.map(|seat| (seat, Some(SourceId([seat.get(); 16]))));
        router.reassign(&changes).unwrap();
        Recording::new(configuration, router).unwrap()
    }
    #[test]
    fn four_distinct_pads_and_actions_round_trip_without_core_or_ram_recipes() {
        let mut recording = recording();
        for frame in 0..8 {
            recording
                .record(InputFrame::new(frame, [0x101, 0x102, 0x104, 0x108], [1, 2, 4, 8]).unwrap())
                .unwrap();
        }
        assert_eq!(recording.runs.len(), 1);
        let bytes = recording.to_bytes().unwrap();
        let restored = Recording::from_bytes(&bytes).unwrap();
        assert_eq!(restored, recording);
        assert_eq!(restored.to_bytes().unwrap(), bytes);
        let timeline = restored.timeline().unwrap();
        assert_eq!(timeline.input_at(7).unwrap().actions, [1, 2, 4, 8]);
        assert_eq!(
            timeline.input_at(2).unwrap().pads.map(|pad| pad.bits()),
            [0x101, 0x102, 0x104, 0x108]
        );
        assert!(timeline.input_at(8).is_err());
        let wrong = session(124);
        assert!(restored.authenticate(wrong.config()).is_err());
        restored.authenticate(recording.configuration()).unwrap();
    }
    #[test]
    fn recordings_authenticate_imported_contracts_without_serializing_them() {
        let template = recording();
        let mut config = template.configuration.clone();
        config.catalogue_identity = [31; 32];
        config.frame_count = 2;
        config
            .animations
            .programs
            .push(crate::visual::ProgramContract {
                selections: 2,
                frame_indices: vec![0, 1],
                flips: [0; 4],
                period: Some(3),
            });
        let mut recording = Recording::new(config.clone(), template.initial_router).unwrap();
        recording.record(InputFrame::neutral(0)).unwrap();
        assert!(recording.configuration.animations.programs.is_empty());
        assert_eq!(recording.configuration_identity, config.identity());
        let restored = Recording::from_bytes(&recording.to_bytes().unwrap()).unwrap();
        restored.authenticate(&config).unwrap();
        assert!(restored.configuration.animations.programs.is_empty());
        config.animations.programs[0].frame_indices.swap(0, 1);
        assert!(restored.authenticate(&config).is_err());
    }
    #[test]
    fn explicit_same_frame_swaps_replay_once_and_disconnect_does_not_reassign() {
        let mut recording = recording();
        recording.record(InputFrame::neutral(0)).unwrap();
        let mut router = recording.initial_router.clone();
        let sources = *router.sources();
        router
            .reassign(&[(SeatId::ALL[0], sources[1]), (SeatId::ALL[1], sources[0])])
            .unwrap();
        recording.record_assignment(&router).unwrap();
        router.reassign(&[(SeatId::ALL[2], None)]).unwrap();
        recording.record_assignment(&router).unwrap();
        recording
            .record(InputFrame::new(1, [0x101, 0x102, 0, 0x108], [1, 2, 0, 8]).unwrap())
            .unwrap();
        let timeline = recording.timeline().unwrap();
        let mut replay = recording.initial_router.clone();
        for event in timeline.assignments_before(1) {
            event.apply(&mut replay).unwrap();
        }
        assert_eq!(replay, router);
        assert!(
            timeline.assignments_before(1)[0]
                .apply(&mut replay)
                .is_err()
        );
        assert_eq!(recording.assignments.len(), 2);
        assert_eq!(timeline.input_at(0).unwrap(), InputFrame::neutral(0));
    }
    #[test]
    fn corrupt_unknown_trailing_and_oversized_envelopes_fail() {
        let mut recording = recording();
        recording.record(InputFrame::neutral(0)).unwrap();
        let bytes = recording.to_bytes().unwrap();
        for index in [0, 8, 10, 12, 16, HEADER_BYTES] {
            let mut bad = bytes.clone();
            bad[index] ^= 1;
            assert!(Recording::from_bytes(&bad).is_err());
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(Recording::from_bytes(&trailing).is_err());
        let mut excessive = bytes;
        excessive[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(Recording::from_bytes(&excessive).is_err());
    }
    #[test]
    fn malformed_events_and_unassigned_input_reject_before_recording_mutation() {
        let mut recording = recording();
        let initial = recording.clone();
        assert!(recording.record(InputFrame::neutral(1)).is_err());
        assert_eq!(recording, initial);
        let mut router = recording.initial_router.clone();
        router.reassign(&[(SeatId::ALL[0], None)]).unwrap();
        recording.record_assignment(&router).unwrap();
        let before = recording.clone();
        assert!(
            recording
                .record(InputFrame::new(0, [0x101, 0, 0, 0], [0; 4]).unwrap())
                .is_err()
        );
        assert_eq!(recording, before);
        recording.record(InputFrame::neutral(0)).unwrap();
        recording.assignments[0].epoch += 1;
        assert!(recording.validate().is_err());
    }
}
