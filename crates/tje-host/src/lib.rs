//! Headless REV 02 reference and hybrid host built on jgenesis.
//!
//! `Check` compares atomic function contracts on independent state forks, with
//! interrupts/peripheral clocks frozen, then lets the live ROM execute with its
//! original instruction timing. This preserves timing-dependent random draws and
//! lag measurements. `Replace` runs Rust instead, but is NOT cycle-equivalent.
//! Dead stack bytes below the entry SP are informational, not native game state;
//! reads of those bytes before a write are explicitly audited on the original.

pub mod abi;
pub mod catalogue;
pub mod hooks;
pub mod input;
mod sinks;

use abi::{HookContext, UninitializedStackRead, run_original_atomic, set_sp};
use genesis_config::{GenesisEmulatorConfig, GenesisInputs};
use genesis_core::{GenesisEmulator, HybridWrite};
use hooks::{Hook, HookRegistry};
use jgenesis_common::frontend::{Color, EmulatorTrait, FrameSize, PartialClone, TickEffect};
use serde::Serialize;
use sinks::{HeadlessAudio, HeadlessOutput, Inputs, NoCartridgeSaves};
use std::collections::{BTreeMap, BTreeSet};
use tje_rom::{Revision, Rom};

pub type Result<T, E = HostError> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error("ROM import failed: {0}")]
    Rom(#[from] tje_rom::ImportError),
    #[error("this host's hook/measurement metadata is REV 02 only")]
    WrongRevision,
    #[error("invalid metadata: {0}")]
    Metadata(String),
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
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct StackReadSite {
    pub function: String,
    pub pc: u32,
    pub address: u32,
}

#[derive(Debug, Default, Serialize)]
pub struct CheckReport {
    pub calls: u64,
    pub by_function: BTreeMap<String, u64>,
    pub mismatches: u64,
    pub nested_original_calls: u64,
    pub dead_stack_differing_calls: u64,
    pub dead_stack_differing_bytes: u64,
    pub uninitialized_stack_reads: BTreeSet<StackReadSite>,
}

impl CheckReport {
    fn record_function(&mut self, name: &str) {
        if let Some(count) = self.by_function.get_mut(name) {
            *count += 1;
        } else {
            self.by_function.insert(name.to_owned(), 1);
        }
    }
}

pub struct Host {
    emulator: GenesisEmulator,
    registry: HookRegistry,
    mode: Mode,
    frame: u64,
    main_idle: bool,
    output: HeadlessOutput,
    audio: HeadlessAudio,
    saves: NoCartridgeSaves,
    pub checks: CheckReport,
    pub measurements: Measurements,
}

impl Host {
    pub fn new(bytes: &[u8], mode: Mode, registry: HookRegistry) -> Result<Self> {
        let rom = Rom::import(bytes)?;
        if rom.revision() != Revision::Rev02 {
            return Err(HostError::WrongRevision);
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
        Ok(Self {
            emulator,
            registry,
            mode,
            frame: 0,
            main_idle: false,
            output: HeadlessOutput,
            audio: HeadlessAudio::default(),
            saves,
            checks: CheckReport::default(),
            measurements: Measurements::default(),
        })
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }
    pub fn frame_count(&self) -> u64 {
        self.frame
    }
    pub fn frame_buffer(&self) -> (&[Color], FrameSize) {
        let vdp = &self.emulator.hybrid_bus().vdp;
        (vdp.frame_buffer(), vdp.frame_size())
    }
    pub fn cpu(&self) -> &m68000_emu::M68000 {
        self.emulator.hybrid_cpu()
    }
    pub fn context(&mut self) -> HookContext<'_> {
        HookContext::new(&mut self.emulator)
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
        let mut inputs = Inputs(inputs);
        let mut idle_draws = 0;
        let mut lag_events = Vec::new();
        loop {
            let pc = self.emulator.hybrid_cpu().pc();
            let bus = self.emulator.hybrid_bus();
            let ready = bus.cycles.m68k_wait_cpu_cycles == 0
                && self.emulator.hybrid_cpu().hybrid_instruction_ready(bus);
            if ready {
                match pc {
                    catalogue::INTRO => {
                        self.measurements.intro_frame.get_or_insert(self.frame);
                    }
                    0x027A10 => {
                        self.measurements.title_frame.get_or_insert(self.frame);
                    }
                    catalogue::NEW_GAME_MENU => {
                        self.measurements.menu_frame.get_or_insert(self.frame);
                    }
                    catalogue::MAIN_LOOP => {
                        self.measurements.game_frame.get_or_insert(self.frame);
                        self.measurements.logic_ticks += 1;
                        self.main_idle = false;
                    }
                    catalogue::MAIN_WAIT => {
                        self.main_idle = true;
                    }
                    catalogue::RANDOM => {
                        let sp = self.emulator.hybrid_cpu().stack_pointer();
                        let return_pc = self.context().read_u32(sp);
                        if return_pc == catalogue::IDLE_RANDOM_RETURN {
                            idle_draws += 1;
                        }
                    }
                    _ => {}
                }
                if self.mode != Mode::Reference
                    && let Some(hook) = self.registry.at(pc).cloned()
                {
                    if self.mode == Mode::Check {
                        self.check_hook(&hook)?;
                    } else {
                        self.replace_hook(&hook)?;
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
                self.measurements.lag_events.push(event.clone());
                lag_events.push(event);
            }
            if effect == TickEffect::FrameRendered {
                break;
            }
        }
        let snapshot = self.snapshot(idle_draws, lag_events)?;
        self.frame += 1;
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
        let audit = self.execute_fork(&mut original, |emulator| {
            run_original_atomic(emulator, return_pc, sp.wrapping_add(4))
        })?;
        let nested_calls = self.execute_fork(&mut translated, |emulator| {
            let mut context = HookContext::new(emulator);
            let value = (hook.function)(&mut context)?;
            context.return_to_caller(value);
            Ok(context.nested_calls)
        })?;
        self.checks.calls += 1;
        self.checks.record_function(&hook.symbol.name);
        self.checks.nested_original_calls += nested_calls;
        for UninitializedStackRead { pc, address } in audit.uninitialized_reads {
            self.checks.uninitialized_stack_reads.insert(StackReadSite {
                function: hook.symbol.name.clone(),
                pc,
                address,
            });
        }
        let comparison = compare_calls(&mut original, &mut translated, sp, hook.returns_value);
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
                Err(HostError::Mismatch {
                    frame: self.frame,
                    function: hook.symbol.name.clone(),
                    detail,
                })
            }
        }
    }

    fn replace_hook(&mut self, hook: &Hook) -> Result<()> {
        let mut context = HookContext::new(&mut self.emulator);
        let value = (hook.function)(&mut context)?;
        context.return_to_caller(value);
        self.checks.nested_original_calls += context.nested_calls;
        self.checks.record_function(&hook.symbol.name);
        Ok(())
    }

    /// ROM-backed function fixture. Does not advance hardware clocks. Useful for
    /// edge cases that the live game never generates (e.g. minimum signed seed).
    pub fn check_call(&mut self, address: u32, arguments: &[u32]) -> Result<u32> {
        let hook = self
            .registry
            .at(address)
            .cloned()
            .ok_or_else(|| HostError::Metadata(format!("no hook at {address:06X}")))?;
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
        run_original_atomic(&mut self.emulator, abi::CALL_SENTINEL, cleanup_sp)?;
        let (cpu, bus) = self.emulator.hybrid_parts();
        let result = cpu.data_registers()[0];
        set_sp(cpu, saved_sp);
        cpu.set_pc(saved_pc, bus);
        Ok(result)
    }
}

fn compare_calls(
    original: &mut GenesisEmulator,
    translated: &mut GenesisEmulator,
    entry_sp: u32,
    returns_value: bool,
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
    let (da, db) = (a.data_registers(), b.data_registers());
    if returns_value && da[0] != db[0] {
        return Err(format!("D0: original={:08X}, Rust={:08X}", da[0], db[0]));
    }
    if da[2..] != db[2..] || a.address_registers()[2..] != b.address_registers()[2..] {
        return Err("MCC68K callee-saved register mismatch (D2-D7/A2-A6)".into());
    }
    let (a, b) = (original.hybrid_bus_mut(), translated.hybrid_bus_mut());
    let (ra, aa) = a.memory.debug_ram_view();
    let (rb, ab) = b.memory.debug_ram_view();
    let mut dead_bytes = 0;
    let dead_end = (entry_sp & 0xFF_FFFF).min(catalogue::STACK_TOP);
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
    if aa != ab {
        return Err("Z80 audio RAM differs".into());
    }
    if a.vdp.hybrid_memories() != b.vdp.hybrid_memories() {
        return Err("VDP VRAM/CRAM/VSRAM differs".into());
    }
    if a.vdp.hybrid_registers() != b.vdp.hybrid_registers() {
        return Err("VDP registers differ".into());
    }
    if a.hybrid_writes != b.hybrid_writes {
        return Err("VDP/YM2612/PSG write trace differs".into());
    }
    Ok(dead_bytes)
}
