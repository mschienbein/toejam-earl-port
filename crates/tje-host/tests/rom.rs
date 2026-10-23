//! Opt-in local tests: TJE_REV02_ROM must point at the user's private REV 02 ROM.
use m68000_emu::BusInterface;
use std::{fs, path::PathBuf};
use tje_host::{
    Host, Mode,
    abi::HookContext,
    catalogue::{self, FunctionCatalogue},
    hooks::{self, HookRegistry},
    input::InputScript,
};

fn rom() -> Vec<u8> {
    let path = std::env::var_os("TJE_REV02_ROM")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("../../roms/rev02.bin"));
    fs::read(&path).unwrap_or_else(|error| panic!("private REV 02 ROM {}: {error}", path.display()))
}
fn host(mode: Mode) -> Host {
    Host::new(
        &rom(),
        mode,
        HookRegistry::demo(&FunctionCatalogue::rev02_demo()).unwrap(),
    )
    .unwrap()
}

#[test]
#[ignore = "requires the user's private REV 02 ROM"]
fn native_leaves_match_original_rom_including_seed_edges() {
    let mut host = host(Mode::Check);
    for seed in [
        i32::MIN,
        i32::MIN + 1,
        -127773,
        -1,
        0,
        1,
        12345,
        127773,
        i32::MAX - 1,
        i32::MAX,
    ] {
        host.context().write_u32(catalogue::RNG_STATE, 0x1234_5678);
        let previous = host
            .check_call(catalogue::SEED_RANDOM, &[seed as u32])
            .unwrap();
        assert_eq!(previous, 0x1234_5678);
        assert_eq!(
            host.context().read_u32(catalogue::RNG_STATE),
            hooks::normalize_seed(seed) as u32
        );
        host.context().write_u32(catalogue::RNG_STATE, seed as u32);
        let result = host.check_call(catalogue::RANDOM, &[]).unwrap();
        assert_eq!(result, u32::from(hooks::park_miller(seed) as u16));
    }
    host.context().write_u32(catalogue::RNG_STATE, 1);
    for _ in 0..1000 {
        host.check_call(catalogue::RANDOM, &[]).unwrap();
    }
    host.context().write_u8(0xFF8012, 1);
    host.context().write_u8(0xFF8016, 1);
    host.check_call(catalogue::DISABLE_GRAPHICS, &[]).unwrap();
    assert_eq!(host.context().read_u8(0xFF8012), 0);
    assert_eq!(host.context().read_u8(0xFF8016), 0);
    assert_eq!(host.checks.mismatches, 0);
    assert!(host.checks.uninitialized_stack_reads.is_empty());
    println!(
        "leaf checks={} mismatches={} dead-stack-bytes={} uninitialized-sites={}",
        host.checks.calls,
        host.checks.mismatches,
        host.checks.dead_stack_differing_bytes,
        host.checks.uninitialized_stack_reads.len()
    );
}

#[test]
#[ignore = "requires the user's private REV 02 ROM"]
fn rust_hook_calls_an_untranslated_original_function() {
    let registry = HookRegistry::nested_demo(&FunctionCatalogue::rev02_demo()).unwrap();
    assert!(registry.at(catalogue::RANDOM).is_none());
    let mut host = Host::new(&rom(), Mode::Check, registry).unwrap();
    host.context().write_u32(catalogue::RNG_STATE, 12345);
    host.check_call(catalogue::PREGENERATE_RANDOMS, &[])
        .unwrap();
    assert_eq!(host.context().read_u32(catalogue::RNG_STATE), 12345);
    assert_eq!(host.context().read_u16(0xFF91E0), 0);
    let mut expected = 12345;
    for index in 0..59 {
        expected = hooks::park_miller(expected);
        assert_eq!(
            host.context().read_u16(0xFF916A + 2 * index),
            expected as u16
        );
    }
    assert_eq!(host.checks.nested_original_calls, 59);
    assert_eq!(host.checks.mismatches, 0);
    assert!(host.checks.uninitialized_stack_reads.is_empty());
    println!(
        "nested original calls={} mismatches={}",
        host.checks.nested_original_calls, host.checks.mismatches
    );
}

#[test]
#[ignore = "requires the user's private REV 02 ROM"]
fn check_mode_detects_ram_return_register_and_chip_write_mismatches() {
    fn wrong_return(context: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        hooks::get_random_number(context)?;
        Ok(Some(0xFFFF_FFFF))
    }
    fn wrong_ram(context: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        let result = hooks::get_random_number(context)?;
        context.write_u8(0xFF912C, 77);
        Ok(result)
    }
    fn wrong_preserved(context: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        let result = hooks::get_random_number(context)?;
        let mut data = context.cpu().data_registers();
        data[7] ^= 1;
        context.cpu().set_data_registers(data);
        Ok(result)
    }
    fn wrong_sound(context: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        let result = hooks::get_random_number(context)?;
        context.bus().write_byte(0xC00011, 0x9F);
        Ok(result)
    }
    fn wrong_vdp(context: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        let result = hooks::get_random_number(context)?;
        context.bus().write_word(0xC00004, 0x8707);
        Ok(result)
    }
    for function in [
        wrong_return as hooks::HookFn,
        wrong_ram,
        wrong_preserved,
        wrong_sound,
        wrong_vdp,
    ] {
        let mut registry = HookRegistry::default();
        registry
            .register(
                &FunctionCatalogue::rev02_demo(),
                "GetRandomNumber",
                true,
                function,
            )
            .unwrap();
        let mut host = Host::new(&rom(), Mode::Check, registry).unwrap();
        host.context().write_u32(catalogue::RNG_STATE, 1);
        let error = host.check_call(catalogue::RANDOM, &[]).unwrap_err();
        assert!(matches!(error, tje_host::HostError::Mismatch { .. }));
        assert_eq!(host.checks.mismatches, 1);
    }
}

#[test]
#[ignore = "requires the user's private REV 02 ROM; runs 3600 frames twice"]
fn power_on_title_started_game_3600_frames_matches_timed_reference() {
    let script = InputScript::parse(include_str!("../examples/start-game.json")).unwrap();
    let bytes = rom();
    let registry = HookRegistry::demo(&FunctionCatalogue::rev02_demo()).unwrap();
    let mut checked = Host::new(&bytes, Mode::Check, registry.clone()).unwrap();
    let mut reference = Host::new(&bytes, Mode::Reference, registry).unwrap();
    for frame in 0..3600 {
        let a = checked.run_frame(script.at(frame)).unwrap();
        let b = reference.run_frame(script.at(frame)).unwrap();
        assert_eq!(a.frame_counter, b.frame_counter, "frame {frame}");
        assert_eq!(a.ram, b.ram, "whole work RAM at frame {frame}");
        assert_eq!(a.vdp, b.vdp, "VDP at frame {frame}");
        assert_eq!(a.writes, b.writes, "VDP/sound writes at frame {frame}");
        assert_eq!(
            a.idle_random_draws, b.idle_random_draws,
            "idle RNG at frame {frame}"
        );
        assert!(
            a.audio_samples.iter().flatten().map(|v| v.to_bits()).eq(b
                .audio_samples
                .iter()
                .flatten()
                .map(|v| v.to_bits())),
            "audio at frame {frame}"
        );
        assert_eq!(
            a.lag_events, b.lag_events,
            "lag classification at frame {frame}"
        );
        let (pa, sa) = checked.frame_buffer();
        let (pb, sb) = reference.frame_buffer();
        assert_eq!(sa, sb);
        assert_eq!(
            &pa[..sa.len() as usize],
            &pb[..sb.len() as usize],
            "pixels at frame {frame}"
        );
    }
    assert!(checked.measurements.title_frame.is_some());
    assert!(checked.measurements.menu_frame.is_some());
    assert!(checked.measurements.game_frame.is_some());
    assert!(checked.measurements.logic_ticks > 1000);
    assert!(checked.checks.calls > 10_000);
    assert_eq!(checked.checks.mismatches, 0);
    assert!(checked.checks.uninitialized_stack_reads.is_empty());
    println!(
        "frames=3600 checks={} mismatches={} title={:?} menu={:?} game={:?} logic_ticks={} lag_frames={} idle_draws={}",
        checked.checks.calls,
        checked.checks.mismatches,
        checked.measurements.title_frame,
        checked.measurements.menu_frame,
        checked.measurements.game_frame,
        checked.measurements.logic_ticks,
        checked.measurements.lag_events.len(),
        checked
            .measurements
            .idle_draws_per_frame
            .iter()
            .sum::<u64>()
    );
}

#[test]
#[ignore = "requires the user's private REV 02 ROM"]
fn replacement_mode_executes_rust_on_the_live_machine() {
    use std::sync::atomic::{AtomicU64, Ordering};
    static CALLS: AtomicU64 = AtomicU64::new(0);
    fn counted(context: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        CALLS.fetch_add(1, Ordering::Relaxed);
        hooks::get_random_number(context)
    }
    let mut registry = HookRegistry::default();
    registry
        .register(
            &FunctionCatalogue::rev02_demo(),
            "GetRandomNumber",
            true,
            counted,
        )
        .unwrap();
    let mut host = Host::new(&rom(), Mode::Replace, registry).unwrap();
    for _ in 0..30 {
        host.run_frame(InputScript::default().at(0)).unwrap();
    }
    assert!(CALLS.load(Ordering::Relaxed) >= 59);
    assert_eq!(host.checks.calls, 0); // these are replacements, not shadow checks
    assert_eq!(
        host.checks.by_function["GetRandomNumber"],
        CALLS.load(Ordering::Relaxed)
    );
}
