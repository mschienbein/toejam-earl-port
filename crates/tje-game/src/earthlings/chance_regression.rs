//! Opt-in RH06/RH07 unchecked-definition probes. Inputs, not cartridge patches.
use tje_game::ids;
use tje_host::{Host, Mode, hooks::HookRegistry};
use tje_mem::{Ptr, Revision, ram, rom};

fn host(revision: Revision) -> Host {
    let key = if revision == Revision::Rev02 {
        "TJE_REV02_ROM"
    } else {
        "TJE_REV00_ROM"
    };
    let bytes = std::fs::read(std::env::var_os(key).expect(key)).unwrap();
    let selected = [
        ids::EARTHLING_THINK_HORIZONTAL_WANDER,
        ids::EARTHLING_THINK_TIMED_MOVEMENT,
    ];
    let registry = HookRegistry::filtered(revision, &[], &selected).unwrap();
    let mut host = Host::new(&bytes, Mode::Reference, registry).unwrap();
    let registers = host.cpu().address_registers();
    host.context()
        .cpu()
        .set_address_registers(registers, 0xff0400, 0xff0400);
    host
}

#[test]
#[ignore = "requires both private ROM revisions"]
fn signed_negative_chances_match_original_and_consume_identical_rng() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for id in [
            ids::EARTHLING_THINK_HORIZONTAL_WANDER,
            ids::EARTHLING_THINK_TIMED_MOVEMENT,
        ] {
            let mut host = host(revision);
            let record = ram::g_earthling_banks(revision).at(0).at(0);
            let object = ram::g_player_info(revision).at(6);
            {
                let mut context = host.context();
                record.kind().write(&mut context, -123);
                record.object().write(&mut context, 6);
                record.state().write(&mut context, 2);
                record.timer().write(&mut context, 0);
                object.level().write(&mut context, 1);
                object.sprite_state().write(&mut context, 6);
                object.x().write(&mut context, 200);
                object.y().write(&mut context, 500);
                let target = ram::g_player_info(revision).at(0);
                target.level().write(&mut context, 1);
                target.x().write(&mut context, 160);
                target.y().write(&mut context, 500);
                ram::g_player_info(revision)
                    .at(1)
                    .level()
                    .write(&mut context, 2);
                let definition = rom::earthling_definitions(revision).at(-123);
                assert!(definition.target_chance().read(&context) < 0);
                let animation = definition.states().read(&context).target().offset(6);
                animation.sequence().write(
                    &mut context,
                    Ptr::new(rom::single_frame_hold_animation(revision).address()),
                );
                animation.frames().write(&mut context, Ptr::new(0));
                animation.period().write(&mut context, 6);
            }
            host.check_fixture(id.metadata().address(revision), &[record.address()])
                .unwrap_or_else(|error| {
                    panic!(
                        "{revision:?} {} negative target: {error}",
                        id.metadata().name
                    )
                });
            assert_eq!(host.checks.mismatches, 0);
            assert_eq!(host.checks.by_function[id.metadata().name].calls_checked, 1);
            println!(
                "SIGNED_CHANCE {revision:?} {} target: 1 checked, 0 mismatches",
                id.metadata().name
            );
        }
        let mut host = host(revision);
        let record = ram::g_earthling_banks(revision).at(0).at(0);
        let object = ram::g_player_info(revision).at(6);
        {
            let mut context = host.context();
            record.kind().write(&mut context, 101);
            record.object().write(&mut context, 6);
            record.state().write(&mut context, 6);
            object.sprite_state().write(&mut context, 6);
            ram::g_cached_random_values(revision)
                .at(1)
                .write(&mut context, 50);
            ram::g_cached_random_values(revision)
                .at(2)
                .write(&mut context, 50);
            ram::g_cached_random_values(revision)
                .at(3)
                .write(&mut context, 2);
            assert!(
                rom::earthling_definitions(revision)
                    .at(101)
                    .turn_chance()
                    .read(&context)
                    < 0
            );
        }
        let id = ids::EARTHLING_THINK_TIMED_MOVEMENT;
        host.check_fixture(id.metadata().address(revision), &[record.address()])
            .unwrap_or_else(|error| panic!("{revision:?} negative turn: {error}"));
        assert_eq!(host.checks.mismatches, 0);
        assert_eq!(host.checks.by_function[id.metadata().name].calls_checked, 1);
        println!("SIGNED_CHANCE {revision:?} timed turn: 1 checked, 0 mismatches");
    }
}
