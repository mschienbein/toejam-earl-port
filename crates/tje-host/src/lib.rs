//! Headless REV 02/REV 00 reference and hybrid host built on jgenesis.
//!
//! `Check` compares atomic function contracts on independent state forks, with
//! interrupts/peripheral clocks frozen, then lets the live ROM execute with its
//! original instruction timing. This preserves timing-dependent random draws and
//! lag measurements. `Replace` runs Rust instead, but is NOT cycle-equivalent.
//! Dead stack bytes below the entry SP are informational, not native game state;
//! reads of those bytes before a write are explicitly audited on the original.

pub mod abi;
pub mod catalogue;
pub mod execution;
pub mod hooks;
pub mod input;
mod play;
pub mod session_trace;
mod sinks;

use abi::{HookContext, OrderedWrite, UninitializedStackRead, run_original_atomic, set_sp};
use execution::{WaitClock, WaitInputPoller, run_original_timed};
use genesis_config::{GenesisEmulatorConfig, GenesisInputs};
use genesis_core::{GenesisEmulator, HybridWrite};
use hooks::{Hook, HookRegistry};
use jgenesis_common::frontend::{Color, EmulatorTrait, FrameSize, PartialClone, TickEffect};
use serde::Serialize;
use sinks::{HeadlessAudio, HeadlessOutput, Inputs, NoCartridgeSaves};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use tje_game::execution::ExecutionClass;
use tje_rom::{Revision, Rom};

pub type Result<T, E = HostError> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error("ROM import failed: {0}")]
    Rom(#[from] tje_rom::ImportError),
    #[error("hook registry targets {registry:?}, ROM is {rom:?}")]
    HookRevisionMismatch { registry: Revision, rom: Revision },
    #[error(transparent)]
    Translation(#[from] tje_game::Error),
    #[error("invalid metadata: {0}")]
    Metadata(String),
    #[error(
        "{function} has no atomic-capable contract; clock/interrupt progress is unsupported by atomic hooks"
    )]
    NotAtomicCapable { function: String },
    #[error("invalid input script: {0}")]
    Input(String),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    #[error("emulator: {0}")]
    Emulator(String),
    #[error(
        "original call did not return to {return_pc:06X} (PC={pc:06X}); atomic calls must not wait for peripheral time"
    )]
    CallDidNotReturn { pc: u32, return_pc: u32 },
    #[error("check mismatch at frame {frame}, {function}: {detail}")]
    Mismatch {
        frame: u64,
        function: String,
        detail: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Mode {
    Reference,
    Check,
    Replace,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum HardwareWrite {
    Vdp { port: u8, value: u16 },
    Ym2612 { port: u8, value: u8 },
    Psg { value: u8 },
}
impl From<HybridWrite> for HardwareWrite {
    fn from(value: HybridWrite) -> Self {
        match value {
            HybridWrite::Vdp { port, value } => Self::Vdp { port, value },
            HybridWrite::Ym2612 { port, value } => Self::Ym2612 { port, value },
            HybridWrite::Psg { value } => Self::Psg { value },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VdpSnapshot {
    pub vram: Box<[u8]>,
    pub cram: Box<[u16]>,
    pub vsram: Box<[u16]>,
    /// bincode encoding of jgenesis's typed `vdp::registers::Registers`.
    pub registers: Vec<u8>,
}

#[derive(Debug)]
pub struct FrameSnapshot {
    /// Zero-based completed video frame, independent of the resettable game counter.
    pub frame: u64,
    pub frame_counter: u16,
    pub pc: u32,
    pub ram: Box<[u8]>,
    pub vdp: VdpSnapshot,
    pub writes: Vec<HardwareWrite>,
    /// Stereo 48 kHz resampled output, preserving the emulator's f64 bits.
    pub audio_samples: Vec<[f64; 2]>,
    pub idle_random_draws: u64,
    pub lag_events: Vec<LagEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LagEvent {
    pub frame: u64,
    pub pc_at_vblank: u32,
    pub frame_counter: u16,
}

#[derive(Debug, Default, Serialize)]
pub struct Measurements {
    pub intro_frame: Option<u64>,
    pub title_frame: Option<u64>,
    pub menu_frame: Option<u64>,
    pub game_frame: Option<u64>,
    pub logic_ticks: u64,
    pub idle_draws_per_frame: Vec<u64>,
    pub lag_events: Vec<LagEvent>,
    pub vdp_writes: u64,
    pub ym2612_writes: u64,
    pub psg_writes: u64,
    pub wait_video_frames: u64,
    pub check_original_wait_frames: u64,
    pub check_rust_wait_frames: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct StackReadSite {
    pub function: String,
    pub pc: u32,
    pub address: u32,
}

#[derive(Debug, Default, Serialize)]
pub struct FunctionCoverage {
    pub calls_checked: u64,
    pub calls_replaced: u64,
    pub mismatches: u64,
}

#[derive(Debug, Default, Serialize)]
pub struct SubsystemCoverage {
    pub registered_functions: usize,
    pub calls_checked: u64,
    pub calls_replaced: u64,
    pub mismatches: u64,
    pub unexercised: BTreeSet<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct CheckReport {
    pub calls: u64,
    pub by_function: BTreeMap<String, FunctionCoverage>,
    pub by_subsystem: BTreeMap<String, SubsystemCoverage>,
    /// Registered entries with no checked or replacement call in this run.
    pub unexercised: BTreeSet<String>,
    pub mismatches: u64,
    pub nested_original_calls: u64,
    pub dead_stack_differing_calls: u64,
    pub dead_stack_differing_bytes: u64,
    pub uninitialized_stack_reads: BTreeSet<StackReadSite>,
}

impl CheckReport {
    fn record_function(&mut self, name: &str, subsystem: &str, checked: bool) {
        let coverage = self.by_function.entry(name.to_owned()).or_default();
        if checked {
            coverage.calls_checked += 1;
        } else {
            coverage.calls_replaced += 1;
        }
        self.unexercised.remove(name);
        let coverage = self
            .by_subsystem
            .get_mut(subsystem)
            .expect("registered subsystem");
        if checked {
            coverage.calls_checked += 1;
        } else {
            coverage.calls_replaced += 1;
        }
        coverage.unexercised.remove(name);
    }
}

pub struct Host {
    emulator: GenesisEmulator,
    registry: HookRegistry,
    mode: Mode,
    revision: Revision,
    symbols: catalogue::RevisionSymbols,
    frame: u64,
    main_idle: bool,
    output: HeadlessOutput,
    audio: HeadlessAudio,
    saves: NoCartridgeSaves,
    audio_sample_rate: u32,
    pub(crate) session_trace: Option<session_trace::SessionTrace>,
    wait_clock: WaitClock,
    pub checks: CheckReport,
    pub measurements: Measurements,
}

impl Host {
    pub fn new(bytes: &[u8], mode: Mode, registry: HookRegistry) -> Result<Self> {
        Self::from_rom(Rom::import(bytes)?, mode, registry)
    }

    pub fn from_rom(rom: Rom, mode: Mode, registry: HookRegistry) -> Result<Self> {
        let revision = rom.revision();
        if registry.revision() != revision {
            return Err(HostError::HookRevisionMismatch {
                registry: registry.revision(),
                rom: revision,
            });
        }
        let initial_sp = u32::from_be_bytes(
            rom.bytes()[..4]
                .try_into()
                .expect("identified ROM has vectors"),
        );
        if initial_sp & 0xFF_FFFF != catalogue::STACK_TOP {
            return Err(HostError::Metadata(format!(
                "reset SP {initial_sp:08X} does not meet RAM map's stack boundary"
            )));
        }
        let mut saves = NoCartridgeSaves;
        let mut emulator = GenesisEmulator::create(
            Some(rom.into_bytes()),
            GenesisEmulatorConfig::default(),
            &mut saves,
        );
        emulator.hybrid_bus_mut().hybrid_writes = Some(Vec::new());
        let mut checks = CheckReport::default();
        for subsystem in tje_game::SUBSYSTEMS {
            checks
                .by_subsystem
                .insert(subsystem.name.to_owned(), SubsystemCoverage::default());
        }
        for hook in registry.iter() {
            checks
                .by_function
                .insert(hook.symbol.name.to_owned(), FunctionCoverage::default());
            checks.unexercised.insert(hook.symbol.name.to_owned());
            let coverage = checks
                .by_subsystem
                .entry(hook.symbol.subsystem.to_owned())
                .or_default();
            coverage.registered_functions += 1;
            coverage.unexercised.insert(hook.symbol.name.to_owned());
        }
        Ok(Self {
            emulator,
            registry,
            mode,
            revision,
            symbols: catalogue::RevisionSymbols::for_revision(revision),
            frame: 0,
            main_idle: false,
            output: HeadlessOutput,
            audio: HeadlessAudio::default(),
            saves,
            audio_sample_rate: 48_000,
            session_trace: None,
            wait_clock: WaitClock::default(),
            checks,
            measurements: Measurements::default(),
        })
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }
    pub fn revision(&self) -> Revision {
        self.revision
    }
    pub fn frame_count(&self) -> u64 {
        self.frame
    }

    pub fn set_wait_input_poller(&mut self, poller: Arc<dyn WaitInputPoller>) {
        self.wait_clock.poller = Some(poller);
    }
    pub fn frame_buffer(&self) -> (&[Color], FrameSize) {
        let vdp = &self.emulator.hybrid_bus().vdp;
        (vdp.frame_buffer(), vdp.frame_size())
    }
    pub fn cpu(&self) -> &m68000_emu::M68000 {
        self.emulator.hybrid_cpu()
    }
    pub fn context(&mut self) -> HookContext<'_> {
        HookContext::new(
            &mut self.emulator,
            self.revision,
            self.registry.dispatcher(),
        )
    }

    /// Inspect the typed VDP registers directly; unlike the snapshot encoding,
    /// this does not allocate and retains the upstream register field names.
    pub fn vdp_registers(&self) -> &genesis_components::vdp::registers::Registers {
        self.emulator.hybrid_bus().vdp.hybrid_registers()
    }

    fn tick(&mut self, inputs: &mut Inputs) -> Result<TickEffect> {
        self.emulator
            .tick(&mut self.output, &mut self.audio, inputs, &mut self.saves)
            .map_err(|e| HostError::Emulator(e.to_string()))
    }

    pub fn run_frame(&mut self, inputs: GenesisInputs) -> Result<FrameSnapshot> {
        self.wait_clock.frame = self.frame;
        self.wait_clock.inputs = inputs;
        let mut inputs = Inputs(inputs);
        let mut idle_draws = 0;
        let mut lag_events = Vec::new();
        loop {
            let pc = self.emulator.hybrid_cpu().pc();
            let bus = self.emulator.hybrid_bus();
            let ready = bus.cycles.m68k_wait_cpu_cycles == 0
                && self.emulator.hybrid_cpu().hybrid_instruction_ready(bus);
            if ready {
                self.trace_instruction(pc);
                if pc == self.symbols.intro {
                    self.measurements.intro_frame.get_or_insert(self.frame);
                } else if pc == self.symbols.title {
                    self.measurements.title_frame.get_or_insert(self.frame);
                } else if pc == self.symbols.new_game_menu {
                    self.measurements.menu_frame.get_or_insert(self.frame);
                } else if pc == self.symbols.main_loop {
                    self.measurements.game_frame.get_or_insert(self.frame);
                    self.measurements.logic_ticks += 1;
                    self.main_idle = false;
                } else if pc == self.symbols.main_wait {
                    self.main_idle = true;
                } else if pc == self.symbols.random {
                    let sp = self.emulator.hybrid_cpu().stack_pointer();
                    let return_pc = self.context().read_u32(sp);
                    if return_pc == self.symbols.idle_random_return {
                        idle_draws += 1;
                    }
                }
                if self.mode != Mode::Reference
                    && let Some(hook) = self.registry.at(pc).cloned()
                {
                    if self.mode == Mode::Check {
                        self.check_hook(&hook)?;
                    } else {
                        self.replace_hook(&hook)?;
                        inputs.0 = self.wait_clock.inputs;
                        continue;
                    }
                }
            }
            let previous_line = self.emulator.hybrid_bus().vdp.scanline();
            let effect = self.tick(&mut inputs)?;
            let vdp = &self.emulator.hybrid_bus().vdp;
            let blank_line = vdp
                .hybrid_registers()
                .vertical_display_size
                .active_scanlines();
            if previous_line != vdp.scanline()
                && vdp.scanline() == blank_line
                && self.measurements.game_frame.is_some()
                && !self.main_idle
            {
                let event = LagEvent {
                    frame: self.frame,
                    pc_at_vblank: pc,
                    frame_counter: self.context().read_u16(catalogue::FRAME_COUNTER),
                };
                self.trace_lag();
                self.measurements.lag_events.push(event.clone());
                lag_events.push(event);
            }
            if effect == TickEffect::FrameRendered {
                break;
            }
        }
        let snapshot = self.snapshot(idle_draws, lag_events)?;
        self.frame += 1;
        self.wait_clock.frame = self.frame;
        self.measurements.idle_draws_per_frame.push(idle_draws);
        for write in &snapshot.writes {
            match write {
                HardwareWrite::Vdp { .. } => self.measurements.vdp_writes += 1,
                HardwareWrite::Ym2612 { .. } => self.measurements.ym2612_writes += 1,
                HardwareWrite::Psg { .. } => self.measurements.psg_writes += 1,
            }
        }
        Ok(snapshot)
    }

    fn snapshot(
        &mut self,
        idle_random_draws: u64,
        lag_events: Vec<LagEvent>,
    ) -> Result<FrameSnapshot> {
        let frame_counter = self.context().read_u16(catalogue::FRAME_COUNTER);
        let bus = self.emulator.hybrid_bus_mut();
        let (ram, _) = bus.memory.debug_ram_view();
        let mut ram_bytes = Vec::with_capacity(65_536);
        ram_bytes.extend(ram.iter().flat_map(|word| word.to_be_bytes()));
        let ram = ram_bytes.into_boxed_slice();
        let (vram, cram, vsram) = bus.vdp.hybrid_memories();
        let vdp = VdpSnapshot {
            vram: vram.into(),
            cram: cram.into(),
            vsram: vsram.into(),
            registers: bincode::encode_to_vec(
                bus.vdp.hybrid_registers(),
                bincode::config::standard(),
            )
            .map_err(|e| HostError::Emulator(e.to_string()))?,
        };
        let writes = bus
            .hybrid_writes
            .as_mut()
            .expect("trace enabled")
            .drain(..)
            .map(Into::into)
            .collect();
        Ok(FrameSnapshot {
            frame: self.frame,
            frame_counter,
            pc: self.emulator.hybrid_cpu().pc(),
            ram,
            vdp,
            audio_samples: std::mem::replace(&mut self.audio.samples, Vec::with_capacity(1024)),
            writes,
            idle_random_draws,
            lag_events,
        })
    }

    fn fork(&self) -> GenesisEmulator {
        let mut fork = self.emulator.partial_clone();
        fork.hybrid_bus_mut().hybrid_writes = Some(Vec::new());
        fork.hybrid_bus_mut().hybrid_accesses = Some(Vec::new());
        fork
    }

    /// Move, don't copy, the read-only ROM into a sequentially executed fork.
    /// Always return it to the live machine, including when the closure errors.
    fn execute_fork<T>(
        &mut self,
        fork: &mut GenesisEmulator,
        execute: impl FnOnce(&mut GenesisEmulator) -> Result<T>,
    ) -> Result<T> {
        fork.hybrid_bus_mut()
            .cartridge
            .as_mut()
            .expect("standalone cartridge")
            .take_rom_from(
                self.emulator
                    .hybrid_bus_mut()
                    .cartridge
                    .as_mut()
                    .expect("cartridge"),
            );
        let result = execute(fork);
        self.emulator
            .hybrid_bus_mut()
            .cartridge
            .as_mut()
            .expect("cartridge")
            .take_rom_from(fork.hybrid_bus_mut().cartridge.as_mut().expect("cartridge"));
        result
    }

    fn check_hook(&mut self, hook: &Hook) -> Result<()> {
        // Do not copy this frame's accumulated event trace into either fork.
        let live_trace = self.emulator.hybrid_bus_mut().hybrid_writes.take();
        let result = self.check_hook_inner(hook);
        self.emulator.hybrid_bus_mut().hybrid_writes = live_trace;
        result
    }

    fn check_hook_inner(&mut self, hook: &Hook) -> Result<()> {
        let sp = self.emulator.hybrid_cpu().stack_pointer();
        let return_pc = self.context().read_u32(sp);
        let mut original = self.fork();
        let mut translated = self.fork();
        let mut original_clock = self.wait_clock.clone();
        let audit = self.execute_fork(&mut original, |emulator| {
            if hook.execution_class == ExecutionClass::Functional {
                emulator.hybrid_bus_mut().hybrid_set_functional_mode(true);
            }
            if hook.execution_class == ExecutionClass::TimeAware {
                let (audit, _) = run_original_timed(
                    emulator,
                    &mut original_clock,
                    return_pc,
                    sp.wrapping_add(4),
                )?;
                Ok(audit)
            } else {
                run_original_atomic(emulator, return_pc, sp.wrapping_add(4))
            }
        })?;
        self.measurements.check_original_wait_frames +=
            original_clock.frame - self.wait_clock.frame;
        let revision = self.revision;
        let dispatcher = self.registry.dispatcher();
        let translated_clock = self.wait_clock.clone();
        let (nested_calls, translated_writes, nested_reads, translated_frames) = self
            .execute_fork(&mut translated, |emulator| {
                if hook.execution_class == ExecutionClass::Functional {
                    emulator.hybrid_bus_mut().hybrid_set_functional_mode(true);
                }
                let mut context = HookContext::new(emulator, revision, dispatcher)
                    .with_execution(hook.execution_class, translated_clock);
                context.enable_stack_audit();
                let value = hook.execute(&mut context)?;
                context.return_to_caller(value);
                Ok((
                    context.nested_calls,
                    context.writes,
                    context.uninitialized_reads.into_inner(),
                    context.time_frames,
                ))
            })?;
        self.measurements.check_rust_wait_frames += translated_frames;
        self.checks.calls += 1;
        self.checks
            .record_function(hook.symbol.name, hook.symbol.subsystem, true);
        self.checks.nested_original_calls += nested_calls;
        let stack_dependency = !audit.uninitialized_reads.is_empty() || !nested_reads.is_empty();
        for UninitializedStackRead { pc, address } in
            audit.uninitialized_reads.into_iter().chain(nested_reads)
        {
            self.checks.uninitialized_stack_reads.insert(StackReadSite {
                function: hook.symbol.name.to_owned(),
                pc,
                address,
            });
        }
        let comparison = if stack_dependency {
            Err("uninitialized stack read: a stateful stack dependency cannot be approved as an atomic Rust function".into())
        } else { compare_calls_with_execution(&mut original, &mut translated, sp, &hook.symbol.abi, hook.execution_class) }
            .and_then(|dead_bytes| {
                let (entry_ram, _) = self.emulator.hybrid_bus_mut().memory.debug_ram_view();
                compare_io_ram_order(&audit.writes, &translated_writes, entry_ram, sp)?;
                Ok(dead_bytes)
            });
        match comparison {
            Ok(dead_bytes) => {
                self.checks.dead_stack_differing_bytes += dead_bytes;
                if dead_bytes != 0 {
                    self.checks.dead_stack_differing_calls += 1;
                }
                Ok(())
            }
            Err(detail) => {
                self.checks.mismatches += 1;
                self.checks
                    .by_function
                    .get_mut(hook.symbol.name)
                    .expect("registered hook")
                    .mismatches += 1;
                self.checks
                    .by_subsystem
                    .get_mut(hook.symbol.subsystem)
                    .expect("registered subsystem")
                    .mismatches += 1;
                Err(HostError::Mismatch {
                    frame: self.frame,
                    function: hook.symbol.name.to_owned(),
                    detail,
                })
            }
        }
    }

    fn replace_hook(&mut self, hook: &Hook) -> Result<()> {
        let mut context = HookContext::new(
            &mut self.emulator,
            self.revision,
            self.registry.dispatcher(),
        )
        .with_execution(hook.execution_class, self.wait_clock.clone());
        let value = hook.execute(&mut context)?;
        context.return_to_caller(value);
        self.checks.nested_original_calls += context.nested_calls;
        self.wait_clock = context.wait_clock;
        self.frame = self.wait_clock.frame;
        self.measurements.wait_video_frames += context.time_frames;
        self.audio.samples.extend(context.time_audio);
        self.checks
            .record_function(hook.symbol.name, hook.symbol.subsystem, false);
        Ok(())
    }

    /// ROM-backed function fixture. Does not advance hardware clocks. Useful for
    /// edge cases that the live game never generates (e.g. minimum signed seed).
    pub fn check_call(&mut self, address: u32, arguments: &[u32]) -> Result<u32> {
        self.check_call_inner(address, arguments, false)
    }

    fn check_call_inner(
        &mut self,
        address: u32,
        arguments: &[u32],
        isolated_fixture: bool,
    ) -> Result<u32> {
        let hook = self
            .registry
            .at(address)
            .cloned()
            .ok_or_else(|| HostError::Metadata(format!("no hook at {address:06X}")))?;
        if hook.execution_class != ExecutionClass::Atomic && !isolated_fixture {
            return Err(HostError::Metadata(
                "non-atomic diagnostics require check_fixture isolation".into(),
            ));
        }
        let saved_pc = self.emulator.hybrid_cpu().pc();
        let saved_sp = self.emulator.hybrid_cpu().stack_pointer();
        let mut sp = saved_sp;
        for &argument in arguments.iter().rev() {
            sp = sp.wrapping_sub(4);
            self.context().write_u32(sp, argument);
        }
        let cleanup_sp = sp;
        sp = sp.wrapping_sub(4);
        self.context().write_u32(sp, abi::CALL_SENTINEL);
        let (cpu, bus) = self.emulator.hybrid_parts();
        set_sp(cpu, sp);
        cpu.set_pc(address, bus);
        self.check_hook(&hook)?;
        match hook.execution_class {
            ExecutionClass::Atomic => {
                run_original_atomic(&mut self.emulator, abi::CALL_SENTINEL, cleanup_sp)?;
            }
            ExecutionClass::Functional => {
                self.emulator
                    .hybrid_bus_mut()
                    .hybrid_set_functional_mode(true);
                run_original_atomic(&mut self.emulator, abi::CALL_SENTINEL, cleanup_sp)?;
            }
            ExecutionClass::TimeAware => {
                run_original_timed(
                    &mut self.emulator,
                    &mut self.wait_clock.clone(),
                    abi::CALL_SENTINEL,
                    cleanup_sp,
                )?;
            }
        }
        let (cpu, bus) = self.emulator.hybrid_parts();
        let result = cpu.data_registers()[0];
        set_sp(cpu, saved_sp);
        cpu.set_pc(saved_pc, bus);
        Ok(result)
    }

    /// Differential fixture on a fork of the current live state. Restore CPU and
    /// every device/storage byte afterward, but retain the measured coverage.
    /// This prevents diagnostic writes from filling a frozen VDP FIFO across calls.
    pub fn check_fixture(&mut self, address: u32, arguments: &[u32]) -> Result<u32> {
        let mut saved = self.emulator.partial_clone();
        let result = self.check_call_inner(address, arguments, true);
        saved
            .hybrid_bus_mut()
            .cartridge
            .as_mut()
            .expect("cartridge")
            .take_rom_from(
                self.emulator
                    .hybrid_bus_mut()
                    .cartridge
                    .as_mut()
                    .expect("cartridge"),
            );
        self.emulator = saved;
        result
    }
}

#[cfg(test)]
fn compare_calls(
    original: &mut GenesisEmulator,
    translated: &mut GenesisEmulator,
    entry_sp: u32,
    abi: &tje_game::Abi,
) -> std::result::Result<u64, String> {
    compare_calls_with_execution(original, translated, entry_sp, abi, ExecutionClass::Atomic)
}

fn compare_calls_with_execution(
    original: &mut GenesisEmulator,
    translated: &mut GenesisEmulator,
    entry_sp: u32,
    abi: &tje_game::Abi,
    execution_class: ExecutionClass,
) -> std::result::Result<u64, String> {
    let (a, b) = (original.hybrid_cpu(), translated.hybrid_cpu());
    if a.pc() != b.pc() || a.stack_pointer() != b.stack_pointer() {
        return Err(format!(
            "return PC/SP: original={:08X}/{:08X}, Rust={:08X}/{:08X}",
            a.pc(),
            a.stack_pointer(),
            b.pc(),
            b.stack_pointer()
        ));
    }
    if a.user_stack_pointer() != b.user_stack_pointer()
        || a.supervisor_stack_pointer() != b.supervisor_stack_pointer()
    {
        return Err("banked USP/SSP state differs".into());
    }
    let (da, db) = (a.data_registers(), b.data_registers());
    if abi.returns_value && da[0] != db[0] {
        return Err(format!("D0: original={:08X}, Rust={:08X}", da[0], db[0]));
    }
    for index in 0..8 {
        if (abi.strict_registers || abi.preserved_data & (1 << index) != 0)
            && da[index] != db[index]
        {
            return Err(format!(
                "ABI-preserved D{index}: original={:08X}, Rust={:08X}",
                da[index], db[index]
            ));
        }
    }
    for index in 0..7 {
        if (abi.strict_registers || abi.preserved_address & (1 << index) != 0)
            && a.address_registers()[index] != b.address_registers()[index]
        {
            return Err(format!("ABI-preserved A{index} mismatch"));
        }
    }
    let sr_mask = if abi.strict_registers { 0xFFFF } else { 0xFFE0 };
    if a.status_register() & sr_mask != b.status_register() & sr_mask {
        return Err(format!(
            "SR control bits: original={:04X}, Rust={:04X}",
            a.status_register(),
            b.status_register()
        ));
    }
    let (a, b) = (original.hybrid_bus_mut(), translated.hybrid_bus_mut());
    let (ra, aa) = a.memory.debug_ram_view();
    let (rb, ab) = b.memory.debug_ram_view();
    let mut dead_bytes = 0;
    let dead_end = abi::canonical_ram_address(entry_sp).min(catalogue::STACK_TOP);
    for (index, (&wa, &wb)) in ra.iter().zip(rb.iter()).enumerate() {
        if wa == wb {
            continue;
        }
        let address = catalogue::STACK_BOTTOM + 2 * index as u32;
        for (offset, (ba, bb)) in wa
            .to_be_bytes()
            .into_iter()
            .zip(wb.to_be_bytes())
            .enumerate()
        {
            if ba == bb {
                continue;
            }
            let address = address + offset as u32;
            if address < dead_end {
                dead_bytes += 1;
            } else {
                return Err(format!(
                    "RAM {address:06X}: original={ba:02X}, Rust={bb:02X}"
                ));
            }
        }
    }
    if execution_class != ExecutionClass::TimeAware && aa != ab {
        return Err("Z80 audio RAM differs".into());
    }
    if a.z80_signals != b.z80_signals
        || (execution_class != ExecutionClass::TimeAware && a.z80_bank != b.z80_bank)
        || a.pending_writes != b.pending_writes
    {
        return Err("Z80 bus/reset/bank or pending CPU write state differs".into());
    }
    if execution_class == ExecutionClass::Functional
        && (a.hybrid_functional_violation.is_some() || b.hybrid_functional_violation.is_some())
    {
        return Err(format!(
            "functional contract observed VDP status/HV: original={:?}, Rust={:?}",
            a.hybrid_functional_violation, b.hybrid_functional_violation
        ));
    }
    if execution_class == ExecutionClass::TimeAware {
        // Comparison-only completion: execution above used exact timed devices.
        a.hybrid_set_functional_mode(true);
        b.hybrid_set_functional_mode(true);
    }
    let vdp_matches = if execution_class == ExecutionClass::TimeAware {
        a.vdp.hybrid_logic_state_eq(&b.vdp)
    } else {
        a.vdp.hybrid_state_eq(&b.vdp)
    };
    if !vdp_matches {
        return Err(
            "VDP state differs (ports/latches/FIFO/DMA/render state/memories/registers)".into(),
        );
    }
    let accesses_a = a.hybrid_accesses.as_deref().unwrap_or(&[]);
    let accesses_b = b.hybrid_accesses.as_deref().unwrap_or(&[]);
    if accesses_a.len() != accesses_b.len()
        || !accesses_a
            .iter()
            .zip(accesses_b)
            .all(|(a, b)| accesses_match(a, b))
    {
        let a = a.hybrid_accesses.as_deref().unwrap_or(&[]);
        let b = b.hybrid_accesses.as_deref().unwrap_or(&[]);
        let index = a
            .iter()
            .zip(b)
            .position(|(a, b)| !accesses_match(a, b))
            .unwrap_or(a.len().min(b.len()));
        return Err(format!(
            "ordered device read/write trace differs at {index}: original={:?}, Rust={:?}",
            a.get(index),
            b.get(index)
        ));
    }
    if a.hybrid_writes != b.hybrid_writes {
        return Err("VDP/YM2612/PSG write trace differs".into());
    }
    Ok(dead_bytes)
}

/// Compare only physically driven read bits; raw observations remain intact for
/// diagnostics. Unused open-bus bits are not a fabricated device output.
pub(crate) fn accesses_match(
    a: &genesis_core::HybridAccess,
    b: &genesis_core::HybridAccess,
) -> bool {
    use genesis_core::HybridAccess;
    match (*a, *b) {
        (
            HybridAccess::Read {
                address: aa,
                word: aw,
                value: av,
                ..
            },
            HybridAccess::Read {
                address: ba,
                word: bw,
                value: bv,
                ..
            },
        ) if aa == ba && aw == bw => {
            let mask = match aa & 0xffffff {
                0xc00004..=0xc00007 => {
                    if aw {
                        0x03ff
                    } else if aa & 1 == 0 {
                        0x03
                    } else {
                        0xff
                    }
                }
                0xa11100 => {
                    if aw {
                        0x0100
                    } else {
                        0x01
                    }
                }
                _ => {
                    if aw {
                        0xffff
                    } else {
                        0x00ff
                    }
                }
            };
            av & mask == bv & mask
        }
        (
            HybridAccess::Write {
                address: aa,
                word: aw,
                value: av,
            },
            HybridAccess::Write {
                address: ba,
                word: bw,
                value: bv,
            },
        ) => aa == ba && aw == bw && av == bv,
        _ => false,
    }
}

/// Compare RAM as observed at every device write, not merely after return. Sparse
/// overlays avoid copying 64 KiB at every port access and tolerate redundant stores.
fn compare_io_ram_order(
    original: &[OrderedWrite],
    translated: &[OrderedWrite],
    entry_ram: &[u16],
    entry_sp: u32,
) -> std::result::Result<(), String> {
    fn next_device<'a>(
        events: &mut impl Iterator<Item = &'a OrderedWrite>,
        overlay: &mut BTreeMap<u32, u8>,
        entry_sp: u32,
    ) -> Option<&'a OrderedWrite> {
        for event in events {
            if let OrderedWrite::Ram { address, value } = *event {
                if address >= abi::canonical_ram_address(entry_sp).min(catalogue::STACK_TOP) {
                    overlay.insert(address, value);
                }
            } else {
                return Some(event);
            }
        }
        None
    }
    let (mut a, mut b) = (original.iter(), translated.iter());
    let (mut ma, mut mb) = (BTreeMap::new(), BTreeMap::new());
    loop {
        let (ea, eb) = (
            next_device(&mut a, &mut ma, entry_sp),
            next_device(&mut b, &mut mb, entry_sp),
        );
        if ea.is_none() && eb.is_none() {
            return Ok(());
        }
        if ea != eb {
            return Err("RAM/device ordered write trace differs".into());
        }
        for address in ma.keys().chain(mb.keys()) {
            let offset = (address & 0xffff) as usize;
            let initial = entry_ram[offset / 2].to_be_bytes()[offset % 2];
            if ma.get(address).copied().unwrap_or(initial)
                != mb.get(address).copied().unwrap_or(initial)
            {
                return Err(format!("RAM {address:06X} differs at device access {ea:?}"));
            }
        }
    }
}

#[cfg(test)]
mod checker_regressions {
    use super::*;
    use genesis_core::GenesisBus;
    use jgenesis_common::{debug::DebugMemoryView, frontend::TimingMode};
    use m68000_emu::BusInterface;

    #[test]
    fn immutable_vdp_comparison_observes_memories_registers_and_command_latch() {
        let bus = GenesisBus::new(TimingMode::Ntsc, None, &GenesisEmulatorConfig::default());
        let mutations: [fn(&mut GenesisBus); 4] = [
            |bus| bus.vdp.debug_vram_view().write(10, 1),
            |bus| bus.vdp.debug_cram_view().write(1, 2),
            |bus| bus.vdp.debug_vsram_view().write(1, 2),
            |bus| bus.vdp.write_control(0x8701),
        ];
        for mutate in mutations {
            let mut changed = bus.partial_clone();
            mutate(&mut changed);
            assert!(!bus.vdp.hybrid_state_eq(&changed.vdp));
        }
        let mut original = bus.partial_clone();
        original.vdp.write_control(0x4000);
        let mut changed = original.partial_clone();
        changed.vdp.read_status(0, 7);
        assert!(!original.vdp.hybrid_state_eq(&changed.vdp));
        let mut pending = bus.partial_clone();
        pending.write_word(0xc00004, 0x8701);
        assert_ne!(bus.pending_writes, pending.pending_writes);
    }

    #[test]
    fn z80_audio_ram_corruption_is_not_masked_as_dead_68000_stack() {
        let mut original = GenesisEmulator::create(
            None,
            GenesisEmulatorConfig::default(),
            &mut NoCartridgeSaves,
        );
        let mut changed = original.partial_clone();
        changed.hybrid_bus_mut().memory.debug_ram_view().1[0] ^= 1;
        let symbol = tje_game::ids::GET_RANDOM_NUMBER.metadata();
        let error = compare_calls(&mut original, &mut changed, 0xff7ffc, &symbol.abi).unwrap_err();
        assert_eq!(error, "Z80 audio RAM differs");
    }

    #[test]
    fn ram_io_timeline_compares_intermediate_not_only_final_values() {
        let ram = [0_u16; 32768];
        let device = OrderedWrite::DeviceWord {
            address: 0xc00000,
            value: 1,
        };
        let mirror = OrderedWrite::Ram {
            address: 0xff8000,
            value: 1,
        };
        assert!(
            compare_io_ram_order(
                &[device.clone(), mirror.clone()],
                &[device.clone(), mirror.clone()],
                &ram,
                0xff7ffc
            )
            .is_ok()
        );
        assert!(
            compare_io_ram_order(
                &[device.clone(), mirror.clone()],
                &[mirror, device],
                &ram,
                0xff7ffc
            )
            .is_err()
        );
    }
}
