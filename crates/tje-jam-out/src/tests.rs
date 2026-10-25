use super::*;
use tje_mem::AddressSpace;

fn enabled() -> Settings {
    Settings {
        enabled: true,
        ..Settings::default()
    }
}

fn world(revision: Revision) -> AddressSpace {
    let mut bus = AddressSpace::new(vec![0; 0x100000], revision);
    for index in 0..2 {
        let actor = ram::g_player_info(revision).at(index);
        actor.draw_flags().write(&mut bus, 1);
        ram::g_player_health(revision).at(index).write(&mut bus, 10);
        ram::g_player_powerups(revision)
            .at(index)
            .write(&mut bus, -1);
    }
    ram::g_player1_terrain_underfoot(revision).write(&mut bus, 0x10);
    ram::g_player2_terrain_underfoot(revision).write(&mut bus, 0x10);
    bus
}

#[test]
fn disabled_preserves_every_original_input_byte() {
    assert!(!Settings::default().enabled);
    for raw in 0..=u8::MAX {
        for modifier in [false, true] {
            let mut seat = Seat::default();
            assert_eq!(seat.input(raw, modifier, true, Settings::default()), raw);
            assert_eq!(seat.sequence, Sequence::None);
            assert_eq!(seat.pending, Instrument::None);
        }
    }
}

#[test]
fn modifier_consumes_only_jam_actions_and_never_pause() {
    let mut seat = Seat::default();
    assert_eq!(seat.input(A, true, true, enabled()), 0);
    assert_eq!(seat.sequence, Sequence::Bass);
    assert_eq!(seat.pending, Instrument::Bass);
    assert_eq!(seat.input(START | A | 1, true, true, enabled()), START | 1);
    assert_eq!(seat.sequence, Sequence::None);
    assert_eq!(seat.pending, Instrument::None);
    assert_eq!(seat.input(1, true, true, enabled()), 1);
    for actions in [A, B, C, ACTIONS] {
        let mut fresh = Seat::default();
        assert_eq!(fresh.input(START | actions, true, true, enabled()), START);
        assert_eq!(fresh.sequence, Sequence::None);
        assert_eq!(fresh.pending, Instrument::None);
        assert_eq!(fresh.input(actions, false, true, enabled()), 0);
    }
}

#[test]
fn release_latch_prevents_inventory_or_map_leaking() {
    for action in [A, B, C] {
        let mut seat = Seat::default();
        assert_eq!(seat.input(action, true, true, enabled()), 0);
        assert_eq!(seat.input(action, false, true, enabled()), 0);
        assert_eq!(seat.sequence, Sequence::None);
        assert_eq!(seat.input(0, false, true, enabled()), 0);
        assert_eq!(seat.input(action, false, true, enabled()), action);
    }
}

#[test]
fn chord_arms_on_tenth_input_phase_and_cancels_without_dispatch() {
    let settings = Settings {
        chord: true,
        ..enabled()
    };
    let mut seat = Seat::default();
    for tick in 1..=9 {
        assert_eq!(seat.input(B | C, false, true, settings), 0);
        assert_eq!(seat.chord_ticks, tick);
        assert_eq!(seat.sequence, Sequence::None);
    }
    assert_eq!(seat.input(B | C, false, true, settings), 0);
    assert_eq!(seat.sequence, Sequence::None);
    // B/C are the arming gesture, not instruments. Only A should perform here.
    assert_eq!(seat.input(B | C | A, false, true, settings), 0);
    assert_eq!(seat.sequence, Sequence::Bass);
    assert_eq!(seat.input(C, false, true, settings), 0);
    assert_eq!(seat.sequence, Sequence::None);
    assert_eq!(seat.input(0, false, true, settings), 0);
    assert_eq!(seat.input(C, false, true, settings), C);
}

#[test]
fn gestures_use_separate_alternation_and_locked_actions() {
    let mut seat = Seat::default();
    seat.input(A, true, true, enabled());
    assert_eq!(seat.sequence, Sequence::Bass);
    seat.input(0, true, true, enabled());
    seat.input(A, true, true, enabled());
    assert_eq!(seat.sequence, Sequence::BassAlternate);
    seat.input(C, true, true, enabled());
    assert_eq!(seat.sequence, Sequence::Clap);
    assert!(seat.locked);
    seat.pending = Instrument::None;
    seat.input(A | 1, true, true, enabled());
    assert_eq!(seat.sequence, Sequence::Clap);
    assert_eq!(seat.pending, Instrument::None);
    seat.input(0, false, true, enabled());
    assert_eq!(seat.sequence, Sequence::None);
    assert!(!seat.locked);
}

#[test]
fn unsafe_pose_does_not_modify_world_or_original_rng() {
    for revision in Revision::ALL {
        let mut bus = world(revision);
        assert!(eligible(&bus, revision, 0));
        let before = bus.ram().to_vec();
        let mut seat = Seat::default();
        seat.input(A, true, eligible(&bus, revision, 0), enabled());
        assert_eq!(before, bus.ram());
        for state in [2, 3, 5, 6, 70, 78, 79, 89, 116, 120] {
            ram::g_player_info(revision)
                .at(0)
                .sprite_state()
                .write(&mut bus, state);
            assert!(!eligible(&bus, revision, 0));
            seat.input(A, true, false, enabled());
            assert_eq!(seat.sequence, Sequence::None);
        }
        ram::g_player_info(revision)
            .at(0)
            .sprite_state()
            .write(&mut bus, 0);
        for terrain in [0x20, 0x80, 0x90] {
            ram::g_player1_terrain_underfoot(revision).write(&mut bus, terrain);
            assert!(!eligible(&bus, revision, 0));
        }
        ram::g_player1_terrain_underfoot(revision).write(&mut bus, 0x10);
        for powerup in [0, 1, 2, 3, 4, 5] {
            ram::g_player_powerups(revision)
                .at(0)
                .write(&mut bus, powerup);
            assert!(!eligible(&bus, revision, 0), "active powerup {powerup}");
        }
        ram::g_player_powerups(revision).at(0).write(&mut bus, -1);
        assert!(
            eligible(&bus, revision, 0),
            "PRESENT_NONE is -1, not Icarus wings (0)"
        );
    }
}

#[test]
fn sounds_while_busy_is_explicit_and_animation_never_overrides_hurt() {
    let mut seat = Seat::default();
    seat.input(A, true, false, enabled());
    assert_eq!(seat.pending, Instrument::None);
    let settings = Settings {
        sounds_while_busy: true,
        ..enabled()
    };
    seat.input(A, true, false, settings);
    assert_eq!(seat.pending, Instrument::Bass);
    assert_eq!(seat.sequence, Sequence::None);
}

#[test]
fn configuration_and_snapshot_roundtrip_include_latches_and_cooldowns() {
    let mut state = State::new(enabled());
    state.seats[0].input(C, true, true, enabled());
    state.pcm_cooldown = 5;
    state.audio_frame = Some(71);
    state.requested.backing = 4;
    let bytes = bincode::encode_to_vec(&state, bincode::config::standard()).unwrap();
    assert!(bytes.len() < 2048);
    let (restored, used): (State, _) =
        bincode::decode_from_slice(&bytes, bincode::config::standard()).unwrap();
    assert_eq!(used, bytes.len());
    assert_eq!(state, restored);
    state.apply_requested();
    assert_eq!(state.configuration_events, 1);
    assert_eq!(state.active_poses(), [false; 2]);
    state.cancel();
}

#[test]
fn runtime_rom_animation_clock_never_uses_draw_count() {
    for revision in Revision::ALL {
        // Authored sequence bytes only; no cartridge animation data is stored.
        let mut bytes = vec![0; 0x100000];
        let sequence = Sequence::Clap.address(revision) as usize;
        bytes[sequence..sequence + 3].copy_from_slice(&[2, 0x83, 0xfe]);
        let transition = Sequence::IdleTransition.address(revision) as usize;
        bytes[transition..transition + 2].copy_from_slice(&[4, 0xfe]);
        let mut bus = AddressSpace::new(bytes, revision);
        ram::g_player_info(revision)
            .at(0)
            .draw_flags()
            .write(&mut bus, 1);
        ram::g_player_health(revision).at(0).write(&mut bus, 10);
        ram::g_player_powerups(revision).at(0).write(&mut bus, -1);
        ram::g_player1_terrain_underfoot(revision).write(&mut bus, 0x10);
        let mut state = State::new(enabled());
        state.in_play = true;
        state.seats[0].input(C, true, true, enabled());
        state.seats[0].animate(&bus, revision);
        assert_eq!(state.seats[0].frame, 0);
        let before = state.clone();
        for _ in 0..20 {
            let _ = state.presentation(&bus, revision);
        }
        assert_eq!(state, before);
        for _ in 0..3 {
            state.seats[0].animate(&bus, revision);
        }
        assert_eq!(state.seats[0].frame, 1);
        for _ in 0..3 {
            state.seats[0].animate(&bus, revision);
        }
        assert_eq!(state.seats[0].sequence, Sequence::IdleTransition);
        assert!(!state.seats[0].locked);
    }
}

#[test]
fn audio_arbiter_preserves_original_owners_and_never_accumulates_latency() {
    let mut state = State::new(enabled());
    state.seats[0].pending = Instrument::Bass;
    state.seats[1].pending = Instrument::Clap;
    assert_eq!(
        state.arbitrate(1, true, false, false),
        [Instrument::None; 2]
    );
    assert_eq!(state.dropped_requests, 2);
    assert_eq!(state.arbitrate(2, true, true, true), [Instrument::None; 2]);
    state.seats[0].pending = Instrument::Bass;
    state.seats[1].pending = Instrument::Clap;
    assert_eq!(
        state.arbitrate(3, true, true, true),
        [Instrument::Bass, Instrument::Clap]
    );
    assert_eq!(state.accepted_pcm, 1);
    assert_eq!(state.accepted_fm, 1);
    assert_eq!(state.arbitrate(3, true, true, true), [Instrument::None; 2]);
    assert_eq!(
        state.pcm_cooldown, 6,
        "same-frame nested service cannot consume a cooldown tick"
    );
    for frame in 4..9 {
        state.seats[0].pending = Instrument::Bass;
        assert_eq!(
            state.arbitrate(frame, true, true, true)[0],
            Instrument::None
        );
    }
    state.seats[0].pending = Instrument::Snare;
    assert_eq!(state.arbitrate(9, true, true, true)[0], Instrument::Snare);
    state.seats[1].pending = Instrument::Clap;
    assert_eq!(
        state.arbitrate(10, false, true, true),
        [Instrument::None; 2],
        "bank changes and authored scenes own their service slots"
    );
}

#[test]
fn muted_performance_is_pose_only_and_disable_clears_a_pending_gesture() {
    let mut state = State::new(Settings {
        performance_sounds: false,
        ..enabled()
    });
    state.seats[0].input(A, true, true, state.settings);
    assert_eq!(state.seats[0].sequence, Sequence::Bass);
    assert_eq!(state.seats[0].pending, Instrument::None);
    state.requested.enabled = false;
    assert_eq!(state.arbitrate(1, true, true, true), [Instrument::None; 2]);
    assert_eq!(state.active_poses(), [false; 2]);
}

#[cfg(feature = "enhanced-mode")]
#[test]
fn profiles_are_distinct_and_sidecars_are_per_session() {
    use tje_engine_runtime::enhanced::EnhancedMode;
    for revision in Revision::ALL {
        let mut mode = EnhancedMode::new(PROFILE, revision).unwrap();
        configure(&mut mode, enabled()).unwrap();
        assert_ne!(mode.profile_id(), "faithful");
        assert_eq!(
            mode.sidecars().get::<State>().unwrap().requested_settings(),
            enabled()
        );
        assert_eq!(EnhancedMode::default().profile_id(), "faithful");
    }
}
