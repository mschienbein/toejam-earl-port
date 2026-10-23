//! Cycle counting and wait state tracking for the Genesis hardware

use bincode::{Decode, Encode};
use genesis_components::vdp;
use std::num::{NonZeroU32, NonZeroU64};
use std::{cmp, mem};

pub const Z80_DIVIDER: u64 = 15;
pub const YM2612_DIVIDER: u64 = 7 * 6;
pub const PSG_DIVIDER: u64 = 15;

// Sync the YM2612 at least once per scanline
pub const MAX_YM2612_LAG_MCLK: u64 = vdp::MCLK_CYCLES_PER_SCANLINE;

pub const REFRESH_INTERVAL: u32 = 128;

#[derive(Debug, Clone, Copy, Encode, Decode)]
pub struct CycleCounters {
    // Store divider as both u64 and u32 for better codegen when doing u32 division
    pub m68k_divider: NonZeroU64,
    pub m68k_divider_u32: NonZeroU32,
    pub max_wait_cpu_cycles: u32,
    pub m68k_wait_cpu_cycles: u32,
    pub m68k_wait_counter: u8,
    pub m68k_mclk_cycles: u64,
    pub z80_mclk_cycles: u64,
    pub ym2612_mclk_cycles: u64,
    pub last_ym2612_drain_mclk: u64,
    pub psg_mclk_cycles: u64,
    pub m68k_refresh_counter: u32,
    pub vdp_owns_bus: bool,
    pub z80_halt: bool,
}

fn max_wait_cpu_cycles(m68k_divider: NonZeroU64) -> u32 {
    // Executing for too many cycles at a time breaks assumptions in the VDP code, checked via an
    // assert in Vdp::tick()
    const MAX_WAIT_MCLK_CYCLES: u32 = 1225;

    MAX_WAIT_MCLK_CYCLES / m68k_divider.get() as u32
}

impl CycleCounters {
    #[inline]
    #[must_use]
    #[allow(clippy::missing_panics_doc)]
    pub fn new(m68k_divider: NonZeroU64) -> Self {
        let m68k_divider_u32 = NonZeroU32::new(m68k_divider.get() as u32).unwrap();
        let max_wait_cpu_cycles = max_wait_cpu_cycles(m68k_divider);

        Self {
            m68k_divider,
            m68k_divider_u32,
            max_wait_cpu_cycles,
            m68k_wait_cpu_cycles: 0,
            m68k_wait_counter: 0,
            m68k_mclk_cycles: 0,
            z80_mclk_cycles: 0,
            ym2612_mclk_cycles: 0,
            last_ym2612_drain_mclk: 0,
            psg_mclk_cycles: 0,
            m68k_refresh_counter: 0,
            vdp_owns_bus: false,
            z80_halt: false,
        }
    }

    #[inline]
    #[allow(clippy::missing_panics_doc)]
    pub fn update_m68k_divider(&mut self, m68k_divider: NonZeroU64) {
        self.m68k_divider = m68k_divider;
        self.m68k_divider_u32 = NonZeroU32::new(m68k_divider.get() as u32).unwrap();
        self.max_wait_cpu_cycles = max_wait_cpu_cycles(m68k_divider);

        self.m68k_wait_cpu_cycles = cmp::min(self.m68k_wait_cpu_cycles, self.max_wait_cpu_cycles);
    }

    #[inline]
    pub fn take_m68k_wait_cpu_cycles(&mut self) -> u32 {
        if self.m68k_wait_cpu_cycles > self.max_wait_cpu_cycles {
            self.m68k_wait_cpu_cycles -= self.max_wait_cpu_cycles;
            return self.max_wait_cpu_cycles;
        }

        mem::take(&mut self.m68k_wait_cpu_cycles)
    }

    #[inline]
    #[must_use]
    pub fn record_68k_instruction(
        &mut self,
        m68k_pc: u32,
        m68k_cycles: u32,
        m68k_wait: bool,
        vdp_owns_bus: bool,
    ) -> u64 {
        const REGION_WAIT_CYCLES: [u32; 8] = [
            2, // $000000-$1FFFFF
            2, // $200000-$3FFFFF
            2, // $400000-$5FFFFF
            2, // $600000-$7FFFFF
            0, // $800000-$9FFFFF
            0, // $A00000-$BFFFFF
            0, // $C00000-$DFFFFF
            3, // $E00000-$FFFFFF
        ];

        // Track memory refresh delay, which stalls the 68000 for roughly 2 out of every 130 CPU cycles
        // when executing from ROM (at least on the standalone Genesis)
        // Clue and Super Airwolf depend on this or they will have graphical glitches, Clue in the
        // main menu and Super Airwolf in the intro
        //
        // TODO this implementation is only approximate, e.g. ROM and RAM refreshes are not aligned
        // and it's possible for the CPU to trigger both independently. A more accurate implementation
        // requires more accurate 68000 timing, in particular tracking the exact cycles on which
        // memory accesses occur
        if vdp_owns_bus {
            // Not sure this is accurate, but it's required for stable images in Direct Color DMA demos
            self.m68k_refresh_counter = 0;
        } else {
            self.m68k_refresh_counter += m68k_cycles;
            if self.m68k_refresh_counter >= REFRESH_INTERVAL {
                if !m68k_wait {
                    let wait_cycles = REGION_WAIT_CYCLES[((m68k_pc >> 21) & 7) as usize];
                    self.m68k_wait_cpu_cycles += wait_cycles;
                }
                self.m68k_refresh_counter %= REFRESH_INTERVAL;
            }
        }

        let mclk_cycles = u64::from(m68k_cycles) * self.m68k_divider.get();
        self.increment_mclk_counters(mclk_cycles, vdp_owns_bus);

        mclk_cycles
    }

    #[inline]
    pub fn increment_mclk_counters(&mut self, mclk_cycles: u64, vdp_owns_bus: bool) {
        self.vdp_owns_bus = vdp_owns_bus;
        self.z80_halt &= vdp_owns_bus;

        self.m68k_mclk_cycles += mclk_cycles;
        if self.z80_halt {
            self.z80_mclk_cycles += mclk_cycles;
        }
    }

    #[inline]
    pub fn record_z80_68k_bus_access(&mut self) {
        // Each time the Z80 accesses the 68K bus, the Z80 is stalled for on average 3 Z80 cycles
        // and the 68K is stalled for on average slightly less than 9.7 68K cycles (based on test ROM)
        self.z80_mclk_cycles += 3 * Z80_DIVIDER;

        // The Z80 should halt if it accesses the 68K bus during a VDP DMA or while the 68K is
        // stalled on a VDP FIFO write
        self.z80_halt |= self.vdp_owns_bus;

        if !self.vdp_owns_bus {
            // Not sure if it's accurate for this to be conditional, but adding this delay after
            // a VDP DMA breaks some effects in Overdrive
            self.m68k_wait_cpu_cycles += 9 + u32::from(self.m68k_wait_counter < 7);
            self.m68k_wait_counter += 1;
            if self.m68k_wait_counter == 10 {
                self.m68k_wait_counter = 0;
            }
        }
    }

    #[inline]
    pub fn record_68k_z80_bus_access(&mut self) {
        // Each time the 68K accesses the Z80 bus, the 68K is stalled for 1 CPU cycle
        // Pac-Man 2: The New Adventures depends on this for its audio code to work correctly
        self.m68k_wait_cpu_cycles += 1;
    }

    #[inline]
    #[must_use]
    pub fn should_tick_z80(&self) -> bool {
        self.z80_mclk_cycles + Z80_DIVIDER <= self.m68k_mclk_cycles
    }

    #[inline]
    pub fn z80_cycle(&mut self) {
        self.z80_mclk_cycles += Z80_DIVIDER;
    }

    #[inline]
    #[must_use]
    pub fn has_ym2612_ticks(&mut self) -> bool {
        self.ym2612_mclk_cycles + YM2612_DIVIDER <= self.z80_mclk_cycles
    }

    #[inline]
    #[must_use]
    pub fn take_ym2612_ticks(&mut self) -> u32 {
        let ticks = (self.z80_mclk_cycles - self.ym2612_mclk_cycles) / YM2612_DIVIDER;
        self.ym2612_mclk_cycles += ticks * YM2612_DIVIDER;
        ticks as u32
    }

    #[inline]
    #[must_use]
    pub fn should_tick_psg(&self) -> bool {
        self.psg_mclk_cycles + PSG_DIVIDER <= self.m68k_mclk_cycles
    }

    #[inline]
    pub fn psg_cycle(&mut self) {
        self.psg_mclk_cycles += PSG_DIVIDER;
    }

    #[inline]
    pub fn ym2612_sync_needed(&self) -> bool {
        self.last_ym2612_drain_mclk + MAX_YM2612_LAG_MCLK <= self.z80_mclk_cycles
    }
}

// TJE timing survey: opt-in instrumentation, isolated from the clock algorithms.
#[cfg(feature = "emulator")]
pub mod survey {
    //! TJE timing survey: passive, opt-in bus observation and explicit retiming experiments.
    //! No observer is installed during ordinary emulation.

    use crate::bus::GenesisBus;
    use bincode::{Decode, Encode};
    use std::collections::VecDeque;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
    pub enum TimingCpu {
        M68k,
        Z80,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Encode, Decode)]
    pub enum TimingKind {
        VdpData,
        VdpControl,
        VdpCounter,
        VdpDebug,
        DmaStart,
        Ym2612,
        Psg,
        Z80BusRequest,
        Z80Reset,
        Z80Ram,
        Controller,
        Z80Bank,
    }

    #[derive(Debug, Clone, Copy, Encode, Decode)]
    pub struct TimingAccess {
        pub clock: u64,
        pub scanline: u16,
        pub line_clock: u16,
        pub active_lines: u16,
        pub cpu: TimingCpu,
        pub pc: u32,
        /// Nonzero only for 68000 interrupt work, never concurrent Z80 work.
        pub interrupt: u8,
        pub kind: TimingKind,
        pub write: bool,
        pub address: u32,
        pub value: u16,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
    pub enum TimingClass {
        VdpControl,
        VdpData,
        VdpAll,
        Ym2612,
        Psg,
        SoundAll,
    }

    #[derive(Debug, Clone, Copy, Encode, Decode)]
    pub struct TimingPolicy {
        pub class: TimingClass,
        pub scanline: u16,
    }

    #[derive(Debug, Clone, Copy, Encode, Decode)]
    struct DeferredWrite {
        kind: TimingKind,
        address: u32,
        value: u16,
    }

    #[derive(Debug, Clone, Encode, Decode)]
    pub struct TimingSurvey {
        pub accesses: Vec<TimingAccess>,
        pub policy: Option<TimingPolicy>,
        pub retimed_writes: u64,
        pub maximum_delay_clocks: u64,
        pub cpu: TimingCpu,
        pub pc: u32,
        interrupts: Vec<u8>,
        deferred: VecDeque<(DeferredWrite, u64)>,
        draining: bool,
        replaying: bool,
        previous_line: u16,
    }

    impl Default for TimingSurvey {
        fn default() -> Self {
            Self {
                accesses: Vec::new(),
                policy: None,
                retimed_writes: 0,
                maximum_delay_clocks: 0,
                cpu: TimingCpu::M68k,
                pc: 0,
                interrupts: Vec::new(),
                deferred: VecDeque::new(),
                draining: false,
                replaying: false,
                previous_line: 0,
            }
        }
    }

    impl GenesisBus {
        pub(crate) fn timing_m68k(&mut self, pc: u32) {
            if let Some(survey) = &mut self.timing_survey {
                survey.cpu = TimingCpu::M68k;
                survey.pc = pc;
            }
        }

        pub(crate) fn timing_z80(&mut self, pc: u16) {
            if let Some(survey) = &mut self.timing_survey {
                survey.cpu = TimingCpu::Z80;
                survey.pc = pc.into();
            }
        }

        pub(crate) fn timing_interrupt(&mut self, level: u8) {
            if let Some(survey) = &mut self.timing_survey {
                survey.interrupts.push(level);
            }
        }

        pub(crate) fn timing_return(&mut self) {
            if let Some(survey) = &mut self.timing_survey {
                survey.interrupts.pop();
            }
        }

        pub(crate) fn timing_access(
            &mut self,
            kind: TimingKind,
            write: bool,
            address: u32,
            value: u16,
        ) {
            let Some(survey) = &mut self.timing_survey else {
                return;
            };
            if survey.replaying {
                return;
            }
            let clock = match survey.cpu {
                TimingCpu::M68k => self.cycles.m68k_mclk_cycles,
                TimingCpu::Z80 => self.cycles.z80_mclk_cycles,
            };
            // Z80 runs ahead of VDP within one 68000 instruction. Use its own clock,
            // not the stale VDP position, to place concurrent chip traffic.
            let (scanline, line_clock) = match survey.cpu {
                TimingCpu::M68k => (self.vdp.scanline(), self.vdp.scanline_mclk() as u16),
                TimingCpu::Z80 => (((clock / 3420) % 262) as u16, (clock % 3420) as u16),
            };
            survey.accesses.push(TimingAccess {
                clock,
                scanline,
                line_clock,
                active_lines: self.vdp.hybrid_registers().vertical_display_size.active_scanlines(),
                cpu: survey.cpu,
                pc: survey.pc,
                interrupt: if survey.cpu == TimingCpu::M68k {
                    survey.interrupts.last().copied().unwrap_or(0)
                } else {
                    0
                },
                kind,
                write,
                address,
                value,
            });
        }

        pub(crate) fn timing_main_access(&mut self, address: u32, write: bool, value: u16) {
            if self.timing_survey.as_ref().is_none_or(|s| s.cpu != TimingCpu::M68k) {
                return;
            }
            let kind = match address & 0xFF_FFFF {
                0xA00000..=0xA03FFF | 0xA08000..=0xA0BFFF => TimingKind::Z80Ram,
                0xA04000..=0xA05FFF | 0xA0C000..=0xA0DFFF => TimingKind::Ym2612,
                0xA10000..=0xA1001F => TimingKind::Controller,
                0xA11100..=0xA11101 => TimingKind::Z80BusRequest,
                0xA11200..=0xA11201 => TimingKind::Z80Reset,
                0xC00000..=0xC00003 => TimingKind::VdpData,
                0xC00004..=0xC00007 => TimingKind::VdpControl,
                0xC00008..=0xC0000B => TimingKind::VdpCounter,
                0xC00010..=0xC00017 => TimingKind::Psg,
                0xC0001C..=0xC0001D => TimingKind::VdpDebug,
                _ => return,
            };
            self.timing_access(kind, write, address, value);
        }

        pub(crate) fn timing_audio_access(&mut self, address: u16, write: bool, value: u8) {
            if self.timing_survey.as_ref().is_none_or(|s| s.cpu != TimingCpu::Z80) {
                return;
            }
            let kind = match address {
                0x0000..=0x3FFF => TimingKind::Z80Ram,
                0x4000..=0x5FFF => TimingKind::Ym2612,
                0x6000..=0x60FF => TimingKind::Z80Bank,
                0x7F00..=0x7F03 => TimingKind::VdpData,
                0x7F04..=0x7F07 => TimingKind::VdpControl,
                0x7F08..=0x7F0B => TimingKind::VdpCounter,
                0x7F10..=0x7F17 => TimingKind::Psg,
                _ => return,
            };
            self.timing_access(kind, write, address.into(), value.into());
        }

        pub(crate) fn timing_defer(&mut self, kind: TimingKind, address: u32, value: u16) -> bool {
            let Some(survey) = &mut self.timing_survey else {
                return false;
            };
            let Some(policy) = survey.policy else {
                return false;
            };
            let clock = match survey.cpu {
                TimingCpu::M68k => self.cycles.m68k_mclk_cycles,
                TimingCpu::Z80 => self.cycles.z80_mclk_cycles,
            };
            let line = match survey.cpu {
                TimingCpu::M68k => self.vdp.scanline(),
                TimingCpu::Z80 => ((clock / 3420) % 262) as u16,
            };
            if survey.replaying
                || (!survey.draining
                    && line >= self.vdp.hybrid_registers().vertical_display_size.active_scanlines())
            {
                return false;
            }
            let selected = match policy.class {
                TimingClass::VdpControl => kind == TimingKind::VdpControl,
                TimingClass::VdpData => kind == TimingKind::VdpData,
                TimingClass::VdpAll => {
                    matches!(
                        kind,
                        TimingKind::VdpControl | TimingKind::VdpData | TimingKind::VdpDebug
                    )
                }
                TimingClass::Ym2612 => kind == TimingKind::Ym2612,
                TimingClass::Psg => kind == TimingKind::Psg,
                TimingClass::SoundAll => matches!(kind, TimingKind::Ym2612 | TimingKind::Psg),
            };
            if selected {
                survey.deferred.push_back((DeferredWrite { kind, address, value }, clock));
                survey.retimed_writes += 1;
            }
            selected
        }

        pub(crate) fn timing_halt(&self) -> bool {
            self.timing_survey.as_ref().is_some_and(|survey| {
                survey.draining
                    && survey.policy.is_some_and(|policy| {
                        matches!(
                            policy.class,
                            TimingClass::VdpControl | TimingClass::VdpData | TimingClass::VdpAll
                        )
                    })
            })
        }

        pub(crate) fn timing_flush(&mut self) {
            let Some(survey) = &mut self.timing_survey else {
                return;
            };
            let line = self.vdp.scanline();
            let crossed = line != survey.previous_line;
            survey.previous_line = line;
            let Some(policy) = survey.policy else {
                return;
            };
            if crossed && line == policy.scanline {
                survey.draining = !survey.deferred.is_empty();
            }
            if !survey.draining {
                return;
            }
            survey.replaying = true;
            loop {
                let Some(&(write, issued)) = self.timing_survey.as_ref().unwrap().deferred.front()
                else {
                    break;
                };
                let vdp_write = matches!(
                    write.kind,
                    TimingKind::VdpData | TimingKind::VdpControl | TimingKind::VdpDebug
                );
                // A burst cannot bypass the real VDP FIFO or DMA bus ownership.
                // Resume on later device ticks, stalling 68000 issue in the meantime.
                if vdp_write && self.vdp.should_halt_cpu() {
                    break;
                }
                let survey = self.timing_survey.as_mut().unwrap();
                survey.deferred.pop_front();
                survey.maximum_delay_clocks =
                    survey.maximum_delay_clocks.max(self.cycles.m68k_mclk_cycles - issued);
                match write.kind {
                    TimingKind::Ym2612 => {
                        self.timing_write_ym(write.address as u16, write.value as u8)
                    }
                    _ => self.write_vdp_psg::<true>(write.address, write.value),
                }
            }
            let survey = self.timing_survey.as_mut().unwrap();
            survey.replaying = false;
            survey.draining = !survey.deferred.is_empty();
        }
    }
}
