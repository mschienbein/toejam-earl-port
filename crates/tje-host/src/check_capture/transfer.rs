//! Serialization of the audited borrowed checkpoints. This is not a quietness
//! predicate: fractional rounding and refresh phases are not outstanding debt.
use genesis_components::vdp::debug::HybridVdpCheckpoint;
use genesis_core::{GenesisBus, HybridBusCheckpoint};
use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize)]
pub struct TransferCheckpoint {
    pub fifo_len: u8,
    pub pending_fifo_writes: u64,
    pub pending_dma_port_writes: u64,
    pub dma_active: bool,
    pub dma_latency_slots: u8,
    pub data_port_read_wait: bool,
    pub control_word_pending: bool,
    pub pending_byte_writes: u64,
    pub pending_word_writes: u64,
    pub m68k_wait_cpu_cycles: u32,
    /// Fractional 9/10-cycle Z80 bus-stall rounding phase, not debt.
    pub m68k_wait_counter: u8,
    /// CPU refresh phase modulo 128, not outstanding debt.
    pub m68k_refresh_counter: u32,
    pub vdp_owns_bus: bool,
    pub z80_halt: bool,
    pub m68k_reset: bool,
    pub z80_bus_requested: bool,
    pub z80_reset: bool,
    pub interrupt_level: u8,
    pub cpu_halt: bool,
    pub z80_bank: u32,
    pub functional_mode: bool,
    pub experimental_retiming_policy: bool,
}

impl TransferCheckpoint {
    pub(crate) fn capture(bus: &GenesisBus) -> Self {
        let HybridVdpCheckpoint {
            fifo_len,
            pending_fifo_writes,
            pending_dma_port_writes,
            dma_active,
            dma_latency_slots,
            data_port_read_wait,
            control_word_pending,
        } = bus.vdp.hybrid_checkpoint();
        let HybridBusCheckpoint {
            pending_byte_writes,
            pending_word_writes,
            m68k_wait_cpu_cycles,
            m68k_wait_counter,
            m68k_refresh_counter,
            vdp_owns_bus,
            z80_halt,
            m68k_reset,
            z80_bus_requested,
            z80_reset,
            interrupt_level,
            cpu_halt,
        } = bus.hybrid_checkpoint();
        Self {
            fifo_len,
            pending_fifo_writes: pending_fifo_writes as u64,
            pending_dma_port_writes: pending_dma_port_writes as u64,
            dma_active,
            dma_latency_slots,
            data_port_read_wait,
            control_word_pending,
            pending_byte_writes: pending_byte_writes as u64,
            pending_word_writes: pending_word_writes as u64,
            m68k_wait_cpu_cycles,
            m68k_wait_counter,
            m68k_refresh_counter,
            vdp_owns_bus,
            z80_halt,
            m68k_reset,
            z80_bus_requested,
            z80_reset,
            interrupt_level,
            cpu_halt,
            z80_bank: bus.z80_bank.value(),
            functional_mode: bus.hybrid_functional_mode(),
            experimental_retiming_policy: bus
                .timing_survey
                .as_ref()
                .is_some_and(|survey| survey.policy.is_some()),
        }
    }
}
