use tje_game::ids;
use tje_host::Host;
use tje_mem::{Ptr, ram};

pub(super) fn run(host: &mut Host) {
    let revision = host.revision();
    for number in 0..2 {
        let p = ram::g_player_info(revision).at(number);
        p.entity_index().write(&mut host.context(), number as i8);
        p.level().write(&mut host.context(), 1);
        p.x().write(&mut host.context(), 512);
        p.y().write(&mut host.context(), 512);
        p.height().write(&mut host.context(), 0);
        p.draw_flags().write(&mut host.context(), 1);
        p.behavior_flags().write(&mut host.context(), 1);
        ram::g_player_inventory_open(revision)
            .at(number)
            .write(&mut host.context(), 0);
        ram::g_player_lives(revision)
            .at(number)
            .write(&mut host.context(), 2);
        ram::g_player_elevator_flags(revision)
            .at(number)
            .write(&mut host.context(), 0);
        ram::g_player_fall_states(revision)
            .at(number)
            .write(&mut host.context(), 0);
        let terrain = if number == 0 {
            ram::g_player1_nearby_terrain(revision)
        } else {
            ram::g_player2_nearby_terrain(revision)
        }
        .cast::<u8>();
        for cell in 0..25 {
            terrain.offset(cell).write(&mut host.context(), 0x10);
        }
        p.sprite_state().write(&mut host.context(), 8);
        p.animation_group_mask().write(&mut host.context(), 4);
        p.animation_motion_axes().write(&mut host.context(), 0);
        let frame = p.current_frame().read(&host.context()).address();
        super::check(
            host,
            if number == 0 {
                ids::PLAYER1_PROCESS_TERRAIN_MOTION
            } else {
                ids::PLAYER2_PROCESS_TERRAIN_MOTION
            },
            &[frame],
        );
        for (state, mask) in [(48, 0x1000), (40, 0x400), (64, 0x10000), (3, 1)] {
            p.sprite_state().write(&mut host.context(), state);
            p.animation_group_mask().write(&mut host.context(), mask);
            p.frame_duration().write(&mut host.context(), 0);
            super::check(
                host,
                ids::PLAYER_UPDATE_PRESENT_COMPLETION_AND_TERRAIN_POSE,
                &[p.address(), 0x10],
            );
        }
        for (state, mask, frame) in [(80, 0x100000, 1), (84, 0x200000, 2), (8, 4, 0)] {
            p.sprite_state().write(&mut host.context(), state);
            p.animation_group_mask().write(&mut host.context(), mask);
            p.frame().write(&mut host.context(), frame);
            p.frame_duration().write(&mut host.context(), 0);
            ram::g_player_powerups(revision)
                .at(number)
                .write(&mut host.context(), 3);
            ram::g_player_processed_input(revision)
                .at(number)
                .write(&mut host.context(), 0x48);
            for slot in 0..4 {
                ram::g_shared_projectile_entities(revision)
                    .at(slot)
                    .write(&mut host.context(), 0);
            }
            super::check(
                host,
                ids::PLAYER_UPDATE_PRESENT_SHOOTING,
                &[p.address(), 0x10],
            );
        }
        ram::g_player_powerups(revision)
            .at(number)
            .write(&mut host.context(), -1);
        ram::g_player_wake_effect_entity_handles(revision)
            .at(number)
            .write(&mut host.context(), 0);
        for state in [0, 8, 116, 117] {
            p.sprite_state().write(&mut host.context(), state);
            for delay in i8::MIN..=i8::MAX {
                ram::g_player_book_sleep_delay(revision)
                    .at(number)
                    .write(&mut host.context(), delay);
                ram::g_player_idle_ticks(revision)
                    .at(number)
                    .write(&mut host.context(), 300);
                ram::g_player_interaction_choice_active(revision)
                    .at(number)
                    .write(&mut host.context(), 1);
                super::check(host, ids::PROCESS_SLEEP, &[p.address()]);
            }
        }
        p.sprite_state().write(&mut host.context(), 0);
        p.animation_group_mask().write(&mut host.context(), 1);
        ram::g_player_interaction_choice_active(revision)
            .at(number)
            .write(&mut host.context(), 0);
        let e = ram::g_earthling_banks(revision).at(0).at(0);
        e.kind().write(&mut host.context(), 8);
        e.state().write(&mut host.context(), 4);
        e.object().write(&mut host.context(), 25);
        ram::g_player_info(revision)
            .at(25)
            .draw_flags()
            .write(&mut host.context(), 1);
        super::check(host, ids::WHIRL_WIND_ABILITY, &[number as u32, e.address()]);
    }
    let effect = ram::g_player_info(revision).at(25);
    effect.draw_flags().write(&mut host.context(), 1);
    effect.frame_duration().write(&mut host.context(), 0);
    for slot in 0..2 {
        ram::g_timed_effect_entities(revision)
            .at(slot)
            .write(&mut host.context(), 25);
        super::check(
            host,
            ids::ENTITY_RELEASE_FINISHED_TIMED_EFFECT,
            &[slot as u32],
        );
    }
    let p = ram::g_player_info(revision).at(0);
    ram::g_entity_draw_list_head(revision).write(&mut host.context(), Ptr::new(p.address()));
    p.next().write(&mut host.context(), Ptr::new(0));
    p.earthling().write(&mut host.context(), Ptr::new(0));
    p.draw_flags().write(&mut host.context(), 1);
    p.sprite_state().write(&mut host.context(), 8);
    p.animation_group_mask().write(&mut host.context(), 4);
    p.animation_motion_axes().write(&mut host.context(), 0);
    p.height().write(&mut host.context(), 10);
    for (x, y) in [(512, 512), (8, 8), (2650, 1735), (-100, -100)] {
        p.x().write(&mut host.context(), x);
        p.y().write(&mut host.context(), y);
        p.velocity_x().write(&mut host.context(), 320);
        p.velocity_y().write(&mut host.context(), 240);
        p.velocity_height().write(&mut host.context(), -300);
        p.acceleration_x().write(&mut host.context(), -128);
        p.acceleration_y().write(&mut host.context(), 128);
        p.acceleration_height().write(&mut host.context(), -32);
        p.fraction_x().write(&mut host.context(), 250);
        p.fraction_y().write(&mut host.context(), 250);
        p.fraction_height().write(&mut host.context(), 250);
        super::check(host, ids::PROCESS_MOVEMENT, &[]);
    }
}
