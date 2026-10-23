//! Non-atomic logic contracts and real-clock suspension.
//!
//! Functional forks discard only device transaction latency. Time-aware forks
//! execute real interrupts, but approve logic/transactions, not raster/audio phase.
//! A native scheduler must be verified separately against whole-session timing.

use crate::{
    HostError, Result,
    abi::{AtomicRun, OrderedWrite, UninitializedStackRead, canonical_ram_address},
    catalogue::{STACK_BOTTOM, STACK_TOP},
    check_projection::{LogicEvent, LogicTrace, tight_ym_busy_loop},
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
    pub(crate) fn advance_frame(&mut self, emulator: &mut GenesisEmulator) -> Result<()> {
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
    pub logic_trace: LogicTrace,
    exception_base: u32,
    entry_boundary: Option<crate::exits::ReferenceBoundary>,
    instructions: u64,
    previous_pc: Option<u32>,
    pub audio: Vec<[f64; 2]>,
}

// G src/core/error_trap.c and both revision linker maps: the original two-byte
// ErrorTrap self-branch is at 0x200. Execute the immutable ROM instruction with
// the real CPU; never borrow work RAM for host-made code or a prefetch word.
const PARK_PC: u32 = 0x000200;
const MAX_TIMED_TICKS: u64 = 1_000_000;

fn condition_holds(
    emulator: &mut GenesisEmulator,
    condition: WaitCondition,
    interrupt_returned: bool,
) -> Result<bool> {
    let bus = emulator.hybrid_bus_mut();
    Ok(match condition {
        WaitCondition::Interrupt => interrupt_returned,
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
            if mask & !crate::open_bus::driven_mask(bus, address, true) != 0 {
                return Err(tje_game::Error::from(
                    tje_game::open_bus::OpenBusError::SourceRequired {
                        address: address & tje_mem::ADDRESS_MASK,
                        word: true,
                    },
                )
                .into());
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
            if u16::from(mask) & !crate::open_bus::driven_mask(bus, address, false) != 0 {
                return Err(tje_game::Error::from(
                    tje_game::open_bus::OpenBusError::SourceRequired {
                        address: address & tje_mem::ADDRESS_MASK,
                        word: false,
                    },
                )
                .into());
            }
            bus.read_byte(address) & mask == value
        }
    })
}

pub(crate) fn functional_wait_supported(request: WaitRequest) -> bool {
    matches!(
        request.condition,
        WaitCondition::DeviceByte {
            address: 0xa04000 | 0xa04002,
            mask: 0x80,
            value: 0
        } | WaitCondition::DeviceWord {
            address: 0xa11100,
            mask: 0x0100,
            value: 0
        }
    )
}

/// Discharge only an already-ready device predicate on a functional check fork.
/// The actual bus read still occurs, including its trace/latch/cache effects.
/// Unsatisfied readiness requires timed execution; never invent success.
pub(crate) fn discharge_functional_wait(
    emulator: &mut GenesisEmulator,
    request: WaitRequest,
) -> Result<()> {
    if !emulator.hybrid_bus().hybrid_functional_mode() {
        return Err(HostError::Metadata(
            "functional readiness discharge requires a functional check fork".into(),
        ));
    }
    if !functional_wait_supported(request) {
        return Err(HostError::Metadata(format!(
            "functional wait cannot discharge {:?}",
            request.condition
        )));
    }
    emulator.hybrid_bus_mut().apply_pending_writes();
    if !condition_holds(emulator, request.condition, false)? {
        return Err(HostError::Metadata(format!(
            "functional device predicate {:?} is not ready; timed execution is required",
            request.condition
        )));
    }
    Ok(())
}

fn drain_accesses(
    emulator: &mut GenesisEmulator,
    pc: u32,
    master_clock: u64,
    ym_busy_loop: bool,
    run: &mut ClockRun,
) {
    let bus = emulator.hybrid_bus_mut();
    if let Some(accesses) = bus.hybrid_cpu_accesses.as_mut() {
        for access in accesses.drain(..) {
            run.accesses.push((pc, access));
            run.logic_trace
                .access(pc, master_clock, access, ym_busy_loop);
        }
    }
}

fn timed_tick(
    emulator: &mut GenesisEmulator,
    clock: &mut WaitClock,
    run: &mut ClockRun,
    audio: &mut HeadlessAudio,
) -> Result<()> {
    let pc = emulator.hybrid_cpu().pc();
    let master_clock = emulator.hybrid_bus().cycles.m68k_mclk_cycles;
    let depth_before = emulator.hybrid_cpu().hybrid_exception_depth();
    let auditing = emulator.hybrid_bus().hybrid_cpu_accesses.is_some();
    let bus = emulator.hybrid_bus();
    let ready =
        bus.cycles.m68k_wait_cpu_cycles == 0 && emulator.hybrid_cpu().hybrid_instruction_ready(bus);
    if ready {
        run.previous_pc = Some(pc);
        run.instructions += 1;
    }
    let ym_busy_loop = auditing && ready && tight_ym_busy_loop(emulator, pc);
    let effect = emulator
        .tick(
            &mut HeadlessOutput,
            audio,
            &mut Inputs(clock.inputs),
            &mut NoCartridgeSaves,
        )
        .map_err(|error| HostError::Emulator(error.to_string()))?;
    run.ticks += 1;
    let depth_after = emulator.hybrid_cpu().hybrid_exception_depth();
    if auditing && depth_after > depth_before {
        let vector = emulator
            .hybrid_cpu()
            .hybrid_last_exception_vector()
            .ok_or_else(|| {
                HostError::Metadata("accepted exception has no observed vector".into())
            })?;
        run.logic_trace.events.push(LogicEvent::ExceptionEntered {
            vector,
            handler: emulator.hybrid_cpu().pc(),
            depth: depth_after - run.exception_base,
            master_clock,
        });
    }
    drain_accesses(emulator, pc, master_clock, ym_busy_loop, run);
    if auditing && depth_after < depth_before {
        let root_return = run.entry_boundary.is_some_and(|boundary| {
            boundary.entry_exception_return
                && depth_before == run.exception_base
                && depth_after.checked_add(1) == Some(run.exception_base)
                && boundary.reached(emulator.hybrid_cpu(), run.previous_pc, run.instructions)
        });
        if !root_return {
            let depth = depth_after.checked_sub(run.exception_base).ok_or_else(|| {
                HostError::Metadata("timed call returned through its entry exception frame".into())
            })?;
            run.logic_trace.events.push(LogicEvent::ExceptionReturned {
                depth,
                master_clock: emulator.hybrid_bus().cycles.m68k_mclk_cycles,
            });
        }
    }
    if effect == TickEffect::FrameRendered {
        run.frames += 1;
        clock.advance_frame(emulator)?;
        // Timeline writes belong to this completed tick, not to a later IRQ
        // that happens to enter before the next ordinary instruction.
        drain_accesses(
            emulator,
            emulator.hybrid_cpu().pc(),
            emulator.hybrid_bus().cycles.m68k_mclk_cycles,
            false,
            run,
        );
    }
    Ok(())
}

/// A suspended native continuation is an instruction boundary: accept every IRQ
/// the pinned CPU would accept there, including its ten-cycle recognition stage,
/// before resuming Rust. Masked/same-level requests are intentionally ready.
fn native_resume_ready(emulator: &GenesisEmulator) -> bool {
    let bus = emulator.hybrid_bus();
    bus.cycles.m68k_wait_cpu_cycles == 0 && emulator.hybrid_cpu().hybrid_instruction_ready(bus)
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
    if emulator.hybrid_cpu().status_register() & 0x8000 != 0 {
        return Err(HostError::Metadata(
            "trace exceptions are not an interrupt-wait contract".into(),
        ));
    }
    emulator.hybrid_bus_mut().apply_pending_writes();
    let saved_pc = emulator.hybrid_cpu().pc();
    let saved_sp = emulator.hybrid_cpu().stack_pointer();
    if !emulator
        .hybrid_bus()
        .cartridge
        .as_ref()
        .is_some_and(|cartridge| cartridge.peek_word(PARK_PC) == 0x60fe)
    {
        return Err(HostError::Metadata(
            "a wait requires the original ROM ErrorTrap self-branch at 000200".into(),
        ));
    }
    let saved_trace = std::mem::replace(
        &mut emulator.hybrid_bus_mut().hybrid_cpu_accesses,
        audit.then(Vec::new),
    );
    {
        let (cpu, bus) = emulator.hybrid_parts();
        cpu.set_pc(PARK_PC, bus);
    }
    let mut run = ClockRun {
        exception_base: emulator.hybrid_cpu().hybrid_exception_depth(),
        ..ClockRun::default()
    };
    let mut audio = HeadlessAudio { samples };
    let mut handler_entered = false;
    let result = (|| {
        loop {
            let depth = emulator.hybrid_cpu().hybrid_exception_depth();
            let at_park = emulator.hybrid_cpu().pc() == PARK_PC
                && emulator.hybrid_cpu().stack_pointer() == saved_sp;
            if depth < run.exception_base || (depth == run.exception_base && !at_park) {
                return Err(HostError::Metadata(format!(
                    "an ISR escaped the wait continuation: PC={:06X}, SP={:08X}, depth={depth}",
                    emulator.hybrid_cpu().pc(),
                    emulator.hybrid_cpu().stack_pointer(),
                )));
            }
            handler_entered |= depth > run.exception_base;
            if at_park && depth == run.exception_base {
                let satisfied = condition_holds(emulator, request.condition, handler_entered)?;
                drain_accesses(
                    emulator,
                    PARK_PC,
                    emulator.hybrid_bus().cycles.m68k_mclk_cycles,
                    false,
                    &mut run,
                );
                if satisfied && native_resume_ready(emulator) {
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
    // A failure aborts the call; never pretend an interrupted ISR returned.
    if result.is_ok() {
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
    run_original_timed_until_with_audio(
        emulator,
        clock,
        crate::exits::ReferenceBoundary::returned(return_pc, return_sp),
        samples,
        audit,
    )
}

pub(crate) fn run_original_timed_until(
    emulator: &mut GenesisEmulator,
    clock: &mut WaitClock,
    boundary: crate::exits::ReferenceBoundary,
) -> Result<(AtomicRun, ClockRun)> {
    run_original_timed_until_with_audio(emulator, clock, boundary, Vec::new(), true)
}

pub(crate) fn run_original_timed_until_with_audio(
    emulator: &mut GenesisEmulator,
    clock: &mut WaitClock,
    boundary: crate::exits::ReferenceBoundary,
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
    let mut run = ClockRun {
        exception_base: emulator.hybrid_cpu().hybrid_exception_depth(),
        entry_boundary: Some(boundary),
        ..ClockRun::default()
    };
    let mut audio = HeadlessAudio { samples };
    let result = (|| {
        let ordinary_return =
            boundary.sp.is_some() && boundary.source.is_none() && !boundary.entry_exception_return;
        let mut return_reached = false;
        loop {
            let reached =
                boundary.reached(emulator.hybrid_cpu(), run.previous_pc, run.instructions);
            let depth = emulator.hybrid_cpu().hybrid_exception_depth();
            return_reached |= ordinary_return && reached && depth == run.exception_base;
            if return_reached
                && (depth < run.exception_base || (depth == run.exception_base && !reached))
            {
                return Err(HostError::Metadata(format!(
                    "an ISR escaped the timed return boundary: PC={:06X}, SP={:08X}, depth={depth}",
                    emulator.hybrid_cpu().pc(),
                    emulator.hybrid_cpu().stack_pointer(),
                )));
            }
            if reached
                && (!ordinary_return
                    || (depth == run.exception_base && native_resume_ready(emulator)))
            {
                break;
            }
            // At the actual caller PC/SP, only IRQ recognition or real bus debt
            // can make ready false. Their ticks do not execute the caller opcode.
            if run.ticks == MAX_TIMED_TICKS {
                return Err(HostError::CallDidNotReturn {
                    pc: emulator.hybrid_cpu().pc(),
                    return_pc: boundary.pc,
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
        stack_reads: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::set_sp;
    use genesis_config::GenesisEmulatorConfig;
    use genesis_core::GenesisBus;
    use jgenesis_common::frontend::{PartialClone, TimingMode};

    fn write(bus: &mut GenesisBus, address: u32, value: u16) {
        bus.write_word(address, value);
        bus.apply_pending_writes();
    }

    fn emulator_with_park_rom() -> GenesisEmulator {
        // Authored reset vectors and real self-branches; no cartridge bytes.
        let mut rom = vec![0; 1024];
        rom[..4].copy_from_slice(&0xff7ffc_u32.to_be_bytes());
        rom[4..8].copy_from_slice(&0x100_u32.to_be_bytes());
        rom[0x100..0x102].copy_from_slice(&0x60fe_u16.to_be_bytes());
        rom[PARK_PC as usize..PARK_PC as usize + 2].copy_from_slice(&0x60fe_u16.to_be_bytes());
        rom[PARK_PC as usize + 2..PARK_PC as usize + 4].copy_from_slice(&0x4e71_u16.to_be_bytes());
        GenesisEmulator::create(
            Some(rom),
            GenesisEmulatorConfig::default(),
            &mut NoCartridgeSaves,
        )
    }

    fn assert_whole_ram_visible_during_wait(emulator: &mut GenesisEmulator) {
        use std::sync::atomic::{AtomicUsize, Ordering};

        fn pattern(offset: u32) -> u8 {
            (offset.wrapping_mul(37) ^ (offset >> 8)) as u8
        }

        struct WholeRamReader {
            reads: AtomicUsize,
        }
        impl WaitInputPoller for WholeRamReader {
            fn at(&self, _: u64) -> Result<GenesisInputs> {
                Ok(GenesisInputs::default())
            }

            fn before_frame(&self, _: u64, memory: &mut dyn tje_mem::Bus) -> Result<()> {
                // Read every byte while Rust is suspended, not after wait cleanup.
                for offset in 0..0x10000 {
                    assert_eq!(
                        memory.read_u8(0xff0000 + offset),
                        pattern(offset),
                        "whole-RAM observer saw a changed byte at FF{offset:04X}",
                    );
                }
                self.reads.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
        }

        {
            let bus = emulator.hybrid_bus_mut();
            for offset in 0..0x10000 {
                bus.memory
                    .write_main_ram_byte(0xff0000 + offset, pattern(offset));
            }
        }
        // No game ISR is allowed to modify the authored fixture: all 64 KiB
        // should remain visible exactly as supplied while the real clocks run.
        emulator.hybrid_parts().0.set_status_register(0x2700);
        let previous = emulator.hybrid_bus().memory.read_main_ram::<true>(0xff8000);
        let reader = Arc::new(WholeRamReader {
            reads: AtomicUsize::new(0),
        });
        let mut clock = WaitClock {
            poller: Some(reader.clone()),
            ..WaitClock::default()
        };
        let request = WaitRequest {
            condition: WaitCondition::RamWordChanged {
                address: 0xff8000,
                previous,
            },
            max_ticks: 50_000,
        };
        let error = run_wait(emulator, &mut clock, request).unwrap_err();
        assert!(error.to_string().contains("was not satisfied"), "{error}");
        assert!(clock.frame >= 2);
        assert!(reader.reads.load(Ordering::Relaxed) >= 2);
        assert_eq!(emulator.hybrid_cpu().pc(), PARK_PC);
        assert_eq!(
            emulator.hybrid_bus().memory.read_main_ram::<true>(0xff0000),
            u16::from_be_bytes([pattern(0), pattern(1)]),
        );
    }

    #[test]
    fn whole_ram_reader_never_sees_host_parking_bytes() {
        assert_whole_ram_visible_during_wait(&mut emulator_with_park_rom());
    }

    #[test]
    #[ignore = "requires both private retail ROMs; reads all RAM during real-clock waits"]
    fn both_revisions_keep_whole_ram_visible_during_waits() {
        for (key, revision) in [
            ("TJE_REV02_ROM", tje_rom::Revision::Rev02),
            ("TJE_REV00_ROM", tje_rom::Revision::Rev00),
        ] {
            assert_eq!(
                tje_game::ids::ERROR_TRAP.metadata().address(revision),
                PARK_PC,
            );
            let rom = std::fs::read(std::env::var_os(key).expect("private ROM path")).unwrap();
            let mut emulator = GenesisEmulator::create(
                Some(rom),
                GenesisEmulatorConfig::default(),
                &mut NoCartridgeSaves,
            );
            assert_whole_ram_visible_during_wait(&mut emulator);
            println!("PASS {revision:?}: all 65536 RAM bytes unchanged during suspended wait");
        }
    }

    #[test]
    fn wait_without_the_original_rom_loop_fails_before_parking() {
        let mut emulator = GenesisEmulator::create(
            None,
            GenesisEmulatorConfig::default(),
            &mut NoCartridgeSaves,
        );
        let before = (
            emulator.hybrid_cpu().pc(),
            emulator.hybrid_cpu().stack_pointer(),
        );
        let error = run_wait(
            &mut emulator,
            &mut WaitClock::default(),
            WaitRequest::vblank_frame(0xff8000, 1),
        )
        .unwrap_err();
        assert!(error.to_string().contains("original ROM ErrorTrap"));
        assert_eq!(
            (
                emulator.hybrid_cpu().pc(),
                emulator.hybrid_cpu().stack_pointer()
            ),
            before,
        );
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
    fn wait_timeout_does_not_change_work_ram() {
        let mut emulator = emulator_with_park_rom();
        set_sp(emulator.hybrid_parts().0, 0xff7ffc);
        emulator.hybrid_parts().0.set_status_register(0x2700);
        emulator
            .hybrid_bus_mut()
            .memory
            .write_main_ram::<true>(0xff0000, 0x1234);
        emulator
            .hybrid_bus_mut()
            .memory
            .write_main_ram::<true>(0xff0002, 0x5678);
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
            emulator.hybrid_bus().memory.read_main_ram::<true>(0xff0000),
            0x1234
        );
        assert_eq!(
            emulator.hybrid_bus().memory.read_main_ram::<true>(0xff0002),
            0x5678
        );
    }

    #[test]
    fn unchecked_wait_moves_and_retains_the_audio_allocation() {
        let mut emulator = emulator_with_park_rom();
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

    #[test]
    fn interrupt_wait_does_not_succeed_without_an_accepted_handler() {
        let mut emulator = emulator_with_park_rom();
        set_sp(emulator.hybrid_parts().0, 0xff7ffc);
        emulator.hybrid_parts().0.set_status_register(0x2700);
        let request = WaitRequest {
            max_ticks: 3,
            ..WaitRequest::interrupt()
        };
        let error = run_wait(&mut emulator, &mut WaitClock::default(), request).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("was not satisfied after 3 real ticks")
        );
    }

    #[test]
    fn trace_mode_cannot_masquerade_as_an_external_interrupt() {
        let mut emulator = GenesisEmulator::create(
            None,
            GenesisEmulatorConfig::default(),
            &mut NoCartridgeSaves,
        );
        set_sp(emulator.hybrid_parts().0, 0xff7ffc);
        emulator.hybrid_parts().0.set_status_register(0xa000);
        let error = run_wait(
            &mut emulator,
            &mut WaitClock::default(),
            WaitRequest::interrupt(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("trace exceptions"));
    }

    #[test]
    fn functional_wait_uses_actual_busack_and_keeps_the_read() {
        let mut emulator = GenesisEmulator::create(
            None,
            GenesisEmulatorConfig::default(),
            &mut NoCartridgeSaves,
        );
        let bus = emulator.hybrid_bus_mut();
        bus.hybrid_set_functional_mode(true);
        write(bus, 0xa11100, 0x100);
        bus.hybrid_accesses = Some(Vec::new());
        let phase = (bus.vdp.scanline(), bus.vdp.scanline_mclk());
        assert!(discharge_functional_wait(&mut emulator, WaitRequest::z80_busack()).is_err());
        let bus = emulator.hybrid_bus_mut();
        assert!(matches!(bus.hybrid_accesses.as_ref().unwrap().as_slice(),
            [HybridAccess::Read { address: 0xa11100, word: true, value, .. }] if value & 0x100 != 0));
        write(bus, 0xa11200, 0x100);
        bus.hybrid_accesses.as_mut().unwrap().clear();
        discharge_functional_wait(&mut emulator, WaitRequest::z80_busack()).unwrap();
        let bus = emulator.hybrid_bus();
        assert!(matches!(bus.hybrid_accesses.as_ref().unwrap().as_slice(),
            [HybridAccess::Read { address: 0xa11100, word: true, value, .. }] if value & 0x100 == 0));
        assert_eq!((bus.vdp.scanline(), bus.vdp.scanline_mclk()), phase);
    }

    #[test]
    fn functional_wait_does_not_forge_ym_cached_readiness() {
        let config = GenesisEmulatorConfig {
            opn2_busy_behavior: genesis_config::Opn2BusyBehavior::Ym2612,
            ..GenesisEmulatorConfig::default()
        };
        let mut emulator = GenesisEmulator::create(None, config, &mut NoCartridgeSaves);
        let bus = emulator.hybrid_bus_mut();
        write(bus, 0xa11200, 0x100);
        write(bus, 0xa11100, 0x100);
        bus.write_byte(0xa04000, 0x22);
        bus.apply_pending_writes();
        bus.write_byte(0xa04001, 0);
        bus.apply_pending_writes();
        assert_ne!(bus.read_byte(0xa04000) & 0x80, 0);
        bus.hybrid_set_functional_mode(true);
        let bank1 = WaitRequest::new(WaitCondition::DeviceByte {
            address: 0xa04002,
            mask: 0x80,
            value: 0,
        });
        assert!(discharge_functional_wait(&mut emulator, bank1).is_err());
        discharge_functional_wait(&mut emulator, WaitRequest::ym2612_ready()).unwrap();
        discharge_functional_wait(&mut emulator, bank1).unwrap();
    }

    #[test]
    fn functional_wait_rejects_time_and_unreviewed_predicates_before_reading() {
        let mut emulator = GenesisEmulator::create(
            None,
            GenesisEmulatorConfig::default(),
            &mut NoCartridgeSaves,
        );
        assert!(discharge_functional_wait(&mut emulator, WaitRequest::ym2612_ready()).is_err());
        emulator.hybrid_bus_mut().hybrid_set_functional_mode(true);
        emulator.hybrid_bus_mut().hybrid_accesses = Some(Vec::new());
        for request in [
            WaitRequest::vblank_frame(0xff8000, 0),
            WaitRequest::dma_done(),
            WaitRequest::interrupt(),
            WaitRequest::new(WaitCondition::DeviceByte {
                address: 0xa04001,
                mask: 0x80,
                value: 0,
            }),
            WaitRequest::new(WaitCondition::DeviceWord {
                address: 0xa11100,
                mask: 0xff00,
                value: 0,
            }),
        ] {
            assert!(discharge_functional_wait(&mut emulator, request).is_err());
        }
        assert!(
            emulator
                .hybrid_bus()
                .hybrid_accesses
                .as_ref()
                .unwrap()
                .is_empty()
        );
    }

    fn pending_vblank_and_hblank() -> (GenesisEmulator, WaitClock) {
        pending_vblank_and_hblank_with_handler(&[0x5279, 0x00ff, 0x8000, 0x4e73])
    }

    fn pending_vblank_and_hblank_with_handler(
        vblank_handler: &[u16],
    ) -> (GenesisEmulator, WaitClock) {
        // Authored vectors/handlers: VBlank increments FF8000, HBlank FF8002.
        // Main's ADDQ D7 must never execute while native continuation is parked.
        let mut rom = vec![0; 2048];
        rom[..4].copy_from_slice(&0xff7ffc_u32.to_be_bytes());
        rom[4..8].copy_from_slice(&0x100_u32.to_be_bytes());
        rom[28 * 4..29 * 4].copy_from_slice(&0x500_u32.to_be_bytes());
        rom[30 * 4..31 * 4].copy_from_slice(&0x400_u32.to_be_bytes());
        for (address, words) in [
            (0x100, &[0x5287, 0x60fc][..]),
            (PARK_PC as usize, &[0x60fe, 0x4e71]),
            (0x400, vblank_handler),
            (0x500, &[0x5279, 0x00ff, 0x8002, 0x4e73]),
            (0x380, &[0x60fe, 0x4e71]),
            (0x3c0, &[0x5287, 0x60fc]),
        ] {
            for (index, word) in words.iter().enumerate() {
                let offset = address + 2 * index;
                rom[offset..offset + 2].copy_from_slice(&u16::to_be_bytes(*word));
            }
        }
        let mut emulator = GenesisEmulator::create(
            Some(rom),
            GenesisEmulatorConfig::default(),
            &mut NoCartridgeSaves,
        );
        emulator.hybrid_parts().0.set_status_register(0x2700);
        let (cpu, bus) = emulator.hybrid_parts();
        cpu.set_pc(0x380, bus);
        write(bus, 0xc00004, 0x8010);
        write(bus, 0xc00004, 0x8120);
        write(bus, 0xc00004, 0x8a00);
        let mut clock = WaitClock {
            poller: Some(Arc::new(InputScript::default())),
            ..WaitClock::default()
        };
        let mut run = ClockRun::default();
        let mut audio = HeadlessAudio::default();
        for _ in 0..100_000 {
            if emulator.hybrid_bus().interrupt_level() == 6
                && emulator.hybrid_bus().cycles.m68k_wait_cpu_cycles == 0
            {
                break;
            }
            timed_tick(&mut emulator, &mut clock, &mut run, &mut audio).unwrap();
        }
        assert_eq!(emulator.hybrid_bus().interrupt_level(), 6);
        let (cpu, bus) = emulator.hybrid_parts();
        cpu.set_pc(0x100, bus);
        (emulator, clock)
    }

    fn accepted_vectors(run: &ClockRun) -> Vec<u32> {
        run.logic_trace
            .events
            .iter()
            .filter_map(|event| match event {
                LogicEvent::ExceptionEntered { vector, .. } => Some(*vector),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn wait_resumes_only_after_pending_vblank_then_hblank_handlers_return() {
        let (mut emulator, mut clock) = pending_vblank_and_hblank();
        emulator.hybrid_parts().0.set_status_register(0x2000);
        let before_d7 = emulator.hybrid_cpu().data_registers()[7];
        let run = run_wait(
            &mut emulator,
            &mut clock,
            WaitRequest::vblank_frame(0xff8000, 0),
        )
        .unwrap();
        assert_eq!(accepted_vectors(&run), [30, 28]);
        assert_eq!(
            emulator.hybrid_bus().memory.read_main_ram::<true>(0xff8000),
            1
        );
        assert_eq!(
            emulator.hybrid_bus().memory.read_main_ram::<true>(0xff8002),
            1
        );
        assert_eq!(emulator.hybrid_cpu().pc(), 0x100);
        assert_eq!(emulator.hybrid_cpu().stack_pointer(), 0xff7ffc);
        assert_eq!(emulator.hybrid_cpu().status_register() & 0xff00, 0x2000);
        assert_eq!(emulator.hybrid_cpu().data_registers()[7], before_d7);
        assert_eq!(emulator.hybrid_cpu().hybrid_exception_depth(), 0);
        assert!(native_resume_ready(&emulator));
    }

    #[test]
    fn wait_leaves_masked_and_same_level_interrupts_pending() {
        for mask in [4_u16, 7] {
            let (mut emulator, mut clock) = pending_vblank_and_hblank();
            if mask == 4 {
                // Drop only the VBlank request using the actual IACK semantics.
                emulator.hybrid_bus_mut().acknowledge_interrupt(6);
                assert_eq!(emulator.hybrid_bus().interrupt_level(), 4);
            }
            emulator
                .hybrid_parts()
                .0
                .set_status_register(0x2000 | (mask << 8));
            let level = emulator.hybrid_bus().interrupt_level();
            let run = run_wait(
                &mut emulator,
                &mut clock,
                WaitRequest::vblank_frame(0xff8000, 1),
            )
            .unwrap();
            assert_eq!(run.ticks, 0);
            assert!(accepted_vectors(&run).is_empty());
            assert_eq!(emulator.hybrid_bus().interrupt_level(), level);
            assert_eq!(
                emulator.hybrid_cpu().status_register() & 0xff00,
                0x2000 | (mask << 8)
            );
        }
    }

    #[test]
    fn already_recognized_irq_is_delivered_before_a_satisfied_wait() {
        let (mut emulator, mut clock) = pending_vblank_and_hblank();
        emulator.hybrid_parts().0.set_status_register(0x2000);
        let before_pc = emulator.hybrid_cpu().pc();
        let before_d7 = emulator.hybrid_cpu().data_registers()[7];
        let mut ignored = ClockRun::default();
        timed_tick(
            &mut emulator,
            &mut clock,
            &mut ignored,
            &mut HeadlessAudio::default(),
        )
        .unwrap();
        // Pinned core recognizes in ten cycles; no caller opcode or frame yet.
        assert_eq!(emulator.hybrid_cpu().pc(), before_pc);
        assert_eq!(emulator.hybrid_cpu().data_registers()[7], before_d7);
        assert_eq!(emulator.hybrid_cpu().hybrid_exception_depth(), 0);
        let run = run_wait(
            &mut emulator,
            &mut clock,
            WaitRequest::vblank_frame(0xff8000, 2),
        )
        .unwrap();
        assert_eq!(accepted_vectors(&run), [30, 28]);
        assert_eq!(emulator.hybrid_cpu().data_registers()[7], before_d7);
    }

    #[test]
    fn timed_return_drains_eligible_irqs_without_executing_the_caller() {
        let (mut emulator, mut clock) = pending_vblank_and_hblank();
        emulator.hybrid_parts().0.set_status_register(0x2000);
        let pc = emulator.hybrid_cpu().pc();
        let sp = emulator.hybrid_cpu().stack_pointer();
        let before_d7 = emulator.hybrid_cpu().data_registers()[7];
        let (_, run) = run_original_timed(&mut emulator, &mut clock, pc, sp).unwrap();
        assert_eq!(accepted_vectors(&run), [30, 28]);
        assert_eq!(emulator.hybrid_cpu().data_registers()[7], before_d7);
        assert_eq!(
            (
                emulator.hybrid_cpu().pc(),
                emulator.hybrid_cpu().stack_pointer()
            ),
            (pc, sp)
        );
        assert_eq!(emulator.hybrid_cpu().status_register() & 0xff00, 0x2000);
    }

    #[test]
    fn escaped_isr_return_fails_before_any_unrelated_caller_instruction() {
        // Actual authored ISR instructions, not a host mutation after RTE.
        // First changes saved PC; second clears saved S to restore the wrong SP bank.
        for (handler, wrong_pc) in [
            (&[0x2f7c, 0x0000, 0x03c0, 0x0002, 0x4e73][..], true),
            (&[0x0257, 0xdfff, 0x4e73][..], false),
        ] {
            for timed_return in [false, true] {
                let (mut emulator, mut clock) = pending_vblank_and_hblank_with_handler(handler);
                let cpu = emulator.hybrid_parts().0;
                cpu.set_address_registers(
                    cpu.address_registers(),
                    0xff7200,
                    cpu.supervisor_stack_pointer(),
                );
                cpu.set_status_register(0x2000);
                let before_pc = cpu.pc();
                let before_sp = cpu.stack_pointer();
                let before_d7 = cpu.data_registers()[7];
                let error = if timed_return {
                    run_original_timed(&mut emulator, &mut clock, before_pc, before_sp).map(|_| ())
                } else {
                    run_wait(&mut emulator, &mut clock, WaitRequest::interrupt()).map(|_| ())
                }
                .unwrap_err();
                assert!(error.to_string().contains("escaped"), "{error}");
                let cpu = emulator.hybrid_cpu();
                assert_eq!(cpu.hybrid_exception_depth(), 0);
                assert_eq!(cpu.data_registers()[7], before_d7);
                if wrong_pc {
                    assert_eq!(cpu.pc(), 0x3c0);
                    assert_eq!(cpu.stack_pointer(), before_sp);
                    assert_eq!(cpu.next_opcode(), 0x5287);
                } else {
                    assert_eq!(cpu.pc(), if timed_return { before_pc } else { PARK_PC });
                    assert_eq!(cpu.stack_pointer(), 0xff7200);
                    assert_eq!(cpu.status_register() & 0x2000, 0);
                }
            }
        }
    }
}
