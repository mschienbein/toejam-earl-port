use tje_game::ids;
use tje_host::Host;
use tje_mem::ram;

pub(super) fn run(host: &mut Host) {
    let revision = host.revision();
    let earthling = ram::g_earthling_banks(revision).at(0).at(0);
    for number in 0..2 {
        let p = ram::g_player_info(revision).at(number);
        p.entity_index().write(&mut host.context(), number as i8);
        p.x().write(&mut host.context(), 512);
        p.y().write(&mut host.context(), 512);
        p.level().write(&mut host.context(), 1);
        ram::g_player_elevator_flags(revision)
            .at(number)
            .write(&mut host.context(), 0);
        ram::g_player_fall_states(revision)
            .at(number)
            .write(&mut host.context(), 0);
        for powerup in [-1, 0, 1, 2, 3, 4, 5, 7] {
            ram::g_player_powerups(revision)
                .at(number)
                .write(&mut host.context(), powerup);
            for terrain in [0x10, 0x80, 0x90] {
                let cache = if number == 0 {
                    ram::g_player1_nearby_terrain(revision)
                } else {
                    ram::g_player2_nearby_terrain(revision)
                }
                .cast::<u8>();
                for cell in 0..25 {
                    cache.offset(cell).write(&mut host.context(), terrain);
                }
                p.sprite_state().write(&mut host.context(), 8);
                p.animation_group_mask().write(&mut host.context(), 4);
                p.height().write(&mut host.context(), 0);
                super::check(host, ids::GET_ACTIVE_PRESENT_SPRITE, &[p.address()]);
                ram::g_pcm_effect_cooldown_ticks(revision).write(&mut host.context(), 0);
                ram::g_terrain_edge_meep_pending(revision).write(&mut host.context(), 1);
                super::check(
                    host,
                    ids::PLAYER_CHECK_TERRAIN_EDGE_HOP,
                    &[p.address(), 513, 513],
                );
            }
        }
        ram::g_player_powerups(revision)
            .at(number)
            .write(&mut host.context(), -1);
        for (state, mask) in [(28, 0x80), (32, 0x100), (36, 0x200)] {
            p.sprite_state().write(&mut host.context(), state);
            p.animation_group_mask().write(&mut host.context(), mask);
            p.frame_duration().write(&mut host.context(), 0);
            p.height().write(&mut host.context(), 0);
            for count in [0, 4, 9, 30, 127] {
                ram::g_player_jump_counters(revision)
                    .at(number)
                    .write(&mut host.context(), count);
                super::check(
                    host,
                    ids::PLAYER_UPDATE_JUMP_LANDING_ANIMATION,
                    &[p.address(), 0x10],
                );
                super::check(
                    host,
                    ids::PLAYER_UPDATE_JUMP_LANDING_ANIMATION,
                    &[p.address(), 0x80],
                );
                ram::g_player_powerups(revision)
                    .at(number)
                    .write(&mut host.context(), 1);
                super::check(
                    host,
                    ids::PLAYER_UPDATE_SPRING_SHOE_JUMP,
                    &[p.address(), 0x10],
                );
                ram::g_player_powerups(revision)
                    .at(number)
                    .write(&mut host.context(), -1);
            }
        }
        p.sprite_state().write(&mut host.context(), 16);
        p.animation_group_mask().write(&mut host.context(), 0x10);
        for count in [0, 1, 30, 31, 127] {
            for departure in [0, 1] {
                ram::g_elevator_departure_in_progress(revision)
                    .write(&mut host.context(), departure);
                ram::g_player_jump_counters(revision)
                    .at(number)
                    .write(&mut host.context(), count);
                super::check(host, ids::PLAYER_UPDATE_TEETERING, &[p.address()]);
            }
        }
        for flags in [0, 0x10, 0x20, 0x30] {
            p.sprite_state().write(&mut host.context(), 92);
            p.animation_group_mask()
                .write(&mut host.context(), 0x80_0000);
            p.draw_flags().write(&mut host.context(), flags);
            p.velocity_x().write(&mut host.context(), 900);
            p.velocity_y().write(&mut host.context(), 700);
            ram::g_player_processed_input(revision)
                .at(number)
                .write(&mut host.context(), 8);
            ram::g_player_previous_processed_input(revision)
                .at(number)
                .write(&mut host.context(), 0);
            ram::g_player_control_remap_entities(revision)
                .at(number)
                .write(&mut host.context(), 0);
            super::check(
                host,
                ids::PLAYER_UPDATE_ROCKET_SKATES_MOTION,
                &[p.address(), 0x10],
            );
        }
        for occupied in [false, true] {
            for slot in 0..20 {
                ram::g_dynamic_entity_slot_allocated(revision)
                    .at(slot)
                    .write(&mut host.context(), 1);
            }
            ram::g_dynamic_entity_slot_allocated(revision)
                .at(19)
                .write(&mut host.context(), u8::from(occupied) as i8);
            for slot in 0..4 {
                ram::g_shared_projectile_entities(revision)
                    .at(slot)
                    .write(&mut host.context(), 0);
            }
            for input in [1, 2, 4, 8, 5, 10] {
                ram::g_player_processed_input(revision)
                    .at(number)
                    .write(&mut host.context(), input);
                p.sprite_state()
                    .write(&mut host.context(), 80 + (input.trailing_zeros() as i8 & 3));
                for powerup in [3, 4] {
                    super::check(
                        host,
                        ids::PLAYER_LAUNCH_PRESENT_PROJECTILE,
                        &[p.address(), number as u32, powerup],
                    );
                }
            }
            for count in [0, 1, 5, 16] {
                for slot in 0..16 {
                    ram::g_player_inventory(revision).at(number).at(slot).write(
                        &mut host.context(),
                        if slot < count { (slot % 8) as i8 } else { -1 },
                    );
                }
                super::check(host, ids::DO_MOLE_ABILITY, &[p.address(), number as u32]);
            }
        }
        ram::g_player_interaction_choice_active(revision)
            .at(number)
            .write(&mut host.context(), 0);
        ram::g_player_terrain_sinking(revision)
            .at(number)
            .write(&mut host.context(), 0);
        p.sprite_state().write(&mut host.context(), 0);
        p.animation_group_mask().write(&mut host.context(), 1);
        for (kind, state) in [(1, 0), (19, 0), (19, 4)] {
            earthling.kind().write(&mut host.context(), kind);
            earthling.state().write(&mut host.context(), state);
            super::check(
                host,
                ids::PLAYER_SQUISH,
                &[number as u32, earthling.address()],
            );
        }
    }
    for occupied in [0, 1] {
        ram::g_timed_effect_entities(revision)
            .at(0)
            .write(&mut host.context(), occupied);
        ram::g_timed_effect_entities(revision)
            .at(1)
            .write(&mut host.context(), 0);
        super::check(host, ids::ENTITY_SPAWN_TIMED_EFFECT, &[512, 512, 1]);
    }
    let p = ram::g_player_info(revision).at(0);
    let q = ram::g_player_info(revision).at(1);
    p.level().write(&mut host.context(), 1);
    q.level().write(&mut host.context(), 1);
    p.x().write(&mut host.context(), 500);
    q.x().write(&mut host.context(), 520);
    p.y().write(&mut host.context(), 500);
    q.y().write(&mut host.context(), 503);
    p.sprite_state().write(&mut host.context(), 11);
    q.sprite_state().write(&mut host.context(), 10);
    super::check(host, ids::PLAYER_UPDATE_HEALTH_SHARING_CLAP, &[]);
    p.sprite_state().write(&mut host.context(), 91);
    q.sprite_state().write(&mut host.context(), 90);
    p.frame().write(&mut host.context(), 2);
    p.frame_duration().write(&mut host.context(), 1);
    q.frame_duration().write(&mut host.context(), 0);
    for health in [1, 20, 100, 127] {
        ram::g_player_health(revision)
            .at(0)
            .write(&mut host.context(), health);
        ram::g_player_health(revision)
            .at(1)
            .write(&mut host.context(), health);
        super::check(host, ids::PLAYER_UPDATE_HEALTH_SHARING_CLAP, &[]);
    }
}
