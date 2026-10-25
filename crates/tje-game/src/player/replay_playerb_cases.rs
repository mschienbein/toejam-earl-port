//! Labelled mutations of a settled, power-on-reached game. These are diagnostic
//! paths, never described as natural-session coverage or saved-state inputs.
use tje_game::{FunctionId, ids};
use tje_host::Host;
use tje_mem::ram;

pub const FUNCTIONS: &[FunctionId] = &[
    ids::LEMONADE_ABILITY,
    ids::HULA_ABILITY,
    ids::PLAYER_UPDATE_WHIRLWIND_AND_RECOVERY,
    ids::PLAYER_YOUCH,
    ids::CHICKEN_ABILITY,
    ids::AI_BOOGIE_TOUCH,
    ids::HOTTUB_ABILITY,
    ids::PLAYER_APPLY_PHASED_JUMP_INPUT,
    ids::PLAYER_UPDATE_PHASED_JUMP_MOTION,
    ids::PLAYER_CHOOSE_DIRECTIONAL_MOVEMENT,
    ids::SPRITE_CHECKS,
];

pub fn labels(id: FunctionId) -> &'static [&'static str] {
    match id {
        ids::LEMONADE_ABILITY => &[
            "approach",
            "burp",
            "reward-pending",
            "earthling-frame-finished",
            "no-op",
        ],
        ids::HULA_ABILITY => &[
            "level-zero",
            "outside-range",
            "delay",
            "dance-no-speech",
            "near-speech",
            "near-cooldown",
            "leave-dance",
        ],
        ids::PLAYER_UPDATE_WHIRLWIND_AND_RECOVERY => &[
            "inactive",
            "recovery-tick",
            "recovery-free",
            "recovery-sick",
            "recovery-water",
            "recovery-void",
            "whirlwind-expire",
            "whirlwind-diagonal",
            "whirlwind-countdown",
        ],
        ids::PLAYER_YOUCH => &[
            "ordinary",
            "shark-reject",
            "shark-swim",
            "dentist",
            "dentist-variant",
            "moving-left",
        ],
        ids::CHICKEN_ABILITY => &["near-hit", "distant-target", "state-eight"],
        ids::AI_BOOGIE_TOUCH => &["hit", "state-eight", "ineligible"],
        ids::HOTTUB_ABILITY => &[
            "walk-in-right",
            "windup",
            "takeoff",
            "descend",
            "splash",
            "exit-input",
            "chat",
            "giggle",
            "exit-animation",
            "heal-wrap",
        ],
        ids::PLAYER_APPLY_PHASED_JUMP_INPUT => &[
            "unchanged",
            "phase-boost",
            "phase-opposite",
            "high-clamp",
            "speech",
            "remapped",
        ],
        ids::PLAYER_UPDATE_PHASED_JUMP_MOTION => &[
            "powerup-skip",
            "walking",
            "rest-unarmed",
            "jump-start",
            "phase-release",
            "phase-press",
            "landing-ground",
            "landing-water",
            "landing-void",
            "void-fall",
        ],
        ids::PLAYER_CHOOSE_DIRECTIONAL_MOVEMENT => &[
            "right",
            "fast-sinking",
            "flight",
            "windup-held",
            "stopped",
            "gate-up",
        ],
        ids::SPRITE_CHECKS => &[
            "right",
            "physics-skip",
            "hula-countdown",
            "hula-expire",
            "lean-held",
            "edge-clip",
            "remapped",
        ],
        _ => panic!("outside PlayerB scope"),
    }
}

fn prepare(host: &mut Host) {
    let r = host.revision();
    for number in 0..2 {
        let p = ram::g_player_info(r).at(number);
        let mut c = host.context();
        p.entity_index().write(&mut c, number as i8);
        p.level().write(&mut c, 1);
        p.x().write(&mut c, 512);
        p.y().write(&mut c, 512);
        p.height().write(&mut c, 0);
        p.velocity_x().write(&mut c, 0);
        p.acceleration_x().write(&mut c, 0);
        p.velocity_y().write(&mut c, 0);
        p.acceleration_y().write(&mut c, 0);
        p.velocity_height().write(&mut c, 0);
        p.acceleration_height().write(&mut c, 0);
        p.draw_flags().write(&mut c, 3);
        p.behavior_flags().write(&mut c, 1);
        p.sprite_state().write(&mut c, 0);
        p.animation_group_mask().write(&mut c, 1);
        p.animation_motion_axes().write(&mut c, 0);
        p.frame().write(&mut c, 0);
        p.frame_duration().write(&mut c, 1);
        p.frame_phase().write(&mut c, 0);
        p.width().write(&mut c, 16);
        p.height_pixels().write(&mut c, 16);
        ram::g_player_inventory_open(r).at(number).write(&mut c, 0);
        ram::g_player_lives(r).at(number).write(&mut c, 2);
        ram::g_player_health(r).at(number).write(&mut c, 20);
        ram::g_player_powerups(r).at(number).write(&mut c, -1);
        ram::g_player_elevator_flags(r).at(number).write(&mut c, 0);
        ram::g_player_fall_states(r).at(number).write(&mut c, 0);
        ram::g_player_terrain_sinking(r).at(number).write(&mut c, 0);
        ram::g_player_interaction_choice_active(r)
            .at(number)
            .write(&mut c, 0);
        ram::g_player_control_remap_entities(r)
            .at(number)
            .write(&mut c, 0);
        ram::g_player_control_remap_indices(r)
            .at(number)
            .write(&mut c, 0);
        ram::g_player_processed_input(r).at(number).write(&mut c, 0);
        ram::g_player_choice_action_release_latches(r)
            .at(number)
            .write(&mut c, 0);
        ram::g_player_phased_jump_armed(r)
            .at(number)
            .write(&mut c, 0);
        ram::g_player_phased_jump_phase(r)
            .at(number)
            .write(&mut c, 2);
        ram::g_player_phased_jump_phase_ticks(r)
            .at(number)
            .write(&mut c, 0);
        ram::g_player_phased_jump_speech_used(r)
            .at(number)
            .write(&mut c, 1);
        ram::g_player_jump_counters(r).at(number).write(&mut c, 0);
        ram::g_player_hula_contact_delay(r)
            .at(number)
            .write(&mut c, 0);
        ram::g_player_hula_dance_ticks(r)
            .at(number)
            .write(&mut c, 0);
        ram::g_player_whirlwind_timers(r)
            .at(number)
            .write(&mut c, 0);
        ram::g_player_whirlwind_recovery_timers(r)
            .at(number)
            .write(&mut c, 0);
        ram::g_player_whirlwind_direction_timers(r)
            .at(number)
            .write(&mut c, 0);
        ram::g_player_lemonade_reward_states(r)
            .at(number)
            .write(&mut c, 0);
        ram::g_player_elevator_approach_input_gate(r)
            .at(number)
            .write(&mut c, 0);
        let terrain = if number == 0 {
            ram::g_player1_nearby_terrain(r)
        } else {
            ram::g_player2_nearby_terrain(r)
        }
        .cast::<u8>();
        for cell in 0..25 {
            terrain.offset(cell).write(&mut c, 0x10);
        }
    }
    let mut c = host.context();
    ram::g_player1_terrain_underfoot(r).write(&mut c, 0x10);
    ram::g_player2_terrain_underfoot(r).write(&mut c, 0x10);
    ram::g_elevator_departure_in_progress(r).write(&mut c, 0);
    ram::g_doorway_player_contact_flags(r)
        .at(0)
        .write(&mut c, 0);
    ram::g_second_doorway_player_contact_flags(r).write(&mut c, 0);
    ram::g_random_cache_index(r).write(&mut c, 58);
    ram::g_cached_random_values(r).at(0).write(&mut c, 3);
    ram::g_cached_random_values(r).at(1).write(&mut c, 11);
    ram::g_hula_speech_cooldown(r).write(&mut c, 0);
    let e = ram::g_earthling_banks(r).at(0).at(0);
    e.kind().write(&mut c, 1);
    e.object().write(&mut c, 25);
    e.level().write(&mut c, 1);
    e.state().write(&mut c, 0);
    e.x().write(&mut c, 500);
    e.y().write(&mut c, 512);
    let attacker = ram::g_player_info(r).at(25);
    attacker.entity_index().write(&mut c, 25);
    attacker.x().write(&mut c, 500);
    attacker.y().write(&mut c, 512);
    attacker.height().write(&mut c, 0);
    attacker.level().write(&mut c, 1);
    attacker.sprite_state().write(&mut c, 0);
    attacker.frame_duration().write(&mut c, 0);
    attacker.draw_flags().write(&mut c, 3);
    attacker.behavior_flags().write(&mut c, 1);
}

pub fn configure(host: &mut Host, id: FunctionId, label: &str, number: i32) -> Vec<u32> {
    assert!(labels(id).contains(&label));
    prepare(host);
    let r = host.revision();
    let p = ram::g_player_info(r).at(number);
    let e = ram::g_earthling_banks(r).at(0).at(0);
    let attacker = ram::g_player_info(r).at(25);
    let mut args = vec![p.address(), 0x10];
    let mut c = host.context();
    match id {
        ids::LEMONADE_ABILITY => {
            args = vec![number as u32, e.address()];
            e.kind().write(&mut c, 23);
            e.y().write(&mut c, 500);
            p.sprite_state().write(&mut c, 8);
            match label {
                "burp" => {
                    p.sprite_state().write(&mut c, 118);
                    p.frame_duration().write(&mut c, 0);
                }
                "reward-pending" => {
                    ram::g_player_lemonade_reward_states(r)
                        .at(number)
                        .write(&mut c, 1);
                    ram::g_pcm_effect_cooldown_ticks(r).write(&mut c, 0);
                }
                "earthling-frame-finished" => {
                    e.state().write(&mut c, 1);
                }
                "no-op" => {
                    p.sprite_state().write(&mut c, 0);
                }
                "approach" => {}
                _ => unreachable!(),
            }
        }
        ids::HULA_ABILITY => {
            args = vec![number as u32, e.address()];
            e.kind().write(&mut c, 5);
            attacker.x().write(&mut c, 512);
            match label {
                "level-zero" => p.level().write(&mut c, 0),
                "outside-range" => attacker.x().write(&mut c, 800),
                "delay" => ram::g_player_hula_contact_delay(r)
                    .at(number)
                    .write(&mut c, 2),
                "dance-no-speech" => attacker.x().write(&mut c, 540),
                "near-speech" => {}
                "near-cooldown" => ram::g_hula_speech_cooldown(r).write(&mut c, 10),
                "leave-dance" => {
                    p.sprite_state().write(&mut c, 89);
                    p.animation_group_mask().write(&mut c, 0x2000_0000);
                }
                _ => unreachable!(),
            }
        }
        ids::PLAYER_UPDATE_WHIRLWIND_AND_RECOVERY => {
            p.sprite_state().write(&mut c, 78);
            ram::g_cached_random_values(r).at(0).write(&mut c, 1);
            match label {
                "inactive" => p.sprite_state().write(&mut c, 0),
                "recovery-tick" => ram::g_player_whirlwind_recovery_timers(r)
                    .at(number)
                    .write(&mut c, 17),
                "recovery-free" => ram::g_player_powerups(r).at(number).write(&mut c, 0),
                "recovery-sick" => {}
                "recovery-water" => args[1] = 0x80,
                "recovery-void" => args[1] = 0x90,
                "whirlwind-expire" => {
                    p.sprite_state().write(&mut c, 79);
                    ram::g_player_whirlwind_timers(r)
                        .at(number)
                        .write(&mut c, 1);
                }
                "whirlwind-diagonal" | "whirlwind-countdown" => {
                    p.sprite_state().write(&mut c, 79);
                    ram::g_player_whirlwind_timers(r)
                        .at(number)
                        .write(&mut c, 100);
                    ram::g_cached_random_values(r).at(0).write(&mut c, 3);
                    if label == "whirlwind-countdown" {
                        ram::g_player_whirlwind_direction_timers(r)
                            .at(number)
                            .write(&mut c, 3);
                    }
                }
                _ => unreachable!(),
            }
        }
        ids::PLAYER_YOUCH => {
            args = vec![number as u32, e.address()];
            match label {
                "ordinary" => {}
                "shark-reject" => e.kind().write(&mut c, 14),
                "shark-swim" => {
                    e.kind().write(&mut c, 14);
                    p.sprite_state().write(&mut c, 3);
                }
                "dentist" => e.kind().write(&mut c, 4),
                "dentist-variant" => e.kind().write(&mut c, 21),
                "moving-left" => p.velocity_x().write(&mut c, -120),
                _ => unreachable!(),
            }
        }
        ids::CHICKEN_ABILITY => {
            args = vec![number as u32, e.address()];
            e.kind().write(&mut c, 15);
            match label {
                "near-hit" => {}
                "distant-target" => {
                    e.x().write(&mut c, 700);
                    e.y().write(&mut c, 650);
                    ram::g_cached_random_values(r).at(0).write(&mut c, 99);
                }
                "state-eight" => {
                    e.state().write(&mut c, 8);
                    e.x().write(&mut c, 700);
                }
                _ => unreachable!(),
            }
        }
        ids::AI_BOOGIE_TOUCH => {
            args = vec![number as u32, e.address()];
            e.kind().write(&mut c, 16);
            match label {
                "hit" => {}
                "state-eight" => e.state().write(&mut c, 8),
                "ineligible" => ram::g_player_interaction_choice_active(r)
                    .at(number)
                    .write(&mut c, 1),
                _ => unreachable!(),
            }
        }
        ids::HOTTUB_ABILITY => {
            args = vec![number as u32, e.address()];
            e.kind().write(&mut c, 24);
            e.x().write(&mut c, 560);
            e.y().write(&mut c, 516);
            match label {
                "walk-in-right" => p.sprite_state().write(&mut c, 11),
                "windup" => {
                    p.sprite_state().write(&mut c, 28);
                    p.animation_group_mask().write(&mut c, 0x80);
                    ram::g_player_jump_counters(r).at(number).write(&mut c, 4);
                }
                "takeoff" => {
                    p.sprite_state().write(&mut c, 32);
                    p.animation_group_mask().write(&mut c, 0x100);
                    p.frame_duration().write(&mut c, 0);
                }
                "descend" => {
                    p.sprite_state().write(&mut c, 36);
                    p.animation_group_mask().write(&mut c, 0x200);
                    p.height().write(&mut c, 21);
                }
                "splash" => {
                    p.sprite_state().write(&mut c, 40);
                    p.animation_group_mask().write(&mut c, 0x400);
                    p.frame_duration().write(&mut c, 0);
                }
                "exit-animation" => {
                    p.sprite_state().write(&mut c, 120);
                    p.frame_duration().write(&mut c, 0);
                }
                "exit-input" | "chat" | "giggle" | "heal-wrap" => {
                    p.sprite_state().write(&mut c, 119);
                    p.animation_group_mask().write(&mut c, 0x2000_0000);
                    match label {
                        "exit-input" => ram::g_player_processed_input(r)
                            .at(number)
                            .write(&mut c, if number == 0 { 4 } else { 8 }),
                        "chat" => ram::g_rng_state(r).write(&mut c, 32),
                        "giggle" => ram::g_rng_state(r).write(&mut c, 15),
                        "heal-wrap" => {
                            ram::g_player_processed_input(r)
                                .at(number)
                                .write(&mut c, if number == 0 { 4 } else { 8 });
                            ram::g_player_jump_counters(r).at(number).write(&mut c, 127);
                        }
                        _ => unreachable!(),
                    }
                }
                _ => unreachable!(),
            }
        }
        ids::PLAYER_APPLY_PHASED_JUMP_INPUT => {
            args = vec![p.address(), number as u32, u32::MAX, 0];
            p.velocity_x().write(&mut c, 600);
            p.velocity_y().write(&mut c, -600);
            match label {
                "unchanged" => {}
                "phase-boost" | "phase-opposite" => {
                    args[2] = 1;
                    args[3] = 64;
                    ram::g_player_processed_input(r)
                        .at(number)
                        .write(&mut c, if label == "phase-boost" { 9 } else { 6 });
                }
                "high-clamp" => {
                    args[2] = 1;
                    args[3] = 64;
                    p.height().write(&mut c, 50);
                }
                "speech" => {
                    p.height().write(&mut c, 20);
                    ram::g_player_phased_jump_speech_used(r)
                        .at(number)
                        .write(&mut c, 0);
                }
                "remapped" => {
                    ram::g_player_control_remap_entities(r)
                        .at(number)
                        .write(&mut c, 25);
                    ram::g_player_processed_input(r).at(number).write(&mut c, 8);
                }
                _ => unreachable!(),
            }
        }
        ids::PLAYER_UPDATE_PHASED_JUMP_MOTION => {
            ram::g_player_powerups(r).at(number).write(&mut c, 0);
            match label {
                "powerup-skip" => ram::g_player_powerups(r).at(number).write(&mut c, 5),
                "walking" => {
                    p.sprite_state().write(&mut c, 8);
                    p.animation_group_mask().write(&mut c, 4);
                }
                "rest-unarmed" => {}
                "jump-start" => {
                    ram::g_player_phased_jump_armed(r)
                        .at(number)
                        .write(&mut c, 1);
                    ram::g_player_processed_input(r)
                        .at(number)
                        .write(&mut c, 64);
                }
                "phase-release" | "phase-press" | "landing-ground" | "landing-water"
                | "landing-void" | "void-fall" => {
                    p.sprite_state().write(&mut c, 61);
                    p.animation_group_mask().write(&mut c, 0x8000);
                    p.height().write(&mut c, 2);
                    if label == "phase-release" {
                        ram::g_player_phased_jump_phase(r)
                            .at(number)
                            .write(&mut c, 0);
                        ram::g_player_phased_jump_phase_ticks(r)
                            .at(number)
                            .write(&mut c, 2);
                    } else if label == "phase-press" {
                        ram::g_player_phased_jump_phase(r)
                            .at(number)
                            .write(&mut c, 1);
                        ram::g_player_processed_input(r)
                            .at(number)
                            .write(&mut c, 64);
                    } else if label == "void-fall" {
                        args[1] = 0x90;
                        ram::g_player_jump_counters(r).at(number).write(&mut c, 11);
                    } else {
                        p.velocity_height().write(&mut c, -1);
                        if label == "landing-water" {
                            args[1] = 0x80;
                        } else if label == "landing-void" {
                            args[1] = 0x90;
                        }
                    }
                }
                _ => unreachable!(),
            }
        }
        ids::PLAYER_CHOOSE_DIRECTIONAL_MOVEMENT => {
            args = vec![p.address(), number as u32, 8, 0x10];
            match label {
                "right" => {}
                "fast-sinking" => {
                    args[3] = 0x30;
                    p.height().write(&mut c, -8);
                    ram::g_player_terrain_sinking(r).at(number).write(&mut c, 1);
                }
                "flight" => {
                    p.sprite_state().write(&mut c, 61);
                    p.animation_group_mask().write(&mut c, 0x8000);
                    p.acceleration_x().write(&mut c, 80);
                }
                "windup-held" => {
                    args[2] = 1;
                    p.sprite_state().write(&mut c, 28);
                    p.animation_group_mask().write(&mut c, 0x80);
                }
                "stopped" => {
                    args[2] = 0;
                    p.velocity_x().write(&mut c, 320);
                }
                "gate-up" => {
                    args[2] = 9;
                    ram::g_player_elevator_approach_input_gate(r)
                        .at(number)
                        .write(&mut c, 1);
                }
                _ => unreachable!(),
            }
        }
        ids::SPRITE_CHECKS => {
            args = vec![number as u32, 8];
            match label {
                "right" => {}
                "physics-skip" => p.draw_flags().write(&mut c, 8),
                "hula-countdown" | "hula-expire" => {
                    p.sprite_state().write(&mut c, 89);
                    ram::g_player_hula_dance_ticks(r)
                        .at(number)
                        .write(&mut c, i8::from(label == "hula-countdown"));
                }
                "lean-held" => {
                    args[1] = 1;
                    p.sprite_state().write(&mut c, 16);
                    p.animation_group_mask().write(&mut c, 0x10);
                }
                "edge-clip" => {
                    args[1] = 15;
                    p.x().write(&mut c, 8);
                    p.y().write(&mut c, 8);
                }
                "remapped" => {
                    ram::g_player_control_remap_entities(r)
                        .at(number)
                        .write(&mut c, 25);
                    ram::g_player_processed_input(r).at(number).write(&mut c, 8);
                }
                _ => unreachable!(),
            }
        }
        _ => unreachable!(),
    }
    args
}
