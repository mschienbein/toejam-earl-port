//! Opt-in strict full-register/CCR proofs for actual player producers.
//! The original-callee stage isolates the corrected producer bodies; the
//! composed stage uses the actual production dispatcher, never output shims.
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, LazyLock},
};
use tje_game::{Context, FunctionId, Translation, ids};
use tje_host::{Host, Mode, abi::HookContext, hooks::HookRegistry, input::InputScript};
use tje_mem::{Address, ObjectState, Revision, ram};

mod proof_support;

const ROOTS: [FunctionId; 3] = [
    ids::STOP_PLAYER,
    ids::GET_ACTIVE_PRESENT_SPRITE,
    ids::OPEN_ROCKET_SKATES,
];
static OWN: LazyLock<Vec<Translation>> = LazyLock::new(|| {
    ROOTS
        .iter()
        .map(|id| {
            *tje_game::player::TRANSLATIONS
                .iter()
                .find(|translation| translation.id == *id)
                .expect("production player producer")
        })
        .collect()
});
static WITHOUT_STOP: LazyLock<Vec<Translation>> = LazyLock::new(|| {
    OWN.iter()
        .copied()
        .filter(|translation| translation.id != ids::STOP_PLAYER)
        .collect()
});
static WITHOUT_GETTER: LazyLock<Vec<Translation>> = LazyLock::new(|| {
    OWN.iter()
        .copied()
        .filter(|translation| translation.id != ids::GET_ACTIVE_PRESENT_SPRITE)
        .collect()
});
static WITHOUT_SKATES: LazyLock<Vec<Translation>> = LazyLock::new(|| {
    OWN.iter()
        .copied()
        .filter(|translation| translation.id != ids::OPEN_ROCKET_SKATES)
        .collect()
});

fn body(
    id: FunctionId,
    context: &mut HookContext<'_>,
    arguments: &[u32],
) -> tje_host::Result<Option<u32>> {
    let translation = OWN.iter().find(|translation| translation.id == id).unwrap();
    Ok((translation.function)(context, arguments)?)
}
fn strict_stop(context: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
    let player = context.argument_u32(0);
    body(ids::STOP_PLAYER, context, &[player])
}
fn strict_getter(context: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
    let player = context.argument_u32(0);
    body(ids::GET_ACTIVE_PRESENT_SPRITE, context, &[player])
}
fn strict_skates(context: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
    let arguments = [
        context.argument_u32(0),
        context.argument_u32(1),
        context.argument_u32(2),
    ];
    body(ids::OPEN_ROCKET_SKATES, context, &arguments)
}

#[derive(Clone, Copy)]
struct Case {
    label: &'static str,
    power: i8,
    terrain: u8,
    elevator: i8,
    height: i16,
    group: u32,
    state: i8,
    result: i32,
}
const CASES: &[Case] = &[
    Case {
        label: "innertube-water",
        power: 2,
        terrain: 0x8f,
        elevator: 0,
        height: 0,
        group: 4,
        state: 11,
        result: 77,
    },
    Case {
        label: "innertube-land",
        power: 2,
        terrain: 0x1f,
        elevator: 0,
        height: 0,
        group: 4,
        state: 11,
        result: 76,
    },
    Case {
        label: "innertube-elevator",
        power: 2,
        terrain: 0x10,
        elevator: 1,
        height: 0,
        group: 4,
        state: 11,
        result: 0,
    },
    Case {
        label: "swimming",
        power: -1,
        terrain: 0x80,
        elevator: 0,
        height: 0,
        group: 4,
        state: 11,
        result: 3,
    },
    Case {
        label: "spring-shoes",
        power: 1,
        terrain: 0x10,
        elevator: 0,
        height: 0,
        group: 4,
        state: 11,
        result: 57,
    },
    Case {
        label: "spring-elevator",
        power: 1,
        terrain: 0x10,
        elevator: 1,
        height: 0,
        group: 4,
        state: 11,
        result: 0,
    },
    Case {
        label: "icarus-airborne-early-return",
        power: 0,
        terrain: 0x10,
        elevator: 0,
        height: 1,
        group: 0x8000,
        state: 60,
        result: -1,
    },
    Case {
        label: "icarus-flying-group-on-ground",
        power: 0,
        terrain: 0x10,
        elevator: 0,
        height: 0,
        group: 0x8000,
        state: 60,
        result: 4,
    },
    Case {
        label: "icarus-idle",
        power: 0,
        terrain: 0x10,
        elevator: 0,
        height: 1,
        group: 4,
        state: 11,
        result: 4,
    },
    Case {
        label: "icarus-elevator",
        power: 0,
        terrain: 0x10,
        elevator: -1,
        height: 1,
        group: 0x8000,
        state: 60,
        result: 0,
    },
    Case {
        label: "ordinary-idle",
        power: -1,
        terrain: 0x10,
        elevator: 0,
        height: 0,
        group: 4,
        state: 11,
        result: 0,
    },
    Case {
        label: "other-present-idle",
        power: 6,
        terrain: 0x10,
        elevator: 0,
        height: 0,
        group: 4,
        state: 11,
        result: 0,
    },
    Case {
        label: "above-water",
        power: -1,
        terrain: 0x80,
        elevator: 0,
        height: 1,
        group: 4,
        state: 11,
        result: 0,
    },
    Case {
        label: "skates-ground",
        power: 5,
        terrain: 0x10,
        elevator: 0,
        height: 0,
        group: 4,
        state: 8,
        result: -1,
    },
    Case {
        label: "skates-water",
        power: 5,
        terrain: 0x80,
        elevator: 0,
        height: 0,
        group: 4,
        state: 9,
        result: -1,
    },
    Case {
        label: "skates-void",
        power: 5,
        terrain: 0x90,
        elevator: 0,
        height: 0,
        group: 4,
        state: 10,
        result: -1,
    },
    Case {
        label: "skates-airborne",
        power: 5,
        terrain: 0x10,
        elevator: 0,
        height: 20,
        group: 4,
        state: 11,
        result: -1,
    },
    Case {
        label: "skates-underwater-group",
        power: 5,
        terrain: 0x80,
        elevator: 0,
        height: 0,
        group: 0x800,
        state: 44,
        result: -1,
    },
    Case {
        label: "skates-swim-state",
        power: 5,
        terrain: 0x80,
        elevator: 0,
        height: 0,
        group: 0,
        state: 3,
        result: -1,
    },
    Case {
        label: "skates-fall-state",
        power: 5,
        terrain: 0x10,
        elevator: 0,
        height: 0,
        group: 0,
        state: 7,
        result: -1,
    },
    Case {
        label: "skates-unclassified-group",
        power: 5,
        terrain: 0x10,
        elevator: 0,
        height: -1,
        group: 0,
        state: 11,
        result: -1,
    },
];

fn seed(host: &mut Host, number: i32, case: Case, pattern: u32, ccr: u16) -> Address<ObjectState> {
    let revision = host.revision();
    let player = ram::g_player_info(revision).at(number);
    let mut context = host.context();
    player.entity_index().write(&mut context, number as i8);
    player.level().write(&mut context, 1);
    player.x().write(&mut context, 512);
    player.y().write(&mut context, 512);
    player.height().write(&mut context, case.height);
    player.sprite_state().write(&mut context, case.state);
    player
        .animation_group_mask()
        .write(&mut context, case.group);
    player
        .draw_flags()
        .write(&mut context, if pattern & 1 != 0 { 0x81 } else { 1 });
    player.frame().write(&mut context, 3);
    player.animation_motion_axes().write(&mut context, 3);
    player.velocity_x().write(&mut context, -320);
    player.acceleration_x().write(&mut context, 71);
    player.velocity_y().write(&mut context, 240);
    player.acceleration_y().write(&mut context, -57);
    player.velocity_height().write(&mut context, 180);
    player.acceleration_height().write(&mut context, -17);
    ram::g_player_powerups(revision)
        .at(number)
        .write(&mut context, case.power);
    ram::g_player_elevator_flags(revision)
        .at(number)
        .write(&mut context, case.elevator);
    ram::g_player_previous_processed_input(revision)
        .at(number)
        .write(&mut context, 0xff);
    ram::g_player1_terrain_underfoot(revision).write(&mut context, case.terrain);
    ram::g_player2_terrain_underfoot(revision).write(&mut context, case.terrain);
    for index in 0..8 {
        context.set_data_register(index, pattern ^ ((index as u32 + 1) * 0x1020_304));
    }
    for index in 0..7 {
        context.set_address_register(index, 0x00ff_e000 + index as u32 * 0x100);
    }
    let control = context.status_register() & !0x001f;
    context.set_status_register(control | ccr);
    player
}

pub fn main() {
    prove(&[]);
}

/// Private joint-proof harnesses may supply another owner's canonical bodies;
/// this never changes their production registration or substitutes their outputs.
pub fn prove(overrides: &'static [Translation]) {
    let output = PathBuf::from(std::env::args_os().nth(1).expect("evidence directory"));
    fs::create_dir_all(&output).unwrap();
    let production = std::env::var_os("PLAYER_PRODUCER_PRODUCTION").is_some();
    let timeline = Arc::new(
        InputScript::parse(
            &fs::read_to_string("crates/tje-host/examples/start-game.json").unwrap(),
        )
        .unwrap(),
    );
    let mut failures = 0;
    for (revision, name) in [(Revision::Rev02, "rev02"), (Revision::Rev00, "rev00")] {
        for root in ROOTS {
            let mut registry = if production {
                HookRegistry::proof(revision, overrides).unwrap()
            } else {
                let callees: &'static [Translation] = match root {
                    ids::STOP_PLAYER => &WITHOUT_STOP,
                    ids::GET_ACTIVE_PRESENT_SPRITE => &WITHOUT_GETTER,
                    ids::OPEN_ROCKET_SKATES => &WITHOUT_SKATES,
                    _ => unreachable!(),
                };
                HookRegistry::from_translations(revision, callees).unwrap()
            };
            let mut symbol = root.metadata().clone();
            symbol.abi.strict_registers = true;
            let hook: tje_host::hooks::HookFn = match root {
                ids::STOP_PLAYER => strict_stop,
                ids::GET_ACTIVE_PRESENT_SPRITE => strict_getter,
                ids::OPEN_ROCKET_SKATES => strict_skates,
                _ => unreachable!(),
            };
            registry.register_contract(symbol, true, hook).unwrap();
            let mut host = Host::new(
                &fs::read(format!("roms/{name}.bin")).unwrap(),
                Mode::Reference,
                registry,
            )
            .unwrap();
            host.set_wait_input_poller(timeline.clone());
            for frame in 0..1300 {
                host.run_frame(timeline.at(frame)).unwrap();
            }
            let boundary = proof_support::settle(&mut host, &timeline);
            let mut rows = Vec::new();
            let mut errors = Vec::new();
            for number in 0..2 {
                for pattern in [0_u32, 0x1234_5678, 0xffff_ffff] {
                    for ccr in 0..32 {
                        for &case in CASES {
                            if root == ids::STOP_PLAYER && case.label != "ordinary-idle" {
                                continue;
                            }
                            if root == ids::OPEN_ROCKET_SKATES && case.power != 5 {
                                continue;
                            }
                            let player = seed(&mut host, number, case, pattern, ccr);
                            let arguments = if root == ids::OPEN_ROCKET_SKATES {
                                vec![
                                    player.address(),
                                    0xa55a_0000 | number as u32,
                                    0x5aa5_0000 | u32::from(case.terrain & 0xf0),
                                ]
                            } else {
                                vec![player.address()]
                            };
                            let result =
                                host.check_fixture(root.metadata().address(revision), &arguments);
                            if let Err(error) = result {
                                errors.push(serde_json::json!({"case":case.label,"player":number,"pattern":pattern,"ccr":ccr,"error":error.to_string()}));
                                break;
                            }
                            if root == ids::GET_ACTIVE_PRESENT_SPRITE {
                                assert_eq!(
                                    result.unwrap(),
                                    case.result as u32,
                                    "getter fixture expected return"
                                );
                            }
                        }
                        if !errors.is_empty() {
                            break;
                        }
                    }
                    if !errors.is_empty() {
                        break;
                    }
                }
                if !errors.is_empty() {
                    break;
                }
            }
            rows.extend(
                CASES
                    .iter()
                    .filter(|case| root != ids::STOP_PLAYER || case.label == "ordinary-idle")
                    .filter(|case| root != ids::OPEN_ROCKET_SKATES || case.power == 5)
                    .map(|case| case.label),
            );
            let evidence = serde_json::json!({"revision":name,"function":root.metadata().name,"strict_registers":true,"production_callees":production,"canonical_overrides":overrides.iter().map(|translation|translation.id.metadata().name).collect::<Vec<_>>(),"kind":"labelled live gameplay fixtures; both players, three full data-register patterns and all32 entryCCR values","case_labels":rows,"settled_boundary":boundary,"checks":host.checks,"errors":errors});
            fs::write(
                output.join(format!("{name}-{}-producer.json", root.metadata().name)),
                serde_json::to_vec_pretty(&evidence).unwrap(),
            )
            .unwrap();
            println!(
                "PLAYER_PRODUCER {name} {} checked={} mismatches={} errors={}",
                root.metadata().name,
                host.checks.calls,
                host.checks.mismatches,
                errors.len()
            );
            failures += u32::from(
                !errors.is_empty()
                    || host.checks.mismatches != 0
                    || host.checks.by_function[root.metadata().name].calls_checked == 0,
            );
        }
    }
    assert_eq!(
        failures, 0,
        "strict producer mismatch; both revision receipts retained"
    );
}
