//! Private-evidence helpers for the external lane proof binaries only.
use tje_host::{Host, input::InputScript};

pub(super) fn settle(host: &mut Host, script: &InputScript) -> serde_json::Value {
    let boundary = host
        .settle_fixture_boundary(script.at(host.frame_count()))
        .expect("settle original before any fixture mutation");
    assert!(host.fixture_boundary_is_settled());
    let frames: Vec<_> = boundary.completed_frames.iter().map(|frame| serde_json::json!({
        "frame": frame.frame, "frame_counter": frame.frame_counter, "pc": frame.pc,
        "ram": &*frame.ram,
        "vdp": {"vram": &*frame.vdp.vram, "cram": &*frame.vdp.cram, "vsram": &*frame.vdp.vsram, "registers": frame.vdp.registers},
        "writes": frame.writes, "audio_bits": frame.audio_samples.iter().map(|sample| sample.map(f64::to_bits)).collect::<Vec<_>>(),
        "idle_random_draws": frame.idle_random_draws, "lag_events": frame.lag_events
    })).collect();
    serde_json::json!({"start_pc":boundary.start_pc,"end_pc":boundary.end_pc,"start_frame":boundary.start_frame,"end_frame":boundary.end_frame,"ticks":boundary.ticks,"master_clocks":boundary.master_clocks,"handlers_completed":boundary.handlers_completed,"completed_frames":frames})
}
