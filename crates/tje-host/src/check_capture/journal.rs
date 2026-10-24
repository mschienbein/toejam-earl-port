//! Existing native observation streams retain their different semantics. CPU
//! issues are not RAM commits, and chip dispatch clocks are not guessed from a
//! hybrid access's observation checkpoint.
use genesis_core::{GenesisBus, native};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CpuIssue {
    /// Stable source purpose code: semantic=0, prefetch=1, discarded=2.
    Read {
        address: u32,
        word: bool,
        value: u16,
        purpose_code: u8,
    },
    Write {
        address: u32,
        word: bool,
        value: u16,
    },
    AcknowledgeInterrupt {
        level: u8,
    },
}
#[derive(Debug, Clone, Serialize)]
pub struct RamCommit {
    pub address: u32,
    pub word: bool,
    pub value: u16,
}
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "device", rename_all = "snake_case")]
pub enum ChipWrite {
    Vdp { port: u8, value: u16 },
    Ym2612 { port: u8, value: u8 },
    Psg { value: u8 },
}
#[derive(Debug, Clone, Serialize)]
pub struct TimedChipWrite {
    pub master_clock: u64,
    pub origin: &'static str,
    pub pc: u32,
    pub write: ChipWrite,
}
#[derive(Debug, Clone, Copy, Serialize)]
pub struct JournalLengths {
    pub cpu_issues: Option<u64>,
    pub ram_commits: Option<u64>,
    pub timed_chip_writes: Option<u64>,
}
impl JournalLengths {
    pub(crate) fn capture(bus: &GenesisBus) -> Self {
        Self {
            cpu_issues: bus
                .native_cpu_accesses
                .as_ref()
                .map(|values| values.len() as u64),
            ram_commits: bus
                .native_ram_writes
                .as_ref()
                .map(|values| values.len() as u64),
            timed_chip_writes: bus.native_writes.as_ref().map(|values| values.len() as u64),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct BusJournals {
    /// Includes actual immutable-ROM fetches omitted by the hybrid audit stream.
    pub cpu_issues: Option<Vec<CpuIssue>>,
    /// Bus-committed writes; direct native memory effects remain in observations.
    pub ram_commits: Option<Vec<RamCommit>>,
    pub timed_chip_writes: Option<Vec<TimedChipWrite>>,
}
impl BusJournals {
    pub(crate) fn capture(bus: &GenesisBus) -> Self {
        Self {
            cpu_issues: bus.native_cpu_accesses.as_ref().map(|issues| {
                issues
                    .iter()
                    .map(|issue| match *issue {
                        native::CpuAccess::Read {
                            address,
                            word,
                            value,
                            purpose,
                        } => CpuIssue::Read {
                            address,
                            word,
                            value,
                            purpose_code: purpose,
                        },
                        native::CpuAccess::Write {
                            address,
                            word,
                            value,
                        } => CpuIssue::Write {
                            address,
                            word,
                            value,
                        },
                        native::CpuAccess::AcknowledgeInterrupt { level } => {
                            CpuIssue::AcknowledgeInterrupt { level }
                        }
                    })
                    .collect()
            }),
            ram_commits: bus.native_ram_writes.as_ref().map(|commits| {
                commits
                    .iter()
                    .map(|commit| {
                        let native::RamWrite {
                            address,
                            word,
                            value,
                        } = *commit;
                        RamCommit {
                            address,
                            word,
                            value,
                        }
                    })
                    .collect()
            }),
            timed_chip_writes: bus.native_writes.as_ref().map(|writes| {
                writes
                    .iter()
                    .map(|write| {
                        let native::TimedWrite {
                            clock,
                            origin,
                            pc,
                            write,
                        } = *write;
                        let origin = match origin {
                            native::Origin::Main => "main_cpu",
                            native::Origin::Z80 => "z80",
                        };
                        let write = match write {
                            native::DeviceWrite::Vdp { port, value } => {
                                ChipWrite::Vdp { port, value }
                            }
                            native::DeviceWrite::Ym2612 { port, value } => {
                                ChipWrite::Ym2612 { port, value }
                            }
                            native::DeviceWrite::Psg { value } => ChipWrite::Psg { value },
                        };
                        TimedChipWrite {
                            master_clock: clock,
                            origin,
                            pc,
                            write,
                        }
                    })
                    .collect()
            }),
        }
    }
}
