// TJE standalone cutover: only cartridge Genesis hardware is present; all
// remaining memory accesses, clock order and DMA handling retain upstream logic.
use crate::input::InputState;
use crate::timing::CycleCounters;
use bincode::{Decode, Encode};
use genesis_components::cartridge::{Cartridge, GenesisRegionExt};
use genesis_components::memory::Memory;
use genesis_components::vdp::{Vdp, VdpBusView, VdpTickEffect};
use genesis_components::ym2612::Ym2612;
use genesis_components::{GenesisEmulatorConfigExt, vdp};
use genesis_config::{GenesisEmulatorConfig, GenesisRegion};
use jgenesis_common::frontend::{PartialClone, TimingMode};
use jgenesis_common::num::{GetBit, U16Ext};
use m68000_emu::debug::DummyM68000Debugger;
use std::{cmp, mem};
use ti_sn76489::{Sn76489, Sn76489TickEffect, Sn76489Version};
use z80_emu::debug::DummyZ80Debugger;
use z80_emu::traits::InterruptLine;

use crate::audio::GenesisAudioOutput;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Encode, Decode)]
pub struct Z80BankRegister {
    bank_number: u32,
    current_bit: u8,
}

impl Z80BankRegister {
    const BITS: u8 = 9;

    pub fn value(self) -> u32 {
        self.bank_number
    }

    pub fn map_to_68k_address(self, z80_address: u16) -> u32 {
        (self.bank_number << 15) | u32::from(z80_address & 0x7FFF)
    }

    pub fn write_bit(&mut self, bit: bool) {
        self.bank_number = (self.bank_number >> 1) | (u32::from(bit) << (Self::BITS - 1));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct Z80Signals {
    busreq: bool,
    reset: bool,
}

impl Default for Z80Signals {
    fn default() -> Self {
        Self { busreq: false, reset: true }
    }
}

impl Z80Signals {
    // TJE hybrid inspection: read-only drive-mask query; no signal/timing change.
    #[inline]
    pub fn busack(self) -> bool {
        self.busreq && !self.reset
    }

    // TJE hybrid inspection: actual signal fields, never a bus/MMIO read.
    #[inline]
    pub fn bus_requested(self) -> bool {
        self.busreq
    }

    #[inline]
    pub fn reset_asserted(self) -> bool {
        self.reset
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Encode, Decode)]
pub struct PendingWrites {
    byte: Vec<(u32, u8)>,
    word: Vec<(u32, u16)>,
}

impl PendingWrites {
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self { byte: Vec::with_capacity(20), word: Vec::with_capacity(20) }
    }

    fn clear(&mut self) {
        self.byte.clear();
        self.word.clear();
    }

    #[cfg(feature = "reference-proof-hooks")]
    pub(crate) fn reference_is_empty(&self) -> bool {
        self.byte.is_empty() && self.word.is_empty()
    }
}

struct VdpView<'bus> {
    cartridge: Option<&'bus mut Cartridge>,
    memory: &'bus mut Memory,
    open_bus: &'bus mut u16,
    ram_audit: &'bus mut Option<Vec<HybridAccess>>,
    all_dma_sources: bool,
}

impl VdpBusView for VdpView<'_> {
    fn read_word_for_dma(&mut self, address: u32) -> u16 {
        let value = match address {
            0x000000..=0x3FFFFF => {
                if let Some(cartridge) = &mut self.cartridge {
                    cartridge.read_word_for_dma(address, self.open_bus)
                } else {
                    *self.open_bus
                }
            }
            0xE00000..=0xFFFFFF => self.memory.read_main_ram_word(address),
            _ => *self.open_bus,
        };
        // TJE hybrid: reviewed exact-transfer proofs additionally retain ROM
        // and open-bus source beats. The real read and its effects occur once.
        if (self.all_dma_sources || address >= 0xE00000)
            && let Some(trace) = self.ram_audit
        {
            trace.push(HybridAccess::DmaRead { address, value });
        }
        value
    }
}
// TJE hybrid observation: writes at chip dispatch, including Z80-origin writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum HybridWrite {
    Vdp { port: u8, value: u16 },
    Ym2612 { port: u8, value: u8 },
    Psg { value: u8 },
}

/// TJE hybrid observation of ordered 68000 device accesses, including reads
/// whose values/latch effects matter even when there is no resulting write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum HybridAccess {
    Read {
        address: u32,
        word: bool,
        value: u16,
        purpose: m68000_emu::ReadPurpose,
    },
    Write {
        address: u32,
        word: bool,
        value: u16,
    },
    /// Actual VDP source read, not a 68000 instruction or prefetch.
    DmaRead {
        address: u32,
        value: u16,
    },
    /// Actual banked Z80 work-RAM commit, never a 68000 stack save.
    Z80Write {
        address: u32,
        word: bool,
        value: u16,
    },
    /// Physical external input-recipe store, never a game CPU frame producer.
    HostMutation {
        address: u32,
        word: bool,
        value: u16,
    },
    /// Index into the opt-in boundary payload; common accesses stay compact.
    ExceptionBoundary {
        index: u32,
    },
}

#[derive(Clone, Copy)]
enum WriteActor {
    M68k,
    Z80,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct HybridExceptionBoundary {
    pub master_clock: u64,
    pub boundary: m68000_emu::ExceptionBoundary,
}

/// Actual hardware DMA activation at its dispatch-time exception scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct HybridDmaStart {
    pub origin: crate::native::Origin,
    pub pc: u32,
    pub master_clock: u64,
    pub address: u32,
    pub value: u16,
    pub entry_exception_depth: u32,
    pub exception_depth: u32,
}

/// Opt-in owned observations for one speculative root, never IRQ authority.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct HybridDmaCapture {
    pub entry_exception_depth: u32,
    pub current_exception_depth: u32,
    pub starts: Vec<HybridDmaStart>,
    pub boundaries: Vec<HybridExceptionBoundary>,
    pub invalid_depth_sequence: bool,
    pub missing_dispatch_context: bool,
    pub active_dma_port_write_seen: bool,
    pub deferred_dma_port_write_seen: bool,
}

impl HybridDmaCapture {
    fn observe_boundary(&mut self, master_clock: u64, boundary: m68000_emu::ExceptionBoundary) {
        let depth = match boundary {
            m68000_emu::ExceptionBoundary::Entered { vector: _, handler: _, depth } => {
                if self.current_exception_depth.checked_add(1) != Some(depth) {
                    self.invalid_depth_sequence = true;
                }
                depth
            }
            m68000_emu::ExceptionBoundary::Returned { depth } => {
                if depth.checked_add(1) != Some(self.current_exception_depth) {
                    self.invalid_depth_sequence = true;
                }
                depth
            }
        };
        if depth < self.entry_exception_depth {
            self.invalid_depth_sequence = true;
        }
        self.current_exception_depth = depth;
        self.boundaries.push(HybridExceptionBoundary { master_clock, boundary });
    }
}

/// Passive queue/stall checkpoint; no MMIO read or pending-write flush.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HybridBusCheckpoint {
    pub pending_byte_writes: usize,
    pub pending_word_writes: usize,
    pub m68k_wait_cpu_cycles: u32,
    /// Fractional Z80 bus-stall rounding phase (0..9), not outstanding debt.
    pub m68k_wait_counter: u8,
    /// CPU refresh phase modulo REFRESH_INTERVAL (128 CPU cycles), not debt.
    pub m68k_refresh_counter: u32,
    pub vdp_owns_bus: bool,
    pub z80_halt: bool,
    pub m68k_reset: bool,
    pub z80_bus_requested: bool,
    pub z80_reset: bool,
    pub interrupt_level: u8,
    pub cpu_halt: bool,
}

/// The passive actual-read journal exists only in reference-proof builds.
#[cfg_attr(
    not(feature = "reference-proof-hooks"),
    doc = "```compile_fail\nlet _: genesis_core::reference_proof::ReadContext;\n```"
)]
#[derive(Debug, Encode, Decode, PartialClone)]
pub struct GenesisBus {
    #[partial_clone(partial)]
    pub cartridge: Option<Cartridge>,
    pub memory: Memory,
    pub vdp: Vdp,
    pub psg: Sn76489,
    pub ym2612: Ym2612,
    pub input: InputState,
    pub z80_bank: Z80BankRegister,
    pub z80_signals: Z80Signals,
    pub pending_writes: PendingWrites,
    pub cycles: CycleCounters,
    pub timing_mode: TimingMode,
    pub open_bus: u16,
    pub m68k_opcode: u16,
    pub m68k_reset: bool,
    // TJE hybrid observation is opt-in; ordinary emulation does not allocate a trace.
    pub hybrid_writes: Option<Vec<HybridWrite>>,
    pub hybrid_accesses: Option<Vec<HybridAccess>>,
    // TJE timing survey is separate from the function-check write contract.
    pub timing_survey: Option<crate::timing_survey::TimingSurvey>,
    /// TJE native: actual chip dispatches, not attempted address writes.
    /// Enable with `enable_native_write_trace` so processor/PC provenance exists.
    pub native_writes: Option<Vec<crate::native::TimedWrite>>,
    /// TJE native: opt-in CPU-issued transactions at every address, including ROM.
    pub native_cpu_accesses: Option<Vec<crate::native::CpuAccess>>,
    /// TJE reference proof: actual read context, never a source/readiness grant.
    #[cfg(feature = "reference-proof-hooks")]
    pub reference_read_contexts: Option<Vec<crate::reference_proof::ReadContext>>,
    /// TJE native: all-origin actual work-RAM commits, separate from CPU issues.
    pub native_ram_writes: Option<Vec<crate::native::RamWrite>>,
    /// TJE hybrid: chronological RAM/device audit for time-progressing calls.
    /// ROM fetches are omitted; normal emulation leaves this disabled.
    pub hybrid_cpu_accesses: Option<Vec<HybridAccess>>,
    /// Installed only for frame provenance. Payloads are separate for size, not
    /// chronology: the marker's position in hybrid_cpu_accesses is authoritative.
    pub hybrid_exception_boundaries: Option<Vec<HybridExceptionBoundary>>,
    /// TJE hybrid: opt-in complete DMA source stream for reviewed zero-count
    /// proofs. Normal checks retain only RAM-source audit events.
    pub hybrid_dma_all_sources: bool,
    /// TJE strict proof only: default None, owned dispatch/boundary observations.
    hybrid_dma_capture: Option<HybridDmaCapture>,
    // TJE hybrid: opt-in untimed transaction semantics, enabled only on check forks.
    hybrid_functional: bool,
    pub hybrid_functional_violation: Option<u32>,
}

impl GenesisBus {
    /// TJE strict data-plane control view. All bus fields except the VDP and
    /// physical main RAM remain encoded, including diagnostic provenance.
    /// Exhaustive binding makes a newly added state field a compile-time review.
    pub(crate) fn hybrid_control_without_vdp_main_ram(&self) -> impl Encode + '_ {
        let Self {
            cartridge,
            memory,
            vdp: _,
            psg,
            ym2612,
            input,
            z80_bank,
            z80_signals,
            pending_writes,
            cycles,
            timing_mode,
            open_bus,
            m68k_opcode,
            m68k_reset,
            hybrid_writes,
            hybrid_accesses,
            timing_survey,
            native_writes,
            native_cpu_accesses,
            #[cfg(feature = "reference-proof-hooks")]
            reference_read_contexts,
            native_ram_writes,
            hybrid_cpu_accesses,
            hybrid_exception_boundaries,
            hybrid_dma_all_sources,
            hybrid_dma_capture,
            hybrid_functional,
            hybrid_functional_violation,
        } = self;
        (
            (
                cartridge,
                memory.hybrid_control_without_main_ram(),
                psg,
                ym2612,
                input,
                z80_bank,
                z80_signals,
                pending_writes,
                cycles,
                timing_mode,
                open_bus,
                m68k_opcode,
                m68k_reset,
            ),
            (
                hybrid_writes,
                hybrid_accesses,
                timing_survey,
                native_writes,
                native_cpu_accesses,
                #[cfg(feature = "reference-proof-hooks")]
                reference_read_contexts,
                native_ram_writes,
                hybrid_cpu_accesses,
                hybrid_exception_boundaries,
                hybrid_dma_all_sources,
                hybrid_dma_capture,
                hybrid_functional,
                hybrid_functional_violation,
            ),
        )
    }

    /// TJE DiscardedReadValueVariance probe control view, by borrow only.
    /// The caller first validates the complete actual CPU journal under that
    /// named contract. Only its contents are excluded here; presence and length
    /// and every other non-VDP/non-main-RAM field remain encoded.
    pub(crate) fn hybrid_control_without_vdp_main_ram_cpu_journal(&self) -> impl Encode + '_ {
        let Self {
            cartridge,
            memory,
            vdp: _,
            psg,
            ym2612,
            input,
            z80_bank,
            z80_signals,
            pending_writes,
            cycles,
            timing_mode,
            open_bus,
            m68k_opcode,
            m68k_reset,
            hybrid_writes,
            hybrid_accesses,
            timing_survey,
            native_writes,
            native_cpu_accesses,
            #[cfg(feature = "reference-proof-hooks")]
            reference_read_contexts,
            native_ram_writes,
            hybrid_cpu_accesses,
            hybrid_exception_boundaries,
            hybrid_dma_all_sources,
            hybrid_dma_capture,
            hybrid_functional,
            hybrid_functional_violation,
        } = self;
        (
            (
                cartridge,
                memory.hybrid_control_without_main_ram(),
                psg,
                ym2612,
                input,
                z80_bank,
                z80_signals,
                pending_writes,
                cycles,
                timing_mode,
                open_bus,
                m68k_opcode,
                m68k_reset,
            ),
            (
                hybrid_writes,
                hybrid_accesses,
                timing_survey,
                native_writes,
                native_cpu_accesses.as_ref().map(Vec::len),
                #[cfg(feature = "reference-proof-hooks")]
                reference_read_contexts,
                native_ram_writes,
                hybrid_cpu_accesses,
                hybrid_exception_boundaries,
                hybrid_dma_all_sources,
                hybrid_dma_capture,
                hybrid_functional,
                hybrid_functional_violation,
            ),
        )
    }

    /// Snapshot actual scalar state by borrow. New fields force explicit review.
    pub fn hybrid_checkpoint(&self) -> HybridBusCheckpoint {
        let Self {
            cartridge: _,
            memory: _,
            vdp: _,
            psg: _,
            ym2612: _,
            input: _,
            z80_bank: _,
            z80_signals,
            pending_writes,
            cycles,
            timing_mode: _,
            open_bus: _,
            m68k_opcode: _,
            m68k_reset,
            hybrid_writes: _,
            hybrid_accesses: _,
            timing_survey: _,
            native_writes: _,
            native_cpu_accesses: _,
            #[cfg(feature = "reference-proof-hooks")]
                reference_read_contexts: _,
            native_ram_writes: _,
            hybrid_cpu_accesses: _,
            hybrid_exception_boundaries: _,
            hybrid_dma_all_sources: _,
            hybrid_dma_capture: _,
            hybrid_functional: _,
            hybrid_functional_violation: _,
        } = self;
        let PendingWrites { byte, word } = pending_writes;
        let Z80Signals { busreq, reset } = z80_signals;
        let CycleCounters {
            m68k_divider: _,
            m68k_divider_u32: _,
            max_wait_cpu_cycles: _,
            m68k_wait_cpu_cycles,
            m68k_wait_counter,
            m68k_mclk_cycles: _,
            z80_mclk_cycles: _,
            ym2612_mclk_cycles: _,
            last_ym2612_drain_mclk: _,
            psg_mclk_cycles: _,
            m68k_refresh_counter,
            vdp_owns_bus,
            z80_halt,
        } = cycles;
        HybridBusCheckpoint {
            pending_byte_writes: byte.len(),
            pending_word_writes: word.len(),
            m68k_wait_cpu_cycles: *m68k_wait_cpu_cycles,
            m68k_wait_counter: *m68k_wait_counter,
            m68k_refresh_counter: *m68k_refresh_counter,
            vdp_owns_bus: *vdp_owns_bus,
            z80_halt: *z80_halt,
            m68k_reset: *m68k_reset,
            z80_bus_requested: *busreq,
            z80_reset: *reset,
            interrupt_level: m68000_emu::BusInterface::interrupt_level(self),
            cpu_halt: m68000_emu::BusInterface::halt(self),
        }
    }

    pub fn new(
        timing_mode: TimingMode,
        cartridge: Option<Cartridge>,
        config: &GenesisEmulatorConfig,
    ) -> Self {
        let memory = Memory::new(config);
        let vdp = Vdp::new(timing_mode, config.to_vdp_config(false));
        let psg = Sn76489::new(Sn76489Version::Standard, config.psg_channels_enabled);
        let ym2612 = Ym2612::new(config);

        let six_button_incompatible = cartridge
            .as_ref()
            .is_some_and(|cartridge| cartridge.metadata().six_button_incompatible);
        let input = InputState::new(config, six_button_incompatible);

        let cycles = CycleCounters::new(config.clamped_m68k_divider());

        Self {
            cartridge,
            memory,
            vdp,
            psg,
            ym2612,
            input,
            z80_bank: Z80BankRegister::default(),
            z80_signals: Z80Signals::default(),
            pending_writes: PendingWrites::new(),
            cycles,
            timing_mode,
            open_bus: 0,
            m68k_opcode: 0,
            hybrid_writes: None,
            hybrid_accesses: None,
            timing_survey: None,
            native_writes: None,
            native_cpu_accesses: None,
            #[cfg(feature = "reference-proof-hooks")]
            reference_read_contexts: None,
            native_ram_writes: None,
            hybrid_cpu_accesses: None,
            hybrid_exception_boundaries: None,
            hybrid_dma_all_sources: false,
            hybrid_dma_capture: None,
            hybrid_functional: false,
            hybrid_functional_violation: None,
            m68k_reset: true,
        }
    }

    pub fn enable_native_write_trace(&mut self) {
        self.timing_survey.get_or_insert_with(crate::timing_survey::TimingSurvey::default);
        self.native_writes.get_or_insert_with(Vec::new);
    }

    /// Arm only on a speculative fork with the actual CPU's entry depth.
    pub fn hybrid_arm_dma_capture(&mut self, actual_entry_depth: u32) {
        self.hybrid_dma_capture = Some(HybridDmaCapture {
            entry_exception_depth: actual_entry_depth,
            current_exception_depth: actual_entry_depth,
            starts: Vec::new(),
            boundaries: Vec::new(),
            invalid_depth_sequence: false,
            missing_dispatch_context: false,
            active_dma_port_write_seen: false,
            deferred_dma_port_write_seen: false,
        });
    }

    pub fn hybrid_dma_capture(&self) -> Option<&HybridDmaCapture> {
        self.hybrid_dma_capture.as_ref()
    }

    fn record_hybrid_dma_start(&mut self, address: u32, value: u16) {
        let Some(capture) = &mut self.hybrid_dma_capture else { return };
        let Some(survey) = &self.timing_survey else {
            capture.missing_dispatch_context = true;
            return;
        };
        let (origin, master_clock) = match survey.cpu {
            crate::timing_survey::TimingCpu::M68k => {
                (crate::native::Origin::Main, self.cycles.m68k_mclk_cycles)
            }
            crate::timing_survey::TimingCpu::Z80 => {
                (crate::native::Origin::Z80, self.cycles.z80_mclk_cycles)
            }
        };
        capture.starts.push(HybridDmaStart {
            origin,
            pc: survey.pc,
            master_clock,
            address,
            value,
            entry_exception_depth: capture.entry_exception_depth,
            exception_depth: capture.current_exception_depth,
        });
    }

    fn observe_active_dma_port_write(&mut self) -> Option<usize> {
        let capture = self.hybrid_dma_capture.as_mut()?;
        if self.vdp.timing_dma_active() {
            capture.active_dma_port_write_seen = true;
        }
        Some(self.vdp.hybrid_checkpoint().pending_dma_port_writes)
    }

    fn observe_deferred_dma_port_write(&mut self, before: Option<usize>) {
        if let Some(before) = before {
            let after = self.vdp.hybrid_checkpoint().pending_dma_port_writes;
            if after > before {
                self.hybrid_dma_capture
                    .as_mut()
                    .expect("active capture")
                    .deferred_dma_port_write_seen = true;
            }
        }
    }

    fn record_native_write(&mut self, write: crate::native::DeviceWrite) {
        let Some(trace) = &mut self.native_writes else {
            return;
        };
        let survey =
            self.timing_survey.as_ref().expect("native write trace requires timing survey");
        let (origin, clock) = match survey.cpu {
            crate::timing_survey::TimingCpu::M68k => {
                (crate::native::Origin::Main, self.cycles.m68k_mclk_cycles)
            }
            crate::timing_survey::TimingCpu::Z80 => {
                (crate::native::Origin::Z80, self.cycles.z80_mclk_cycles)
            }
        };
        trace.push(crate::native::TimedWrite { clock, origin, pc: survey.pc, write });
    }

    // TJE standalone cutover: no addon clocks or coupled channel debugger.
    #[inline]
    pub fn tick_components(
        &mut self,
        m68k_cycles: u32,
        mclk_cycles: u64,
        m68k_wait: bool,
        audio_output: &mut impl GenesisAudioOutput,
    ) -> VdpTickEffect {
        if let Some(cartridge) = &mut self.cartridge {
            cartridge.tick(m68k_cycles);
        }

        self.input.tick(m68k_cycles);

        while self.cycles.should_tick_psg() {
            if self.psg.tick() == Sn76489TickEffect::Clocked {
                // PSG only has mono output in the Genesis; stereo output is only for Game Gear
                let (psg_sample, _) = self.psg.sample();
                audio_output.collect_psg(psg_sample);
            }

            self.cycles.psg_cycle();
        }

        let vdp_tick_effect = self.vdp.tick(
            mclk_cycles,
            &mut VdpView {
                cartridge: self.cartridge.as_mut(),
                memory: &mut self.memory,
                open_bus: &mut self.open_bus,
                ram_audit: &mut self.hybrid_cpu_accesses,
                all_dma_sources: self.hybrid_dma_all_sources,
            },
        );

        if self.cycles.ym2612_sync_needed() || vdp_tick_effect == VdpTickEffect::FrameComplete {
            self.sync_ym2612();

            for sample in self.ym2612.drain_output_samples() {
                audio_output.collect_ym2612(sample);
            }
        }

        check_for_long_dma_skip(&self.vdp, &mut self.cycles);

        if !m68k_wait {
            self.vdp.update_interrupt_latches();
        }

        self.timing_flush();
        self.apply_pending_writes();

        vdp_tick_effect
    }

    pub(crate) fn cartridge(&self) -> Option<&Cartridge> {
        self.cartridge.as_ref()
    }

    pub(crate) fn cartridge_mut(&mut self) -> Option<&mut Cartridge> {
        self.cartridge.as_mut()
    }

    pub fn game_title(&mut self) -> Option<String> {
        self.cartridge().map(|cartridge| cartridge.program_title().to_owned())
    }

    pub fn has_persistent_ram(&self) -> bool {
        self.cartridge().is_some_and(Cartridge::is_ram_persistent)
    }

    pub fn get_and_clear_persistent_ram_dirty(&mut self) -> bool {
        self.cartridge_mut().is_some_and(Cartridge::get_and_clear_ram_dirty)
    }

    pub fn reload_config(&mut self, config: &GenesisEmulatorConfig) {
        if let Some(cartridge) = &mut self.cartridge {
            cartridge.reload_config(config);
        }

        self.memory.reload_config(config);
        self.vdp.reload_config(config.to_vdp_config(false));
        self.ym2612.reload_config(config);
        self.psg.set_channels_enabled(config.psg_channels_enabled);
        self.input.reload_config(config);
        self.cycles.update_m68k_divider(config.clamped_m68k_divider());
    }

    pub fn reset(&mut self) {
        self.z80_signals = Z80Signals::default();
        self.ym2612.reset();
    }

    /// TJE hybrid: canonicalize outstanding transaction latency on a check fork.
    /// No ordinary tick/create path enables this mode.
    pub fn hybrid_set_functional_mode(&mut self, enabled: bool) {
        self.hybrid_functional = enabled;
        if enabled {
            self.hybrid_complete_transfers();
            self.ym2612.hybrid_settle_write_busy();
        }
    }

    pub fn hybrid_functional_mode(&self) -> bool {
        self.hybrid_functional
    }

    fn hybrid_complete_transfers(&mut self) {
        self.vdp.hybrid_complete_transfers(&mut VdpView {
            cartridge: self.cartridge.as_mut(),
            memory: &mut self.memory,
            open_bus: &mut self.open_bus,
            ram_audit: &mut self.hybrid_cpu_accesses,
            all_dma_sources: self.hybrid_dma_all_sources,
        });
    }

    // TJE hybrid access: flush a CPU-only instruction without advancing clocks.
    pub fn apply_pending_writes(&mut self) {
        let mut pending_writes = mem::take(&mut self.pending_writes);

        for &(address, value) in &pending_writes.byte {
            self.apply_write::<false>(address, value.into(), WriteActor::M68k);
        }

        for &(address, value) in &pending_writes.word {
            self.apply_write::<true>(address, value, WriteActor::M68k);
        }

        pending_writes.clear();
        self.pending_writes = pending_writes;
    }

    fn m68k_read<const WORD: bool>(&mut self, address: u32) -> u16 {
        self.m68k_read_with_purpose::<WORD>(address, m68000_emu::ReadPurpose::Semantic)
    }

    /// TJE reference proof/host operand contract: one decoder for driven lanes.
    /// Pure inspection; it does not read MMIO, flush writes or advance clocks.
    pub fn hybrid_read_driven_mask(&self, address: u32, word: bool) -> u16 {
        let full = if word { 0xffff } else { 0xff };
        match address & 0xffffff {
            0x000000..=0x7fffff | 0xe00000..=0xffffff => full,
            0xa00000..=0xa0ffff if self.z80_signals.busack() => full,
            0xa10000..=0xa1001f => full,
            0xa11100 => {
                if word {
                    0x0100
                } else {
                    0x01
                }
            }
            0xc00000..=0xc00003 | 0xc00008..=0xc0000b => full,
            0xc00004..=0xc00007 => {
                if word {
                    0x03ff
                } else if address & 1 == 0 {
                    0x03
                } else {
                    0xff
                }
            }
            _ => 0,
        }
    }
    fn m68k_read_with_purpose<const WORD: bool>(
        &mut self,
        address: u32,
        purpose: m68000_emu::ReadPurpose,
    ) -> u16 {
        #[cfg(feature = "reference-proof-hooks")]
        if let Some(survey) = &mut self.timing_survey {
            survey.reference_proof.cpu_access();
        }
        #[cfg(feature = "reference-proof-hooks")]
        let reset_open_bus_before = self
            .timing_survey
            .as_ref()
            .filter(|survey| survey.reference_proof.reset_capture_active())
            .map(|_| self.open_bus);
        #[cfg(feature = "reference-proof-hooks")]
        let read_context_before = self.reference_read_contexts.as_ref().map(|_| {
            let physical = address & 0xffffff;
            let mut mask = self.hybrid_read_driven_mask(physical, WORD);
            if physical < 0x800000
                && !self.cartridge.as_ref().is_some_and(|cartridge| {
                    cartridge.reference_basic_rom_is_mapped(physical, WORD)
                })
            {
                mask = 0;
            }
            (mask, self.z80_signals.busack())
        });
        let value = self.m68k_read_inner::<WORD>(address, purpose);
        #[cfg(feature = "reference-proof-hooks")]
        if let Some(open_bus_before) = reset_open_bus_before {
            self.timing_survey.as_mut().unwrap().reference_proof.reset_access(
                crate::reference_proof::ResetAccess::Read {
                    address: address & 0xffffff,
                    word: WORD,
                    value,
                    purpose: purpose.into(),
                    raw_reset_line: self.m68k_reset,
                    open_bus_before,
                    open_bus_after: self.open_bus,
                },
            );
        }
        #[cfg(feature = "reference-proof-hooks")]
        if let Some((mask, busack)) = read_context_before {
            self.reference_read_contexts
                .as_mut()
                .expect("actual read observer remains installed")
                .push(crate::reference_proof::ReadContext::new(
                    address, WORD, purpose, value, mask, busack,
                ));
        }
        if let Some(trace) = &mut self.native_cpu_accesses {
            let purpose = match purpose {
                m68000_emu::ReadPurpose::Semantic => 0,
                m68000_emu::ReadPurpose::Prefetch => 1,
                m68000_emu::ReadPurpose::Discarded => 2,
            };
            trace.push(crate::native::CpuAccess::Read {
                address: address & 0xFFFFFF,
                word: WORD,
                value,
                purpose,
            });
        }
        value
    }

    fn m68k_read_inner<const WORD: bool>(
        &mut self,
        address: u32,
        purpose: m68000_emu::ReadPurpose,
    ) -> u16 {
        let address = address & 0xFFFFFF;
        log::trace!("Main bus {} read, address={address:06X}", if WORD { "word" } else { "byte" });

        let value = match address {
            0x000000..=0x3FFFFF => {
                if let Some(cartridge) = &mut self.cartridge {
                    cartridge.read::<WORD>(address, self.open_bus)
                } else {
                    self.read_open_bus::<WORD>(address)
                }
            }
            0x400000..=0x7FFFFF => {
                if let Some(cartridge) = &mut self.cartridge {
                    cartridge.read::<WORD>(address, self.open_bus)
                } else {
                    self.read_open_bus::<WORD>(address)
                }
            }
            0xA00000..=0xA0FFFF if self.z80_signals.busack() => {
                // Z80 memory map; 68k can only access when the Z80 is running and removed from the bus
                self.cycles.record_68k_z80_bus_access();

                // For 68k access, $8000-$FFFF mirrors $0000-$7FFF
                let byte =
                    <Self as z80_emu::BusInterface>::read_memory(self, (address & 0x7FFF) as u16);
                if WORD {
                    // All Z80 access is byte-size; word reads mirror the byte in both MSB and LSB
                    u16::from_ne_bytes([byte; 2])
                } else {
                    byte.into()
                }
            }
            0xA10000..=0xA1001F => {
                // I/O ports, even addresses mirror odd addresses
                let byte = self.read_io_register(address | 1);
                if WORD { u16::from_ne_bytes([byte; 2]) } else { byte.into() }
            }
            0xA11100 => {
                // Bit 8 is Z80 BUSACK (active low), other bits are unused
                let busack_bit = u16::from(!self.z80_signals.busack()) << 8;

                // Unused bits should read open bus; Danny Sullivan's Indy Heat (Proto) and Time Killers
                // depend on this or they will fail to boot
                let busack_word = busack_bit | (self.open_bus & !(1 << 8));
                if WORD { busack_word } else { busack_word.msb().into() }
            }
            0xA15000..=0xA15FFF => {
                if let Some(cartridge) = &mut self.cartridge {
                    cartridge.read::<WORD>(address, self.open_bus)
                } else {
                    self.read_open_bus::<WORD>(address)
                }
            }
            0xC00000..=0xC0001F => self.read_vdp::<WORD>(address),
            0xE00000..=0xFFFFFF => self.memory.read_main_ram::<WORD>(address),
            _ => self.read_open_bus::<WORD>(address),
        };

        self.timing_main_access(address, false, value);
        if WORD {
            self.open_bus = value;
        } else {
            // TODO this is probably not right, probably depends on memory region
            self.open_bus.set_msb(value as u8);
        }

        if (0x800000..0xE00000).contains(&address)
            && let Some(trace) = &mut self.hybrid_accesses
        {
            trace.push(HybridAccess::Read { address, word: WORD, value, purpose });
        }
        if address >= 0x800000
            && let Some(trace) = &mut self.hybrid_cpu_accesses
        {
            trace.push(HybridAccess::Read { address, word: WORD, value, purpose });
        }

        value
    }

    #[inline(always)]
    fn read_open_bus<const WORD: bool>(&self, address: u32) -> u16 {
        if WORD { self.open_bus } else { self.open_bus.be_byte(address & 1).into() }
    }

    fn apply_write<const WORD: bool>(&mut self, address: u32, value: u16, actor: WriteActor) {
        let address = address & 0xFFFFFF;
        #[cfg(feature = "reference-proof-hooks")]
        if matches!(actor, WriteActor::M68k)
            && let Some(survey) = &mut self.timing_survey
        {
            survey.reference_proof.physical_commit();
        }
        // Device dispatch precedes any DMA it triggers; RAM is observed below
        // only after its queued write has actually committed.
        if (0x400000..0xE00000).contains(&address)
            && let Some(trace) = &mut self.hybrid_cpu_accesses
        {
            trace.push(HybridAccess::Write { address, word: WORD, value });
        }
        if (0x800000..0xE00000).contains(&address)
            && let Some(trace) = &mut self.hybrid_accesses
        {
            trace.push(HybridAccess::Write { address, word: WORD, value });
        }

        if WORD {
            log::trace!("Main bus word write: address={address:06X}, value={value:04X}");
        } else {
            log::trace!("Main bus byte write: address={address:06X}, value={:02X}", value & 0xFF);
        }
        self.timing_main_access(address, true, value);

        match address {
            0x000000..=0x3FFFFF => {
                if let Some(cartridge) = &mut self.cartridge {
                    cartridge.write::<WORD>(address, value);
                }
            }
            0x400000..=0x7FFFFF => {
                if let Some(cartridge) = &mut self.cartridge {
                    cartridge.write::<WORD>(address, value);
                }
            }
            0xA00000..=0xA0FFFF if self.z80_signals.busack() => {
                // Z80 memory map; writable by the 68k only when the Z80 is removed from the bus
                // and not reset
                self.cycles.record_68k_z80_bus_access();

                // Word-size writes write the MSB as a byte-size write
                let byte = if WORD { value.msb() } else { value as u8 };

                // For 68k access, $8000-$FFFF mirrors $0000-$7FFF
                <Self as z80_emu::BusInterface>::write_memory(
                    self,
                    (address & 0x7FFF) as u16,
                    byte,
                );
            }
            0xA10000..=0xA1001F => {
                // I/O ports, even addresses mirror odd addresses
                self.write_io_register(address | 1, value as u8);
            }
            0xA11100 => {
                self.z80_signals.busreq = if WORD { value.bit(8) } else { value.bit(0) };
                log::trace!("Set Z80 BUSREQ to {}", self.z80_signals.busreq);
                #[cfg(feature = "reference-proof-hooks")]
                if let Some(survey) = &mut self.timing_survey {
                    survey.reference_proof.busreq_commit(
                        &mut self.memory,
                        self.cycles.m68k_mclk_cycles,
                        WORD,
                        value,
                        matches!(actor, WriteActor::M68k),
                        self.z80_signals.reset_asserted(),
                    );
                }
            }
            0xA11200 => {
                let z80_reset = if WORD { !value.bit(8) } else { !value.bit(0) };
                self.set_z80_reset(z80_reset);
                #[cfg(feature = "reference-proof-hooks")]
                if let Some(survey) = &mut self.timing_survey {
                    survey.reference_proof.reset_commit(self.cycles.m68k_mclk_cycles);
                }
            }
            0xA13000..=0xA13FFF if let Some(cartridge) = self.cartridge_mut() => {
                // TIME signal; used by various cartridge mapper registers
                cartridge.write::<WORD>(address, value);
            }
            0xA15000..=0xA15FFF => {
                if let Some(cartridge) = &mut self.cartridge {
                    // SVP maps registers to $A15xxx
                    cartridge.write::<WORD>(address, value);
                }
            }
            0xC00000..=0xC0001F => {
                self.write_vdp_psg::<WORD>(address, value);
            }
            0xE00000..=0xFFFFFF => {
                self.memory.write_main_ram::<WORD>(address, value);
                if let Some(trace) = &mut self.native_ram_writes {
                    trace.push(crate::native::RamWrite::new(address, WORD, value));
                }
                if let Some(trace) = &mut self.hybrid_cpu_accesses {
                    trace.push(match actor {
                        WriteActor::M68k => HybridAccess::Write { address, word: WORD, value },
                        WriteActor::Z80 => HybridAccess::Z80Write { address, word: WORD, value },
                    });
                }
            }
            _ => {}
        }
    }

    fn read_io_register(&mut self, address: u32) -> u8 {
        match address {
            // Version register
            0xA10001 => {
                let region =
                    self.cartridge().map(Cartridge::region).unwrap_or(GenesisRegion::Americas);

                // TODO version (lowest 4 bits) hardcoded to 0
                (u8::from(region.version_bit()) << 7)
                    | (u8::from(self.timing_mode == TimingMode::Pal) << 6)
                    | (1 << 5)
            }
            0xA10003 => self.input.read_p1_data(),
            0xA10005 => self.input.read_p2_data(),
            0xA10007 => self.input.read_ext_data(),
            0xA10009 => self.input.read_p1_ctrl(),
            0xA1000B => self.input.read_p2_ctrl(),
            0xA1000D => self.input.read_ext_ctrl(),
            0xA1000F => self.input.read_p1_tx_data(),
            0xA10015 => self.input.read_p2_tx_data(),
            0xA1001B => self.input.read_ext_tx_data(),
            // Other I/O registers return 0x00 by default
            _ => 0x00,
        }
    }

    fn write_io_register(&mut self, address: u32, value: u8) {
        match address {
            0xA10003 => self.input.write_p1_data(value),
            0xA10005 => self.input.write_p2_data(value),
            0xA10007 => self.input.write_ext_data(value),
            0xA10009 => self.input.write_p1_ctrl(value),
            0xA1000B => self.input.write_p2_ctrl(value),
            0xA1000D => self.input.write_ext_ctrl(value),
            0xA1000F => self.input.write_p1_tx_data(value),
            0xA10015 => self.input.write_p2_tx_data(value),
            0xA1001B => self.input.write_ext_tx_data(value),
            _ => {}
        }
    }

    fn read_vdp<const WORD: bool>(&mut self, address: u32) -> u16 {
        if self.hybrid_functional && (0x04..=0x0B).contains(&(address & 0x1F)) {
            // Status/HV reads observe discarded time/FIFO state: fail closed.
            self.hybrid_functional_violation.get_or_insert(address);
        }
        let word = match address & 0x1F {
            0x00..=0x03 => self.vdp.read_data(),
            0x04..=0x07 => {
                // Highest 6 bits of VDP status register are open bus; VDPFIFOTesting DMA busy flag tests
                // depend on this
                self.vdp.read_status(self.m68k_opcode, self.cycles.m68k_divider.get())
                    | (self.open_bus & 0xFC00)
            }
            0x08..=0x0B => self.vdp.hv_counter(self.m68k_opcode, self.cycles.m68k_divider.get()),
            0x0C..=0x1F => {
                // PSG / unused space; PSG is not readable
                self.open_bus
            }
            _ => unreachable!("address & 0x1F is always <= 0x1F"),
        };

        if WORD { word } else { word.to_be_bytes()[(address & 1) as usize].into() }
    }

    pub(crate) fn write_vdp_psg<const WORD: bool>(&mut self, address: u32, value: u16) {
        // Byte-size VDP writes duplicate the byte into a word
        let vdp_word = if WORD { value } else { (value & 0xFF) | (value << 8) };

        match address & 0x1F {
            0x00..=0x03
                if self.timing_defer(
                    crate::timing_survey::TimingKind::VdpData,
                    address,
                    vdp_word,
                ) => {}
            0x04..=0x07
                if self.timing_defer(
                    crate::timing_survey::TimingKind::VdpControl,
                    address,
                    vdp_word,
                ) => {}
            0x10..=0x17
                if (WORD || address.bit(0))
                    && self.timing_defer(
                        crate::timing_survey::TimingKind::Psg,
                        address,
                        value as u8 as u16,
                    ) => {}
            0x1C..=0x1D
                if self.timing_defer(
                    crate::timing_survey::TimingKind::VdpDebug,
                    address,
                    vdp_word,
                ) => {}
            0x00..=0x03 => {
                if let Some(trace) = &mut self.hybrid_writes {
                    trace.push(HybridWrite::Vdp { port: 0, value: vdp_word });
                }
                self.record_native_write(crate::native::DeviceWrite::Vdp {
                    port: 0,
                    value: vdp_word,
                });
                let pending_before = self.observe_active_dma_port_write();
                self.vdp.write_data(vdp_word);
                self.observe_deferred_dma_port_write(pending_before);
            }
            0x04..=0x07 => {
                if let Some(trace) = &mut self.hybrid_writes {
                    trace.push(HybridWrite::Vdp { port: 4, value: vdp_word });
                }
                self.record_native_write(crate::native::DeviceWrite::Vdp {
                    port: 4,
                    value: vdp_word,
                });
                let was_dma = self.vdp.timing_dma_active();
                let pending_before = self.observe_active_dma_port_write();
                self.vdp.write_control(vdp_word);
                self.observe_deferred_dma_port_write(pending_before);
                if !was_dma && self.vdp.timing_dma_active() {
                    self.timing_access(
                        crate::timing_survey::TimingKind::DmaStart,
                        true,
                        address,
                        vdp_word,
                    );
                    self.record_hybrid_dma_start(address, vdp_word);
                }
            }
            0x10..=0x17 if WORD || address.bit(0) => {
                if let Some(trace) = &mut self.hybrid_writes {
                    trace.push(HybridWrite::Psg { value: value as u8 });
                }
                self.record_native_write(crate::native::DeviceWrite::Psg { value: value as u8 });
                self.psg.write(value as u8);
            }
            0x1C..=0x1D => {
                if let Some(trace) = &mut self.hybrid_writes {
                    trace.push(HybridWrite::Vdp { port: 0x1C, value: vdp_word });
                }
                self.record_native_write(crate::native::DeviceWrite::Vdp {
                    port: 0x1C,
                    value: vdp_word,
                });
                self.vdp.write_debug_register(vdp_word);
            }
            _ => {}
        }
        if self.hybrid_functional {
            self.hybrid_complete_transfers();
        }
    }

    fn set_z80_reset(&mut self, z80_reset: bool) {
        if !self.z80_signals.reset && z80_reset {
            // Z80 RESET also resets the YM2612
            // Fantastic Dizzy depends on this or music will not mute correctly when you pause the game
            self.ym2612.reset();
        }

        self.z80_signals.reset = z80_reset;
        log::trace!("Set Z80 RESET to {}", self.z80_signals.reset);
    }

    pub(crate) fn timing_write_ym(&mut self, address: u16, value: u8) {
        if self.timing_defer(crate::timing_survey::TimingKind::Ym2612, address.into(), value.into())
        {
            return;
        }
        self.sync_ym2612();
        if let Some(trace) = &mut self.hybrid_writes {
            trace.push(HybridWrite::Ym2612 { port: (address & 3) as u8, value });
        }
        self.record_native_write(crate::native::DeviceWrite::Ym2612 {
            port: (address & 3) as u8,
            value,
        });
        match address & 0x03 {
            0x00 => self.ym2612.write_address_1(value),
            0x02 => self.ym2612.write_address_2(value),
            0x01 | 0x03 => self.ym2612.write_data(value),
            _ => unreachable!("value & 0x03 is always <= 0x03"),
        }
        // TJE hybrid: check forks settle the write's busy period (functional mode only).
        if self.hybrid_functional {
            self.ym2612.hybrid_settle_write_busy();
        }
    }

    fn sync_ym2612(&mut self) {
        if self.cycles.has_ym2612_ticks() {
            let ticks = self.cycles.take_ym2612_ticks();
            self.ym2612.tick(ticks);
        }
    }
}

// If a long DMA is in progress (i.e. the DMA will not finish on this line), preemptively skip the
// 68000 forward by a large number of mclk cycles (up to 1250).
//
// This function is public so that it can be used by the Sega CD core
#[inline]
fn check_for_long_dma_skip(vdp: &Vdp, cycles: &mut CycleCounters) {
    if !vdp.long_halting_dma_in_progress() {
        return;
    }

    if !cycles.z80_halt {
        // Don't advance for very long time slices if the Z80 is still active; doing so causes
        // video/audio desync in Overdrive 2.
        // 8 68K cycles is slightly less than 4 Z80 cycles
        cycles.m68k_wait_cpu_cycles = 8;
        return;
    }

    // Skip as close as possible to the end of the current scanline
    let wait_cycles = cmp::max(
        cycles.m68k_wait_cpu_cycles,
        cmp::min(
            cycles.max_wait_cpu_cycles,
            (vdp::MCLK_CYCLES_PER_SCANLINE - vdp.scanline_mclk()) as u32 / cycles.m68k_divider_u32,
        ),
    );
    cycles.m68k_wait_cpu_cycles = wait_cycles;

    log::trace!(
        "Skipping {wait_cycles} 68000 CPU cycles in long DMA optimization, scanline mclk is {}",
        vdp.scanline_mclk()
    );
}

impl m68000_emu::BusInterface for GenesisBus {
    type DebugView<'a>
        = DummyM68000Debugger
    where
        Self: 'a;

    #[inline]
    fn read_byte(&mut self, address: u32) -> u8 {
        self.m68k_read::<false>(address) as u8
    }

    #[inline]
    fn read_word(&mut self, address: u32) -> u16 {
        self.m68k_read::<true>(address)
    }

    // TJE hybrid: preserve read purpose in the optional observation only. The
    // exact same physical access/latch/open-bus side effects always take place.
    fn read_byte_tagged(&mut self, address: u32, purpose: m68000_emu::ReadPurpose) -> u8 {
        self.m68k_read_with_purpose::<false>(address, purpose) as u8
    }

    fn read_word_tagged(&mut self, address: u32, purpose: m68000_emu::ReadPurpose) -> u16 {
        self.m68k_read_with_purpose::<true>(address, purpose)
    }

    #[inline]
    fn write_byte(&mut self, address: u32, value: u8) {
        #[cfg(feature = "reference-proof-hooks")]
        if let Some(survey) = &mut self.timing_survey {
            survey.reference_proof.cpu_access();
            if survey.reference_proof.reset_capture_active() {
                survey.reference_proof.reset_access(
                    crate::reference_proof::ResetAccess::WriteIssued {
                        address: address & 0xffffff,
                        word: false,
                        value: value.into(),
                    },
                );
            }
        }
        if let Some(trace) = &mut self.native_cpu_accesses {
            trace.push(crate::native::CpuAccess::Write {
                address: address & 0xFFFFFF,
                word: false,
                value: value.into(),
            });
        }
        self.pending_writes.byte.push((address, value));
    }

    #[inline]
    fn write_word(&mut self, address: u32, value: u16) {
        #[cfg(feature = "reference-proof-hooks")]
        if let Some(survey) = &mut self.timing_survey {
            survey.reference_proof.cpu_access();
            if survey.reference_proof.reset_capture_active() {
                survey.reference_proof.reset_access(
                    crate::reference_proof::ResetAccess::WriteIssued {
                        address: address & 0xffffff,
                        word: true,
                        value,
                    },
                );
            }
        }
        if let Some(trace) = &mut self.native_cpu_accesses {
            trace.push(crate::native::CpuAccess::Write {
                address: address & 0xFFFFFF,
                word: true,
                value,
            });
        }
        self.pending_writes.word.push((address, value));
    }

    #[inline]
    fn interrupt_level(&self) -> u8 {
        self.vdp.m68k_interrupt_level()
    }

    #[inline]
    fn acknowledge_interrupt(&mut self, interrupt_level: u8) {
        #[cfg(feature = "reference-proof-hooks")]
        if let Some(survey) = &mut self.timing_survey
            && survey.reference_proof.reset_capture_active()
        {
            survey.reference_proof.reset_access(
                crate::reference_proof::ResetAccess::AcknowledgeInterrupt {
                    level: interrupt_level,
                },
            );
        }
        if let Some(trace) = &mut self.native_cpu_accesses {
            trace.push(crate::native::CpuAccess::AcknowledgeInterrupt { level: interrupt_level });
        }
        self.timing_interrupt(interrupt_level);
        // When the 68000 acknowledges a VDP interrupt, the VDP acknowledges whatever level it is
        // currently raising rather than paying attention to the 68000's IACK lines. This is noted
        // in official documentation which describes this hardware bug: If both HINT are VINT are
        // enabled and the 68000 executes a long instruction right before HINT would trigger on line
        // 224, it's possible for the following sequence of events to happen:
        //   1. The VDP sets IPL2 to indicate a level 4 interrupt (HINT)
        //   2. After the 68000 finishes its long instruction, it begins to handle the level 4 interrupt
        //   3. Before the acknowledge, VBlank begins and the VDP sets IPL2+IPL1 for level 6 interrupt (VINT)
        //   4. The 68000 sets its VPA signal to acknowledge the interrupt, and the VDP acknowledges VINT instead of HINT
        //   5. After the 68000 returns from its HINT handler, it immediately handles the HINT a second time (having missed VINT)
        self.vdp.acknowledge_m68k_interrupt();
    }

    #[inline]
    fn observe_exception_boundary(&mut self, boundary: m68000_emu::ExceptionBoundary) {
        if let Some(capture) = &mut self.hybrid_dma_capture {
            capture.observe_boundary(self.cycles.m68k_mclk_cycles, boundary);
        }
        if let (Some(accesses), Some(boundaries)) =
            (&mut self.hybrid_cpu_accesses, &mut self.hybrid_exception_boundaries)
        {
            let index = u32::try_from(boundaries.len()).expect("exception observation index");
            boundaries.push(HybridExceptionBoundary {
                master_clock: self.cycles.m68k_mclk_cycles,
                boundary,
            });
            accesses.push(HybridAccess::ExceptionBoundary { index });
        }
    }

    #[inline]
    fn halt(&self) -> bool {
        self.vdp.should_halt_cpu() || self.timing_halt()
    }

    #[inline]
    fn reset(&self) -> bool {
        self.m68k_reset
    }
}

impl z80_emu::BusInterface for GenesisBus {
    type DebugView<'a>
        = DummyZ80Debugger
    where
        Self: 'a;

    #[inline]
    #[allow(clippy::match_same_arms)]
    fn read_memory(&mut self, address: u16) -> u8 {
        log::trace!("Z80 bus read from {address:04X}");

        let value = match address {
            0x0000..=0x3FFF => {
                // Z80 RAM (mirrored at $2000-$3FFF)
                self.memory.read_audio_ram(address)
            }
            0x4000..=0x5FFF => {
                // YM2612 registers/ports (mirrored every 4 addresses)
                self.sync_ym2612();
                if self.hybrid_functional {
                    self.ym2612.hybrid_settle_write_busy();
                }
                self.ym2612.read_register(address)
            }
            0x6000..=0x60FF => {
                // Bank number register
                // TODO what should this do on reads?
                0xFF
            }
            0x6100..=0x7EFF => {
                // Unused address space
                0xFF
            }
            0x7F00..=0x7F1F => {
                // VDP ports
                self.cycles.record_z80_68k_bus_access();
                self.read_vdp::<false>(address.into()) as u8
            }
            0x7F20..=0x7FFF => {
                // Invalid addresses
                0xFF
            }
            0x8000..=0xFFFF => {
                self.cycles.record_z80_68k_bus_access();

                let m68k_addr = self.z80_bank.map_to_68k_address(address);
                match m68k_addr {
                    0xA00000..=0xA0FFFF => {
                        // TODO this should lock up the system
                        log::error!(
                            "Z80 attempted to read its own memory from the 68k bus; z80_addr={address:04X}, m68k_addr={m68k_addr:08X}"
                        );
                        0xFF
                    }
                    0xE00000..=0xFFFFFF => {
                        // Z80 cannot read from 68000 working RAM
                        // TODO should probably return Z80 open bus instead?
                        0xFF
                    }
                    // TJE native: a Z80 bank fetch is not a CPU-issued transaction.
                    _ => {
                        self.m68k_read_inner::<false>(m68k_addr, m68000_emu::ReadPurpose::Semantic)
                            as u8
                    }
                }
            }
        };
        self.timing_audio_access(address, false, value);
        value
    }

    #[inline]
    fn write_memory(&mut self, address: u16, value: u8) {
        log::trace!("Z80 bus write at {address:04X}");
        self.timing_audio_access(address, true, value);

        match address {
            0x0000..=0x3FFF => {
                // Z80 RAM (mirrored at $2000-$3FFF)
                #[cfg(feature = "reference-proof-hooks")]
                if address & 0x1fff == 0x41
                    && let Some(survey) = &mut self.timing_survey
                {
                    survey
                        .reference_proof
                        .check_unclaimed(&mut self.memory, self.cycles.m68k_mclk_cycles);
                }
                self.memory.write_audio_ram(address, value);
                #[cfg(feature = "reference-proof-hooks")]
                if address & 0x1fff == 0x41
                    && let Some(survey) = &mut self.timing_survey
                {
                    survey
                        .reference_proof
                        .physical_audio_write(&mut self.memory, self.cycles.m68k_mclk_cycles);
                }
            }
            0x4000..=0x5FFF => {
                self.timing_write_ym(address, value);
            }
            0x6000..=0x60FF => {
                self.z80_bank.write_bit(value.bit(0));
            }
            0x6100..=0x7EFF | 0x7F20..=0x7FFF => {
                // Unused / invalid addresses
                // TODO writes to $7F20-$7FFF should halt the system
            }
            0x7F00..=0x7F1F => {
                // VDP addresses
                self.cycles.record_z80_68k_bus_access();
                self.write_vdp_psg::<false>(address.into(), value.into());
            }
            0x8000..=0xFFFF => {
                self.cycles.record_z80_68k_bus_access();

                let m68k_addr = self.z80_bank.map_to_68k_address(address);
                if !(0xA00000..=0xA0FFFF).contains(&m68k_addr) {
                    self.apply_write::<false>(m68k_addr, value.into(), WriteActor::Z80);
                } else {
                    // TODO this should lock up the system
                    log::error!(
                        "Z80 attempted to write to its own memory from the 68k bus; z80_addr={address:04X}, m68k_addr={m68k_addr:08X}, value={value:02X}"
                    );
                }
            }
        }
    }

    #[inline]
    fn read_io(&mut self, _address: u16) -> u8 {
        // I/O ports are not wired up to the Z80
        0xFF
    }

    #[inline]
    fn write_io(&mut self, _address: u16, _value: u8) {
        // I/O ports are not wired up to the Z80
    }

    #[inline]
    fn nmi(&self) -> InterruptLine {
        // The NMI line is not connected to anything
        InterruptLine::High
    }

    #[inline]
    fn int(&self) -> InterruptLine {
        self.vdp.z80_interrupt_line()
    }

    #[inline]
    fn busreq(&self) -> bool {
        self.z80_signals.busreq
    }

    #[inline]
    fn reset(&self) -> bool {
        self.z80_signals.reset
    }
}

#[cfg(test)]
mod checkpoint_tests {
    use super::*;

    #[test]
    fn pending_and_stall_checkpoint_never_flushes_or_changes_any_state() {
        let mut bus = GenesisBus::new(TimingMode::Ntsc, None, &GenesisEmulatorConfig::default());
        bus.pending_writes.byte.push((0xff1001, 0xab));
        bus.pending_writes.word.push((0xff1002, 0xcdef));
        bus.cycles.m68k_wait_cpu_cycles = 333;
        bus.cycles.m68k_wait_counter = 9;
        bus.cycles.m68k_refresh_counter = 107;
        bus.cycles.vdp_owns_bus = true;
        bus.cycles.z80_halt = true;
        bus.m68k_reset = false;
        bus.z80_signals = Z80Signals { busreq: true, reset: false };
        bus.hybrid_accesses = Some(Vec::new());
        bus.hybrid_cpu_accesses = Some(Vec::new());
        let before = bincode::encode_to_vec(&bus, bincode::config::standard()).unwrap();
        for _ in 0..8 {
            let checkpoint = bus.hybrid_checkpoint();
            assert_eq!(checkpoint.pending_byte_writes, 1);
            assert_eq!(checkpoint.pending_word_writes, 1);
            assert_eq!(checkpoint.m68k_wait_cpu_cycles, 333);
            assert_eq!(checkpoint.m68k_wait_counter, 9);
            assert_eq!(checkpoint.m68k_refresh_counter, 107);
            assert!(checkpoint.vdp_owns_bus);
            assert!(checkpoint.z80_halt);
            assert!(!checkpoint.m68k_reset);
            assert!(checkpoint.z80_bus_requested);
            assert!(!checkpoint.z80_reset);
            assert_eq!(checkpoint.interrupt_level, m68000_emu::BusInterface::interrupt_level(&bus));
            assert_eq!(checkpoint.cpu_halt, m68000_emu::BusInterface::halt(&bus));
        }
        assert_eq!(bus.memory.read_main_ram::<true>(0xff1002), 0);
        assert_eq!(before, bincode::encode_to_vec(&bus, bincode::config::standard()).unwrap());
    }

    #[test]
    fn checkpoint_reports_real_memory_dma_halt_without_completing_it() {
        let mut bus = GenesisBus::new(TimingMode::Ntsc, None, &GenesisEmulatorConfig::default());
        for value in [0x8114, 0x9301, 0x9400, 0x9500, 0x9600, 0x977f, 0x4000, 0x0080] {
            bus.vdp.write_control(value);
        }
        assert!(bus.vdp.timing_dma_active());
        let before = bincode::encode_to_vec(&bus, bincode::config::standard()).unwrap();
        for _ in 0..8 {
            assert!(bus.hybrid_checkpoint().cpu_halt);
        }
        assert_eq!(before, bincode::encode_to_vec(&bus, bincode::config::standard()).unwrap());
    }
}

#[cfg(test)]
mod hybrid_control_tests {
    use super::*;
    use std::num::{NonZeroU32, NonZeroU64};

    fn bus() -> GenesisBus {
        GenesisBus::new(TimingMode::Ntsc, None, &GenesisEmulatorConfig::default())
    }

    fn control(bus: &GenesisBus) -> Vec<u8> {
        bincode::encode_to_vec(
            bus.hybrid_control_without_vdp_main_ram(),
            bincode::config::standard(),
        )
        .unwrap()
    }

    fn probe_control(bus: &GenesisBus) -> Vec<u8> {
        bincode::encode_to_vec(
            bus.hybrid_control_without_vdp_main_ram_cpu_journal(),
            bincode::config::standard(),
        )
        .unwrap()
    }

    #[test]
    fn every_included_bus_field_retains_its_control_and_observation_state() {
        type Mutation = (&'static str, fn(&mut GenesisBus));
        let mutations: &[Mutation] = &[
            ("cartridge", |b| b.cartridge = Some(Cartridge::new(vec![0; 1024], None, None, &[]))),
            ("audio_ram", |b| b.memory.write_audio_ram(0, 1)),
            ("psg", |b| b.psg.write(0x91)),
            ("ym", |b| b.ym2612.write_address_1(0x2a)),
            ("input", |b| b.input.write_p1_data(0x42)),
            ("z80_bank", |b| b.z80_bank.write_bit(true)),
            ("busreq", |b| b.z80_signals.busreq = true),
            ("reset", |b| b.z80_signals.reset = false),
            ("pending_bytes", |b| b.pending_writes.byte.push((0xff0010, 7))),
            ("pending_words", |b| b.pending_writes.word.push((0xff0010, 7))),
            ("divider64", |b| b.cycles.m68k_divider = NonZeroU64::new(8).unwrap()),
            ("divider32", |b| b.cycles.m68k_divider_u32 = NonZeroU32::new(8).unwrap()),
            ("max_wait", |b| b.cycles.max_wait_cpu_cycles ^= 1),
            ("wait", |b| b.cycles.m68k_wait_cpu_cycles ^= 1),
            ("rounding", |b| b.cycles.m68k_wait_counter ^= 1),
            ("m68k_clock", |b| b.cycles.m68k_mclk_cycles ^= 1),
            ("z80_clock", |b| b.cycles.z80_mclk_cycles ^= 1),
            ("ym_clock", |b| b.cycles.ym2612_mclk_cycles ^= 1),
            ("ym_drain", |b| b.cycles.last_ym2612_drain_mclk ^= 1),
            ("psg_clock", |b| b.cycles.psg_mclk_cycles ^= 1),
            ("refresh", |b| b.cycles.m68k_refresh_counter ^= 1),
            ("vdp_bus", |b| b.cycles.vdp_owns_bus = true),
            ("z80_halt", |b| b.cycles.z80_halt = true),
            ("timing_mode", |b| b.timing_mode = TimingMode::Pal),
            ("open_bus", |b| b.open_bus ^= 1),
            ("opcode", |b| b.m68k_opcode ^= 1),
            ("m68k_reset", |b| b.m68k_reset = !b.m68k_reset),
            ("hybrid_writes", |b| b.hybrid_writes = Some(Vec::new())),
            ("hybrid_accesses", |b| b.hybrid_accesses = Some(Vec::new())),
            ("survey", |b| b.timing_survey = Some(crate::timing_survey::TimingSurvey::default())),
            ("native_writes", |b| b.native_writes = Some(Vec::new())),
            ("native_cpu", |b| b.native_cpu_accesses = Some(Vec::new())),
            ("native_ram", |b| b.native_ram_writes = Some(Vec::new())),
            ("hybrid_cpu", |b| b.hybrid_cpu_accesses = Some(Vec::new())),
            ("boundaries", |b| b.hybrid_exception_boundaries = Some(Vec::new())),
            ("all_dma", |b| b.hybrid_dma_all_sources = true),
            ("dma_capture", |b| b.hybrid_arm_dma_capture(0)),
            ("functional", |b| b.hybrid_functional = true),
            ("violation", |b| b.hybrid_functional_violation = Some(7)),
        ];
        let baseline = control(&bus());
        let probe_baseline = probe_control(&bus());
        for &(name, mutate) in mutations {
            let mut changed = bus();
            mutate(&mut changed);
            assert_ne!(baseline, control(&changed), "{name}");
            assert_ne!(probe_baseline, probe_control(&changed), "{name}");
        }
    }

    #[test]
    fn excluded_ram_and_vdp_still_require_their_separate_comparisons() {
        let mut bus = bus();
        let before = bincode::encode_to_vec(&bus, bincode::config::standard()).unwrap();
        let baseline = control(&bus);
        assert_eq!(before, bincode::encode_to_vec(&bus, bincode::config::standard()).unwrap());
        bus.memory.write_main_ram_word(0xff0010, 0xabcd);
        bus.vdp.write_control(0x4000);
        assert_eq!(baseline, control(&bus));
        assert_ne!(before, bincode::encode_to_vec(&bus, bincode::config::standard()).unwrap());
    }
}

#[cfg(test)]
mod dma_capture_tests {
    use super::*;
    use m68000_emu::BusInterface;

    fn bus() -> GenesisBus {
        GenesisBus::new(TimingMode::Ntsc, None, &GenesisEmulatorConfig::default())
    }

    fn prepare(bus: &mut GenesisBus, depth: u32) {
        bus.enable_native_write_trace();
        bus.timing_survey.as_mut().unwrap().pc = 0x100;
        bus.hybrid_arm_dma_capture(depth);
    }

    fn start(bus: &mut GenesisBus) {
        for value in [0x8114, 0x9301, 0x9400, 0x9500, 0x9600, 0x977f, 0x4000, 0x0080] {
            bus.write_word(0xc00004, value);
            bus.apply_pending_writes();
        }
    }

    #[test]
    fn unarmed_success_markers_allocate_nothing_and_change_no_bus_state() {
        let mut bus = bus();
        let before = bincode::encode_to_vec(&bus, bincode::config::standard()).unwrap();
        bus.observe_exception_boundary(m68000_emu::ExceptionBoundary::Entered {
            vector: 30,
            handler: 0x300,
            depth: 1,
        });
        bus.observe_exception_boundary(m68000_emu::ExceptionBoundary::Returned { depth: 0 });
        assert!(bus.hybrid_dma_capture().is_none());
        assert_eq!(before, bincode::encode_to_vec(&bus, bincode::config::standard()).unwrap());
    }

    #[test]
    fn actual_dispatch_records_relative_root_depth_without_index_logger() {
        let mut bus = bus();
        prepare(&mut bus, 1);
        assert!(bus.hybrid_cpu_accesses.is_none());
        assert!(bus.hybrid_exception_boundaries.is_none());
        start(&mut bus);
        let capture = bus.hybrid_dma_capture().unwrap();
        assert_eq!(capture.starts.len(), 1);
        assert_eq!(capture.starts[0].entry_exception_depth, 1);
        assert_eq!(capture.starts[0].exception_depth, 1);
        assert_eq!(capture.starts[0].pc, 0x100);
        assert_eq!(capture.starts[0].origin, crate::native::Origin::Main);
        assert!(!capture.invalid_depth_sequence);
    }

    #[test]
    fn successful_return_updates_scope_before_later_same_clock_device_dispatch() {
        let mut bus = bus();
        prepare(&mut bus, 0);
        bus.observe_exception_boundary(m68000_emu::ExceptionBoundary::Entered {
            vector: 28,
            handler: 0x300,
            depth: 1,
        });
        bus.observe_exception_boundary(m68000_emu::ExceptionBoundary::Returned { depth: 0 });
        start(&mut bus);
        let capture = bus.hybrid_dma_capture().unwrap();
        assert_eq!(capture.boundaries.len(), 2);
        assert_eq!(capture.starts[0].exception_depth, 0);
        assert_eq!(capture.starts[0].master_clock, capture.boundaries[1].master_clock);
        assert!(!capture.invalid_depth_sequence);
    }

    #[test]
    fn out_of_order_or_missing_scope_context_is_latched_unknown_not_guessed() {
        let mut bus = bus();
        prepare(&mut bus, 0);
        bus.observe_exception_boundary(m68000_emu::ExceptionBoundary::Returned { depth: 0 });
        start(&mut bus);
        assert!(bus.hybrid_dma_capture().unwrap().invalid_depth_sequence);
        let mut missing = self::bus();
        missing.hybrid_arm_dma_capture(0);
        start(&mut missing);
        let capture = missing.hybrid_dma_capture().unwrap();
        assert!(capture.missing_dispatch_context);
        assert!(capture.starts.is_empty());
    }

    #[test]
    fn byte_and_word_active_dma_port_writes_cannot_hide_internal_chaining() {
        let mut word = bus();
        prepare(&mut word, 0);
        start(&mut word);
        word.write_word(0xc00004, 0xc000);
        word.apply_pending_writes();
        let capture = word.hybrid_dma_capture().unwrap();
        assert!(capture.active_dma_port_write_seen);
        assert!(capture.deferred_dma_port_write_seen);

        let mut byte = bus();
        prepare(&mut byte, 0);
        start(&mut byte);
        byte.write_byte(0xc00000, 7);
        byte.apply_pending_writes();
        assert!(byte.hybrid_dma_capture().unwrap().deferred_dma_port_write_seen);

        let mut z80 = bus();
        prepare(&mut z80, 0);
        start(&mut z80);
        z80.timing_survey.as_mut().unwrap().cpu = crate::timing_survey::TimingCpu::Z80;
        z80_emu::BusInterface::write_memory(&mut z80, 0x7f00, 7);
        assert!(z80.hybrid_dma_capture().unwrap().deferred_dma_port_write_seen);
    }
}

#[cfg(all(test, not(feature = "reference-proof-hooks")))]
mod read_context_feature_off_tests {
    use super::*;
    #[test]
    fn ordinary_bus_mask_inspection_is_pure_without_proof_feature_or_observer() {
        let bus = GenesisBus::new(TimingMode::Ntsc, None, &GenesisEmulatorConfig::default());
        let before = bincode::encode_to_vec(&bus, bincode::config::standard()).unwrap();
        assert_eq!(bus.hybrid_read_driven_mask(0x1c00004, true), 0x03ff);
        assert_eq!(bus.hybrid_read_driven_mask(0xc00005, false), 0xff);
        assert_eq!(bus.hybrid_read_driven_mask(0xa11100, true), 0x0100);
        assert_eq!(bus.hybrid_read_driven_mask(0xa00000, false), 0);
        assert!(bus.native_cpu_accesses.is_none());
        assert_eq!(before, bincode::encode_to_vec(&bus, bincode::config::standard()).unwrap());
    }
}
