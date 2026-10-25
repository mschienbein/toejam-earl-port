//! Opt-in, ROM-bound complete machine images. Compiled code is immutable and
//! retained in the destination backend; no cartridge bytes are in an image.
use crate::{Backend, CpuState, Machine, PlatformDevices};
use sha1::{Digest, Sha1};
use std::{collections::VecDeque, fmt};

pub const SNAPSHOT_VERSION: u32 = 2;
const MAGIC: &[u8; 8] = b"TJESTATE";
const CPU_OFFSET: usize = 40;
const PROFILE_OFFSET: usize = CPU_OFFSET + 25 * 4;
const HARDWARE_OFFSET: usize = PROFILE_OFFSET + 20;
const CHECKSUM_LEN: usize = 20;
const MAX_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotError {
    InvalidFormat,
    UnsupportedVersion(u32),
    RomMismatch,
    ProfileMismatch,
    Corrupt,
    Hardware(String),
}
impl fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFormat => f.write_str("not a ToeJam & Earl machine snapshot"),
            Self::UnsupportedVersion(version) => write!(
                f,
                "unsupported snapshot format {version}; expected {SNAPSHOT_VERSION}"
            ),
            Self::RomMismatch => f.write_str("snapshot belongs to a different ROM"),
            Self::ProfileMismatch => {
                f.write_str("snapshot belongs to a different execution profile")
            }
            Self::Corrupt => f.write_str("snapshot is truncated or corrupt"),
            Self::Hardware(error) => write!(f, "invalid snapshot hardware state: {error}"),
        }
    }
}
impl std::error::Error for SnapshotError {}

/// Contains copyrighted runtime memory. Persist only in private local storage,
/// never in sessions, reports, source control or a public generated-code cache.
#[derive(Debug, Clone, Default)]
pub struct MachineSnapshot {
    bytes: Vec<u8>,
}
impl MachineSnapshot {
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, SnapshotError> {
        validate(&bytes)?;
        Ok(Self { bytes })
    }
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
    pub fn byte_len(&self) -> usize {
        self.bytes.len()
    }
    pub fn frame_count(&self) -> u64 {
        self.bytes
            .get(32..40)
            .map_or(0, |bytes| u64::from_le_bytes(bytes.try_into().unwrap()))
    }
    /// SHA-1 of the complete canonical image, including frame/input/audio state.
    /// Suitable for deterministic divergence detection, not authentication.
    pub fn state_hash(&self) -> [u8; 20] {
        self.bytes
            .get(self.bytes.len().saturating_sub(CHECKSUM_LEN)..)
            .and_then(|bytes| bytes.try_into().ok())
            .unwrap_or([0; 20])
    }
}

fn validate(bytes: &[u8]) -> Result<(), SnapshotError> {
    if bytes.len() < HARDWARE_OFFSET + CHECKSUM_LEN || bytes.len() > MAX_BYTES {
        return Err(SnapshotError::Corrupt);
    }
    if bytes.get(..8) != Some(MAGIC.as_slice()) {
        return Err(SnapshotError::InvalidFormat);
    }
    let version = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    if version != SNAPSHOT_VERSION {
        return Err(SnapshotError::UnsupportedVersion(version));
    }
    let payload_end = bytes.len() - CHECKSUM_LEN;
    if Sha1::digest(&bytes[..payload_end]).as_slice() != &bytes[payload_end..] {
        return Err(SnapshotError::Corrupt);
    }
    Ok(())
}

fn profile_hash(profile: &str) -> [u8; 20] {
    static FAITHFUL: std::sync::LazyLock<[u8; 20]> =
        std::sync::LazyLock::new(|| Sha1::digest(b"faithful").into());
    if profile == "faithful" {
        *FAITHFUL
    } else {
        Sha1::digest(profile.as_bytes()).into()
    }
}

impl<B: Backend> Machine<PlatformDevices, B> {
    pub fn snapshot(&self) -> Result<MachineSnapshot, SnapshotError> {
        self.snapshot_with_profile("faithful")
    }

    /// Bind a stateless enhanced execution profile to the machine image. The
    /// identifier must include its version/config identity. Host sidecars still
    /// require an outer atomic envelope; this method cannot capture them.
    pub fn snapshot_with_profile(&self, profile: &str) -> Result<MachineSnapshot, SnapshotError> {
        let mut snapshot = MachineSnapshot::default();
        self.snapshot_into_with_profile(&mut snapshot, profile)?;
        Ok(snapshot)
    }

    /// Reuse an existing image allocation for rollback/rewind capture. The
    /// caller must be at a completed runtime step, not inside a backend callback.
    pub fn snapshot_into(&self, snapshot: &mut MachineSnapshot) -> Result<(), SnapshotError> {
        self.snapshot_into_with_profile(snapshot, "faithful")
    }

    pub fn snapshot_into_with_profile(
        &self,
        snapshot: &mut MachineSnapshot,
        profile: &str,
    ) -> Result<(), SnapshotError> {
        let bytes = &mut snapshot.bytes;
        bytes.clear();
        // A 44.1 kHz image is slightly larger than 1 MiB. Leave headroom so
        // normal native capture does not grow/copy its entire image or double
        // the allocation charged to the rewind ring.
        bytes.reserve_exact(1152 * 1024);
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&SNAPSHOT_VERSION.to_le_bytes());
        bytes.extend_from_slice(&self.devices.snapshot_rom_hash());
        bytes.extend_from_slice(&self.frames.to_le_bytes());
        let cpu = &self.cpu;
        for word in cpu.d.into_iter().chain(cpu.a).chain([
            cpu.usp,
            cpu.ssp,
            cpu.pc,
            cpu.sr,
            cpu.prefetch,
            cpu.pending_interrupt,
            cpu.stopped,
            cpu.frozen,
            cpu.address_error,
            cpu.last_muldiv,
        ]) {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        bytes.extend_from_slice(&profile_hash(profile));
        self.devices
            .encode_snapshot(bytes)
            .map_err(SnapshotError::Hardware)?;
        let checksum = Sha1::digest(&*bytes);
        bytes.extend_from_slice(&checksum);
        Ok(())
    }

    /// Restore transactionally: malformed/version/ROM/config errors do not
    /// modify the current machine. No reset, input poll or clock step occurs.
    /// Proof journals are cleared, their enablement and compiled code retained.
    pub fn restore(&mut self, snapshot: &MachineSnapshot) -> Result<(), SnapshotError> {
        self.restore_with_profile(snapshot, "faithful")
    }

    pub fn restore_with_profile(
        &mut self,
        snapshot: &MachineSnapshot,
        profile: &str,
    ) -> Result<(), SnapshotError> {
        let bytes = &snapshot.bytes;
        validate(bytes)?;
        if bytes[12..32] != self.devices.snapshot_rom_hash() {
            return Err(SnapshotError::RomMismatch);
        }
        if bytes[PROFILE_OFFSET..HARDWARE_OFFSET] != profile_hash(profile) {
            return Err(SnapshotError::ProfileMismatch);
        }
        let mut words = bytes[CPU_OFFSET..PROFILE_OFFSET]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|bytes| u32::from_le_bytes(*bytes));
        let cpu = CpuState {
            d: std::array::from_fn(|_| words.next().unwrap()),
            a: std::array::from_fn(|_| words.next().unwrap()),
            usp: words.next().unwrap(),
            ssp: words.next().unwrap(),
            pc: words.next().unwrap(),
            sr: words.next().unwrap(),
            prefetch: words.next().unwrap(),
            pending_interrupt: words.next().unwrap(),
            stopped: words.next().unwrap(),
            frozen: words.next().unwrap(),
            address_error: words.next().unwrap(),
            last_muldiv: words.next().unwrap(),
        };
        self.devices
            .restore_snapshot(&bytes[HARDWARE_OFFSET..bytes.len() - CHECKSUM_LEN])
            .map_err(SnapshotError::Hardware)?;
        self.cpu = cpu;
        self.frames = snapshot.frame_count();
        self.devices.clear_instruction_trace();
        Ok(())
    }
}

/// Oldest-first ring with a hard allocation budget, not an unbounded duration.
/// Full images are sampled by the frontend; no state exists while disabled.
#[derive(Debug)]
pub struct RewindBuffer {
    images: VecDeque<MachineSnapshot>,
    used: usize,
    budget: usize,
}
impl RewindBuffer {
    pub fn new(budget: usize) -> Self {
        Self {
            images: VecDeque::new(),
            used: 0,
            budget,
        }
    }
    pub fn clear(&mut self) {
        self.images = VecDeque::new();
        self.used = 0;
    }
    pub fn used_bytes(&self) -> usize {
        self.used
    }
    pub fn len(&self) -> usize {
        self.images.len()
    }
    pub fn is_empty(&self) -> bool {
        self.images.is_empty()
    }
    pub fn push(&mut self, snapshot: MachineSnapshot) {
        let cost = snapshot.bytes.capacity() + std::mem::size_of::<MachineSnapshot>();
        if cost > self.budget {
            return;
        }
        while self.used + cost > self.budget || self.images.len() >= 1024 {
            self.remove_front();
        }
        self.used += cost;
        self.images.push_back(snapshot);
    }
    fn remove_front(&mut self) {
        if let Some(image) = self.images.pop_front() {
            self.used -= image.bytes.capacity() + std::mem::size_of::<MachineSnapshot>();
        }
    }
    fn pop(&mut self) -> Option<MachineSnapshot> {
        let image = self.images.pop_back()?;
        self.used -= image.bytes.capacity() + std::mem::size_of::<MachineSnapshot>();
        Some(image)
    }
    /// Drop the current/future branch and return the next earlier image.
    pub fn pop_before(&mut self, frame: u64) -> Option<MachineSnapshot> {
        while self
            .images
            .back()
            .is_some_and(|image| image.frame_count() >= frame)
        {
            self.pop();
        }
        self.pop()
    }
}

#[cfg(test)]
mod tests;
