//! Opt-in tests: TJE_REV02_ROM/TJE_REV00_ROM point to private retail cartridges.
use m68000_emu::BusInterface;
use std::{fs, path::PathBuf};
use tje_game::{Context, FunctionId, Translation, ids, map};
use tje_host::{
    Host, HostError, Mode,
    abi::HookContext,
    catalogue::{self, FunctionCatalogue},
    hooks::{HookFn, HookRegistry},
    input::InputScript,
};
use tje_mem::{Address, Platform, Revision, ram};

fn rom(revision: Revision) -> Vec<u8> {
    let (key, default) = match revision {
        Revision::Rev02 => ("TJE_REV02_ROM", "../../roms/rev02.bin"),
        Revision::Rev00 => ("TJE_REV00_ROM", "../../roms/rev00.bin"),
    };
    let path = std::env::var_os(key)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(default));
    fs::read(&path)
        .unwrap_or_else(|error| panic!("private {revision:?} ROM {}: {error}", path.display()))
}
fn host(revision: Revision, mode: Mode) -> Host {
    Host::new(
        &rom(revision),
        mode,
        HookRegistry::translated(revision).unwrap(),
    )
    .unwrap()
}
fn check(host: &mut Host, id: FunctionId, arguments: &[u32]) -> u32 {
    host.check_call(id.metadata().address(host.revision()), arguments)
        .unwrap_or_else(|error| panic!("{}: {error}", id.metadata().name))
}
fn translated_rng(context: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
    Ok(map::get_random_number(context, &[])?)
}

#[test]
#[ignore = "requires both private retail ROMs"]
fn native_leaves_match_original_including_seed_edges() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut host = host(revision, Mode::Check);
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
            assert_eq!(
                check(&mut host, ids::STACK_BASED_RANDOM, &[seed as u32]),
                0x1234_5678
            );
            assert_eq!(
                host.context().read_u32(catalogue::RNG_STATE),
                map::normalize_seed(seed) as u32
            );
            host.context().write_u32(catalogue::RNG_STATE, seed as u32);
            assert_eq!(
                check(&mut host, ids::GET_RANDOM_NUMBER, &[]),
                u32::from(map::park_miller(seed) as u16)
            );
        }
        for _ in 0..1000 {
            check(&mut host, ids::GET_RANDOM_NUMBER, &[]);
        }
        assert_eq!(host.checks.mismatches, 0);
    }
}

#[test]
#[ignore = "requires both private retail ROMs"]
fn translated_call_dispatches_rust_or_nested_original_by_identity() {
    static NESTED: &[Translation] = &[Translation {
        id: ids::PRE_GENERATE_RANDOMS,
        function: map::pre_generate_randoms,
    }];
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (registry, expected_nested) in [
            (
                HookRegistry::from_translations(revision, NESTED).unwrap(),
                59,
            ),
            (HookRegistry::translated(revision).unwrap(), 0),
        ] {
            let mut host = Host::new(&rom(revision), Mode::Check, registry).unwrap();
            host.context().write_u32(catalogue::RNG_STATE, 12345);
            check(&mut host, ids::PRE_GENERATE_RANDOMS, &[]);
            assert_eq!(host.context().read_u32(catalogue::RNG_STATE), 12345);
            let mut expected = 12345;
            for index in 0..59 {
                expected = map::park_miller(expected);
                assert_eq!(
                    host.context().read_u16(0xFF916A + 2 * index),
                    expected as u16
                );
            }
            assert_eq!(host.checks.nested_original_calls, expected_nested);
            assert_eq!(host.checks.mismatches, 0);
        }
    }
}

fn expect_mismatch(
    revision: Revision,
    id: FunctionId,
    function: HookFn,
    arguments: &[u32],
    init: impl FnOnce(&mut Host),
) {
    let mut registry = HookRegistry::empty(revision);
    registry
        .register(
            &FunctionCatalogue::new(revision),
            id.metadata().name,
            function,
        )
        .unwrap();
    let mut host = Host::new(&rom(revision), Mode::Check, registry).unwrap();
    host.context().write_u32(catalogue::RNG_STATE, 1);
    init(&mut host);
    let error = host
        .check_call(id.metadata().address(revision), arguments)
        .unwrap_err();
    assert!(matches!(error, HostError::Mismatch { .. }), "{error}");
    assert_eq!(host.checks.mismatches, 1);
    assert_eq!(host.checks.by_function[id.metadata().name].mismatches, 1);
    println!("REJECTED {revision:?} {}: {error}", id.metadata().name);
}

#[test]
#[ignore = "requires both private retail ROMs"]
fn check_mode_rejects_register_sr_read_latch_and_ram_io_order_mutations() {
    fn wrong_return(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        translated_rng(c)?;
        Ok(Some(u32::MAX))
    }
    fn wrong_ram(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        let r = translated_rng(c)?;
        c.write_u8(0xFF912C, 77);
        Ok(r)
    }
    fn wrong_preserved(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        let r = translated_rng(c)?;
        let mut d = c.cpu().data_registers();
        d[7] ^= 1;
        c.cpu().set_data_registers(d);
        Ok(r)
    }
    fn wrong_address(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        let result = translated_rng(c)?;
        let a6 = c.address_register(6);
        c.set_address_register(6, a6 ^ 1);
        Ok(result)
    }
    fn wrong_stack(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        let result = translated_rng(c)?;
        let mut cpu = c.cpu().clone();
        let sp = cpu.supervisor_stack_pointer();
        cpu.set_supervisor_stack_pointer(sp.wrapping_add(4));
        *c.cpu() = cpu;
        Ok(result)
    }
    fn wrong_seed(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        let result = translated_rng(c)?;
        let seed = c.read_u32(catalogue::RNG_STATE);
        c.write_u32(catalogue::RNG_STATE, seed.wrapping_add(1));
        Ok(result)
    }
    fn wrong_ym(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        let result = translated_rng(c)?;
        tje_game::Platform::write_ym2612(c, 0, 0x22);
        Ok(result)
    }
    fn extra_device_read(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        let result = translated_rng(c)?;
        let address = c.argument_u32(0);
        c.bus().read_byte(address);
        Ok(result)
    }
    fn wrong_sound(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        let r = translated_rng(c)?;
        tje_game::Platform::write_psg(c, 0x9f);
        Ok(r)
    }
    fn wrong_vdp(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        let r = translated_rng(c)?;
        c.write_vdp_control(0x8707);
        Ok(r)
    }
    fn wrong_sr(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        let r = translated_rng(c)?;
        let sr = c.cpu().status_register() ^ 0x0700;
        c.cpu().set_status_register(sr);
        Ok(r)
    }
    fn extra_status(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        let r = translated_rng(c)?;
        c.bus().read_word(0xc00004);
        Ok(r)
    }
    fn extra_hv(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        let r = translated_rng(c)?;
        c.bus().read_word(0xc00008);
        Ok(r)
    }
    fn wrong_assembly_d1(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        let target = ids::DISABLE_ALL_INTERRUPTS.metadata();
        c.call_original(target.address(c.revision()), &[])?;
        let mut d = c.cpu().data_registers();
        d[1] ^= 1;
        c.cpu().set_data_registers(d);
        Ok(None)
    }
    fn early_mirror(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        let link = if c.argument_i16(0) != 0 { 1 } else { 11 };
        ram::g_hardware_sprite_root_link(c.revision()).write(c, link);
        c.write_vdp_command(0x78020003);
        c.write_vdp_data(u16::from(link));
        Ok(None)
    }
    fn skipped_owner_zero(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        let entity = Address::<tje_mem::ObjectState>::new(c.argument_u32(0));
        if entity.owner().read(c) != 0 {
            tje_game::sprites::update_attached_entity_position(c, c.revision(), entity);
        }
        Ok(None)
    }
    fn no_interrupt_change(_: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        Ok(None)
    }
    fn nested_stack_dependency(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        let result = translated_rng(c)?;
        let mut contract = ids::GET_RANDOM_NUMBER.metadata().clone();
        contract.name = "AuthoredNestedStackRead";
        contract.addresses = [0xFFF000; 2];
        c.call_original_with_contract(&contract, true, &[])?;
        Ok(result)
    }
    for revision in [Revision::Rev02, Revision::Rev00] {
        for function in [
            wrong_return as HookFn,
            wrong_ram,
            wrong_preserved,
            wrong_address,
            wrong_stack,
            wrong_seed,
            wrong_ym,
            wrong_sound,
            wrong_vdp,
            wrong_sr,
            extra_status,
            extra_hv,
        ] {
            expect_mismatch(revision, ids::GET_RANDOM_NUMBER, function, &[], |host| {
                host.context().write_vdp_control(0x4000); // pending first command word
            });
        }
        for address in [
            0xA10003, 0xA10005, 0xA11100, 0xA11200, 0xA04000, 0xC00004, 0xC00008,
        ] {
            expect_mismatch(
                revision,
                ids::GET_RANDOM_NUMBER,
                extra_device_read,
                &[address],
                |_| {},
            );
        }
        expect_mismatch(
            revision,
            ids::DISABLE_ALL_INTERRUPTS,
            wrong_assembly_d1,
            &[],
            |host| {
                let mut d = host.context().cpu().data_registers();
                d[1] = 6;
                host.context().cpu().set_data_registers(d);
            },
        );
        expect_mismatch(
            revision,
            ids::ENABLE_ALL_INTERRUPTS,
            no_interrupt_change,
            &[],
            |host| {
                host.context().cpu().set_status_register(0x2700);
            },
        );
        expect_mismatch(
            revision,
            ids::DISABLE_ALL_INTERRUPTS,
            no_interrupt_change,
            &[],
            |host| {
                host.context().cpu().set_status_register(0x2000);
            },
        );
        for mirror in [0x00FF_u16, 0x00EF] {
            expect_mismatch(
                revision,
                ids::GET_RANDOM_NUMBER,
                nested_stack_dependency,
                &[],
                |host| {
                    // Authored synthetic RAM routine: MOVE.L absolute.l,D1; RTS.
                    for (index, word) in [0x2239, mirror, 0x7FE8, 0x4E75].into_iter().enumerate() {
                        host.context().write_u16(0xFFF000 + 2 * index as u32, word);
                    }
                },
            );
        }
        expect_mismatch(
            revision,
            ids::SELECT_SPRITE_CHAIN_START,
            early_mirror,
            &[1],
            |host| {
                ram::g_hardware_sprite_root_link(revision).write(&mut host.context(), 11);
            },
        );
        let entity = ram::g_player_info(revision).at(0);
        expect_mismatch(
            revision,
            ids::UPDATE_ATTACHED_ENTITY_POSITION,
            skipped_owner_zero,
            &[entity.address()],
            |host| {
                let mut c = host.context();
                entity.owner().write(&mut c, 0);
                entity.attachment_x_or_latch().write(&mut c, 1);
            },
        );
    }
}

/// Coverage fixtures use a live state after the 10,000-frame gameplay session.
/// They are labeled separately from natural calls; no ROM-derived state is stored.
fn drive_live_fixture_coverage(host: &mut Host) {
    fn check(host: &mut Host, id: FunctionId, arguments: &[u32]) -> u32 {
        host.check_fixture(id.metadata().address(host.revision()), arguments)
            .unwrap_or_else(|error| panic!("{}: {error}", id.metadata().name))
    }
    let revision = host.revision();
    for value in [i32::MIN, -7, -1, 0, 1, 7, i32::MAX] {
        check(host, ids::ABS, &[value as u32]);
    }
    for id in [ids::LMODT, ids::LDIVT, ids::LMULT, ids::ULMODT, ids::ULDIVT] {
        for (d0, d1) in [
            (0_u32, 1_u32),
            (7, 3),
            ((-7_i32) as u32, 3),
            (7, (-3_i32) as u32),
            (0x80000000, u32::MAX),
            (u32::MAX, 65535),
            (u32::MAX, 65536),
            (u32::MAX, 65537),
            (u32::MAX, 0xffff8000),
        ] {
            let mut d = host.context().cpu().data_registers();
            d[0] = d0;
            d[1] = d1;
            host.context().cpu().set_data_registers(d);
            check(host, id, &[]);
        }
    }
    for value in [i16::MIN, -1, 0, 1, 6, 7, i16::MAX] {
        let argument = value as i32 as u32;
        for id in [
            ids::GET_LEVEL_MUSIC_INDEX,
            ids::GET_SCORE_FOR_PROMO,
            ids::SET_CAMERA_TILE_PRIORITY_ENABLED,
            ids::SELECT_SPRITE_CHAIN_START,
            ids::SET_NEXT_SPEECH_LIFETIME,
        ] {
            check(host, id, &[argument]);
        }
    }
    for player in [0, 1] {
        check(host, ids::PLAYER_GET_MAXIMUM_HEALTH, &[player]);
        check(host, ids::PLAYER_INITIALIZE_PHASED_JUMP_MOTION, &[player]);
        for swapped in [0, -1] {
            for buttons in [0, -1] {
                let mut c = host.context();
                ram::g_controller_ports_swapped(revision).write(&mut c, swapped);
                ram::g_controller_buttons_ac_swapped(revision).write(&mut c, buttons);
                ram::g_controller1_raw_input(revision).write(&mut c, 0xff40);
                ram::g_controller2_raw_input(revision).write(&mut c, 0x20);
                check(host, ids::READ_MAPPED_PLAYER_INPUT, &[player]);
            }
        }
    }
    for id in [
        ids::DISABLE_BLANKING_GRAPHICS,
        ids::ENABLE_BLANKING_GRAPHICS,
        ids::GET_V_BLANK_GRAPHICS_ENABLED,
        ids::ENABLE_TEXT_FONT_ANIMATION,
    ] {
        check(host, id, &[]);
    }
    for open in [0, -1] {
        ram::g_player_inventory_open(revision)
            .at(0)
            .write(&mut host.context(), open);
        check(host, ids::DISABLE_TEXT_FONT_ANIMATION_IF_MENUS_CLOSED, &[]);
    }
    for (a, b) in [(0, 0), (0, -1), (-1, 0), (-1, -1)] {
        ram::g_timed_effect_entities(revision)
            .at(0)
            .write(&mut host.context(), a);
        ram::g_timed_effect_entities(revision)
            .at(1)
            .write(&mut host.context(), b);
        check(host, ids::ENTITY_HAS_FREE_TIMED_EFFECT_SLOT, &[]);
    }
    let entity = ram::g_player_info(revision).at(0);
    entity.owner().write(&mut host.context(), 0);
    entity.attachment_x_or_latch().write(&mut host.context(), 1);
    check(
        host,
        ids::UPDATE_ATTACHED_ENTITY_POSITION,
        &[entity.address()],
    );
    check(host, ids::STOP_PLAYER, &[entity.address()]);
    check(host, ids::STACK_BASED_RANDOM, &[12345]);
    check(host, ids::GET_RANDOM_NUMBER, &[]);
    check(host, ids::PRE_GENERATE_RANDOMS, &[]);
}

#[test]
#[ignore = "requires both private retail ROMs"]
fn repeated_diagnostic_hardware_calls_do_not_fill_live_frozen_fifo() {
    let script = InputScript::parse(include_str!("../examples/start-game.json")).unwrap();
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut host = host(revision, Mode::Reference);
        for frame in 0..1300 {
            host.run_frame(script.at(frame)).unwrap();
        }
        assert!(host.measurements.game_frame.is_some());
        drive_live_fixture_coverage(&mut host);
        assert_eq!(host.frame_count(), 1300);
        assert!(host.checks.unexercised.is_empty());
        assert_eq!(host.checks.mismatches, 0);
    }
}

#[test]
#[ignore = "requires both private retail ROMs; 10,000 frames per revision plus a timed reference"]
fn gameplay_10000_frames_both_revisions_and_every_pilot_hook() {
    let script = InputScript::parse(include_str!("../examples/start-game.json")).unwrap();
    for revision in [Revision::Rev02, Revision::Rev00] {
        let bytes = rom(revision);
        let registry = HookRegistry::translated(revision).unwrap();
        let mut checked = Host::new(&bytes, Mode::Check, registry.clone()).unwrap();
        let mut reference = Host::new(&bytes, Mode::Reference, registry).unwrap();
        for frame in 0..10_000 {
            let a = checked.run_frame(script.at(frame)).unwrap();
            let b = reference.run_frame(script.at(frame)).unwrap();
            assert_eq!(
                a.frame_counter, b.frame_counter,
                "{revision:?} frame {frame}"
            );
            assert_eq!(a.ram, b.ram, "{revision:?} whole RAM frame {frame}");
            assert_eq!(a.vdp, b.vdp, "{revision:?} VDP frame {frame}");
            assert_eq!(
                a.writes, b.writes,
                "{revision:?} hardware writes frame {frame}"
            );
            assert_eq!(a.idle_random_draws, b.idle_random_draws);
            assert_eq!(a.lag_events, b.lag_events);
            assert!(
                a.audio_samples.iter().flatten().map(|v| v.to_bits()).eq(b
                    .audio_samples
                    .iter()
                    .flatten()
                    .map(|v| v.to_bits())),
                "{revision:?} audio frame {frame}"
            );
            let (pa, sa) = checked.frame_buffer();
            let (pb, sb) = reference.frame_buffer();
            assert_eq!(sa, sb);
            assert_eq!(
                &pa[..sa.len() as usize],
                &pb[..sb.len() as usize],
                "{revision:?} pixels frame {frame}"
            );
        }
        assert!(checked.measurements.title_frame.is_some());
        assert!(checked.measurements.menu_frame.is_some());
        assert!(checked.measurements.game_frame.is_some());
        let natural_coverage = serde_json::to_value(&checked.checks).unwrap();
        drive_live_fixture_coverage(&mut checked);
        assert_eq!(checked.checks.mismatches, 0);
        assert!(checked.checks.uninitialized_stack_reads.is_empty());
        assert!(checked.checks.unexercised.is_empty());
        println!(
            "PILOT_COVERAGE {}",
            serde_json::json!({"revision":revision.name(),"frames":checked.frame_count(),"title_frame":checked.measurements.title_frame,"menu_frame":checked.measurements.menu_frame,"game_frame":checked.measurements.game_frame,"logic_ticks":checked.measurements.logic_ticks,"natural":natural_coverage,"with_live_fixtures":checked.checks})
        );
    }
}

#[test]
#[ignore = "requires both private retail ROMs"]
fn replacement_mode_executes_rust_on_live_machine() {
    use std::sync::atomic::{AtomicU64, Ordering};
    static CALLS: AtomicU64 = AtomicU64::new(0);
    fn counted(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        CALLS.fetch_add(1, Ordering::Relaxed);
        translated_rng(c)
    }
    for revision in [Revision::Rev02, Revision::Rev00] {
        CALLS.store(0, Ordering::Relaxed);
        let mut registry = HookRegistry::empty(revision);
        registry
            .register(
                &FunctionCatalogue::new(revision),
                "GetRandomNumber",
                counted,
            )
            .unwrap();
        let mut host = Host::new(&rom(revision), Mode::Replace, registry).unwrap();
        for _ in 0..30 {
            host.run_frame(InputScript::default().at(0)).unwrap();
        }
        assert!(CALLS.load(Ordering::Relaxed) >= 59);
        assert_eq!(host.checks.calls, 0);
        assert_eq!(
            host.checks.by_function["GetRandomNumber"].calls_replaced,
            CALLS.load(Ordering::Relaxed)
        );
    }
}
