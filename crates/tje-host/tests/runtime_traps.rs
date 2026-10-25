//! Labelled input perturbations of a live game state, not natural zero-divisor
//! reachability or a CPU/device timing claim. No game/ROM-derived payload stored.
use std::{fs, path::PathBuf, sync::Arc};
use tje_game::{
    Context,
    exits::{EntryOutcome, Exit, StackBanks},
    ids,
};
use tje_host::{Host, Mode, hooks::HookRegistry, input::InputScript};
use tje_mem::Revision;

fn rom(revision: Revision) -> Vec<u8> {
    let (key, default) = match revision {
        Revision::Rev02 => ("TJE_REV02_ROM", "../../roms/rev02.bin"),
        Revision::Rev00 => ("TJE_REV00_ROM", "../../roms/rev00.bin"),
    };
    fs::read(
        std::env::var_os(key)
            .map(PathBuf::from)
            .unwrap_or_else(|| default.into()),
    )
    .unwrap()
}

#[test]
#[ignore = "private ROMs; labelled real-state zero-divisor exception entries in both revisions"]
fn zero_divisor_enters_actual_vector_five_from_real_states_both_revisions() {
    let script = Arc::new(InputScript::parse(include_str!("../examples/start-game.json")).unwrap());
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut host = Host::new(
            &rom(revision),
            Mode::Reference,
            HookRegistry::translated(revision).unwrap(),
        )
        .unwrap();
        for frame in 0..1300 {
            host.run_frame(script.at(frame)).unwrap();
        }
        host.set_wait_input_poller(script.clone());
        host.settle_fixture_boundary(script.at(host.frame_count()))
            .unwrap();
        let (base_data, base_address, base_status, base_banks) = {
            let context = host.context();
            (
                std::array::from_fn::<_, 8, _>(|i| context.data_register(i)),
                std::array::from_fn::<_, 7, _>(|i| context.address_register(i)),
                context.status_register(),
                context.stack_banks().unwrap(),
            )
        };
        for entry in [ids::LMODT, ids::LDIVT, ids::ULMODT, ids::ULDIVT] {
            let before = host.checks.by_function[entry.metadata().name].calls_checked;
            for dividend in [0, 7, (-7i32) as u32, i32::MIN as u32, u32::MAX, 0x12348000] {
                for incoming_x in [0, 0x10] {
                    for supervisor in [true, false] {
                        {
                            let mut context = host.context();
                            for (i, value) in base_data.into_iter().enumerate() {
                                context.set_data_register(i, value);
                            }
                            for (i, value) in base_address.into_iter().enumerate() {
                                context.set_address_register(i, value);
                            }
                            context.set_status_register(
                                (base_status & !0x801f & !0x2000)
                                    | if supervisor { 0x2000 } else { 0 }
                                    | incoming_x,
                            );
                            context
                                .set_stack_banks(StackBanks {
                                    supervisor: base_banks.supervisor,
                                    user: if supervisor {
                                        base_banks.user
                                    } else {
                                        0xff6000
                                    },
                                })
                                .unwrap();
                            context.set_data_register(0, dividend);
                            context.set_data_register(1, 0);
                            context.set_data_register(2, 0x89abcdef);
                        }
                        // No caller PC/sentinel or exception frame is invented.
                        // The API executes each side from this labelled entry
                        // input on isolated forks and compares at real vector5.
                        let outcome = host.check_entry_fixture(entry.metadata().address(revision)).unwrap_or_else(|e|
                            panic!("labelled zero-divisor {revision:?}/{} d0={dividend:08x} x={incoming_x:x} s={supervisor}: {e}", entry.metadata().name));
                        let EntryOutcome::Transfer(transfer) = outcome else {
                            panic!("trap must not return D0");
                        };
                        let Exit::Exception { request } = transfer.exit else {
                            panic!("trap must enter vector5");
                        };
                        assert_eq!(request.trap.vector(), Some(5));
                        assert_eq!(transfer.entry, entry);
                        assert_eq!(host.checks.by_function[entry.metadata().name].mismatches, 0);
                    }
                }
            }
            assert_eq!(
                host.checks.by_function[entry.metadata().name].calls_checked - before,
                24
            );
            println!(
                "LABELLED_RUNTIME_TRAP {revision:?} {} checked=24 mismatches=0 source=live-original-frame-1300 supervisor-and-user finite-inputs-only",
                entry.metadata().name
            );
        }
        assert_eq!(host.checks.mismatches, 0);
    }
}
