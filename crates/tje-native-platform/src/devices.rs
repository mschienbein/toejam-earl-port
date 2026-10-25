//! Retained Genesis devices. No 68000 object, decoder, or original-call fallback.

use genesis_components::{
    GenesisEmulatorConfigExt,
    cartridge::GenesisRegionExt,
    vdp::{self, Vdp, VdpBusView, VdpTickEffect},
    ym2612::Ym2612,
};
use genesis_config::{GenesisEmulatorConfig, GenesisInputs, GenesisRegion};
use genesis_core::{
    audio::{GenesisAudioOutput, GenesisAudioResampler},
    input::InputState,
    native::PeripheralState,
    timing::CycleCounters,
};
use jgenesis_common::frontend::{AudioOutput, Color, FrameSize, TimingMode};
use ti_sn76489::{Sn76489, Sn76489TickEffect, Sn76489Version};
use tje_mem::{ADDRESS_MASK, AddressSpace, Bus, RAM_SIZE};
use z80_emu::{BusInterface, Z80, debug::DummyZ80Debugger, traits::InterruptLine};

#[cfg(test)]
mod z80_capacity_tests;

#[cfg(test)]
mod discard_tests;

pub use genesis_core::native::RamWrite;
pub use genesis_core::native::{DeviceWrite, Origin, TimedWrite};

/// Bus-read classification independent of any 68000 interpreter dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BusReadPurpose {
    Semantic = 0,
    Prefetch = 1,
    Discarded = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceAccess {
    pub clock: u64,
    pub origin: Origin,
    pub pc: u32,
    pub address: u32,
    pub word: bool,
    pub write: bool,
    pub value: u16,
    /// Stable CPU issue tags: semantic=0, prefetch=1, discarded=2; writes None.
    pub purpose: Option<u8>,
}

/// Disabled by default; allocating observations are confined to explicit proofs.
#[derive(Debug, Clone, Default)]
pub struct DeviceTrace {
    pub accesses: Vec<DeviceAccess>,
    pub writes: Vec<TimedWrite>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceStep {
    pub cpu_cycles: u32,
    pub master_clocks: u64,
    pub frame_complete: bool,
}

/// Instruction continuations depend on this cycle-driven boundary, not a frame
/// callback or wall clock. IRQ acceptance and exception semantics belong to the
/// caller; this interface exposes the actual device interrupt lines and stalls.
pub trait ClockedDevices {
    fn begin_instruction(&mut self, opcode: u16, read_offset_mclk: u64);
    fn read_bus_byte(&mut self, address: u32) -> u8;
    fn read_bus_word(&mut self, address: u32) -> u16;
    fn write_bus_byte(&mut self, address: u32, value: u8);
    fn write_bus_word(&mut self, address: u32, value: u16);
    fn finish_instruction(&mut self, pc: u32, cpu_cycles: u32) -> DeviceStep;
    fn take_wait_step(&mut self, pc: u32) -> Option<DeviceStep>;
    fn cpu_halted(&self) -> bool;
    fn pending_interrupt(&self) -> u8;
    fn acknowledge_interrupt(&mut self);
}

/// Register storage supplied only when capturing a complete native call state.
/// These are values, not an instruction execution engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Registers {
    pub data: [u32; 8],
    pub address: [u32; 8],
    pub status: u16,
    pub program_counter: u32,
    pub user_stack_pointer: u32,
    pub supervisor_stack_pointer: u32,
}

impl Registers {
    pub fn from_state(state: &PeripheralState) -> Self {
        Self {
            data: state.data,
            address: state.address,
            status: state.status,
            program_counter: state.program_counter,
            user_stack_pointer: state.user_stack_pointer,
            supervisor_stack_pointer: state.supervisor_stack_pointer,
        }
    }
}

struct DmaMemory<'a> {
    memory: &'a AddressSpace,
    open_bus: &'a mut u16,
}

impl VdpBusView for DmaMemory<'_> {
    fn read_word_for_dma(&mut self, address: u32) -> u16 {
        if address >= 0xe00000 {
            self.memory.read_u16(address & !1)
        } else if let Some(bytes) = self.memory.rom().get((address & !1) as usize..)
            && let Some(word) = bytes.get(..2)
        {
            *self.open_bus = u16::from_be_bytes([word[0], word[1]]);
            *self.open_bus
        } else {
            *self.open_bus
        }
    }
}

struct DeviceBus {
    memory: AddressSpace,
    audio_ram: Box<[u8; 0x2000]>,
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
    trace: Option<DeviceTrace>,
    ram_writes: Option<Vec<RamWrite>>,
    #[cfg(feature = "game-adapter")]
    ram_dma_observer: genesis_core::ram_dma_observer::ObserverSlot,
}

impl DeviceBus {
    fn busack(&self) -> bool {
        self.bus_request && !self.reset
    }

    fn trace_clock(&self) -> u64 {
        match self.origin {
            Origin::Main => self.cycles.m68k_mclk_cycles,
            Origin::Z80 => self.cycles.z80_mclk_cycles,
        }
    }

    fn record_access(
        &mut self,
        address: u32,
        word: bool,
        write: bool,
        value: u16,
        purpose: Option<u8>,
    ) {
        let clock = self.trace_clock();
        let pc = self.trace_pc();
        if (0x800000..0xe00000).contains(&address)
            && let Some(trace) = &mut self.trace
        {
            trace.accesses.push(DeviceAccess {
                clock,
                origin: self.origin,
                pc,
                address,
                word,
                write,
                value,
                purpose,
            });
        }
    }

    fn record_write(&mut self, write: DeviceWrite) {
        let clock = self.trace_clock();
        let pc = self.trace_pc();
        if let Some(trace) = &mut self.trace {
            trace.writes.push(TimedWrite {
                clock,
                origin: self.origin,
                pc,
                write,
            });
        }
    }

    fn trace_pc(&self) -> u32 {
        match self.origin {
            Origin::Main => self.main_pc,
            Origin::Z80 => self.audio_pc,
        }
    }

    fn sync_ym2612(&mut self) {
        if self.cycles.has_ym2612_ticks() {
            self.ym2612.tick(self.cycles.take_ym2612_ticks());
        }
    }

    fn read_vdp<const WORD: bool>(&mut self, address: u32) -> u16 {
        let word = match address & 0x1f {
            0..=3 => self.vdp.read_data(),
            4..=7 => self.vdp.native_read_status_at(self.read_offset) | (self.open_bus & 0xfc00),
            8..=11 => self.vdp.native_hv_counter_at(self.read_offset),
            _ => self.open_bus,
        };
        if WORD {
            word
        } else {
            u16::from(word.to_be_bytes()[(address & 1) as usize])
        }
    }

    fn read_io(&self, address: u32) -> u8 {
        match address | 1 {
            0xa10001 => {
                (u8::from(self.region.version_bit()) << 7)
                    | (u8::from(self.timing_mode == TimingMode::Pal) << 6)
                    | 0x20
            }
            0xa10003 => self.input.read_p1_data(),
            0xa10005 => self.input.read_p2_data(),
            0xa10007 => self.input.read_ext_data(),
            0xa10009 => self.input.read_p1_ctrl(),
            0xa1000b => self.input.read_p2_ctrl(),
            0xa1000d => self.input.read_ext_ctrl(),
            0xa1000f => self.input.read_p1_tx_data(),
            0xa10015 => self.input.read_p2_tx_data(),
            0xa1001b => self.input.read_ext_tx_data(),
            _ => 0,
        }
    }

    fn write_io(&mut self, address: u32, value: u8) {
        match address | 1 {
            0xa10003 => self.input.write_p1_data(value),
            0xa10005 => self.input.write_p2_data(value),
            0xa10007 => self.input.write_ext_data(value),
            0xa10009 => self.input.write_p1_ctrl(value),
            0xa1000b => self.input.write_p2_ctrl(value),
            0xa1000d => self.input.write_ext_ctrl(value),
            0xa1000f => self.input.write_p1_tx_data(value),
            0xa10015 => self.input.write_p2_tx_data(value),
            0xa1001b => self.input.write_ext_tx_data(value),
            _ => {}
        }
    }

    fn read_main<const WORD: bool>(&mut self, address: u32, purpose: BusReadPurpose) -> u16 {
        let address = address & ADDRESS_MASK;
        #[cfg(feature = "game-adapter")]
        if address >= 0xe00000 && self.origin == Origin::Main && purpose == BusReadPurpose::Semantic
        {
            self.ram_dma_observer
                .before_semantic_ram_read(address, WORD);
        }
        let value = match address {
            0..=0x7fffff => {
                if WORD {
                    self.memory
                        .rom()
                        .get((address & !1) as usize..)
                        .and_then(|bytes| bytes.get(..2))
                        .map_or(self.open_bus, |bytes| {
                            u16::from_be_bytes([bytes[0], bytes[1]])
                        })
                } else {
                    self.memory
                        .rom()
                        .get(address as usize)
                        .copied()
                        .unwrap_or(self.open_bus.to_be_bytes()[(address & 1) as usize])
                        .into()
                }
            }
            0xa00000..=0xa0ffff if self.busack() => {
                self.cycles.record_68k_z80_bus_access();
                let byte = self.read_memory((address & 0x7fff) as u16);
                if WORD {
                    u16::from_be_bytes([byte; 2])
                } else {
                    byte.into()
                }
            }
            0xa10000..=0xa1001f => {
                let byte = DeviceBus::read_io(self, address);
                if WORD {
                    u16::from_be_bytes([byte; 2])
                } else {
                    byte.into()
                }
            }
            0xa11100 => {
                let word = (u16::from(!self.busack()) << 8) | (self.open_bus & !0x100);
                if WORD {
                    word
                } else {
                    word.to_be_bytes()[0].into()
                }
            }
            0xc00000..=0xc0001f => self.read_vdp::<WORD>(address),
            0xe00000..=0xffffff => {
                if WORD {
                    self.memory.read_u16(address & !1)
                } else {
                    self.memory.read_u8(address).into()
                }
            }
            _ => {
                if WORD {
                    self.open_bus
                } else {
                    self.open_bus.to_be_bytes()[(address & 1) as usize].into()
                }
            }
        };
        self.record_access(address, WORD, false, value, Some(purpose as u8));
        if WORD {
            self.open_bus = value;
        } else {
            self.open_bus = (value << 8) | (self.open_bus & 0xff);
        }
        value
    }

    fn write_vdp_psg<const WORD: bool>(&mut self, address: u32, value: u16) {
        let word = if WORD {
            value
        } else {
            u16::from_be_bytes([value as u8; 2])
        };
        match address & 0x1f {
            0..=3 => {
                self.record_write(DeviceWrite::Vdp {
                    port: 0,
                    value: word,
                });
                self.vdp.write_data(word);
            }
            4..=7 => {
                self.record_write(DeviceWrite::Vdp {
                    port: 4,
                    value: word,
                });
                #[cfg(feature = "game-adapter")]
                {
                    let actor = match self.origin {
                        Origin::Main => genesis_core::ram_dma_observer::Actor::M68k,
                        Origin::Z80 => genesis_core::ram_dma_observer::Actor::Z80,
                    };
                    self.vdp
                        .write_control_observed(word, self.ram_dma_observer.command_origin(actor));
                }
                #[cfg(not(feature = "game-adapter"))]
                self.vdp.write_control(word);
            }
            0x10..=0x17 if WORD || address & 1 != 0 => {
                self.record_write(DeviceWrite::Psg { value: value as u8 });
                self.psg.write(value as u8);
            }
            0x1c..=0x1d => {
                self.record_write(DeviceWrite::Vdp {
                    port: 0x1c,
                    value: word,
                });
                self.vdp.write_debug_register(word);
            }
            _ => {}
        }
    }

    fn write_ym(&mut self, address: u16, value: u8) {
        self.sync_ym2612();
        self.record_write(DeviceWrite::Ym2612 {
            port: (address & 3) as u8,
            value,
        });
        match address & 3 {
            0 => self.ym2612.write_address_1(value),
            2 => self.ym2612.write_address_2(value),
            _ => self.ym2612.write_data(value),
        }
    }

    fn apply_main<const WORD: bool>(&mut self, address: u32, value: u16) {
        let address = address & ADDRESS_MASK;
        self.record_access(address, WORD, true, value, None);
        match address {
            0xa00000..=0xa0ffff if self.busack() => {
                self.cycles.record_68k_z80_bus_access();
                let byte = if WORD {
                    value.to_be_bytes()[0]
                } else {
                    value as u8
                };
                self.write_memory((address & 0x7fff) as u16, byte);
            }
            0xa10000..=0xa1001f => self.write_io(address, value as u8),
            0xa11100 => self.bus_request = value & if WORD { 0x100 } else { 1 } != 0,
            0xa11200 => {
                let reset = value & if WORD { 0x100 } else { 1 } == 0;
                if !self.reset && reset {
                    self.ym2612.reset();
                }
                self.reset = reset;
            }
            0xc00000..=0xc0001f => self.write_vdp_psg::<WORD>(address, value),
            0xe00000..=0xffffff => {
                if WORD {
                    self.memory.write_u16(address & !1, value);
                } else {
                    self.memory.write_u8(address, value as u8);
                }
                #[cfg(feature = "game-adapter")]
                self.ram_dma_observer.ram_committed(
                    address,
                    WORD,
                    match self.origin {
                        Origin::Main => genesis_core::ram_dma_observer::Actor::M68k,
                        Origin::Z80 => genesis_core::ram_dma_observer::Actor::Z80,
                    },
                );
                if let Some(trace) = &mut self.ram_writes {
                    trace.push(RamWrite::new(address, WORD, value));
                }
            }
            _ => {}
        }
    }

    fn apply_pending(&mut self) {
        let mut bytes = std::mem::take(&mut self.pending_bytes);
        let mut words = std::mem::take(&mut self.pending_words);
        for &(address, value) in &bytes {
            self.apply_main::<false>(address, value.into());
        }
        for &(address, value) in &words {
            self.apply_main::<true>(address, value);
        }
        bytes.clear();
        words.clear();
        self.pending_bytes = bytes;
        self.pending_words = words;
    }
}

impl BusInterface for DeviceBus {
    type DebugView<'a>
        = DummyZ80Debugger
    where
        Self: 'a;

    fn read_memory(&mut self, address: u16) -> u8 {
        match address {
            0..=0x3fff => self.audio_ram[(address & 0x1fff) as usize],
            0x4000..=0x5fff => {
                self.sync_ym2612();
                self.ym2612.read_register(address)
            }
            0x7f00..=0x7f1f => {
                self.cycles.record_z80_68k_bus_access();
                self.read_vdp::<false>(address.into()) as u8
            }
            0x8000..=0xffff => {
                self.cycles.record_z80_68k_bus_access();
                let main = (self.bank << 15) | u32::from(address & 0x7fff);
                match main {
                    0xa00000..=0xa0ffff | 0xe00000..=0xffffff => 0xff,
                    _ => self.read_main::<false>(main, BusReadPurpose::Semantic) as u8,
                }
            }
            _ => 0xff,
        }
    }

    fn write_memory(&mut self, address: u16, value: u8) {
        match address {
            0..=0x3fff => self.audio_ram[(address & 0x1fff) as usize] = value,
            0x4000..=0x5fff => self.write_ym(address, value),
            0x6000..=0x60ff => self.bank = (self.bank >> 1) | (u32::from(value & 1) << 8),
            0x7f00..=0x7f1f => {
                self.cycles.record_z80_68k_bus_access();
                self.write_vdp_psg::<false>(address.into(), value.into());
            }
            0x8000..=0xffff => {
                self.cycles.record_z80_68k_bus_access();
                let main = (self.bank << 15) | u32::from(address & 0x7fff);
                if !(0xa00000..=0xa0ffff).contains(&main) {
                    self.apply_main::<false>(main, value.into());
                }
            }
            _ => {}
        }
    }

    fn read_io(&mut self, _: u16) -> u8 {
        0xff
    }
    fn write_io(&mut self, _: u16, _: u8) {}
    fn nmi(&self) -> InterruptLine {
        InterruptLine::High
    }
    fn int(&self) -> InterruptLine {
        self.vdp.z80_interrupt_line()
    }
    fn busreq(&self) -> bool {
        self.bus_request
    }
    fn reset(&self) -> bool {
        self.reset
    }
}

#[derive(Default)]
struct AudioBuffer(Vec<[f64; 2]>);
impl AudioOutput for AudioBuffer {
    type Err = std::convert::Infallible;
    fn push_sample(&mut self, left: f64, right: f64) -> Result<(), Self::Err> {
        self.0.push([left, right]);
        Ok(())
    }
}

/// Console devices clocked by exact instruction costs. Device memory uses
/// `tje_mem::AddressSpace`; no second work-RAM image can drift out of sync.
pub struct DeviceRuntime {
    bus: DeviceBus,
    z80: Z80,
    audio_resampler: GenesisAudioResampler,
    instruction_open: bool,
    frames: u64,
    main_reset: bool,
    audio: AudioBuffer,
    input_clock_remainder: u64,
    audio_sample_rate: u32,
    config: GenesisEmulatorConfig,
}

impl DeviceRuntime {
    pub fn new(memory: AddressSpace, config: &GenesisEmulatorConfig) -> Self {
        let region = config.forced_region.unwrap_or(GenesisRegion::Americas);
        let timing_mode = config
            .forced_timing_mode
            .unwrap_or(if region == GenesisRegion::Europe {
                TimingMode::Pal
            } else {
                TimingMode::Ntsc
            });
        let mut devices = Self {
            bus: DeviceBus {
                memory,
                audio_ram: Box::new([0; 0x2000]),
                vdp: Vdp::new(timing_mode, config.to_vdp_config(false)),
                ym2612: Ym2612::new(config),
                psg: Sn76489::new(Sn76489Version::Standard, config.psg_channels_enabled),
                input: InputState::new(config, false),
                cycles: CycleCounters::new(config.clamped_m68k_divider()),
                timing_mode,
                region,
                bank: 0,
                bus_request: false,
                reset: true,
                open_bus: 0,
                read_offset: 0,
                origin: Origin::Main,
                main_pc: 0,
                audio_pc: 0,
                opcode: 0,
                pending_bytes: Vec::with_capacity(20),
                pending_words: Vec::with_capacity(32),
                trace: None,
                ram_writes: None,
                #[cfg(feature = "game-adapter")]
                ram_dma_observer: Default::default(),
            },
            z80: Z80::new(),
            audio_resampler: GenesisAudioResampler::new(timing_mode, config),
            instruction_open: false,
            frames: 0,
            input_clock_remainder: 0,
            main_reset: false,
            audio: AudioBuffer(Vec::with_capacity(1024)),
            audio_sample_rate: 48_000,
            config: config.clone(),
        };
        devices.reserve_startup_buffers();
        devices
    }

    pub fn from_state(mut memory: AddressSpace, state: PeripheralState) -> Result<Self, String> {
        if state.ram.len() != RAM_SIZE || state.audio_ram.len() != 0x2000 {
            return Err("native capture has invalid RAM lengths".into());
        }
        memory.ram_mut().copy_from_slice(&state.ram);
        let audio_ram = state
            .audio_ram
            .into_boxed_slice()
            .try_into()
            .map_err(|_| "invalid Z80 RAM length")?;
        let mut devices = Self {
            bus: DeviceBus {
                memory,
                audio_ram,
                vdp: state.vdp,
                ym2612: state.ym2612,
                psg: state.psg,
                input: state.input,
                cycles: state.cycles,
                timing_mode: state.timing_mode,
                region: state.region,
                bank: state.z80_bank,
                bus_request: state.bus_request,
                reset: state.reset,
                open_bus: state.open_bus,
                read_offset: 0,
                origin: Origin::Main,
                main_pc: state.program_counter,
                audio_pc: u32::from(state.z80.pc()),
                opcode: 0,
                pending_bytes: Vec::with_capacity(20),
                pending_words: Vec::with_capacity(32),
                trace: None,
                ram_writes: None,
                #[cfg(feature = "game-adapter")]
                ram_dma_observer: Default::default(),
            },
            z80: state.z80,
            audio_resampler: state.audio_resampler,
            instruction_open: false,
            frames: 0,
            main_reset: false,
            audio: AudioBuffer(Vec::with_capacity(1024)),
            audio_sample_rate: 48_000,
            config: state.config,
            input_clock_remainder: state.native_input_remainder,
        };
        devices.reserve_startup_buffers();
        Ok(devices)
    }

    fn reserve_startup_buffers(&mut self) {
        // One Z80 tick executes a whole instruction. DD/FD prefix fetches can
        // cross all 32 KiB of banked memory before wrapping, but cannot repeat:
        // the unmapped $6000 read is always $FF (not a prefix). The VDP window
        // has only 32 charged bytes before the $7F20 $FF guard. A non-prefix
        // body uses at most six accesses (ED absolute word transfers), plus
        // three trace-mode lookaheads. Real-core opcode/IRQ tests live in
        // z80_capacity_tests; indexed CB/CALL/EX(SP) use at most five.
        const MAXIMUM_Z80_BANK_ACCESSES: u64 = 0x8000 + 6 + 3;
        const MAXIMUM_Z80_PHASE_MCLK: u64 =
            (3 * MAXIMUM_Z80_BANK_ACCESSES + 1) * genesis_core::timing::Z80_DIVIDER;
        const MAXIMUM_AUDIO_BATCH_MCLK: u64 = genesis_core::timing::MAX_YM2612_LAG_MCLK
            + vdp::MCLK_CYCLES_PER_TICK_LIMIT
            + MAXIMUM_Z80_PHASE_MCLK
            + genesis_core::timing::YM2612_DIVIDER;
        self.bus.ym2612.native_reserve_output_ticks(
            MAXIMUM_AUDIO_BATCH_MCLK.div_ceil(genesis_core::timing::YM2612_DIVIDER) as usize,
        );
        self.audio_resampler
            .native_reserve_output_buffers(self.bus.timing_mode, MAXIMUM_AUDIO_BATCH_MCLK);
        self.bus.vdp.native_reserve_sprite_buffers();
    }

    pub fn memory(&self) -> &AddressSpace {
        &self.bus.memory
    }
    pub fn memory_mut(&mut self) -> &mut AddressSpace {
        &mut self.bus.memory
    }
    #[cfg(feature = "game-adapter")]
    pub(crate) fn replace_ram_dma_observer(
        &mut self,
        observer: Option<std::sync::Arc<dyn genesis_core::ram_dma_observer::Observer>>,
    ) -> genesis_core::ram_dma_observer::ObserverSlot {
        self.bus.ram_dma_observer.replace(observer)
    }
    #[cfg(feature = "game-adapter")]
    pub(crate) fn restore_ram_dma_observer(
        &mut self,
        previous: genesis_core::ram_dma_observer::ObserverSlot,
    ) {
        self.bus.ram_dma_observer.restore(previous);
    }
    pub fn vdp(&self) -> &Vdp {
        &self.bus.vdp
    }
    pub fn ym2612(&self) -> &Ym2612 {
        &self.bus.ym2612
    }
    pub fn psg(&self) -> &Sn76489 {
        &self.bus.psg
    }
    pub fn z80(&self) -> &Z80 {
        &self.z80
    }
    pub fn audio_ram(&self) -> &[u8; 0x2000] {
        &self.bus.audio_ram
    }
    pub fn cycles(&self) -> &CycleCounters {
        &self.bus.cycles
    }
    pub fn clock(&self) -> u64 {
        self.bus.cycles.m68k_mclk_cycles
    }
    pub fn frame_count(&self) -> u64 {
        self.frames
    }
    pub fn reset_asserted(&self) -> bool {
        self.main_reset
    }
    pub fn set_reset_asserted(&mut self, asserted: bool) {
        self.main_reset = asserted;
    }
    pub fn set_instruction_pc(&mut self, pc: u32) {
        self.bus.main_pc = pc;
    }
    pub fn set_read_offset_mclk(&mut self, offset: u64) {
        self.bus.read_offset = offset;
    }
    pub fn prefetch_word(&self) -> u16 {
        self.bus.opcode
    }
    pub fn wait_pending(&self) -> bool {
        self.bus.cycles.m68k_wait_cpu_cycles != 0
    }
    pub fn audio_samples(&self) -> &[[f64; 2]] {
        &self.audio.0
    }
    pub fn clear_audio(&mut self) {
        self.audio.0.clear();
    }
    pub fn set_audio_sample_rate(&mut self, frequency: u32) {
        self.audio_sample_rate = frequency;
        self.audio_resampler
            .update_output_frequency(u64::from(frequency));
        self.audio.0.clear();
        let capacity = (f64::from(frequency) / self.target_fps()).ceil() as usize + 16;
        self.audio.0.reserve(capacity);
        self.reserve_startup_buffers();
    }
    pub fn target_fps(&self) -> f64 {
        genesis_components::target_framerate(&self.bus.vdp, self.bus.timing_mode)
    }
    pub fn reset(&mut self) {
        #[cfg(feature = "game-adapter")]
        self.bus.vdp.invalidate_readable_dma_on_reset();
        self.bus.memory.ram_mut().fill(0);
        self.bus.audio_ram.fill(0);
        self.bus.vdp = Vdp::new(self.bus.timing_mode, self.config.to_vdp_config(false));
        self.bus.ym2612 = Ym2612::new(&self.config);
        self.bus.psg = Sn76489::new(Sn76489Version::Standard, self.config.psg_channels_enabled);
        self.bus.input = InputState::new(&self.config, false);
        self.bus.cycles = CycleCounters::new(self.config.clamped_m68k_divider());
        self.bus.bank = 0;
        self.bus.bus_request = false;
        self.bus.reset = true;
        self.bus.open_bus = 0;
        self.bus.read_offset = 0;
        self.bus.main_pc = 0;
        self.bus.audio_pc = 0;
        self.bus.opcode = 0;
        self.bus.pending_bytes.clear();
        self.bus.pending_words.clear();
        self.z80 = Z80::new();
        self.audio_resampler = GenesisAudioResampler::new(self.bus.timing_mode, &self.config);
        self.audio_resampler
            .update_output_frequency(u64::from(self.audio_sample_rate));
        self.instruction_open = false;
        self.main_reset = false;
        self.frames = 0;
        self.input_clock_remainder = 0;
        self.clear_audio();
        self.clear_trace();
        self.clear_ram_write_trace();
    }
    pub fn set_inputs(&mut self, inputs: GenesisInputs) {
        self.bus.input.set_inputs(inputs);
    }
    pub fn frame_buffer(&self) -> (&[Color], FrameSize) {
        (self.bus.vdp.frame_buffer(), self.bus.vdp.frame_size())
    }
    pub fn trace(&self) -> Option<&DeviceTrace> {
        self.bus.trace.as_ref()
    }
    pub fn enable_trace(&mut self) {
        self.bus.trace.get_or_insert_with(DeviceTrace::default);
    }
    pub fn disable_trace(&mut self) {
        self.bus.trace = None;
    }
    pub fn clear_trace(&mut self) {
        if let Some(trace) = &mut self.bus.trace {
            trace.accesses.clear();
            trace.writes.clear();
        }
    }
    pub fn enable_ram_write_trace(&mut self) {
        self.bus.ram_writes.get_or_insert_with(Vec::new);
    }
    pub fn disable_ram_write_trace(&mut self) {
        self.bus.ram_writes = None;
    }
    pub fn clear_ram_write_trace(&mut self) {
        if let Some(trace) = &mut self.bus.ram_writes {
            trace.clear();
        }
    }
    pub fn ram_write_trace(&self) -> Option<&[RamWrite]> {
        self.bus.ram_writes.as_deref()
    }

    /// Immediate facade for the readable, non-cycle-exact translation proofs.
    /// Generated continuations must use begin/write/finish instead.
    pub fn write_immediate_byte(&mut self, address: u32, value: u8) {
        self.bus.apply_main::<false>(address, value.into());
    }
    pub fn write_immediate_word(&mut self, address: u32, value: u16) {
        self.bus.apply_main::<true>(address, value);
    }

    pub fn capture_state(&self, registers: Registers) -> PeripheralState {
        PeripheralState {
            ram: self.bus.memory.ram().to_vec(),
            audio_ram: self.bus.audio_ram.to_vec(),
            vdp: self.bus.vdp.clone(),
            ym2612: self.bus.ym2612.clone(),
            psg: self.bus.psg.clone(),
            input: self.bus.input.clone(),
            z80: self.z80.clone(),
            z80_bank: self.bus.bank,
            bus_request: self.bus.bus_request,
            reset: self.bus.reset,
            cycles: self.bus.cycles,
            timing_mode: self.bus.timing_mode,
            region: self.bus.region,
            audio_resampler: self.audio_resampler.clone(),
            open_bus: self.bus.open_bus,
            config: self.config.clone(),
            program_counter: registers.program_counter,
            native_input_remainder: self.input_clock_remainder,
            data: registers.data,
            address: registers.address,
            status: registers.status,
            user_stack_pointer: registers.user_stack_pointer,
            supervisor_stack_pointer: registers.supervisor_stack_pointer,
        }
    }

    fn advance_recorded_step(&mut self, pc: u32, cpu_cycles: u32, wait: bool) -> DeviceStep {
        self.bus.main_pc = pc;
        let clocks = self.bus.cycles.record_68k_instruction(
            pc,
            cpu_cycles,
            wait,
            self.bus.vdp.should_halt_cpu(),
        );
        self.tick_devices(cpu_cycles, clocks, wait)
    }

    fn tick_devices(&mut self, cpu_cycles: u32, clocks: u64, wait: bool) -> DeviceStep {
        self.bus.origin = Origin::Z80;
        while self.bus.cycles.should_tick_z80() {
            if !self.bus.cycles.z80_halt {
                self.bus.audio_pc = u32::from(self.z80.pc());
                self.z80.tick(&mut self.bus);
            }
            self.bus.cycles.z80_cycle();
        }
        self.bus.origin = Origin::Main;
        self.bus.input.tick(cpu_cycles);
        while self.bus.cycles.should_tick_psg() {
            if self.bus.psg.tick() == Sn76489TickEffect::Clocked {
                self.audio_resampler.collect_psg(self.bus.psg.sample().0);
            }
            self.bus.cycles.psg_cycle();
        }
        let effect = self.bus.vdp.tick(
            clocks,
            &mut DmaMemory {
                memory: &self.bus.memory,
                open_bus: &mut self.bus.open_bus,
            },
        );
        if self.bus.cycles.ym2612_sync_needed() || effect == VdpTickEffect::FrameComplete {
            self.bus.sync_ym2612();
            for sample in self.bus.ym2612.drain_output_samples() {
                self.audio_resampler.collect_ym2612(sample);
            }
        }
        if self.bus.vdp.long_halting_dma_in_progress() {
            if self.bus.cycles.z80_halt {
                self.bus.cycles.m68k_wait_cpu_cycles = self.bus.cycles.m68k_wait_cpu_cycles.max(
                    self.bus.cycles.max_wait_cpu_cycles.min(
                        (vdp::MCLK_CYCLES_PER_SCANLINE - self.bus.vdp.scanline_mclk()) as u32
                            / self.bus.cycles.m68k_divider_u32.get(),
                    ),
                );
            } else {
                self.bus.cycles.m68k_wait_cpu_cycles = 8;
            }
        }
        if !wait {
            self.bus.vdp.update_interrupt_latches();
        }
        self.bus.apply_pending();
        let frame_complete = effect == VdpTickEffect::FrameComplete;
        match self.audio_resampler.output_samples(&mut self.audio) {
            Ok(()) => {}
            Err(never) => match never {},
        }
        self.frames += u64::from(frame_complete);
        DeviceStep {
            cpu_cycles,
            master_clocks: clocks,
            frame_complete,
        }
    }

    /// One actual bus read with its caller-supplied architectural purpose.
    pub fn read_bus_byte_tagged(&mut self, address: u32, purpose: BusReadPurpose) -> u8 {
        self.bus.read_main::<false>(address, purpose) as u8
    }

    /// One physical word read; no semantic pre-read or post-read relabelling.
    pub fn read_bus_word_tagged(&mut self, address: u32, purpose: BusReadPurpose) -> u16 {
        self.bus.read_main::<true>(address, purpose)
    }

    #[cfg(any(test, feature = "game-adapter"))]
    pub(crate) fn discard_bus_byte(&mut self, address: u32) {
        self.read_bus_byte_tagged(address, BusReadPurpose::Discarded);
    }

    #[cfg(any(test, feature = "game-adapter"))]
    pub(crate) fn discard_bus_word(&mut self, address: u32) {
        self.read_bus_word_tagged(address, BusReadPurpose::Discarded);
    }

    /// Advance a precisely supplied noninstruction interval (for a suspended
    /// frontend/STOP). IRQ acceptance is still owned by the instruction runtime.
    pub fn advance_master_clocks(&mut self, mut clocks: u64) -> bool {
        let mut frame_complete = false;
        while clocks != 0 {
            let step = clocks.min(1225);
            self.bus
                .cycles
                .increment_mclk_counters(step, self.bus.vdp.should_halt_cpu());
            let input_clocks = step + self.input_clock_remainder;
            let divider = self.bus.cycles.m68k_divider.get();
            self.input_clock_remainder = input_clocks % divider;
            frame_complete |= self
                .tick_devices((input_clocks / divider) as u32, step, true)
                .frame_complete;
            clocks -= step;
        }
        frame_complete
    }
}

impl ClockedDevices for DeviceRuntime {
    fn begin_instruction(&mut self, opcode: u16, read_offset_mclk: u64) {
        assert!(!self.instruction_open, "unfinished native instruction");
        self.instruction_open = true;
        self.bus.read_offset = read_offset_mclk;
        self.bus.opcode = opcode;
    }
    fn read_bus_byte(&mut self, address: u32) -> u8 {
        self.read_bus_byte_tagged(address, BusReadPurpose::Semantic)
    }
    fn read_bus_word(&mut self, address: u32) -> u16 {
        self.read_bus_word_tagged(address, BusReadPurpose::Semantic)
    }
    fn write_bus_byte(&mut self, address: u32, value: u8) {
        self.bus.pending_bytes.push((address, value));
    }
    fn write_bus_word(&mut self, address: u32, value: u16) {
        self.bus.pending_words.push((address, value));
    }
    fn finish_instruction(&mut self, pc: u32, cpu_cycles: u32) -> DeviceStep {
        assert!(self.instruction_open, "native instruction was not begun");
        self.instruction_open = false;
        self.advance_recorded_step(pc, cpu_cycles, false)
    }
    fn take_wait_step(&mut self, pc: u32) -> Option<DeviceStep> {
        if self.bus.cycles.m68k_wait_cpu_cycles == 0 {
            return None;
        }
        self.instruction_open = false;
        let cycles = self.bus.cycles.take_m68k_wait_cpu_cycles();
        Some(self.advance_recorded_step(pc, cycles, true))
    }
    fn cpu_halted(&self) -> bool {
        self.bus.vdp.should_halt_cpu()
    }
    fn pending_interrupt(&self) -> u8 {
        self.bus.vdp.m68k_interrupt_level()
    }
    fn acknowledge_interrupt(&mut self) {
        self.bus.vdp.acknowledge_m68k_interrupt();
    }
}

#[cfg(test)]
mod allocation_tests {
    use super::*;
    use tje_mem::Revision;

    fn assert_full_movem_fits(mut devices: DeviceRuntime) {
        assert!(devices.bus.pending_words.capacity() >= 32);
        let allocation = devices.bus.pending_words.as_ptr();
        devices.begin_instruction(0x48E7, 0);
        // A full-register MOVEM.L issues two words per register.
        for index in 0..32 {
            devices.write_bus_word(0xFF0000 + 2 * index, index as u16);
        }
        assert_eq!(devices.bus.pending_words.as_ptr(), allocation);
        assert_eq!(devices.bus.pending_words.len(), 32);
        devices.finish_instruction(0x200, 140);
        assert!(devices.bus.pending_words.is_empty());
    }

    #[test]
    fn power_on_and_captured_devices_reserve_the_largest_instruction_store() {
        let config = GenesisEmulatorConfig::default();
        let memory = || AddressSpace::new(vec![0; 0x400], Revision::Rev02);
        let devices = DeviceRuntime::new(memory(), &config);
        let state = devices.capture_state(Registers::default());
        assert_full_movem_fits(devices);
        assert_full_movem_fits(DeviceRuntime::from_state(memory(), state).unwrap());
    }

    #[test]
    fn imported_capacity_reservation_preserves_encoded_devices_and_filter_history() {
        let encoded =
            |vdp: &Vdp, ym: &Ym2612, audio: &GenesisAudioResampler, cycles: &CycleCounters| {
                bincode::encode_to_vec((vdp, ym, audio, cycles), bincode::config::standard())
                    .unwrap()
            };
        for mode in [TimingMode::Ntsc, TimingMode::Pal] {
            for frequency in [8_000, 48_000, 192_000] {
                let config = GenesisEmulatorConfig {
                    forced_timing_mode: Some(mode),
                    ..Default::default()
                };
                let memory = || AddressSpace::new(vec![0; 0x400], Revision::Rev02);
                let mut devices = DeviceRuntime::new(memory(), &config);
                devices.set_audio_sample_rate(frequency);
                devices.bus.ym2612.tick(113);
                for _ in 0..256 {
                    devices.audio_resampler.collect_ym2612((0.25, -0.5));
                    devices.audio_resampler.collect_psg(0.125);
                }
                let state = devices.capture_state(Registers::default());
                let expected = encoded(
                    &state.vdp,
                    &state.ym2612,
                    &state.audio_resampler,
                    &state.cycles,
                );
                let expected_vdp = state.vdp.clone();
                let mut restored = DeviceRuntime::from_state(memory(), state).unwrap();
                assert_eq!(
                    expected,
                    encoded(
                        &restored.bus.vdp,
                        &restored.bus.ym2612,
                        &restored.audio_resampler,
                        &restored.bus.cycles,
                    )
                );
                assert!(restored.bus.vdp.hybrid_state_eq(&expected_vdp));
                restored.reserve_startup_buffers();
                assert_eq!(
                    expected,
                    encoded(
                        &restored.bus.vdp,
                        &restored.bus.ym2612,
                        &restored.audio_resampler,
                        &restored.bus.cycles,
                    )
                );
                assert!(restored.bus.vdp.hybrid_state_eq(&expected_vdp));
            }
        }
    }
}
