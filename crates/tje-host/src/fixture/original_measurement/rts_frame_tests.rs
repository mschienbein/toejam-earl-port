//! Physical frame-edge alignment for the authored successful-RTS regression.
use super::{super::*, encoded, host};
use crate::execution::WaitInputPoller;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

struct MissingNextFrame(Arc<AtomicUsize>);
impl WaitInputPoller for MissingNextFrame {
    fn at(&self, _: u64) -> Result<genesis_config::GenesisInputs> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Err(HostError::Input(
            "next frame must not be polled after measured RTS".into(),
        ))
    }
}

#[test]
fn actual_rts_that_renders_a_frame_returns_before_next_frame_callbacks() {
    // Calibrate only the search window using actual original ticks. No VDP clock,
    // frame flag, CPU readiness or source return result is assigned by the test.
    let mut calibration = host(&[0x4e75]);
    let mut event_clock = None;
    for _ in 0..100_000 {
        let effect = calibration
            .tick_live_original(&mut Inputs(Default::default()))
            .unwrap();
        if effect == TickEffect::FrameRendered {
            event_clock = Some(calibration.emulator.hybrid_bus().cycles.m68k_mclk_cycles);
            break;
        }
    }
    let event_clock = event_clock.expect("authored caller reaches an actual video frame");
    let mut target = host(&[0x4e75]);
    let polls = Arc::new(AtomicUsize::new(0));
    target.set_wait_input_poller(Arc::new(MissingNextFrame(polls.clone())));
    let mut found = false;
    for _ in 0..100_000 {
        let clock = target.emulator.hybrid_bus().cycles.m68k_mclk_cycles;
        if clock + 1024 >= event_clock && target.fixture_boundary_is_settled() {
            let audio = std::mem::take(&mut target.audio.samples);
            let coincides = target
                .isolated_fixture(|probe| {
                    let (caller, sp, entry_sp) = probe.real_fixture_caller(&[0, 0])?;
                    probe.context().write_u32(sp - 4, 0);
                    probe.context().write_u32(sp - 8, 0);
                    probe.context().write_u32(entry_sp, caller);
                    let address = ids::MOVE_DATA_TO_VRAM.metadata().address(probe.revision);
                    let (cpu, bus) = probe.emulator.hybrid_parts();
                    crate::abi::set_sp(cpu, entry_sp);
                    cpu.set_pc(address, bus);
                    cpu.hybrid_enable_instruction_counts();
                    for _ in 0..64 {
                        let before = probe.cpu().hybrid_instruction_counts().unwrap().main_thread;
                        let effect = probe
                            .tick_live_original_counted(&mut Inputs(Default::default()), true)?;
                        let counts = probe.cpu().hybrid_instruction_counts().unwrap();
                        if counts.main_thread == before + 1
                            && counts.last_instruction.is_some_and(|instruction| {
                                instruction.opcode == 0x4e75 && instruction.completed_successfully
                            })
                            && probe.cpu().pc() == caller
                            && probe.cpu().stack_pointer() == entry_sp + 4
                        {
                            return Ok(effect == TickEffect::FrameRendered);
                        }
                        if effect == TickEffect::FrameRendered {
                            return Ok(false);
                        }
                    }
                    Ok(false)
                })
                .unwrap();
            target.audio.samples = audio;
            if coincides {
                found = true;
                break;
            }
        }
        let effect = target
            .tick_live_original(&mut Inputs(Default::default()))
            .unwrap();
        assert_ne!(
            effect,
            TickEffect::FrameRendered,
            "actual RTS alignment must be found before caller crosses frame"
        );
    }
    assert!(found);
    assert_eq!(polls.load(Ordering::Relaxed), 0);
    let before = encoded(&target);
    let audio = target.audio.samples.clone();
    let record = target
        .measure_original_clocked_fixture(ids::MOVE_DATA_TO_VRAM, &[0, 0], super::ceiling())
        .unwrap();
    assert_eq!(record.usage_to_rts.main_thread_instructions, 1);
    assert_eq!(record.usage_to_rts.completed_video_frames, 1);
    assert_eq!(polls.load(Ordering::Relaxed), 0);
    assert_eq!(target.frame, 0);
    assert_eq!(target.audio.samples, audio);
    assert_eq!(encoded(&target), before);
}
