use serde::Serialize;
#[derive(Debug, Default, Clone, Copy, Serialize)]
pub struct ReadableGuardMetrics {
    pub enabled: bool,
    /// Reviewed resource ceiling, not a device FIFO/queue or source bound.
    pub capacity_ceiling: usize,
    pub capacity_hits: u64,
    pub active_owned_dma: u64,
    pub queued_owned_commands: u64,
    pub dma_opened: u64,
    pub dma_completed: u64,
    pub dma_cancelled: u64,
    pub dma_reset_invalidated: u64,
    pub dma_discarded: u64,
    pub dma_failed: u64,
    pub pending_proof_boundaries: u64,
    pub queued_commands_consumed: u64,
    pub queued_commands_discarded: u64,
}
impl From<tje_game::stack_read::DmaMetrics> for ReadableGuardMetrics {
    fn from(m: tje_game::stack_read::DmaMetrics) -> Self {
        Self {
            enabled: true,
            capacity_ceiling: tje_game::stack_read::GUARD_CAPACITY,
            capacity_hits: m.capacity_hits,
            active_owned_dma: m.active,
            queued_owned_commands: m.queued,
            dma_opened: m.opened,
            dma_completed: m.completed,
            dma_cancelled: m.cancelled,
            dma_reset_invalidated: m.reset_invalidated,
            dma_discarded: m.discarded,
            dma_failed: m.failed,
            pending_proof_boundaries: m.pending_proof,
            queued_commands_consumed: m.queued_consumed,
            queued_commands_discarded: m.queued_discarded,
        }
    }
}
