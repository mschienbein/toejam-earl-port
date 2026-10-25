//! Opt-in both-revision campaign for the frozen modular replay API.
//! No registration, original-main substitution or live-replacement admission.
#[path = "held_proof.rs"]
mod held;

use genesis_config::{GenesisButton, GenesisController, GenesisControllerType, GenesisInputs};
use jgenesis_common::{frontend::MappableInputs, input::Player};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tje_game::{Context, FunctionId, earthlings, ids};
use tje_host::{
    Host, HostError, Mode, Result, execution::WaitInputPoller, hooks::HookRegistry,
    input::InputScript,
};
use tje_mem::{Bus, Revision, ram, rom};
use tje_session::Session;

const ROOTS: &[FunctionId] = &[
    ids::FIND_SPAWN_LOCATION_FOR_EARTHLING,
    ids::EARTHLING_STEER_TOWARD_NEAREST_TARGET,
    ids::EARTHLING_RETAIN_LEVEL_BANK,
    ids::AI_GENERIC_ENEMY_THINK,
    ids::EARTHLING_DISPATCH_ABILITIES,
    ids::PLAYER_PROCESS_LIFE_REQUEST,
    ids::AI_MAILBOX_TOUCH,
    ids::AI_WISEMAN_TOUCH,
    ids::EARTHLING_DISPATCH_PLAYER_CONTACTS,
    ids::PROCESS_EARTHLINGS,
];

struct Timeline {
    session: Session,
    pads: Box<[[u16; 2]]>,
}
impl Timeline {
    fn load(path: &Path) -> Self {
        let session = Session::from_json(&std::fs::read_to_string(path).unwrap()).unwrap();
        let pads = session.inputs().collect();
        Self { session, pads }
    }
}
impl WaitInputPoller for Timeline {
    fn at(&self, frame: u64) -> Result<GenesisInputs> {
        let pads = self
            .pads
            .get(frame as usize)
            .ok_or_else(|| HostError::Input(format!("recorded input ends before frame {frame}")))?;
        let controller = |pad| {
            if pad & tje_session::CONNECTED != 0 {
                GenesisController::new(GenesisControllerType::ThreeButton)
            } else {
                GenesisController::None
            }
        };
        let mut inputs = GenesisInputs {
            p1: controller(pads[0]),
            p2: controller(pads[1]),
        };
        for (pad, player) in pads.iter().copied().zip([Player::One, Player::Two]) {
            for (bit, button) in [
                GenesisButton::Up,
                GenesisButton::Down,
                GenesisButton::Left,
                GenesisButton::Right,
                GenesisButton::A,
                GenesisButton::B,
                GenesisButton::C,
                GenesisButton::Start,
            ]
            .into_iter()
            .enumerate()
            {
                inputs.set_field(button, player, pad & (1 << bit) != 0);
            }
        }
        Ok(inputs)
    }
    fn before_frame(&self, frame: u64, memory: &mut dyn Bus) -> Result<()> {
        for write in self
            .session
            .writes()
            .iter()
            .filter(|write| write.before_frame == frame)
        {
            match write.width {
                1 => memory.write_u8(write.address, write.value as u8),
                2 => memory.write_u16(write.address, write.value as u16),
                4 => memory.write_u32(write.address, write.value),
                _ => unreachable!("validated session recipe width"),
            }
        }
        Ok(())
    }
}

fn fixture(host: &mut Host, id: FunctionId, arguments: &[u32], label: &str) {
    println!(
        "REPLAY_FIXTURE {:?} {} {label}",
        host.revision(),
        id.metadata().name
    );
    host.check_fixture(id.metadata().address(host.revision()), arguments)
        .unwrap_or_else(|error| {
            panic!(
                "{:?} {} ({label}): {error}",
                host.revision(),
                id.metadata().name
            )
        });
}

fn allocated_object(host: &mut Host) -> i8 {
    let revision = host.revision();
    let handle = host
        .context()
        .call_original(ids::ALLOCATE_ENTITY.metadata().address(revision), &[])
        .unwrap() as i8;
    assert!(handle >= 6, "real fixture allocation");
    handle
}

fn dispatcher_fixtures(host: &mut Host) {
    let revision = host.revision();
    let handle = allocated_object(host);
    for bank in 0..2 {
        ram::g_earthling_bank_users(revision)
            .at(bank)
            .write(&mut host.context(), i8::from(bank == 0));
        for slot in 0..20 {
            let record = ram::g_earthling_banks(revision).at(bank).at(slot);
            record.kind().write(&mut host.context(), -1);
            record.object().write(&mut host.context(), 0);
        }
    }
    for number in 0..2 {
        let player = ram::g_player_info(revision).at(number);
        player.x().write(&mut host.context(), 512);
        player.y().write(&mut host.context(), 512);
        player.level().write(&mut host.context(), 1);
        player.sprite_state().write(&mut host.context(), 0);
        player.behavior_flags().write(&mut host.context(), 0);
        ram::g_player_inventory_open(revision)
            .at(number)
            .write(&mut host.context(), 0);
        ram::g_player_elevator_flags(revision)
            .at(number)
            .write(&mut host.context(), 0);
        ram::g_player_wizard_interaction_states(revision)
            .at(number)
            .write(&mut host.context(), 2);
    }
    let record = ram::g_earthling_banks(revision).at(0).at(0);
    let object = ram::g_player_info(revision).at(i32::from(handle));
    record.object().write(&mut host.context(), handle);
    record.second_object().write(&mut host.context(), 0);
    record.level().write(&mut host.context(), 1);
    record.state().write(&mut host.context(), 2);
    record.timer().write(&mut host.context(), 1);
    object.x().write(&mut host.context(), 512);
    object.y().write(&mut host.context(), 512);
    object.level().write(&mut host.context(), 1);
    object.sprite_state().write(&mut host.context(), 2);
    ram::g_earthling_update_parity(revision).write(&mut host.context(), 0);
    ram::g_random_cache_index(revision).write(&mut host.context(), 0);
    ram::g_cached_random_values(revision)
        .at(1)
        .write(&mut host.context(), 0);
    let generic_kind = (0..26)
        .find(|&kind| {
            let address = rom::earthling_definitions(revision)
                .at(kind)
                .ability()
                .read(&host.context())
                .address();
            tje_game::function_at(revision, address) == Some(ids::AI_GENERIC_ENEMY_THINK)
        })
        .expect("actual ROM generic ability definition");
    record.kind().write(&mut host.context(), generic_kind as i8);
    fixture(
        host,
        ids::EARTHLING_DISPATCH_ABILITIES,
        &[],
        "allocated generic callback, non-expired timer",
    );
    record.kind().write(&mut host.context(), 10);
    let touch = rom::earthling_definitions(revision)
        .at(10)
        .touch()
        .read(&host.context())
        .address();
    assert_eq!(
        tje_game::function_at(revision, touch),
        Some(ids::AI_WIZARD_TOUCH)
    );
    fixture(
        host,
        ids::EARTHLING_DISPATCH_PLAYER_CONTACTS,
        &[],
        "allocated wizard, both already-completed contact states",
    );
    record.kind().write(&mut host.context(), -1);
    fixture(
        host,
        ids::PROCESS_EARTHLINGS,
        &[],
        "allocated live bank with no active record callbacks",
    );
}

fn nearest_fixtures(host: &mut Host) {
    let revision = host.revision();
    let handle = allocated_object(host);
    let state = ram::g_player_info(revision).at(i32::from(handle));
    state.x().write(&mut host.context(), 500);
    state.y().write(&mut host.context(), 500);
    state.level().write(&mut host.context(), 1);
    state.sprite_state().write(&mut host.context(), 0);
    state.attachment_x_or_latch().write(&mut host.context(), 0);
    state.attachment_y_or_latch().write(&mut host.context(), 0);
    for number in 0..2 {
        let target = ram::g_player_info(revision).at(number);
        target.x().write(&mut host.context(), 512);
        target.y().write(&mut host.context(), 512);
        target.level().write(&mut host.context(), 1);
        target.sprite_state().write(&mut host.context(), 0);
        target.animation_group_mask().write(&mut host.context(), 0);
        ram::g_player_elevator_flags(revision)
            .at(number)
            .write(&mut host.context(), 0);
        ram::g_player_decoy_entities(revision)
            .at(number)
            .write(&mut host.context(), 0);
        ram::g_player_control_remap_entities(revision)
            .at(number)
            .write(&mut host.context(), 0);
        ram::g_player_inventory(revision)
            .at(number)
            .at(0)
            .write(&mut host.context(), 0);
    }
    for mode in [1, 7, 12, 14, 18] {
        fixture(
            host,
            ids::EARTHLING_STEER_TOWARD_NEAREST_TARGET,
            &[state.address(), 20, 0, mode],
            "nearby ordinary targets; actual mode-specific helper",
        );
    }
    state.level().write(&mut host.context(), 3);
    state.sprite_state().write(&mut host.context(), 2);
    let status = host.cpu().status_register() | 0x10;
    host.context().set_status_register(status);
    fixture(
        host,
        ids::EARTHLING_STEER_TOWARD_NEAREST_TARGET,
        &[state.address(), 1, 0, 1],
        "no candidate, early maximum return, incoming X=1",
    );
    state.sprite_state().write(&mut host.context(), 0);
    fixture(
        host,
        ids::EARTHLING_STEER_TOWARD_NEAREST_TARGET,
        &[state.address(), 255, 0, 1],
        "deliberate no-candidate residual delta words; stack not initialized by fixture",
    );
}

fn persist(host: &Host, session: &Path, natural: &serde_json::Value, error: Option<String>) {
    let directory = PathBuf::from(
        std::env::var_os("TJE_EARTHLINGS_REPLAY_EVIDENCE").expect("private evidence directory"),
    );
    std::fs::create_dir_all(&directory).unwrap();
    let output = serde_json::json!({"revision":format!("{:?}",host.revision()),"source_session":session,"frame":host.frame_count(),"natural":natural,"total":host.checks,"measurements":host.measurements,"error":error,"fixture_future_inputs":"authored neutral input timeline, not a continuation recording","live_replacement_admission":false});
    std::fs::write(
        directory.join(format!("{:?}.json", host.revision())),
        serde_json::to_vec_pretty(&output).unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "requires both private ROMs, one committed session, and integrated replay envelope"]
fn all_pending_roots_one_campaign_and_real_state_fixtures_both_revisions() {
    let path = PathBuf::from(
        std::env::var_os("TJE_EARTHLINGS_REPLAY_SESSION").expect("committed session path"),
    );
    let timeline = Arc::new(Timeline::load(&path));
    assert_eq!(ROOTS.len(), 10, "fixed phase scope");
    for revision in [Revision::Rev02, Revision::Rev00] {
        let key = if revision == Revision::Rev02 {
            "TJE_REV02_ROM"
        } else {
            "TJE_REV00_ROM"
        };
        let bytes = std::fs::read(std::env::var_os(key).expect(key)).unwrap();
        let registry = HookRegistry::proof_replayed_environment(
            revision,
            earthlings::PENDING_TRANSLATIONS,
            earthlings::PENDING_FRAMED_TRANSLATIONS,
            ROOTS,
        )
        .unwrap();
        let mut checked = Host::new(&bytes, Mode::Check, registry.clone()).unwrap();
        let mut reference = Host::new(&bytes, Mode::Reference, registry).unwrap();
        checked.set_wait_input_poller(timeline.clone());
        reference.set_wait_input_poller(timeline.clone());
        for frame in 0..timeline.session.frames {
            timeline
                .before_frame(frame, &mut checked.context())
                .unwrap();
            timeline
                .before_frame(frame, &mut reference.context())
                .unwrap();
            let inputs = timeline.at(frame).unwrap();
            let a = checked.run_frame(inputs).unwrap_or_else(|error| {
                persist(
                    &checked,
                    &path,
                    &serde_json::Value::Null,
                    Some(error.to_string()),
                );
                panic!("{revision:?} natural frame {frame}: {error}");
            });
            let b = reference.run_frame(inputs).unwrap();
            assert_eq!(a.ram, b.ram, "{revision:?} natural RAM {frame}");
            assert_eq!(a.vdp, b.vdp, "{revision:?} natural VDP {frame}");
            assert_eq!(a.writes, b.writes, "{revision:?} natural writes {frame}");
            assert_eq!(a.idle_random_draws, b.idle_random_draws);
            assert_eq!(a.lag_events, b.lag_events);
            assert!(
                a.audio_samples
                    .iter()
                    .flatten()
                    .map(|value| value.to_bits())
                    .eq(b
                        .audio_samples
                        .iter()
                        .flatten()
                        .map(|value| value.to_bits()))
            );
        }
        let natural = serde_json::to_value(&checked.checks).unwrap();
        let neutral = Arc::new(InputScript::default());
        checked.set_wait_input_poller(neutral.clone());
        checked
            .settle_fixture_boundary(neutral.at(checked.frame_count()))
            .unwrap();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            held::frame_fixtures(&mut checked);
            for name in [
                "AI_GenericEnemyThink",
                "AI_MailboxTouch",
                "AI_WisemanTouch",
                "PlayerProcessLifeRequest",
            ] {
                held::friendly_fixtures(&mut checked, Some(name));
            }
            dispatcher_fixtures(&mut checked);
            nearest_fixtures(&mut checked);
        }));
        persist(
            &checked,
            &path,
            &natural,
            outcome
                .as_ref()
                .err()
                .map(|_| "fixture assertion failed; see diagnostic output".into()),
        );
        if let Err(error) = outcome {
            std::panic::resume_unwind(error);
        }
        assert_eq!(checked.checks.mismatches, 0);
        for &id in ROOTS {
            assert!(
                checked.checks.by_function[id.metadata().name].calls_checked > 0,
                "{revision:?} {} unexercised",
                id.metadata().name
            );
        }
    }
}
