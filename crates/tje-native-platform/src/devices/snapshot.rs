//! Save only mutable hardware, never the cartridge or opt-in proof journals.
use super::*;
use bincode::{Decode, Encode};
use jgenesis_common::boxedarray::BoxedByteArray;
use sha1::{Digest, Sha1};

// The borrowed encoding and owned decoding deliberately have identical field
// order. Fixed arrays have no length prefix; BoxedByteArray decodes on the heap.
#[derive(Encode)]
struct Capture<'a> {
    ram: &'a [u8; RAM_SIZE],
    audio_ram: &'a [u8; 0x2000],
    vdp: &'a Vdp,
    ym2612: &'a Ym2612,
    psg: &'a Sn76489,
    input: &'a InputState,
    cycles: CycleCounters,
    timing_mode: TimingMode,
    region: GenesisRegion,
    bank: u32,
    bus_request: bool,
    reset: bool,
    open_bus: u16,
    read_offset: u64,
    origin: Origin,
    main_pc: u32,
    audio_pc: u32,
    opcode: u16,
    pending_bytes: &'a [(u32, u8)],
    pending_words: &'a [(u32, u16)],
    z80: &'a Z80,
    audio_resampler: &'a GenesisAudioResampler,
    instruction_open: bool,
    frames: u64,
    main_reset: bool,
    audio: &'a [[f64; 2]],
    input_clock_remainder: u64,
    audio_sample_rate: u32,
    config: &'a GenesisEmulatorConfig,
}

#[derive(Decode)]
struct Restore {
    ram: BoxedByteArray<RAM_SIZE>,
    audio_ram: BoxedByteArray<0x2000>,
    vdp: Vdp,
    ym2612: Ym2612,
    psg: Sn76489,
    input: InputState,
    cycles: CycleCounters,
    timing_mode: TimingMode,
    region: GenesisRegion,
    bank: u32,
    bus_request: bool,
    reset: bool,
    open_bus: u16,
    read_offset: u64,
    origin: Origin,
    main_pc: u32,
    audio_pc: u32,
    opcode: u16,
    pending_bytes: Vec<(u32, u8)>,
    pending_words: Vec<(u32, u16)>,
    z80: Z80,
    audio_resampler: GenesisAudioResampler,
    instruction_open: bool,
    frames: u64,
    main_reset: bool,
    audio: Vec<[f64; 2]>,
    input_clock_remainder: u64,
    audio_sample_rate: u32,
    config: GenesisEmulatorConfig,
}

fn codec() -> impl bincode::config::Config {
    bincode::config::standard()
        .with_fixed_int_encoding()
        .with_limit::<{ 16 * 1024 * 1024 }>()
}

/// Decoded-but-uncommitted hardware for cross-owner restore validation. No MMIO
/// reads, clock progression or copies are performed while this view is borrowed.
pub struct SnapshotView<'a> {
    pub ram: &'a [u8; RAM_SIZE],
    pub vram: &'a [u8],
    pub cram: &'a [u16],
    pub frames: u64,
}

impl DeviceRuntime {
    /// Computed lazily: faithful play never hashes or copies the ROM for extras.
    pub fn snapshot_rom_hash(&self) -> [u8; 20] {
        *self
            .snapshot_rom_hash
            .get_or_init(|| Sha1::digest(self.bus.memory.rom()).into())
    }

    /// Append the exact hardware image to a reusable allocation. No device is
    /// clocked, synchronized or read through MMIO while capturing.
    pub fn encode_snapshot(&self, output: &mut Vec<u8>) -> Result<(), String> {
        let bus = &self.bus;
        bincode::encode_into_std_write(
            Capture {
                ram: bus.memory.ram(),
                audio_ram: &bus.audio_ram,
                vdp: &bus.vdp,
                ym2612: &bus.ym2612,
                psg: &bus.psg,
                input: &bus.input,
                cycles: bus.cycles,
                timing_mode: bus.timing_mode,
                region: bus.region,
                bank: bus.bank,
                bus_request: bus.bus_request,
                reset: bus.reset,
                open_bus: bus.open_bus,
                read_offset: bus.read_offset,
                origin: bus.origin,
                main_pc: bus.main_pc,
                audio_pc: bus.audio_pc,
                opcode: bus.opcode,
                pending_bytes: &bus.pending_bytes,
                pending_words: &bus.pending_words,
                z80: &self.z80,
                audio_resampler: &self.audio_resampler,
                instruction_open: self.instruction_open,
                frames: self.frames,
                main_reset: self.main_reset,
                audio: &self.audio.0,
                input_clock_remainder: self.input_clock_remainder,
                audio_sample_rate: self.audio_sample_rate,
                config: &self.config,
            },
            output,
            codec(),
        )
        .map(|_| ())
        .map_err(|error| error.to_string())
    }

    /// Decode and validate fully before replacing anything. Immutable cartridge
    /// storage, its process identity and observer enablement are retained.
    pub fn restore_snapshot(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.restore_snapshot_checked(bytes, |_| Ok(()))
    }

    pub fn restore_snapshot_checked(
        &mut self,
        bytes: &[u8],
        validate: impl FnOnce(SnapshotView<'_>) -> Result<(), String>,
    ) -> Result<(), String> {
        let (state, used): (Restore, _) =
            bincode::decode_from_slice(bytes, codec()).map_err(|error| error.to_string())?;
        if used != bytes.len() {
            return Err("snapshot has trailing hardware data".into());
        }
        if state.config.clamped_m68k_divider() != self.config.clamped_m68k_divider() {
            return Err("snapshot CPU divider does not match the compiled engine".into());
        }
        if !(8_000..=192_000).contains(&state.audio_sample_rate) {
            return Err("snapshot audio sample rate must be between 8000 and 192000 Hz".into());
        }
        let (vram, cram, _) = state.vdp.hybrid_memories();
        validate(SnapshotView {
            ram: state
                .ram
                .as_slice()
                .try_into()
                .expect("fixed decoded RAM extent"),
            vram,
            cram,
            frames: state.frames,
        })?;
        let bus = &mut self.bus;
        bus.memory.ram_mut().copy_from_slice(state.ram.as_slice());
        bus.audio_ram.copy_from_slice(state.audio_ram.as_slice());
        bus.vdp = state.vdp;
        bus.ym2612 = state.ym2612;
        bus.psg = state.psg;
        bus.input = state.input;
        bus.cycles = state.cycles;
        bus.timing_mode = state.timing_mode;
        bus.region = state.region;
        bus.bank = state.bank;
        bus.bus_request = state.bus_request;
        bus.reset = state.reset;
        bus.open_bus = state.open_bus;
        bus.read_offset = state.read_offset;
        bus.origin = state.origin;
        bus.main_pc = state.main_pc;
        bus.audio_pc = state.audio_pc;
        bus.opcode = state.opcode;
        bus.pending_bytes = state.pending_bytes;
        bus.pending_words = state.pending_words;
        // Warm instruction queues are retained even when the captured queues
        // were empty, so restoring does not reintroduce hot-path allocations.
        bus.pending_bytes.reserve(20);
        bus.pending_words.reserve(32);
        self.z80 = state.z80;
        self.audio_resampler = state.audio_resampler;
        self.instruction_open = state.instruction_open;
        self.frames = state.frames;
        self.main_reset = state.main_reset;
        self.audio.0 = state.audio;
        self.input_clock_remainder = state.input_clock_remainder;
        self.audio_sample_rate = state.audio_sample_rate;
        self.config = state.config;
        // Decoding keeps sample contents, not Vec capacity. Preserve one full
        // output frame of headroom even for a power-on/empty-audio snapshot.
        let capacity = (f64::from(self.audio_sample_rate) / self.target_fps()).ceil() as usize + 16;
        self.audio
            .0
            .reserve(capacity.saturating_sub(self.audio.0.len()));
        self.clear_trace();
        self.clear_ram_write_trace();
        self.reserve_startup_buffers();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tje_mem::Revision;

    #[test]
    fn restoring_empty_audio_retains_warm_output_queue_headroom() {
        let mut devices = DeviceRuntime::new(
            AddressSpace::new(vec![0; 1024], Revision::Rev02),
            &GenesisEmulatorConfig::default(),
        );
        let mut bytes = Vec::new();
        devices.encode_snapshot(&mut bytes).unwrap();
        devices.restore_snapshot(&bytes).unwrap();
        let required =
            (f64::from(devices.audio_sample_rate) / devices.target_fps()).ceil() as usize + 16;
        assert!(devices.audio_samples().is_empty());
        assert!(devices.audio.0.capacity() >= required);
    }

    #[test]
    fn invalid_saved_rate_is_rejected_before_mutation_or_queue_reservation() {
        let mut devices = DeviceRuntime::new(
            AddressSpace::new(vec![0; 1024], Revision::Rev02),
            &GenesisEmulatorConfig::default(),
        );
        devices.audio_sample_rate = u32::MAX;
        let mut bytes = Vec::new();
        devices.encode_snapshot(&mut bytes).unwrap();
        devices.audio_sample_rate = 48_000;
        assert!(devices.restore_snapshot(&bytes).is_err());
        assert_eq!(devices.audio_sample_rate, 48_000);
    }
}
