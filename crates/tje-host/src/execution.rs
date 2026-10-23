//! Non-atomic logic contracts and real-clock suspension.
//!
//! Functional forks discard only device transaction latency. Time-aware forks
//! execute real interrupts, but approve logic/transactions, not raster/audio phase.
//! A native scheduler must be verified separately against whole-session timing.

use crate::{
    HostError, Result,
    abi::{AtomicRun, OrderedWrite, UninitializedStackRead, canonical_ram_address, set_sp},
    catalogue::{STACK_BOTTOM, STACK_TOP},
    input::InputScript,
    sinks::{HeadlessAudio, HeadlessOutput, Inputs, NoCartridgeSaves},
};
use genesis_config::GenesisInputs;
use genesis_core::{GenesisEmulator, HybridAccess};
use jgenesis_common::frontend::{EmulatorTrait, TickEffect};
use m68000_emu::BusInterface;
use std::{collections::BTreeSet, sync::Arc};
use tje_game::execution::{WaitCondition, WaitRequest};

/// Random-access input timeline: identical forks request the same video-frame
/// sample. A live frontend can implement this with its frame-sampled controller
/// source; recorded sessions use InputScript. A missing frame is an error.
pub trait WaitInputPoller: Send + Sync {
    fn at(&self, frame: u64) -> Result<GenesisInputs>;

    /// Session recipes run before the first instruction of each new video frame.
    /// Controller-only timelines intentionally have no RAM recipe.
    fn before_frame(&self, _frame: u64, _memory: &mut dyn tje_mem::Bus) -> Result<()> {
        Ok(())
    }
}
impl WaitInputPoller for InputScript {
    fn at(&self, frame: u64) -> Result<GenesisInputs> {
        Ok(InputScript::at(self, frame))
    }
}

#[derive(Clone, Default)]
pub struct WaitClock {
    pub frame: u64,
    pub inputs: GenesisInputs,
    pub poller: Option<Arc<dyn WaitInputPoller>>,
}
impl WaitClock {
    fn advance_frame(&mut self, emulator: &mut GenesisEmulator) -> Result<()> {
        self.frame += 1;
        let poller = self.poller.as_ref().ok_or_else(|| {
            HostError::Input(format!(
                "time-progressing call crossed video frame {} without an input timeline",
                self.frame
            ))
        })?;
        let inputs = poller.at(self.frame)?;
        poller.before_frame(self.frame, &mut WaitRam(emulator))?;
        self.inputs = inputs;
        Ok(())
    }
}

struct WaitRam<'a>(&'a mut GenesisEmulator);
impl tje_mem::Bus for WaitRam<'_> {
    fn read_u8(&self, address: u32) -> u8 {
        let address = canonical_ram_address(address);
        assert!(
            address >= 0xff0000,
            "input recipes may access only work RAM"
        );
        self.0.hybrid_bus().memory.read_main_ram::<false>(address) as u8
    }
    fn write_u8(&mut self, address: u32, value: u8) {
        let address = canonical_ram_address(address);
        assert!(
            address >= 0xff0000,
            "input recipes may access only work RAM"
        );
        let bus = self.0.hybrid_bus_mut();
        bus.memory
            .write_main_ram::<false>(address, u16::from(value));
        if let Some(trace) = bus.hybrid_cpu_accesses.as_mut() {
            trace.push(HybridAccess::Write {
                address,
                word: false,
                value: u16::from(value),
            });
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct ClockRun {
    pub ticks: u64,
    pub frames: u64,
    pub accesses: Vec<(u32, HybridAccess)>,
    pub audio: Vec<[f64; 2]>,
}

// Synthetic BRA.S to itself, not game code. The bottom four bytes of the declared
// stack area cannot overlap the suspended frame (checked below), and are restored
// even on failure. The second word covers the 68000's instruction prefetch.
const PARK_PC: u32 = STACK_BOTTOM;
const PARK_BYTES: u32 = 4;
const MAX_TIMED_TICKS: u64 = 1_000_000;

fn condition_holds(emulator: &mut GenesisEmulator, condition: WaitCondition) -> Result<bool> {
    let bus = emulator.hybrid_bus_mut();
    Ok(match condition {
        WaitCondition::RamWordChanged { address, previous } => {
            let address = canonical_ram_address(address);
            if !(0xff0000..=0xfffffe).contains(&address) || address & 1 != 0 {
                return Err(HostError::Metadata(
                    "wait condition is not an aligned RAM word".into(),
                ));
            }
            bus.memory.read_main_ram::<true>(address) != previous
        }
        WaitCondition::DeviceWord {
            address,
            mask,
            value,
        } => {
            if !(0x800000..0xe00000).contains(&(address & 0xffffff)) || address & 1 != 0 {
                return Err(HostError::Metadata(
                    "wait condition is not an aligned device word".into(),
                ));
            }
            bus.read_word(address) & mask == value
        }
        WaitCondition::DeviceByte {
            address,
            mask,
            value,
        } => {
            if !(0x800000..0xe00000).contains(&(address & 0xffffff)) {
                return Err(HostError::Metadata(
                    "wait condition is not a device byte".into(),
                ));
            }
            bus.read_byte(address) & mask == value
        }
    })
}

fn drain_accesses(emulator: &mut GenesisEmulator, pc: u32, run: &mut ClockRun) {
    let bus = emulator.hybrid_bus_mut();
    if let Some(accesses) = bus.hybrid_cpu_accesses.as_mut() {
        run.accesses.extend(accesses.drain(..).filter_map(|access| {
            let address = match access {
                HybridAccess::Read { address, .. } | HybridAccess::Write { address, .. } => address,
            };
            // Only instruction fetches of our synthetic parking loop are scaffolding.
            if pc == PARK_PC
                && (PARK_PC..PARK_PC + PARK_BYTES).contains(&canonical_ram_address(address))
            {
                None
            } else {
                Some((pc, access))
            }
        }));
    }
}

fn timed_tick(
    emulator: &mut GenesisEmulator,
    clock: &mut WaitClock,
    run: &mut ClockRun,
    audio: &mut HeadlessAudio,
) -> Result<()> {
    let pc = emulator.hybrid_cpu().pc();
    let effect = emulator
        .tick(
            &mut HeadlessOutput,
            audio,
            &mut Inputs(clock.inputs),
            &mut NoCartridgeSaves,
        )
        .map_err(|error| HostError::Emulator(error.to_string()))?;
    run.ticks += 1;
    drain_accesses(emulator, pc, run);
    if effect == TickEffect::FrameRendered {
        run.frames += 1;
        clock.advance_frame(emulator)?;
    }
    Ok(())
}

/// Suspend Rust at its *current* emulated frame/SP, with its original SR mask.
/// The real 68000 executes the idle loop and all original exception handlers;
/// only a returned-to-idle CPU may resume Rust, never an ISR halfway through.
pub(crate) fn run_wait(
    emulator: &mut GenesisEmulator,
    clock: &mut WaitClock,
    request: WaitRequest,
) -> Result<ClockRun> {
    run_wait_with_audio(emulator, clock, request, Vec::new(), true)
}

/// Replacement callers move their existing audio buffer through this operation;
/// unchecked play does not allocate/collect a RAM/device audit.
pub(crate) fn run_wait_with_audio(
    emulator: &mut GenesisEmulator,
    clock: &mut WaitClock,
    request: WaitRequest,
    samples: Vec<[f64; 2]>,
    audit: bool,
) -> Result<ClockRun> {
    if emulator.hybrid_bus().hybrid_functional_mode() {
        return Err(HostError::Metadata(
            "a wait requires a time-aware contract".into(),
        ));
    }
    emulator.hybrid_bus_mut().apply_pending_writes();
    let saved_pc = emulator.hybrid_cpu().pc();
    let saved_sp = emulator.hybrid_cpu().stack_pointer();
    if canonical_ram_address(saved_sp) < PARK_PC + 0x100 {
        return Err(HostError::Metadata(
            "no disjoint stack space for the wait sentinel".into(),
        ));
    }
    let old_words = [
        emulator.hybrid_bus().memory.read_main_ram::<true>(PARK_PC),
        emulator
            .hybrid_bus()
            .memory
            .read_main_ram::<true>(PARK_PC + 2),
    ];
    let saved_trace = std::mem::replace(
        &mut emulator.hybrid_bus_mut().hybrid_cpu_accesses,
        audit.then(Vec::new),
    );
    {
        let (cpu, bus) = emulator.hybrid_parts();
        bus.memory.write_main_ram::<true>(PARK_PC, 0x60fe);
        bus.memory.write_main_ram::<true>(PARK_PC + 2, 0x4e71);
        cpu.set_pc(PARK_PC, bus);
    }
    let mut run = ClockRun::default();
    let mut audio = HeadlessAudio { samples };
    let result = (|| {
        loop {
            let parked = emulator.hybrid_cpu().pc() == PARK_PC
                && emulator.hybrid_cpu().stack_pointer() == saved_sp;
            if parked {
                if emulator.hybrid_bus().memory.read_main_ram::<true>(PARK_PC) != 0x60fe {
                    return Err(HostError::Metadata(
                        "an ISR overwrote the wait sentinel".into(),
                    ));
                }
                let satisfied = condition_holds(emulator, request.condition)?;
                drain_accesses(emulator, PARK_PC, &mut run);
                if satisfied {
                    return Ok(());
                }
            }
            if run.ticks >= request.max_ticks {
                return Err(HostError::Emulator(format!(
                    "wait condition {:?} was not satisfied after {} real ticks (PC={:06X})",
                    request.condition,
                    run.ticks,
                    emulator.hybrid_cpu().pc()
                )));
            }
            timed_tick(emulator, clock, &mut run, &mut audio)?;
        }
    })();
    let (cpu, bus) = emulator.hybrid_parts();
    bus.memory.write_main_ram::<true>(PARK_PC, old_words[0]);
    bus.memory.write_main_ram::<true>(PARK_PC + 2, old_words[1]);
    // A failure aborts the call; never pretend an interrupted ISR returned.
    if result.is_ok() {
        set_sp(cpu, saved_sp);
        cpu.set_pc(saved_pc, bus);
    }
    bus.hybrid_cpu_accesses = saved_trace;
    run.audio = audio.samples;
    result?;
    Ok(run)
}

pub(crate) fn run_original_timed(
    emulator: &mut GenesisEmulator,
    clock: &mut WaitClock,
    return_pc: u32,
    return_sp: u32,
) -> Result<(AtomicRun, ClockRun)> {
    run_original_timed_with_audio(emulator, clock, return_pc, return_sp, Vec::new(), true)
}

pub(crate) fn run_original_timed_with_audio(
    emulator: &mut GenesisEmulator,
    clock: &mut WaitClock,
    return_pc: u32,
    return_sp: u32,
    samples: Vec<[f64; 2]>,
    audit: bool,
) -> Result<(AtomicRun, ClockRun)> {
    if emulator.hybrid_bus().hybrid_functional_mode() {
        return Err(HostError::Metadata(
            "timed execution cannot use functional devices".into(),
        ));
    }
    let entry_sp = emulator.hybrid_cpu().stack_pointer();
    let saved_trace = std::mem::replace(
        &mut emulator.hybrid_bus_mut().hybrid_cpu_accesses,
        audit.then(Vec::new),
    );
    let mut run = ClockRun::default();
    let mut audio = HeadlessAudio { samples };
    let result = (|| {
        while emulator.hybrid_cpu().pc() != return_pc
            || emulator.hybrid_cpu().stack_pointer() != return_sp
        {
            if run.ticks == MAX_TIMED_TICKS {
                return Err(HostError::CallDidNotReturn {
                    pc: emulator.hybrid_cpu().pc(),
                    return_pc,
                });
            }
            timed_tick(emulator, clock, &mut run, &mut audio)?;
        }
        Ok(())
    })();
    emulator.hybrid_bus_mut().hybrid_cpu_accesses = saved_trace;
    run.audio = audio.samples;
    result?;
    let audit = if audit {
        audit_accesses(&run.accesses, entry_sp)
    } else {
        AtomicRun::default()
    };
    Ok((audit, run))
}

pub(crate) fn audit_accesses(accesses: &[(u32, HybridAccess)], entry_sp: u32) -> AtomicRun {
    let mut initialized = [0_u64; 512];
    let mut reads = BTreeSet::new();
    let mut writes = Vec::new();
    for &(pc, access) in accesses {
        let (address, word, value, writing) = match access {
            HybridAccess::Read { purpose, .. } if purpose != m68000_emu::ReadPurpose::Semantic => {
                continue;
            }
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
        if address >= 0xe00000 {
            for offset in 0..if word { 2 } else { 1 } {
                let physical = canonical_ram_address(address + offset);
                if (STACK_BOTTOM..canonical_ram_address(entry_sp).min(STACK_TOP))
                    .contains(&physical)
                {
                    let index = (physical - STACK_BOTTOM) as usize;
                    let bit = 1_u64 << (index % 64);
                    if writing {
                        initialized[index / 64] |= bit;
                    } else if initialized[index / 64] & bit == 0 {
                        reads.insert(UninitializedStackRead {
                            pc,
                            address: physical,
                        });
                    }
                }
                if writing {
                    let byte = if word && offset == 0 {
                        (value >> 8) as u8
                    } else {
                        value as u8
                    };
                    writes.push(OrderedWrite::Ram {
                        address: physical,
                        value: byte,
                    });
                }
            }
        } else if writing {
            writes.push(if word {
                OrderedWrite::DeviceWord { address, value }
            } else {
                OrderedWrite::DeviceByte {
                    address,
                    value: value as u8,
                }
            });
        }
    }
    AtomicRun {
        instructions: 0,
        cycles: 0,
        uninitialized_reads: reads,
        writes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use genesis_config::GenesisEmulatorConfig;
    use genesis_core::GenesisBus;
    use jgenesis_common::frontend::{PartialClone, TimingMode};

    fn write(bus: &mut GenesisBus, address: u32, value: u16) {
        bus.write_word(address, value);
        bus.apply_pending_writes();
    }

    #[test]
    fn functional_fifo_commits_without_changing_the_timed_default() {
        let mut timed = GenesisBus::new(TimingMode::Ntsc, None, &GenesisEmulatorConfig::default());
        let mut functional = timed.partial_clone();
        functional.hybrid_set_functional_mode(true);
        for bus in [&mut timed, &mut functional] {
            write(bus, 0xc00004, 0x8f02);
            write(bus, 0xc00004, 0x4000);
            write(bus, 0xc00004, 0);
            for value in 0..10 {
                write(bus, 0xc00000, value);
            }
        }
        assert!(!timed.hybrid_functional_mode());
        assert!(timed.vdp.should_halt_cpu());
        assert!(!functional.vdp.should_halt_cpu());
        let (vram, _, _) = functional.vdp.hybrid_memories();
        assert_eq!(&vram[..6], &[0, 0, 0, 1, 0, 2]);
        timed.hybrid_set_functional_mode(true);
        assert!(timed.vdp.hybrid_state_eq(&functional.vdp));
    }

    #[test]
    fn functional_dma_uses_real_source_length_and_address_updates() {
        let mut bus = GenesisBus::new(TimingMode::Ntsc, None, &GenesisEmulatorConfig::default());
        bus.memory.write_main_ram::<true>(0xff0000, 0x1234);
        bus.memory.write_main_ram::<true>(0xff0002, 0xabcd);
        bus.hybrid_set_functional_mode(true);
        for value in [
            0x8114, 0x8f02, 0x9302, 0x9400, 0x9500, 0x9680, 0x977f, 0x4000, 0x0080,
        ] {
            write(&mut bus, 0xc00004, value);
        }
        let (vram, _, _) = bus.vdp.hybrid_memories();
        assert_eq!(&vram[..4], &[0x12, 0x34, 0xab, 0xcd]);
        assert_eq!(bus.vdp.hybrid_registers().dma_length, 0);
        assert_eq!(bus.vdp.hybrid_registers().dma_source_address, 0xff0004);
        assert!(!bus.vdp.should_halt_cpu());
    }

    #[test]
    fn functional_mode_does_not_invent_busack_during_reset() {
        let mut bus = GenesisBus::new(TimingMode::Ntsc, None, &GenesisEmulatorConfig::default());
        bus.hybrid_set_functional_mode(true);
        write(&mut bus, 0xa11100, 0x100);
        assert_ne!(bus.read_word(0xa11100) & 0x100, 0);
        write(&mut bus, 0xa11200, 0x100);
        assert_eq!(bus.read_word(0xa11100) & 0x100, 0);
    }

    #[test]
    fn functional_ym_busy_settles_only_the_fixed_write_countdown() {
        let mut bus = GenesisBus::new(TimingMode::Ntsc, None, &GenesisEmulatorConfig::default());
        write(&mut bus, 0xa11200, 0x100);
        write(&mut bus, 0xa11100, 0x100);
        bus.write_byte(0xa04000, 0x22);
        bus.apply_pending_writes();
        bus.write_byte(0xa04001, 0);
        bus.apply_pending_writes();
        let before = bus.read_byte(0xa04000);
        assert_ne!(before & 0x80, 0);
        let phase = (bus.vdp.scanline(), bus.vdp.scanline_mclk());
        bus.hybrid_set_functional_mode(true);
        assert_eq!(bus.read_byte(0xa04000), before & !0x80);
        assert_eq!((bus.vdp.scanline(), bus.vdp.scanline_mclk()), phase);
    }

    #[test]
    fn direct_functional_raster_read_is_a_contract_violation() {
        let mut bus = GenesisBus::new(TimingMode::Ntsc, None, &GenesisEmulatorConfig::default());
        bus.hybrid_set_functional_mode(true);
        bus.read_word(0xc00008);
        assert_eq!(bus.hybrid_functional_violation, Some(0xc00008));
    }

    #[test]
    fn timed_access_audit_keeps_interrupt_read_before_write_dependencies() {
        let accesses = [
            (
                0x100,
                HybridAccess::Read {
                    address: 0xef7ff0,
                    word: true,
                    value: 0,
                    purpose: m68000_emu::ReadPurpose::Semantic,
                },
            ),
            (
                0x102,
                HybridAccess::Write {
                    address: 0xff7ff0,
                    word: true,
                    value: 1,
                },
            ),
            (
                0x104,
                HybridAccess::Read {
                    address: 0xff7ff0,
                    word: true,
                    value: 1,
                    purpose: m68000_emu::ReadPurpose::Semantic,
                },
            ),
        ];
        let audit = audit_accesses(&accesses, 0xff7ffc);
        assert_eq!(audit.uninitialized_reads.len(), 2);
        assert!(audit.uninitialized_reads.contains(&UninitializedStackRead {
            pc: 0x100,
            address: 0xff7ff0
        }));
        assert_eq!(audit.writes.len(), 2);
    }

    #[test]
    fn parking_storage_is_restored_even_when_the_wait_times_out() {
        let mut emulator = GenesisEmulator::create(
            None,
            GenesisEmulatorConfig::default(),
            &mut NoCartridgeSaves,
        );
        set_sp(emulator.hybrid_parts().0, 0xff7ffc);
        emulator.hybrid_parts().0.set_status_register(0x2700);
        emulator
            .hybrid_bus_mut()
            .memory
            .write_main_ram::<true>(PARK_PC, 0x1234);
        emulator
            .hybrid_bus_mut()
            .memory
            .write_main_ram::<true>(PARK_PC + 2, 0x5678);
        emulator
            .hybrid_bus_mut()
            .memory
            .write_main_ram::<true>(0xff8000, 1);
        let request = WaitRequest {
            condition: WaitCondition::RamWordChanged {
                address: 0xff8000,
                previous: 1,
            },
            max_ticks: 3,
        };
        assert!(run_wait(&mut emulator, &mut WaitClock::default(), request).is_err());
        assert_eq!(
            emulator.hybrid_bus().memory.read_main_ram::<true>(PARK_PC),
            0x1234
        );
        assert_eq!(
            emulator
                .hybrid_bus()
                .memory
                .read_main_ram::<true>(PARK_PC + 2),
            0x5678
        );
    }

    #[test]
    fn unchecked_wait_moves_and_retains_the_audio_allocation() {
        let mut emulator = GenesisEmulator::create(
            None,
            GenesisEmulatorConfig::default(),
            &mut NoCartridgeSaves,
        );
        set_sp(emulator.hybrid_parts().0, 0xff7ffc);
        emulator.hybrid_parts().0.set_status_register(0x2700);
        emulator
            .hybrid_bus_mut()
            .memory
            .write_main_ram::<true>(0xff8000, 1);
        let mut samples = Vec::with_capacity(4096);
        samples.push([0.25, -0.5]);
        let allocation = samples.as_ptr();
        let run = run_wait_with_audio(
            &mut emulator,
            &mut WaitClock::default(),
            WaitRequest::vblank_frame(0xff8000, 0),
            samples,
            false,
        )
        .unwrap();
        assert_eq!(run.audio.as_ptr(), allocation);
        assert_eq!(run.audio, [[0.25, -0.5]]);
        assert!(run.accesses.is_empty());
        assert_eq!(run.ticks, 0);
    }

    #[test]
    fn frame_timeline_applies_recipe_and_polls_each_new_frame() {
        struct Timeline(InputScript);
        impl WaitInputPoller for Timeline {
            fn at(&self, frame: u64) -> Result<GenesisInputs> {
                Ok(self.0.at(frame))
            }
            fn before_frame(&self, frame: u64, memory: &mut dyn tje_mem::Bus) -> Result<()> {
                memory.write_u16(0xff8000, frame as u16);
                Ok(())
            }
        }
        let script = InputScript::parse(r#"{"holds":[{"start":1,"end":2,"p1":["a"]}]}"#).unwrap();
        let initial = script.at(0);
        let held = script.at(1);
        let mut clock = WaitClock {
            frame: 0,
            inputs: initial,
            poller: Some(Arc::new(Timeline(script))),
        };
        let mut emulator = GenesisEmulator::create(
            None,
            GenesisEmulatorConfig::default(),
            &mut NoCartridgeSaves,
        );
        emulator.hybrid_bus_mut().hybrid_cpu_accesses = Some(Vec::new());
        clock.advance_frame(&mut emulator).unwrap();
        assert_eq!(clock.frame, 1);
        assert_eq!(clock.inputs, held);
        assert_eq!(
            emulator.hybrid_bus().memory.read_main_ram::<true>(0xff8000),
            1
        );
        clock.advance_frame(&mut emulator).unwrap();
        assert_eq!(clock.frame, 2);
        assert_eq!(clock.inputs, initial);
        assert_eq!(
            emulator.hybrid_bus().memory.read_main_ram::<true>(0xff8000),
            2
        );
        assert_eq!(
            emulator
                .hybrid_bus()
                .hybrid_cpu_accesses
                .as_ref()
                .unwrap()
                .len(),
            4
        );
    }
}
