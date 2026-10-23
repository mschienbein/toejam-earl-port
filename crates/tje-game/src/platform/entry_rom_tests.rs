//! Opt-in non-RTS proofs. These bodies remain pending until both revisions pass.
use std::{fs, path::PathBuf, sync::Arc};
pub use tje_game::{
    Context, Platform, Result, call_at, call_frame, execution, exits, ids, open_bus,
};
use tje_game::{
    FunctionId, Translation,
    exits::{EntryOutcome, Exit, StackBanks},
};
#[path = "entries.rs"]
mod entries;
use tje_host::{Host, Mode, hooks::HookRegistry, input::InputScript};
use tje_mem::{Ptr, Revision, ram, rom_symbol};

static PREFIXES: &[Translation] = &[
    Translation {
        id: ids::START,
        function: entries::start,
    },
    Translation {
        id: ids::GAME_PROGRAM,
        function: entries::game_program,
    },
    Translation {
        id: ids::RESTART_FROM_RESET_ENTRY,
        function: entries::restart_from_reset_entry,
    },
];
static EXCEPTIONS: &[Translation] = &[
    Translation {
        id: ids::LEVEL4_ISR,
        function: entries::level4_isr,
    },
    Translation {
        id: ids::LEVEL6_ISR,
        function: entries::level6_isr,
    },
];
static DIRECT_PREFIXES: &[Translation] = &[
    Translation {
        id: ids::GAME_PROGRAM,
        function: entries::game_program,
    },
    Translation {
        id: ids::RESTART_FROM_RESET_ENTRY,
        function: entries::restart_from_reset_entry,
    },
];

fn script() -> Arc<InputScript> {
    Arc::new(
        InputScript::parse(include_str!("../../../tje-host/examples/start-game.json")).unwrap(),
    )
}
fn bytes(revision: Revision) -> Vec<u8> {
    let key = match revision {
        Revision::Rev02 => "TJE_REV02_ROM",
        Revision::Rev00 => "TJE_REV00_ROM",
    };
    fs::read(std::env::var_os(key).expect("private cartridge path")).unwrap()
}
fn save(host: &Host, label: &str, natural: &serde_json::Value) {
    let directory = PathBuf::from(std::env::var_os("TJE_PLATFORM_EVIDENCE_DIR").unwrap());
    let report = serde_json::json!({
        "revision": host.revision().name(), "frames": host.frame_count(),
        "title_frame": host.measurements.title_frame, "game_frame": host.measurements.game_frame,
        "source": label, "natural": natural, "with_live_fixtures": host.checks,
    });
    fs::write(
        directory.join(format!("{}-{label}.json", host.revision().name())),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    println!("PLATFORM_ENTRIES {report}");
}
fn prepared_cpu(host: &mut Host, sp: u32, sr: u16) {
    let mut context = host.context();
    context.set_status_register(sr);
    context
        .set_stack_banks(StackBanks {
            user: 0xff7200,
            supervisor: sp,
        })
        .unwrap();
    for index in 0..8 {
        context.set_data_register(index, 0x1234_8000_u32.wrapping_add(index as u32));
    }
    for index in 0..7 {
        context.set_address_register(index, 0x1357_0000_u32.wrapping_add(index as u32));
    }
}
fn check(
    host: &mut Host,
    id: FunctionId,
    expected: Exit,
    label: &str,
    natural: &serde_json::Value,
) {
    let entry_sp = host.context().stack_pointer().unwrap();
    let entry_banks = host.context().stack_banks().unwrap();
    let result = host
        .check_entry_fixture(id.metadata().address(host.revision()))
        .unwrap_or_else(|error| {
            save(host, label, natural);
            panic!("{}: {error}", id.metadata().name)
        });
    assert_eq!(host.context().stack_pointer().unwrap(), entry_sp);
    assert_eq!(host.context().stack_banks().unwrap(), entry_banks);
    println!(
        "ENTRY_PROOF {} {} {result:?}; prepared SP={entry_sp:08x}, banks={entry_banks:?}; isolated state restored",
        host.revision().name(),
        id.metadata().name
    );
    match result {
        EntryOutcome::Transfer(transfer) => {
            assert_eq!(transfer.entry, id);
            assert_eq!(transfer.exit, expected);
        }
        EntryOutcome::Returned(_) => panic!("{} must not RTS", id.metadata().name),
    }
}

#[test]
#[ignore = "requires both private ROMs and gated non-RTS host contracts"]
fn platform_reset_prefixes_natural_sessions_and_prepared_entries() {
    let script = script();
    for revision in [Revision::Rev02, Revision::Rev00] {
        let bytes = bytes(revision);
        let registry = HookRegistry::proof(revision, PREFIXES).unwrap();
        let mut host = Host::new(&bytes, Mode::Check, registry.clone()).unwrap();
        let mut reference = Host::new(&bytes, Mode::Reference, registry).unwrap();
        host.set_wait_input_poller(script.clone());
        for frame in 0..10_000 {
            let a = host.run_frame(script.at(frame)).unwrap_or_else(|error| {
                let natural = serde_json::to_value(&host.checks).unwrap();
                save(&host, "prefixes", &natural);
                panic!("{revision:?} frame {frame}: {error}")
            });
            let b = reference.run_frame(script.at(frame)).unwrap();
            assert_eq!(a.frame_counter, b.frame_counter);
            assert_eq!(a.ram, b.ram);
            assert_eq!(a.vdp, b.vdp);
            assert_eq!(a.writes, b.writes);
            assert_eq!(a.idle_random_draws, b.idle_random_draws);
            assert_eq!(a.lag_events, b.lag_events);
            assert!(
                a.audio_samples.iter().flatten().map(|x| x.to_bits()).eq(b
                    .audio_samples
                    .iter()
                    .flatten()
                    .map(|x| x.to_bits()))
            );
            let (a, size_a) = host.frame_buffer();
            let (b, size_b) = reference.frame_buffer();
            assert_eq!(size_a, size_b);
            assert_eq!(&a[..size_a.len() as usize], &b[..size_b.len() as usize]);
        }
        assert!(host.measurements.game_frame.is_some());
        let natural = serde_json::to_value(&host.checks).unwrap();
        host.settle_fixture_boundary(script.at(host.frame_count()))
            .unwrap();
        let vector = rom_symbol(revision, "StartOfRom").unwrap();
        let initial_sp = host.context().read_u32(vector.address);
        for sr in [0x2700, 0x271f] {
            prepared_cpu(&mut host, 0xff7c00, sr);
            check(
                &mut host,
                ids::RESTART_FROM_RESET_ENTRY,
                Exit::Tail { target: ids::START },
                "prefixes",
                &natural,
            );
            prepared_cpu(&mut host, initial_sp, sr);
            let entry = ids::GAME_PROGRAM.metadata();
            let return_pc = entry.address(revision) + entry.size(revision);
            check(
                &mut host,
                ids::GAME_PROGRAM,
                Exit::Call {
                    target: ids::VDP_INITIALISATION,
                    return_pc,
                },
                "prefixes",
                &natural,
            );
        }
        for warm in [true, false] {
            prepared_cpu(&mut host, initial_sp, 0x271f);
            let mut context = host.context();
            context.write_controller_control(false, if warm { 0x40 } else { 0 });
            context.write_controller_control(true, 0);
            context.write_device_u8(0xa1000d, 0);
            check(
                &mut host,
                ids::START,
                Exit::Tail {
                    target: ids::GAME_PROGRAM,
                },
                "prefixes",
                &natural,
            );
        }
        save(&host, "prefixes", &natural);
        assert_eq!(host.checks.mismatches, 0);
        assert!(host.checks.uninitialized_stack_reads.is_empty());
        assert!(host.checks.unexercised.is_empty());
    }
}

#[test]
#[ignore = "requires both private ROMs; independent of the cold-start DMA-fill join"]
fn platform_direct_restart_and_game_program_entries() {
    let script = script();
    for revision in [Revision::Rev02, Revision::Rev00] {
        let registry = HookRegistry::proof(revision, DIRECT_PREFIXES).unwrap();
        let mut host = Host::new(&bytes(revision), Mode::Reference, registry).unwrap();
        host.set_wait_input_poller(script.clone());
        for frame in 0..1300 {
            host.run_frame(script.at(frame)).unwrap();
        }
        assert!(host.measurements.game_frame.is_some());
        let natural = serde_json::to_value(&host.checks).unwrap();
        host.settle_fixture_boundary(script.at(host.frame_count()))
            .unwrap();
        let vector = rom_symbol(revision, "StartOfRom").unwrap();
        let initial_sp = host.context().read_u32(vector.address);
        for sr in [0x2700, 0x271f] {
            prepared_cpu(&mut host, 0xff7c00, sr);
            check(
                &mut host,
                ids::RESTART_FROM_RESET_ENTRY,
                Exit::Tail { target: ids::START },
                "direct-prefixes",
                &natural,
            );
            prepared_cpu(&mut host, initial_sp, sr);
            let entry = ids::GAME_PROGRAM.metadata();
            let return_pc = entry.address(revision) + entry.size(revision);
            check(
                &mut host,
                ids::GAME_PROGRAM,
                Exit::Call {
                    target: ids::VDP_INITIALISATION,
                    return_pc,
                },
                "direct-prefixes",
                &natural,
            );
        }
        save(&host, "direct-prefixes", &natural);
        assert_eq!(host.checks.mismatches, 0);
        assert!(host.checks.uninitialized_stack_reads.is_empty());
        assert!(host.checks.unexercised.is_empty());
    }
}

#[test]
#[ignore = "requires both private ROMs; quiet ISR fixtures do not replace natural timing proof"]
fn platform_real_exception_frames_with_quiet_live_handlers() {
    let script = script();
    for revision in [Revision::Rev02, Revision::Rev00] {
        let registry = HookRegistry::proof(revision, EXCEPTIONS).unwrap();
        let mut host = Host::new(&bytes(revision), Mode::Reference, registry).unwrap();
        host.set_wait_input_poller(script.clone());
        for frame in 0..1300 {
            host.run_frame(script.at(frame)).unwrap();
        }
        assert!(host.measurements.game_frame.is_some());
        let natural = serde_json::to_value(&host.checks).unwrap();
        host.settle_fixture_boundary(script.at(host.frame_count()))
            .unwrap();
        for id in [ids::LEVEL4_ISR, ids::LEVEL6_ISR] {
            for sp in [0xff7c00, 0xff7b20] {
                for saved_sr in [0x0015, 0x231f] {
                    prepared_cpu(&mut host, sp, 0x2700);
                    let return_pc = ids::NO_OP.metadata().address(revision);
                    let mut context = host.context();
                    context.write_u16(sp, saved_sr);
                    context.write_u32(sp + 2, return_pc);
                    ram::g_h_blank_graphics_enabled(revision).write(&mut context, 0);
                    ram::g_v_blank_func(revision).write(&mut context, Ptr::new(return_pc));
                    check(&mut host, id, Exit::ExceptionReturn, "quiet-isr", &natural);
                }
            }
        }
        save(&host, "quiet-isr", &natural);
        assert_eq!(host.checks.mismatches, 0);
        assert!(host.checks.uninitialized_stack_reads.is_empty());
        assert!(host.checks.unexercised.is_empty());
    }
}
