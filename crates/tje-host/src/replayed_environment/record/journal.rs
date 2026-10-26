//! Passive per-original-tick lease. Existing observers retain their requested
//! journal contents and options; our reusable scratch never accumulates a corpus.
use super::{Access, Boundary, Chronology, Direction};
use crate::replayed_environment::{Unsupported, UnsupportedKind};
use genesis_core::GenesisEmulator;
#[cfg(feature = "reference-proof-hooks")]
use genesis_core::reference_proof::PassiveJournalLoan;
use genesis_core::{HybridAccess, native::CpuAccess};
use m68000_emu::HybridExecutedInstruction;
use m68000_emu::ReadPurpose;

#[derive(Default)]
pub(in crate::replayed_environment) struct Journal {
    issues: Vec<Access>,
    driven_masks: Vec<u16>,
    chronology: Vec<Chronology>,
    ticks: u64,
    active: bool,
}
pub(in crate::replayed_environment) struct Before {
    boundary: Boundary,
    instructions: u64,
    instruction_ready: bool,
    #[cfg(feature = "reference-proof-hooks")]
    loan: PassiveJournalLoan,
}
impl Before {
    pub(in crate::replayed_environment) fn frame(&self) -> u64 {
        self.boundary.frame
    }
}
pub(in crate::replayed_environment) struct ObservedStep<'a> {
    pub before: Boundary,
    pub after: Boundary,
    pub instruction_ready: bool,
    pub actual: Option<HybridExecutedInstruction>,
    pub issues: &'a [Access],
    pub driven_masks: &'a [u16],
    pub chronology: &'a [Chronology],
}
fn unsupported(detail: impl Into<String>) -> Unsupported {
    Unsupported {
        kind: UnsupportedKind::Observation,
        detail: detail.into(),
    }
}
fn count(emulator: &GenesisEmulator) -> Result<u64, Unsupported> {
    emulator
        .hybrid_cpu()
        .hybrid_instruction_counts()
        .and_then(|counts| counts.main_thread.checked_add(counts.exception_handler))
        .ok_or_else(|| unsupported("actual original opcode counter is absent or overflowed"))
}
fn native_access(access: CpuAccess) -> Result<Option<Access>, Unsupported> {
    Ok(match access {
        CpuAccess::Read {
            address,
            word,
            value,
            purpose,
        } => Some(Access {
            address,
            word,
            value,
            direction: Direction::Read(match purpose {
                0 => ReadPurpose::Semantic,
                1 => ReadPurpose::Prefetch,
                2 => ReadPurpose::Discarded,
                _ => return Err(unsupported("unknown actual CPU read purpose")),
            }),
        }),
        CpuAccess::Write {
            address,
            word,
            value,
        } => Some(Access {
            address,
            word,
            value,
            direction: Direction::Write,
        }),
        CpuAccess::AcknowledgeInterrupt { .. } => None,
    })
}
fn next_read(issues: &[Access], cursor: &mut usize) -> Option<(usize, Access)> {
    while let Some(&access) = issues.get(*cursor) {
        let index = *cursor;
        *cursor += 1;
        if access.address >= 0x800000 && matches!(access.direction, Direction::Read(_)) {
            return Some((index, access));
        }
    }
    None
}
fn next_write(issues: &[Access], cursor: &mut usize, word: bool) -> Option<(usize, Access)> {
    while let Some(&access) = issues.get(*cursor) {
        let index = *cursor;
        *cursor += 1;
        if access.address >= 0x400000 && access.word == word && access.direction == Direction::Write
        {
            return Some((index, access));
        }
    }
    None
}
impl Journal {
    pub fn begin(
        &mut self,
        emulator: &mut GenesisEmulator,
        frame: u64,
    ) -> Result<Before, Unsupported> {
        if !cfg!(feature = "reference-proof-hooks") {
            return Err(unsupported(
                "actual-read context capability is not compiled; no full-mask fallback exists",
            ));
        }
        if self.active {
            return Err(unsupported("overlapping original journal leases"));
        }
        let next_tick = self
            .ticks
            .checked_add(1)
            .ok_or_else(|| unsupported("original scheduler-attempt counter overflow"))?;
        // Explicit persistent diagnostic setup, separate from the temporary
        // loan. Existing providers/tallies/settings are never reset here.
        #[cfg(feature = "reference-proof-hooks")]
        emulator.ensure_reference_instruction_counter();
        let instructions = count(emulator)?;
        let cpu = emulator.hybrid_cpu();
        let bus = emulator.hybrid_bus();
        let boundary = Boundary {
            cpu: super::CpuSnapshot::from_cpu(cpu),
            frame,
            ticks: self.ticks,
            instructions,
            master_clock: bus.cycles.m68k_mclk_cycles,
            dma: super::DmaBoundary::capture(bus),
        };
        let instruction_ready =
            bus.cycles.m68k_wait_cpu_cycles == 0 && cpu.hybrid_instruction_ready(bus);
        #[cfg(feature = "reference-proof-hooks")]
        let loan = emulator
            .begin_reference_journal()
            .map_err(|error| unsupported(format!("passive original journal begin: {error}")))?;
        self.ticks = next_tick;
        self.active = true;
        Ok(Before {
            boundary,
            instructions,
            instruction_ready,
            #[cfg(feature = "reference-proof-hooks")]
            loan,
        })
    }

    #[cfg(feature = "reference-proof-hooks")]
    fn bind_read_contexts(&mut self, bus: &genesis_core::GenesisBus) -> Result<(), Unsupported> {
        let mut contexts = bus
            .reference_read_contexts
            .as_deref()
            .ok_or_else(|| unsupported("actual-read context lease was removed"))?
            .iter();
        self.driven_masks.clear();
        for &issue in &self.issues {
            let mask = if let Direction::Read(purpose) = issue.direction {
                let context = contexts
                    .next()
                    .ok_or_else(|| unsupported("actual read lacks its required context"))?;
                let tag = match purpose {
                    ReadPurpose::Semantic => 0,
                    ReadPurpose::Prefetch => 1,
                    ReadPurpose::Discarded => 2,
                };
                if (
                    context.address,
                    context.word,
                    context.purpose,
                    context.value,
                ) != (issue.address, issue.word, tag, issue.value)
                    || context.driven_mask & if issue.word { 0 } else { 0xff00 } != 0
                {
                    return Err(unsupported(
                        "actual read context does not match the exact address/width/purpose/raw-value order",
                    ));
                }
                context.driven_mask
            } else {
                0
            };
            self.driven_masks.push(mask);
        }
        if contexts.next().is_some() {
            return Err(unsupported(
                "actual-read context has an unmatched extra record",
            ));
        }
        Ok(())
    }

    #[cfg(not(feature = "reference-proof-hooks"))]
    fn bind_read_contexts(&mut self, _: &genesis_core::GenesisBus) -> Result<(), Unsupported> {
        Err(unsupported(
            "actual-read context capability is unavailable; no mask is inferred",
        ))
    }

    pub fn observe<'a>(
        &'a mut self,
        emulator: &GenesisEmulator,
        frame: u64,
        before: &Before,
    ) -> Result<ObservedStep<'a>, Unsupported> {
        if !cfg!(feature = "reference-proof-hooks") {
            return Err(unsupported(
                "passive original journal capability is unavailable",
            ));
        }
        if !self.active {
            return Err(unsupported("original observation has no active lease"));
        }
        #[cfg(feature = "reference-proof-hooks")]
        let loan_error = emulator
            .reference_journal(&before.loan)
            .err()
            .map(|error| unsupported(format!("passive original journal observation: {error}")));
        #[cfg(not(feature = "reference-proof-hooks"))]
        let loan_error: Option<Unsupported> = None;
        // Preserve the old verified-prefix diagnostics on a corrupt suffix.
        // Invalid-loan bytes can NEVER leave this wrapper as an ObservedStep:
        // its original typed failure wins even if raw forensic parsing succeeds.
        let parsed = self.observe_raw(emulator, frame, before);
        match loan_error {
            Some(error) => Err(error),
            None => parsed,
        }
    }

    /// Private forensic decoder. Its result is not phase authority and cannot
    /// escape the public observe gate after any passive-loan failure.
    fn observe_raw<'a>(
        &'a mut self,
        emulator: &GenesisEmulator,
        frame: u64,
        before: &Before,
    ) -> Result<ObservedStep<'a>, Unsupported> {
        let delta = count(emulator)?
            .checked_sub(before.instructions)
            .ok_or_else(|| unsupported("actual original opcode counter reset during capture"))?;
        let actual = match delta {
            0 => None,
            1 => Some(
                emulator
                    .hybrid_cpu()
                    .hybrid_instruction_counts()
                    .and_then(|counts| counts.last_instruction)
                    .ok_or_else(|| unsupported("actual executed opcode has no receipt"))?,
            ),
            _ => {
                return Err(unsupported(
                    "one original tick coalesced multiple opcode receipts",
                ));
            }
        };
        if let Some(actual) = actual
            && (actual.pc != before.boundary.cpu.pc()
                || actual.opcode != before.boundary.cpu.opcode
                || actual.exception_depth != before.boundary.cpu.exception_depth)
        {
            return Err(unsupported(
                "actual opcode receipt does not authenticate the pre-step CPU",
            ));
        }
        self.issues.clear();
        self.chronology.clear();
        let bus = emulator.hybrid_bus();
        let native = bus
            .native_cpu_accesses
            .as_deref()
            .ok_or_else(|| unsupported("native issue lease was removed"))?;
        for &access in native {
            if let Some(access) = native_access(access)? {
                if access.address > 0xffffff || (!access.word && access.value > 255) {
                    return Err(unsupported(
                        "actual CPU issue is not a physical 24-bit byte/word beat",
                    ));
                }
                self.issues.push(access);
            } else if let CpuAccess::AcknowledgeInterrupt { level } = access {
                self.chronology.push(Chronology::Acknowledge { level });
            }
        }
        self.bind_read_contexts(bus)?;
        let stream = bus
            .hybrid_cpu_accesses
            .as_deref()
            .ok_or_else(|| unsupported("chronology lease was removed"))?;
        let boundaries = bus
            .hybrid_exception_boundaries
            .as_deref()
            .ok_or_else(|| unsupported("exception lease was removed"))?;
        let mut read_cursor = 0;
        let mut write_cursors = [0, 0];
        for &event in stream {
            match event {
                HybridAccess::Read {
                    address,
                    word,
                    value,
                    purpose,
                } => {
                    let expected = Access {
                        address,
                        word,
                        value,
                        direction: Direction::Read(purpose),
                    };
                    let Some((issue, actual)) = next_read(&self.issues, &mut read_cursor) else {
                        return Err(unsupported("actual read chronology has no issue"));
                    };
                    if actual != expected {
                        return Err(unsupported("actual issue/read chronology differs"));
                    }
                    self.chronology.push(Chronology::CpuRead {
                        access: actual,
                        issue,
                    });
                }
                HybridAccess::Write {
                    address,
                    word,
                    value,
                } => {
                    let expected = Access {
                        address,
                        word,
                        value,
                        direction: Direction::Write,
                    };
                    let Some((issue, actual)) =
                        next_write(&self.issues, &mut write_cursors[usize::from(word)], word)
                    else {
                        return Err(unsupported("physical CPU commit has no actual issue"));
                    };
                    if actual != expected {
                        return Err(unsupported("CPU issue/physical commit differs"));
                    }
                    self.chronology.push(Chronology::CpuCommit {
                        access: actual,
                        issue,
                    });
                }
                HybridAccess::DmaRead { address, value } => {
                    self.chronology.push(Chronology::DmaRead { address, value })
                }
                HybridAccess::Z80Write {
                    address,
                    word,
                    value,
                } => self.chronology.push(Chronology::Z80Write(Access {
                    address,
                    word,
                    value,
                    direction: Direction::Write,
                })),
                HybridAccess::HostMutation {
                    address,
                    word,
                    value,
                } => self.chronology.push(Chronology::HostMutation(Access {
                    address,
                    word,
                    value,
                    direction: Direction::Write,
                })),
                HybridAccess::ExceptionBoundary { index } => {
                    self.chronology.push(Chronology::Exception(
                        *boundaries
                            .get(index as usize)
                            .ok_or_else(|| unsupported("exception marker has no actual payload"))?,
                    ))
                }
            }
        }
        if next_read(&self.issues, &mut read_cursor).is_some()
            || next_write(&self.issues, &mut write_cursors[0], false).is_some()
            || next_write(&self.issues, &mut write_cursors[1], true).is_some()
        {
            return Err(unsupported(
                "actual non-ROM issue lacks its read/commit chronology",
            ));
        }
        Ok(ObservedStep {
            before: before.boundary,
            after: Boundary::capture(emulator, frame, self.ticks, count(emulator)?),
            instruction_ready: before.instruction_ready,
            actual,
            issues: &self.issues,
            driven_masks: &self.driven_masks,
            chronology: &self.chronology,
        })
    }

    /// Restore requested foreign observations without changing their option,
    /// marker-index or all-DMA-source conventions. Fresh input is never revived.
    pub fn restore(
        &mut self,
        emulator: &mut GenesisEmulator,
        before: Before,
    ) -> Result<(), Unsupported> {
        self.active = false;
        #[cfg(feature = "reference-proof-hooks")]
        {
            let mut loan = before.loan;
            emulator
                .finish_reference_journal(&mut loan)
                .map_err(|error| unsupported(format!("passive original journal cleanup: {error}")))
        }
        #[cfg(not(feature = "reference-proof-hooks"))]
        {
            let _ = (emulator, before);
            Err(unsupported(
                "passive original journal capability is unavailable",
            ))
        }
    }
}

#[cfg(all(test, feature = "reference-proof-hooks"))]
mod read_context_tests {
    use super::*;
    use crate::sinks::{HeadlessAudio, HeadlessOutput, Inputs, NoCartridgeSaves};
    use genesis_config::GenesisInputs;
    use genesis_core::reference_proof::ReadContext;
    use jgenesis_common::frontend::EmulatorTrait;

    #[test]
    fn actual_read_metadata_is_bound_exactly_and_missing_or_altered_records_fail_closed() {
        for mutation in 0..7 {
            let mut host = crate::fixture::tests::synthetic_host(&[0x4e71, 0x60fc], &[0x4e73]);
            let prefix = ReadContext {
                address: 0xff9000,
                word: true,
                purpose: 0,
                value: 0x1234,
                driven_mask: 0xffff,
                z80_busack: false,
            };
            host.emulator.hybrid_bus_mut().reference_read_contexts = Some(vec![prefix]);
            let mut journal = Journal::default();
            let before = journal.begin(&mut host.emulator, 0).unwrap();
            host.emulator
                .tick(
                    &mut HeadlessOutput,
                    &mut HeadlessAudio::default(),
                    &mut Inputs(GenesisInputs::default()),
                    &mut NoCartridgeSaves,
                )
                .unwrap();
            let contexts = host
                .emulator
                .hybrid_bus_mut()
                .reference_read_contexts
                .as_mut()
                .unwrap();
            assert!(
                !contexts.is_empty(),
                "the authored original step must issue actual reads"
            );
            let suffix = contexts.clone();
            match mutation {
                0 => {
                    host.emulator.hybrid_bus_mut().reference_read_contexts = None;
                }
                1 => contexts.clear(),
                2 => contexts[0].value ^= 1,
                3 => contexts[0].address ^= 2,
                4 => contexts[0].word = !contexts[0].word,
                5 => contexts[0].purpose = (contexts[0].purpose + 1) % 3,
                6 => contexts.push(suffix[0]),
                _ => unreachable!(),
            }
            let cpu = host.cpu().hybrid_state_words();
            let clock = host.emulator.hybrid_bus().cycles.m68k_mclk_cycles;
            assert!(journal.observe(&host.emulator, 0, &before).is_err());
            assert!(journal.restore(&mut host.emulator, before).is_err());
            assert_eq!(host.cpu().hybrid_state_words(), cpu);
            assert_eq!(host.emulator.hybrid_bus().cycles.m68k_mclk_cycles, clock);
            let contexts = host
                .emulator
                .hybrid_bus()
                .reference_read_contexts
                .as_ref()
                .unwrap();
            assert_eq!(
                contexts[0], prefix,
                "the foreign requested prefix must survive"
            );
            if mutation == 0 {
                assert_eq!(contexts.len(), 1, "never manufacture a missing suffix");
            }
        }
    }

    #[test]
    fn partial_read_chronology_error_returns_the_entire_foreign_raw_prefix_and_suffix() {
        let mut host = crate::fixture::tests::synthetic_host(
            &[0x207c, 0x00ff, 0x9000, 0x2010, 0x60fe],
            &[0x4e73],
        );
        for _ in 0..32 {
            let cpu = host.emulator.hybrid_cpu();
            let bus = host.emulator.hybrid_bus();
            if cpu.pc() == 0x106
                && bus.cycles.m68k_wait_cpu_cycles == 0
                && cpu.hybrid_instruction_ready(bus)
            {
                break;
            }
            host.emulator
                .tick(
                    &mut HeadlessOutput,
                    &mut HeadlessAudio::default(),
                    &mut Inputs(GenesisInputs::default()),
                    &mut NoCartridgeSaves,
                )
                .unwrap();
        }
        assert_eq!(host.cpu().pc(), 0x106);
        let prefix = HybridAccess::Read {
            address: 0xff9110,
            word: true,
            value: 0x1234,
            purpose: ReadPurpose::Semantic,
        };
        host.emulator.hybrid_bus_mut().hybrid_cpu_accesses = Some(vec![prefix]);
        let mut journal = Journal::default();
        let before = journal.begin(&mut host.emulator, 0).unwrap();
        host.emulator
            .tick(
                &mut HeadlessOutput,
                &mut HeadlessAudio::default(),
                &mut Inputs(GenesisInputs::default()),
                &mut NoCartridgeSaves,
            )
            .unwrap();
        let stream = host
            .emulator
            .hybrid_bus_mut()
            .hybrid_cpu_accesses
            .as_mut()
            .unwrap();
        let mut reads = 0;
        for event in stream.iter_mut() {
            if let HybridAccess::Read { address, value, .. } = event
                && *address >= 0xe00000
            {
                reads += 1;
                if reads == 2 {
                    *value ^= 1;
                }
            }
        }
        assert_eq!(
            reads, 2,
            "authored MOVE.L must issue two physical RAM reads"
        );
        let suffix = stream.clone();
        let cpu = host.cpu().hybrid_state_words();
        let clock = host.emulator.hybrid_bus().cycles.m68k_mclk_cycles;
        assert!(journal.observe(&host.emulator, 0, &before).is_err());
        assert_eq!(
            journal
                .chronology
                .iter()
                .filter(|event| matches!(event, Chronology::CpuRead { .. }))
                .count(),
            1,
            "retain the verified actual read before the second marker fails"
        );
        assert!(journal.restore(&mut host.emulator, before).is_err());
        assert_eq!(host.cpu().hybrid_state_words(), cpu);
        assert_eq!(host.emulator.hybrid_bus().cycles.m68k_mclk_cycles, clock);
        let restored = host
            .emulator
            .hybrid_bus()
            .hybrid_cpu_accesses
            .as_ref()
            .unwrap();
        assert_eq!(restored[0], prefix);
        assert_eq!(
            &restored[1..],
            suffix.as_slice(),
            "restoration forwards actual raw data, never a synthesized completed step"
        );
    }

    #[test]
    fn passive_original_journal_does_not_spend_genuine_reset_input() {
        let mut rom = vec![0; 0x400];
        rom[..4].copy_from_slice(&0xff8000u32.to_be_bytes());
        rom[4..8].copy_from_slice(&0x100u32.to_be_bytes());
        rom[0x100..0x102].copy_from_slice(&0x4e71u16.to_be_bytes());
        rom[0x102..0x104].copy_from_slice(&0x60feu16.to_be_bytes());
        let mut emulator = GenesisEmulator::create_with_reference_proof(
            Some(rom),
            genesis_config::GenesisEmulatorConfig::default(),
            &mut NoCartridgeSaves,
        );
        let cpu = emulator.hybrid_cpu().hybrid_state_words();
        let reset = emulator.reference_reset_observation().unwrap().after;
        let mut journal = Journal::default();
        let before = journal.begin(&mut emulator, 0).unwrap();
        {
            let step = journal.observe(&emulator, 0, &before).unwrap();
            assert!(step.actual.is_none());
            assert!(step.issues.is_empty());
            assert!(step.chronology.is_empty());
        }
        journal.restore(&mut emulator, before).unwrap();
        assert_eq!(emulator.hybrid_cpu().hybrid_state_words(), cpu);
        assert_eq!(emulator.reference_reset_observation().unwrap().after, reset);
        emulator.take_fresh_reset_input().unwrap();
    }
}
