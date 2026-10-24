//! Fixture API/observer units, not canonical timing-source or IRQ proof.
use super::*;
use crate::{CheckReport, Mode, hooks::HookRegistry};
use std::sync::atomic::{AtomicU32, Ordering};
use tje_game::{FramedTranslation, ids};

static RETURN: AtomicU32 = AtomicU32::new(0);
static PROVIDED: AtomicU32 = AtomicU32::new(0);
static UNPROVIDED: AtomicU32 = AtomicU32::new(0);

fn inspect_then_fail(context: &mut dyn tje_game::Context) -> tje_game::Result<Option<u32>> {
    let sp = context.stack_pointer()?;
    RETURN.store(context.read_u32(sp), Ordering::Relaxed);
    PROVIDED.store(context.read_u32(sp + 4), Ordering::Relaxed);
    UNPROVIDED.store(context.read_u32(sp + 8), Ordering::Relaxed);
    Err(tje_game::Error::Platform(
        "authored fixture observer abort".into(),
    ))
}
static FRAMED: &[FramedTranslation] = &[FramedTranslation {
    id: ids::DMA_COPY_WORDS_TO_VRAM,
    function: inspect_then_fail,
}];

fn host() -> Host {
    let mut host = tests::synthetic_host(&[0x60fe], &[0x4e73]);
    host.mode = Mode::Check;
    host.registry = HookRegistry::proof_clocked_strict(host.revision, &[], FRAMED, &[]).unwrap();
    host.checks = CheckReport::registered(&host.registry);
    host
}
fn encoded(host: &Host) -> Vec<u8> {
    bincode::encode_to_vec(&host.emulator, bincode::config::standard()).unwrap()
}
fn entry(host: &Host) -> u32 {
    ids::DMA_COPY_WORDS_TO_VRAM
        .metadata()
        .address(host.revision)
}

#[test]
fn real_return_and_only_provided_lazy_slots_live_inside_discarded_error_trajectory() {
    let mut host = host();
    let caller = host.cpu().pc();
    let sp = host.cpu().stack_pointer();
    host.context().write_u32(sp, 0x89abcdef);
    let before = encoded(&host);
    let result = host.check_clocked_fixture(entry(&host), &[0x12345678]);
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("authored fixture observer abort")
    );
    assert_eq!(RETURN.load(Ordering::Relaxed), caller);
    assert_eq!(PROVIDED.load(Ordering::Relaxed), 0x12345678);
    assert_eq!(UNPROVIDED.load(Ordering::Relaxed), 0x89abcdef);
    assert_ne!(caller, crate::abi::CALL_SENTINEL);
    assert_eq!(before, encoded(&host));
}

#[test]
fn bad_return_targets_reject_without_mirroring_seeding_or_caller_rewrite() {
    for pc in [crate::abi::CALL_SENTINEL, 0xff1000, 0x3ffff0, 0x101] {
        let mut host = host();
        let (cpu, bus) = host.emulator.hybrid_parts();
        cpu.set_pc(pc, bus);
        let before = encoded(&host);
        assert!(
            host.check_clocked_fixture(entry(&host), &[1, 2, 3])
                .is_err()
        );
        assert_eq!(before, encoded(&host));
        assert_eq!(host.checks.calls, 0);
    }
}

#[test]
fn mismatched_raw_rom_prefetch_is_rejected_before_any_fixture_seed() {
    let mut host = host();
    let pc = host.cpu().pc();
    // Authored synthetic ROM only: deliberately construct invalid prefetch.
    host.emulator
        .hybrid_bus_mut()
        .cartridge
        .as_mut()
        .unwrap()
        .debug_rom_view()[pc as usize / 2] ^= 1;
    let before = encoded(&host);
    let error = host
        .check_clocked_fixture(entry(&host), &[1, 2, 3])
        .unwrap_err();
    assert!(error.to_string().contains("prefetch"));
    assert_eq!(before, encoded(&host));
    assert_eq!(host.checks.calls, 0);
}

#[test]
fn mode_route_and_unsettled_boundary_cannot_be_implicitly_repaired() {
    let mut wrong_mode = host();
    wrong_mode.mode = Mode::Reference;
    let before = encoded(&wrong_mode);
    assert!(
        wrong_mode
            .check_clocked_fixture(entry(&wrong_mode), &[])
            .is_err()
    );
    assert_eq!(before, encoded(&wrong_mode));

    let mut frozen = host();
    frozen.registry = HookRegistry::proof_with_frames(frozen.revision, &[], FRAMED).unwrap();
    let before = encoded(&frozen);
    assert!(frozen.check_clocked_fixture(entry(&frozen), &[]).is_err());
    assert_eq!(before, encoded(&frozen));

    let mut pending = host();
    pending
        .emulator
        .hybrid_bus_mut()
        .cycles
        .m68k_wait_cpu_cycles = 1;
    let before = encoded(&pending);
    assert!(pending.check_clocked_fixture(entry(&pending), &[]).is_err());
    assert_eq!(before, encoded(&pending));
}
