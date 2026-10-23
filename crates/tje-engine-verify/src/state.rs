use serde::{Deserialize, Serialize};

/// Positions are in the reference's master-clock domain, not nominal 60 Hz time.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    pub pc: u32,
    pub master_clock: u64,
    pub scanline: u16,
    pub line_master_clock: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CpuSnapshot {
    pub data: [u32; 8],
    pub address: [u32; 8],
    pub usp: u32,
    pub ssp: u32,
    pub sr: u16,
    pub prefetch: u16,
    pub last_muldiv: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    M68k,
    Z80,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "chip", rename_all = "snake_case")]
pub enum ChipWrite {
    Ym2612 { port: u8, value: u8 },
    Psg { value: u8 },
}

/// Dispatch order is significant even when two writes have equal timestamps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimedChipWrite {
    pub master_clock: u64,
    pub origin: Origin,
    pub pc: u32,
    pub write: ChipWrite,
}

/// Runtime-only evidence: intentionally not serializable, to avoid accidentally
/// exporting copyrighted RAM, pixels, or audio as a public verification receipt.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FrameState {
    pub position: Position,
    pub cpu: CpuSnapshot,
    pub work_ram: Vec<u8>,
    pub vram: Vec<u8>,
    pub cram: Vec<u16>,
    pub vsram: Vec<u16>,
    pub vdp_registers: Vec<u8>,
    /// The complete bincode encoding of the reference VDP includes command/HV
    /// latches, FIFO, DMA, IRQ flags, renderer caches and internal raster state.
    /// No subset or hash is used as a substitute for byte equality.
    pub vdp_state: Vec<u8>,
    pub z80_ram: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Packed RGBA in channel order, independent of host endianness.
    pub pixels: Vec<[u8; 4]>,
    /// The f64 sample bit patterns, not approximate numerical comparisons.
    pub audio_bits: Vec<[u64; 2]>,
    pub chip_writes: Vec<TimedChipWrite>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Difference {
    pub domain: String,
    /// Element offset in the named domain; byte offset for encoded device state.
    pub index: Option<usize>,
    pub reference: String,
    pub candidate: String,
}

fn scalar<T: std::fmt::Debug + PartialEq>(
    domain: &str,
    reference: &T,
    candidate: &T,
) -> Option<Difference> {
    (reference != candidate).then(|| Difference {
        domain: domain.into(),
        index: None,
        reference: format!("{reference:?}"),
        candidate: format!("{candidate:?}"),
    })
}

fn elements<T: std::fmt::Debug + PartialEq>(
    domain: &str,
    reference: &[T],
    candidate: &[T],
) -> Option<Difference> {
    for (index, (expected, actual)) in reference.iter().zip(candidate).enumerate() {
        if expected != actual {
            return Some(Difference {
                domain: domain.into(),
                index: Some(index),
                reference: format!("{expected:?}"),
                candidate: format!("{actual:?}"),
            });
        }
    }
    scalar(
        &format!("{domain}.count"),
        &reference.len(),
        &candidate.len(),
    )
}

/// Compare the actual values, never hashes. The first unequal element of the
/// first unequal domain is returned; temporal localization is the runner's job.
pub fn compare_frames(reference: &FrameState, candidate: &FrameState) -> Option<Difference> {
    macro_rules! field {
        ($name:ident) => {
            if let Some(difference) = scalar(stringify!($name), &reference.$name, &candidate.$name)
            {
                return Some(difference);
            }
        };
    }
    macro_rules! array {
        ($name:ident) => {
            if let Some(difference) =
                elements(stringify!($name), &reference.$name, &candidate.$name)
            {
                return Some(difference);
            }
        };
    }
    field!(position);
    field!(cpu);
    array!(work_ram);
    array!(vram);
    array!(cram);
    array!(vsram);
    array!(vdp_registers);
    array!(z80_ram);
    field!(width);
    field!(height);
    array!(pixels);
    array!(audio_bits);
    array!(chip_writes);
    // This includes every mutable VDP latch, even when its output has not yet
    // reached VRAM or the visible frame. Keep separate named memory diagnostics.
    array!(vdp_state);
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> FrameState {
        FrameState {
            work_ram: vec![0; 8],
            vram: vec![0; 8],
            cram: vec![0; 4],
            vsram: vec![0; 4],
            vdp_registers: vec![0; 4],
            vdp_state: vec![0; 8],
            z80_ram: vec![0; 8],
            width: 2,
            height: 2,
            pixels: vec![[0, 0, 0, 255]; 4],
            audio_bits: vec![[0, 0]],
            chip_writes: vec![TimedChipWrite {
                master_clock: 12,
                origin: Origin::Z80,
                pc: 2,
                write: ChipWrite::Psg { value: 0x90 },
            }],
            ..FrameState::default()
        }
    }

    #[test]
    fn every_required_observation_is_compared() {
        type Mutation = (&'static str, fn(&mut FrameState));
        let original = frame();
        let mutations: [Mutation; 14] = [
            ("work_ram", |s| s.work_ram[3] ^= 1),
            ("vram", |s| s.vram[3] ^= 1),
            ("cram", |s| s.cram[3] ^= 1),
            ("vsram", |s| s.vsram[3] ^= 1),
            ("vdp_registers", |s| s.vdp_registers[3] ^= 1),
            ("vdp_state", |s| s.vdp_state[3] ^= 1),
            ("z80_ram", |s| s.z80_ram[3] ^= 1),
            ("pixels", |s| s.pixels[3][2] ^= 1),
            ("audio_bits", |s| s.audio_bits[0][1] ^= 1),
            ("audio_bits.count", |s| s.audio_bits.push([0, 0])),
            ("chip_writes", |s| s.chip_writes[0].master_clock += 1),
            ("chip_writes.count", |s| s.chip_writes.clear()),
            ("position", |s| s.position.line_master_clock += 1),
            ("cpu", |s| s.cpu.usp += 1),
        ];
        assert_eq!(compare_frames(&original, &original), None);
        for (domain, mutate) in mutations {
            let mut changed = original.clone();
            mutate(&mut changed);
            assert_eq!(compare_frames(&original, &changed).unwrap().domain, domain);
        }
    }

    #[test]
    fn negative_zero_and_nan_payloads_are_not_float_equal() {
        let mut expected = frame();
        let mut actual = expected.clone();
        actual.audio_bits[0][0] = (-0.0_f64).to_bits();
        assert_eq!(compare_frames(&expected, &actual).unwrap().index, Some(0));
        expected.audio_bits[0][0] = 0x7ff8_0000_0000_0001;
        actual.audio_bits[0][0] = 0x7ff8_0000_0000_0002;
        assert!(compare_frames(&expected, &actual).is_some());
        assert_eq!(compare_frames(&expected, &expected), None);
    }

    #[test]
    fn same_clock_chip_writes_remain_ordered() {
        let mut expected = frame();
        let first = expected.chip_writes[0];
        expected.chip_writes.push(TimedChipWrite {
            write: ChipWrite::Ym2612 { port: 1, value: 42 },
            ..first
        });
        let mut actual = expected.clone();
        actual.chip_writes.swap(0, 1);
        assert_eq!(
            compare_frames(&expected, &actual).unwrap().domain,
            "chip_writes"
        );
    }
}
