//! Authored controller fixtures; actual peripheral DMA, no source-body promotion.
use super::*;
use tje_game::{
    Platform as _, ids,
    stack_read::{GUARD_CAPACITY, Reader, UnsupportedReason},
};
use tje_mem::{Bus, Platform};
const SP: u32 = 0xff7f00;
const SOURCE: u32 = 0xff7e00;

fn platform() -> NativePlatform {
    let mut rom = vec![0; 1 << 20];
    rom[..4].copy_from_slice(&SP.to_be_bytes());
    rom[4..8].copy_from_slice(&0x100_u32.to_be_bytes());
    NativePlatform::new(
        AddressSpace::new(rom, Revision::Rev02),
        &[ids::GET_RANDOM_NUMBER],
        InterruptHandlers::default(),
        &GenesisEmulatorConfig::default(),
    )
    .unwrap()
}
fn issue(platform: &mut NativePlatform, source: u32, count: u16, command: u32) {
    for value in [
        0x8114,
        0x8f02,
        0x9300 | (count & 255),
        0x9400 | (count >> 8),
        0x9500 | ((source >> 1) & 255) as u16,
        0x9600 | ((source >> 9) & 255) as u16,
        0x9700 | ((source >> 17) & 127) as u16,
        (command >> 16) as u16,
        command as u16,
    ] {
        platform.write_vdp_control(value);
    }
}
fn advance(platform: &mut NativePlatform, clocks: u64) -> Result<()> {
    platform.devices.advance_master_clocks(clocks);
    Ok(())
}

#[test]
fn vram_cram_vsram_and_zero_count_dma_keep_the_issuing_root_after_return() {
    for (command, count) in [
        (0x40000080, 1),
        (0xc0000080, 1),
        (0x40000090, 1),
        (0x40000080, 0),
    ] {
        let mut platform = platform();
        platform.devices.memory_mut().write_u16(SOURCE, 0xa53c);
        platform
            .run_readable_root(ids::DMA_COPY_WORDS_TO_VRAM, |platform| {
                issue(platform, SOURCE, count, command);
                Ok(())
            })
            .unwrap();
        assert_eq!(platform.readable_dma_metrics().active, 1);
        let mut error = None;
        for _ in 0..1024 {
            if let Err(failure) =
                platform.run_readable_root(ids::GET_RANDOM_NUMBER, |platform| advance(platform, 64))
            {
                error = Some(failure);
                break;
            }
        }
        let error = error
            .expect("actual DMA must reach its first source read")
            .unsupported_stack_read()
            .unwrap();
        assert_eq!(error.root, ids::DMA_COPY_WORDS_TO_VRAM);
        assert_eq!(
            (error.entry_sp, error.address, error.width, error.reader),
            (SP, SOURCE, 2, Reader::VdpDma)
        );
        assert_eq!(error.reason, UnsupportedReason::Unwritten);
        assert_eq!(platform.readable_dma_metrics().failed, 1);
        let failure = platform
            .enter(ids::GET_RANDOM_NUMBER, &[])
            .unwrap_err()
            .unsupported_stack_read()
            .unwrap();
        assert_eq!(
            failure, error,
            "late DMA failure must remain the first cause"
        );
    }
}

#[test]
fn queued_control_word_retains_its_exact_root_until_the_later_start() {
    let mut platform = platform();
    platform
        .run_readable_root(ids::DMA_COPY_WORDS_TO_VRAM, |platform| {
            issue(platform, 0, 0, 0x40000080);
            Ok(())
        })
        .unwrap();
    platform
        .run_readable_root(ids::UPLOAD_QUEUED_SPRITE_GRAPHICS, |platform| {
            issue(platform, SOURCE, 1, 0x40000080);
            Ok(())
        })
        .unwrap();
    assert!(platform.readable_dma_metrics().queued > 0);
    let mut failure = None;
    for _ in 0..8192 {
        if let Err(error) =
            platform.run_readable_root(ids::GET_RANDOM_NUMBER, |platform| advance(platform, 512))
        {
            failure = Some(error);
            break;
        }
    }
    let error = failure
        .expect("the actual queued DMA must start")
        .unsupported_stack_read()
        .unwrap();
    assert_eq!(error.root, ids::UPLOAD_QUEUED_SPRITE_GRAPHICS);
    assert_eq!(error.address, SOURCE);
    let metrics = platform.readable_dma_metrics();
    assert_eq!(metrics.opened, 2);
    assert_eq!(metrics.completed, 1);
    assert_eq!(metrics.active, 1);
    assert_eq!(metrics.queued, 0);
    assert!(metrics.queued_consumed > 0);
}

#[test]
fn warm_pending_dma_then_leaf_root_uses_no_new_guard_or_device_allocation() {
    let mut platform = platform();
    platform.devices.memory_mut().write_u16(0xfff000, 0xa53c);
    for measured in [false, true] {
        if measured {
            crate::allocation_counter::ALLOCATIONS.with(|counter| counter.set((true, 0, 0)));
        }
        platform
            .run_readable_root(ids::DMA_COPY_WORDS_TO_VRAM, |platform| {
                issue(platform, 0xfff000, 1, 0x40000080);
                Ok(())
            })
            .unwrap();
        for _ in 0..128 {
            platform.enter(ids::GET_RANDOM_NUMBER, &[]).unwrap();
            platform
                .run_readable_root(ids::GET_RANDOM_NUMBER, |platform| advance(platform, 32))
                .unwrap();
        }
        if measured {
            let (calls, bytes) = crate::allocation_counter::ALLOCATIONS.with(|counter| {
                let (_, calls, bytes) = counter.get();
                counter.set((false, 0, 0));
                (calls, bytes)
            });
            assert_eq!((calls, bytes), (0, 0));
        }
        let metrics = platform.readable_dma_metrics();
        assert_eq!(metrics.active, 0);
        assert_eq!(metrics.queued, 0);
        assert_eq!(metrics.failed, 0);
        assert_eq!(metrics.pending_proof, 0);
    }
    assert_eq!(platform.readable_dma_metrics().opened, 2);
    assert_eq!(platform.readable_dma_metrics().completed, 2);
}

#[test]
fn resource_ceiling_rejects_before_any_new_root_effect_and_never_grows() {
    let mut platform = platform();
    let retained: Vec<_> = (0..GUARD_CAPACITY)
        .map(|_| {
            platform
                .readable_stack
                .acquire(ids::GET_RANDOM_NUMBER, SP)
                .unwrap()
        })
        .collect();
    let before = platform.devices.clock();
    let mut ran = false;
    let error = platform
        .run_readable_root(ids::GET_RANDOM_NUMBER, |_| {
            ran = true;
            Ok(())
        })
        .unwrap_err()
        .unsupported_stack_read()
        .unwrap();
    assert!(!ran);
    assert_eq!(error.reason, UnsupportedReason::GuardCapacityExhausted);
    assert_eq!(platform.devices.clock(), before);
    assert_eq!(platform.readable_dma_metrics().capacity_hits, 1);
    assert_eq!(GUARD_CAPACITY, 12);
    drop(retained);
}

#[test]
fn physical_snapshot_has_no_written_byte_or_token_authority() {
    let mut original = platform();
    original
        .run_readable_root(ids::GET_RANDOM_NUMBER, |platform| {
            platform.write_u16(SOURCE, 0xa53c);
            Ok(())
        })
        .unwrap();
    let state = original.devices.capture_state(original.registers);
    let memory = original.devices.memory().clone();
    let mut restored = NativePlatform::from_state(
        memory,
        state,
        &[ids::GET_RANDOM_NUMBER],
        InterruptHandlers::default(),
    )
    .unwrap();
    let error = restored
        .run_readable_root(ids::GET_RANDOM_NUMBER, |platform| {
            let _ = platform.read_u16(SOURCE);
            Ok(())
        })
        .unwrap_err()
        .unsupported_stack_read()
        .unwrap();
    assert_eq!(
        (error.address, error.reader, error.reason),
        (SOURCE, Reader::RustMemory, UnsupportedReason::Unwritten)
    );
    assert_eq!(restored.readable_dma_metrics().opened, 0);
}

#[test]
fn actual_reset_invalidates_retained_dma_instead_of_completing_it() {
    let mut platform = platform();
    platform
        .run_readable_root(ids::DMA_COPY_WORDS_TO_VRAM, |platform| {
            issue(platform, SOURCE, 1, 0x40000080);
            Ok(())
        })
        .unwrap();
    assert_eq!(platform.readable_dma_metrics().active, 1);
    platform.devices.reset();
    let metrics = platform.readable_dma_metrics();
    assert_eq!(metrics.active, 0);
    assert_eq!(metrics.reset_invalidated, 1);
    assert_eq!(metrics.completed, 0);
}

#[test]
fn odd_hardware_word_guards_the_aligned_fetch_and_retains_raw_diagnostics() {
    let mut bad = platform();
    let error = bad
        .run_readable_root(ids::GET_RANDOM_NUMBER, |platform| {
            platform.write_u8(SP - 1, 0xcd);
            let _ = platform.read_device_u16(SP - 1);
            Ok(())
        })
        .unwrap_err()
        .unsupported_stack_read()
        .unwrap();
    assert_eq!(
        (error.raw_address, error.address, error.width),
        (SP - 1, SP - 2, 2)
    );
    assert_eq!(error.reason, UnsupportedReason::Unwritten);
    let mut valid = platform();
    let value = valid
        .run_readable_root(ids::GET_RANDOM_NUMBER, |platform| {
            platform.write_u16(SP - 2, 0xabcd);
            Ok(platform.read_device_u16(SP - 1))
        })
        .unwrap();
    assert_eq!(value, 0xabcd);
}

#[test]
fn discarded_device_runtime_is_unresolved_proof_debt_not_completion() {
    let mut platform = platform();
    platform
        .run_readable_root(ids::DMA_COPY_WORDS_TO_VRAM, |platform| {
            issue(platform, SOURCE, 1, 0x40000080);
            Ok(())
        })
        .unwrap();
    let replacement = DeviceRuntime::new(
        platform.devices.memory().clone(),
        &GenesisEmulatorConfig::default(),
    );
    let discarded = std::mem::replace(platform.devices_mut(), replacement);
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(discarded))).is_ok());
    let error = platform
        .finish_readable_proof()
        .unwrap_err()
        .unsupported_stack_read()
        .unwrap();
    assert_eq!(error.root, ids::DMA_COPY_WORDS_TO_VRAM);
    assert_eq!(error.reason, UnsupportedReason::DiscardedOwnedDma);
    let metrics = platform.readable_dma_metrics();
    assert_eq!(
        (metrics.active, metrics.discarded, metrics.completed),
        (0, 1, 0)
    );
}

#[test]
fn owned_fill_keeps_its_issuer_when_r23_switches_to_ram_dma() {
    let mut platform = platform();
    platform
        .run_readable_root(ids::DMA_COPY_WORDS_TO_VRAM, |platform| {
            for command in [
                0x8114,
                0x8f02,
                0x9301,
                0x9400,
                0x9500 | ((SOURCE >> 1) & 255) as u16,
                0x9600 | ((SOURCE >> 9) & 255) as u16,
                0x97bf,
                0x4000,
                0x0080,
            ] {
                platform.write_vdp_control(command);
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(platform.readable_dma_metrics().active, 1);
    platform
        .run_readable_root(ids::GET_RANDOM_NUMBER, |platform| {
            platform.write_vdp_control(0x977f);
            Ok(())
        })
        .unwrap();
    let mut failure = None;
    for _ in 0..1024 {
        if let Err(error) = platform.advance_master_clocks(64) {
            failure = Some(error);
            break;
        }
    }
    let error = failure
        .expect("actual mode-changed RAM source must be checked")
        .unsupported_stack_read()
        .unwrap();
    assert_eq!(
        (error.root, error.address, error.reader),
        (ids::DMA_COPY_WORDS_TO_VRAM, SOURCE, Reader::VdpDma)
    );
}

#[test]
fn actual_fill_cancellation_is_distinct_from_unresolved_discard() {
    let mut platform = platform();
    platform
        .run_readable_root(ids::DMA_COPY_WORDS_TO_VRAM, |platform| {
            for command in [0x8114, 0x9301, 0x9400, 0x97bf, 0x4000, 0x0080] {
                platform.write_vdp_control(command);
            }
            Ok(())
        })
        .unwrap();
    platform
        .run_readable_root(ids::GET_RANDOM_NUMBER, |platform| {
            platform.write_vdp_control(0x4000);
            platform.write_vdp_control(0x0000);
            Ok(())
        })
        .unwrap();
    platform.finish_readable_proof().unwrap();
    let metrics = platform.readable_dma_metrics();
    assert_eq!(
        (
            metrics.active,
            metrics.cancelled,
            metrics.discarded,
            metrics.completed
        ),
        (0, 1, 0, 0)
    );
}

#[test]
fn discarding_a_queued_issuer_cannot_pass_proof_behind_a_foreign_dma() {
    let mut platform = platform();
    // No readable root owns this physical front transfer.
    issue(&mut platform, 0, 0, 0x40000080);
    platform
        .run_readable_root(ids::UPLOAD_QUEUED_SPRITE_GRAPHICS, |platform| {
            issue(platform, SOURCE, 1, 0x40000080);
            Ok(())
        })
        .unwrap();
    assert!(platform.readable_dma_metrics().queued > 0);
    let replacement = DeviceRuntime::new(
        platform.devices.memory().clone(),
        &GenesisEmulatorConfig::default(),
    );
    drop(std::mem::replace(platform.devices_mut(), replacement));
    let error = platform
        .finish_readable_proof()
        .unwrap_err()
        .unsupported_stack_read()
        .unwrap();
    assert_eq!(error.root, ids::UPLOAD_QUEUED_SPRITE_GRAPHICS);
    assert_eq!(error.reason, UnsupportedReason::DiscardedOwnedDma);
    assert_eq!(platform.readable_dma_metrics().queued, 0);
    assert!(platform.readable_dma_metrics().queued_discarded > 0);
}
