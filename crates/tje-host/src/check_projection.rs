//! Logic-level observation contract for readable TimeAware translations.
//!
//! Raw clocks and reads are retained. Only a completed, instruction-verified ISR
//! YM BUSY polling loop may stutter; no main-thread semantic access is removed.

use crate::{abi::canonical_ram_address, accesses_match, catalogue};
use genesis_core::{GenesisEmulator, HybridAccess};
use m68000_emu::ReadPurpose;
use serde::Serialize;

/// Raw observation, retained independently of the equality projection.
#[derive(Debug, Clone)]
pub enum LogicEvent {
    Access {
        pc: u32,
        master_clock: u64,
        access: HybridAccess,
        /// Explicit source-backed operands compare every bit, including open bus.
        exact_operand: bool,
        /// Verified against the executing instruction and its backward branch.
        ym_busy_loop: bool,
    },
    ExceptionEntered {
        vector: u32,
        handler: u32,
        depth: u32,
        master_clock: u64,
    },
    ExceptionReturned {
        depth: u32,
        master_clock: u64,
    },
}

#[derive(Debug, Default)]
pub(crate) struct LogicTrace {
    pub events: Vec<LogicEvent>,
}

impl LogicTrace {
    pub fn access(&mut self, pc: u32, master_clock: u64, access: HybridAccess, ym_busy_loop: bool) {
        self.access_with_exact(pc, master_clock, access, ym_busy_loop, false);
    }

    pub fn access_with_exact(
        &mut self,
        pc: u32,
        master_clock: u64,
        access: HybridAccess,
        ym_busy_loop: bool,
        exact_operand: bool,
    ) {
        self.events.push(LogicEvent::Access {
            pc,
            master_clock,
            access,
            exact_operand,
            ym_busy_loop,
        });
    }

    pub fn append(&mut self, other: Self) {
        self.events.extend(other.events);
    }
}

/// A complete pair of raw traces from the latest TimeAware check. These are
/// private runtime observations, not distributable cartridge-derived fixtures.
#[derive(Debug)]
pub struct TimeAwareObservation {
    pub function: String,
    pub original: Vec<LogicEvent>,
    pub translated: Vec<LogicEvent>,
}

#[derive(Debug, Default, Serialize)]
pub struct TimeAwareCheckSummary {
    pub calls: u64,
    pub original_interrupts: u64,
    pub translated_interrupts: u64,
    pub original_ym_poll_reads: u64,
    pub translated_ym_poll_reads: u64,
    pub original_ym_busy_reads: u64,
    pub translated_ym_busy_reads: u64,
    /// Calls whose raw, unprojected 68000 device trace differs.
    pub raw_device_trace_differing_calls: u64,
    /// Calls whose clocked chip-write stream differs, including concurrent Z80.
    pub raw_sound_video_trace_differing_calls: u64,
}

impl TimeAwareCheckSummary {
    pub(crate) fn record(
        &mut self,
        original: &LogicTrace,
        translated: &LogicTrace,
        raw_writes_differ: bool,
    ) {
        fn counts(trace: &LogicTrace) -> (u64, u64, u64) {
            let (mut interrupts, mut polls, mut busy) = (0, 0, 0);
            let mut depth = 0;
            for event in &trace.events {
                match event {
                    LogicEvent::ExceptionEntered { .. } => {
                        interrupts += 1;
                        depth += 1;
                    }
                    LogicEvent::ExceptionReturned {
                        depth: returned, ..
                    } => depth = *returned,
                    LogicEvent::Access {
                        access:
                            HybridAccess::Read {
                                address: 0xa04000 | 0xa04002,
                                word: false,
                                value,
                                purpose: ReadPurpose::Semantic,
                            },
                        ym_busy_loop: true,
                        ..
                    } if depth != 0 => {
                        polls += 1;
                        busy += u64::from(value & 0x80 != 0);
                    }
                    _ => {}
                }
            }
            (interrupts, polls, busy)
        }
        let (a_irq, a_poll, a_busy) = counts(original);
        let (b_irq, b_poll, b_busy) = counts(translated);
        self.calls += 1;
        self.original_interrupts += a_irq;
        self.translated_interrupts += b_irq;
        self.original_ym_poll_reads += a_poll;
        self.translated_ym_poll_reads += b_poll;
        self.original_ym_busy_reads += a_busy;
        self.translated_ym_busy_reads += b_busy;
        fn device_access(event: &LogicEvent) -> Option<&HybridAccess> {
            match event {
                LogicEvent::Access { access, .. } if is_device(*access) => Some(access),
                _ => None,
            }
        }
        let mut a = original.events.iter().filter_map(device_access);
        let mut b = translated.events.iter().filter_map(device_access);
        let differs = loop {
            match (a.next(), b.next()) {
                (None, None) => break false,
                (Some(a), Some(b)) if accesses_match(a, b) => {}
                _ => break true,
            }
        };
        self.raw_device_trace_differing_calls += u64::from(differs);
        self.raw_sound_video_trace_differing_calls += u64::from(raw_writes_differ);
    }
}

fn is_semantic(access: HybridAccess) -> bool {
    !matches!(access, HybridAccess::Read { purpose, .. } if purpose != ReadPurpose::Semantic)
}

fn is_device(access: HybridAccess) -> bool {
    let address = match access {
        HybridAccess::Read { address, .. } | HybridAccess::Write { address, .. } => {
            address & 0xffffff
        }
    };
    (0x800000..0xe00000).contains(&address)
}

/// Decode only the reviewed, side-effect-free two-instruction spin shape, from
/// the user's current memory. Postincrement/predecrement, counter/timeout loops,
/// wider observations and all non-YM predicates are deliberately ineligible.
pub(crate) fn tight_ym_busy_loop(emulator: &GenesisEmulator, pc: u32) -> bool {
    fn peek(emulator: &GenesisEmulator, address: u32) -> Option<u16> {
        let address = address & 0xffffff;
        match address {
            0..=0x3ffffe => emulator
                .hybrid_bus()
                .cartridge
                .as_ref()
                .map(|rom| rom.peek_word(address)),
            0xe00000..=0xfffffe => {
                Some(emulator.hybrid_bus().memory.read_main_ram::<true>(address))
            }
            _ => None,
        }
    }
    let Some(opcode) = peek(emulator, pc) else {
        return false;
    };
    let length = match opcode {
        0x0810..=0x0817 => 4,
        0x0828..=0x082f | 0x0838 => 6,
        0x0839 => 8,
        _ => return false,
    };
    if peek(emulator, pc.wrapping_add(2)) != Some(7) {
        return false;
    }
    let branch_pc = pc.wrapping_add(length);
    let Some(branch) = peek(emulator, branch_pc) else {
        return false;
    };
    if branch & 0xff00 != 0x6600 {
        return false;
    }
    let displacement = if branch as u8 == 0 {
        let Some(extension) = peek(emulator, branch_pc.wrapping_add(2)) else {
            return false;
        };
        i32::from(extension as i16)
    } else {
        i32::from(branch as i8)
    };
    branch_pc.wrapping_add(2).wrapping_add_signed(displacement) == pc
}

#[derive(Debug, PartialEq, Eq)]
enum Effect {
    Enter {
        vector: u32,
        handler: u32,
        depth: u32,
    },
    Return {
        depth: u32,
    },
    RamWrite {
        address: u32,
        value: u8,
    },
    IsrRamRead {
        address: u32,
        value: u8,
    },
    Device {
        access: HybridAccess,
        exact_operand: bool,
    },
    YmReady {
        pc: u32,
        address: u32,
    },
}

fn project(trace: &LogicTrace, entry_sp: u32) -> Result<Vec<Effect>, String> {
    let mut effects = Vec::new();
    let mut depth = 0_u32;
    let mut polling = None;
    let live_start = canonical_ram_address(entry_sp).min(catalogue::STACK_TOP);
    let relevant = |event: &&LogicEvent| match **event {
        LogicEvent::Access { access, .. } => {
            let address = match access {
                HybridAccess::Read { address, .. } | HybridAccess::Write { address, .. } => address,
            };
            is_device(access)
                || (is_semantic(access) && canonical_ram_address(address) >= live_start)
        }
        _ => true,
    };
    for event in trace.events.iter().filter(relevant) {
        if let LogicEvent::Access {
            pc,
            access:
                HybridAccess::Read {
                    address,
                    word: false,
                    value,
                    purpose: ReadPurpose::Semantic,
                },
            ym_busy_loop: true,
            exact_operand: false,
            ..
        } = event
            && depth != 0
            && matches!(*address, 0xa04000 | 0xa04002)
        {
            let site = (*pc, *address);
            if polling.is_some_and(|previous| previous != site) {
                return Err("ISR YM poll changed site without observing ready".into());
            }
            if value & 0x80 == 0 {
                effects.push(Effect::YmReady {
                    pc: *pc,
                    address: *address,
                });
                polling = None;
            } else {
                polling = Some(site);
            }
            continue;
        }
        if polling.is_some() {
            return Err("ISR YM poll left its loop without a real ready observation".into());
        }
        match *event {
            LogicEvent::ExceptionEntered {
                vector,
                handler,
                depth: entered,
                ..
            } => {
                if !matches!(vector, 28 | 30) || entered != depth + 1 {
                    return Err(format!(
                        "unsupported or unbalanced TimeAware exception vector {vector} at depth {entered}"
                    ));
                }
                depth = entered;
                effects.push(Effect::Enter {
                    vector,
                    handler,
                    depth,
                });
            }
            LogicEvent::ExceptionReturned {
                depth: returned, ..
            } => {
                if depth == 0 || returned + 1 != depth {
                    return Err("unbalanced TimeAware exception return".into());
                }
                depth = returned;
                effects.push(Effect::Return { depth });
            }
            LogicEvent::Access {
                access,
                exact_operand,
                ..
            } if is_device(access) => {
                effects.push(Effect::Device {
                    access,
                    exact_operand,
                });
            }
            LogicEvent::Access { access, .. } if is_semantic(access) => {
                let (address, word, value, writing) = match access {
                    HybridAccess::Read {
                        address,
                        word,
                        value,
                        ..
                    } => (address, word, value, false),
                    HybridAccess::Write {
                        address,
                        word,
                        value,
                    } => (address, word, value, true),
                };
                let address = address & 0xffffff;
                if address >= 0xe00000 {
                    for offset in 0..if word { 2 } else { 1 } {
                        let address = canonical_ram_address(address + offset);
                        if address < live_start {
                            continue;
                        }
                        let value = if word && offset == 0 {
                            (value >> 8) as u8
                        } else {
                            value as u8
                        };
                        if writing {
                            effects.push(Effect::RamWrite { address, value });
                        } else if depth != 0 {
                            effects.push(Effect::IsrRamRead { address, value });
                        }
                    }
                }
            }
            LogicEvent::Access { .. } => {}
        }
    }
    if depth != 0 || polling.is_some() {
        return Err("TimeAware call returned inside an ISR or unfinished YM poll".into());
    }
    Ok(effects)
}

pub(crate) fn compare(
    original: &LogicTrace,
    translated: &LogicTrace,
    entry_sp: u32,
) -> Result<(), String> {
    let a = project(original, entry_sp).map_err(|error| format!("original {error}"))?;
    let b = project(translated, entry_sp).map_err(|error| format!("Rust {error}"))?;
    let matches = |a: &Effect, b: &Effect| match (a, b) {
        (
            Effect::Device {
                access: a,
                exact_operand: exact_a,
            },
            Effect::Device {
                access: b,
                exact_operand: exact_b,
            },
        ) => {
            if *exact_a || *exact_b {
                matches!((a, b), (
                    HybridAccess::Read { address: aa, word: aw, value: av, .. },
                    HybridAccess::Read { address: ba, word: bw, value: bv, .. },
                ) if aa == ba && aw == bw && av == bv)
            } else {
                accesses_match(a, b)
            }
        }
        _ => a == b,
    };
    if a.len() != b.len() || !a.iter().zip(&b).all(|(a, b)| matches(a, b)) {
        let index = a
            .iter()
            .zip(&b)
            .position(|(a, b)| !matches(a, b))
            .unwrap_or(a.len().min(b.len()));
        return Err(format!(
            "TimeAware logic trace differs at {index}: original={:?}, Rust={:?}",
            a.get(index),
            b.get(index)
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SP: u32 = 0xff7ffc;
    fn access(
        trace: &mut LogicTrace,
        pc: u32,
        address: u32,
        value: u16,
        writing: bool,
        poll: bool,
    ) {
        let event = if writing {
            HybridAccess::Write {
                address,
                word: false,
                value,
            }
        } else {
            HybridAccess::Read {
                address,
                word: false,
                value,
                purpose: ReadPurpose::Semantic,
            }
        };
        trace.access(pc, trace.events.len() as u64 * 42, event, poll);
    }
    fn enter(trace: &mut LogicTrace, vector: u32) {
        trace.events.push(LogicEvent::ExceptionEntered {
            vector,
            handler: 0x1000,
            depth: 1,
            master_clock: 100,
        });
    }
    fn leave(trace: &mut LogicTrace) {
        trace.events.push(LogicEvent::ExceptionReturned {
            depth: 0,
            master_clock: 200,
        });
    }
    fn polling_trace(busy_reads: usize) -> LogicTrace {
        let mut trace = LogicTrace::default();
        access(&mut trace, 0x100, 0xff9000, 1, true, false);
        enter(&mut trace, 30);
        access(&mut trace, 0x1000, 0xff9000, 1, false, false);
        for _ in 0..busy_reads {
            access(&mut trace, 0x1020, 0xa04000, 0x80, false, true);
        }
        access(&mut trace, 0x1020, 0xa04000, 0, false, true);
        access(&mut trace, 0x1030, 0xa04000, 0x22, true, false);
        access(&mut trace, 0x1034, 0xa04001, 0x34, true, false);
        access(&mut trace, 0x1040, 0xff9001, 2, true, false);
        leave(&mut trace);
        trace
    }

    #[test]
    fn completed_isr_busy_stutters_preserve_writes_and_raw_observations() {
        let (a, b) = (polling_trace(0), polling_trace(5));
        compare(&a, &b, SP).unwrap();
        let mut summary = TimeAwareCheckSummary::default();
        summary.record(&a, &b, true);
        assert_eq!(summary.original_ym_poll_reads, 1);
        assert_eq!(summary.translated_ym_poll_reads, 6);
        assert_eq!(summary.translated_ym_busy_reads, 5);
        assert_eq!(summary.raw_device_trace_differing_calls, 1);
        assert_eq!(summary.raw_sound_video_trace_differing_calls, 1);
        assert_eq!(a.events.len() + 5, b.events.len());
    }

    #[test]
    fn main_thread_reads_never_gain_the_isr_busy_quotient() {
        let (mut a, mut b) = (LogicTrace::default(), LogicTrace::default());
        access(&mut a, 0x200, 0xa04000, 0, false, true);
        access(&mut b, 0x200, 0xa04000, 0x80, false, true);
        assert!(compare(&a, &b, SP).is_err());
        b.events.clear();
        assert!(compare(&a, &b, SP).is_err());
        let (mut a, mut b) = (LogicTrace::default(), LogicTrace::default());
        enter(&mut a, 30);
        enter(&mut b, 30);
        access(&mut a, 0x200, 0xa04000, 0, false, false);
        access(&mut b, 0x200, 0xa04000, 0x80, false, false);
        leave(&mut a);
        leave(&mut b);
        assert!(
            compare(&a, &b, SP).is_err(),
            "unreviewed ISR read must remain exact"
        );
    }

    #[test]
    fn wrong_wait_dropped_main_write_and_changed_isr_visible_update_fail() {
        let original = polling_trace(1);
        let mut wrong_wait = LogicTrace::default();
        access(&mut wrong_wait, 0x100, 0xff9000, 1, true, false);
        assert!(
            compare(&original, &wrong_wait, SP).is_err(),
            "early wait skips its ISR"
        );
        let mut dropped = polling_trace(1);
        dropped.events.remove(0);
        assert!(
            compare(&original, &dropped, SP).is_err(),
            "dropped main write"
        );
        let mut changed = polling_trace(1);
        if let LogicEvent::Access {
            access: HybridAccess::Read { value, .. },
            ..
        } = &mut changed.events[2]
        {
            *value = 9;
        }
        assert!(
            compare(&original, &changed, SP).is_err(),
            "ISR-visible RAM update"
        );
        let mut dropped_isr = polling_trace(1);
        dropped_isr.events.remove(5);
        assert!(
            compare(&original, &dropped_isr, SP).is_err(),
            "ISR writes cannot disappear"
        );
    }

    #[test]
    fn unfinished_poll_and_wrong_invocation_order_fail_closed() {
        let mut trace = polling_trace(1);
        trace.events.remove(4);
        assert!(compare(&trace, &trace, SP).is_err());
        let mut a = polling_trace(0);
        let mut b = polling_trace(0);
        enter(&mut a, 28);
        leave(&mut a);
        let mut prefix = LogicTrace::default();
        enter(&mut prefix, 28);
        leave(&mut prefix);
        prefix.append(b);
        b = prefix;
        assert!(compare(&a, &b, SP).is_err());
    }

    #[test]
    fn an_entry_handler_body_does_not_approve_an_unmatched_child_return() {
        // Root handler entry predates this check: its ordinary body is owner 0.
        // Its declared RTE is checked by the exit contract, not a child ISR event.
        let body = polling_trace(1);
        compare(&body, &body, SP).unwrap();
        let mut unmatched = LogicTrace::default();
        leave(&mut unmatched);
        assert!(compare(&unmatched, &unmatched, SP).is_err());
    }

    #[test]
    fn only_side_effect_free_busy_bit_backedge_shapes_are_eligible() {
        use genesis_config::GenesisEmulatorConfig;
        let mut emulator = GenesisEmulator::create(
            None,
            GenesisEmulatorConfig::default(),
            &mut crate::sinks::NoCartridgeSaves,
        );
        for (words, eligible) in [
            (&[0x0815, 7, 0x66fa][..], true),
            (&[0x0839, 7, 0x00a0, 0x4000, 0x66f6][..], true),
            (&[0x0815, 6, 0x66fa][..], false),
            (&[0x0815, 7, 0x67fa][..], false),
            (&[0x081d, 7, 0x66fa][..], false),
            (&[0x0815, 7, 0x66fc][..], false),
        ] {
            for (index, word) in words.iter().enumerate() {
                emulator
                    .hybrid_bus_mut()
                    .memory
                    .write_main_ram::<true>(0xfff000 + index as u32 * 2, *word);
            }
            assert_eq!(tight_ym_busy_loop(&emulator, 0xfff000), eligible);
        }
    }

    #[test]
    fn explicit_operands_and_discarded_device_accesses_cannot_be_masked() {
        let read = |value, purpose| HybridAccess::Read {
            address: 0xa11100,
            word: true,
            value,
            purpose,
        };
        let mut original = LogicTrace::default();
        original.access(0x100, 1, read(0x1234, ReadPurpose::Semantic), false);
        let mut changed = LogicTrace::default();
        changed.access(0x100, 2, read(0x9234, ReadPurpose::Semantic), false);
        compare(&original, &changed, SP).unwrap();
        changed.events.clear();
        changed.access_with_exact(0x100, 2, read(0x9234, ReadPurpose::Semantic), false, true);
        assert!(
            compare(&original, &changed, SP).is_err(),
            "explicit-source operand must match in full"
        );
        changed.events.clear();
        changed.access_with_exact(0x100, 2, read(0x1234, ReadPurpose::Semantic), false, true);
        compare(&original, &changed, SP).unwrap();
        original.access(0x102, 3, read(0x1234, ReadPurpose::Discarded), false);
        assert!(
            compare(&original, &changed, SP).is_err(),
            "discarded device read still has ordered bus effects"
        );
    }
}
