//! Opt-in, labelled live-state TimeAware fixtures; never linked into the game.
use crate::{fixture, prepare_effect_fixture};
use tje_game::{FunctionId, ids};
use tje_host::Host;
use tje_mem::{Ptr, ram};

fn common(host: &mut Host) {
    let r = host.revision();
    prepare_effect_fixture(host);
    let mut c = host.context();
    for index in 0..2 {
        let p = ram::g_player_info(r).at(index);
        p.draw_flags().write(&mut c, 3);
        p.behavior_flags().write(&mut c, 1);
        p.x().write(&mut c, 400 + index as i16 * 128);
        p.y().write(&mut c, 400);
        p.height().write(&mut c, 0);
        p.width().write(&mut c, 32);
        p.height_pixels().write(&mut c, 32);
        p.animation_group_mask().write(&mut c, 1);
        p.velocity_x().write(&mut c, 0);
        p.velocity_y().write(&mut c, 0);
        p.velocity_height().write(&mut c, 0);
        p.next().write(&mut c, Ptr::new(0));
        ram::g_player_health(r).at(index).write(&mut c, 23);
        ram::g_player_rank(r).at(index).write(&mut c, 2);
        ram::g_player_powerups(r).at(index).write(&mut c, -1);
        ram::g_player_fall_states(r).at(index).write(&mut c, 0);
        ram::g_player_interaction_choice_active(r)
            .at(index)
            .write(&mut c, 0);
        ram::g_player_pending_hurt_damage(r)
            .at(index)
            .write(&mut c, 0);
        ram::g_player_promotion_display_entities(r)
            .at(index)
            .write(&mut c, 0);
        ram::g_player_burps_left(r).at(index).write(&mut c, 0);
        ram::g_player_doorway_entities(r).at(index).write(&mut c, 0);
        ram::g_player_boom_box_entities(r)
            .at(index)
            .write(&mut c, 0);
        ram::g_player_rain_cloud_states(r)
            .at(index)
            .write(&mut c, 0);
        ram::g_player_tomato_rain_ticks(r)
            .at(index)
            .write(&mut c, 0);
        ram::g_telephone_states(r)
            .at(index)
            .level()
            .write(&mut c, 0);
        ram::g_delayed_speech_reply_timers(r)
            .at(index)
            .write(&mut c, 0);
    }
    for slot in 0..34 {
        ram::g_dynamic_entity_slot_allocated(r)
            .at(slot)
            .write(&mut c, 0);
    }
    for slot in 0..54 {
        ram::g_viewport_sprite_slot_flags(r)
            .at(slot)
            .write(&mut c, 0);
    }
    for slot in 0..80 {
        ram::g_sprite_tile_group_allocated(r)
            .at(slot)
            .write(&mut c, 0);
    }
    for slot in 0..4 {
        ram::g_speech_bubble_entities(r).at(slot).write(&mut c, 0);
    }
    ram::g_sprite_graphics_upload_pending(r).write(&mut c, 0);
    ram::g_pending_unfall_player(r).write(&mut c, 0);
    ram::g_split_screen_enabled(r).write(&mut c, 0);
    ram::g_player1_map_level(r).write(&mut c, 1);
    ram::g_player2_map_level(r).write(&mut c, 1);
}

pub fn run(host: &mut Host, id: FunctionId) {
    let r = host.revision();
    match id {
        ids::PRESENT_TRY_END_ACTIVE_POWERUP => crate::powerup_end_time_fixtures(host),
        ids::EAT_YUMMY_FOOD | ids::EAT_YUCKY_FOOD | ids::EAT_FUDGE_SUNDAE => {
            for index in 0..2 {
                for health in [-128_i8, 0, 23, 127] {
                    common(host);
                    ram::g_player_health(r)
                        .at(index)
                        .write(&mut host.context(), health);
                    fixture(host, id, &[index as u32]);
                }
            }
        }
        ids::REDUCE_PRESENT_TIMER => {
            for timer in [0_u16, 1, 2, 99, 100, 101, 299, 300, 301, 1000, 65535] {
                common(host);
                for index in 0..2 {
                    ram::g_player_powerups(r)
                        .at(index)
                        .write(&mut host.context(), 0);
                    ram::g_player_powerup_timers(r)
                        .at(index)
                        .write(&mut host.context(), timer);
                }
                fixture(host, id, &[]);
            }
        }
        ids::PERFORM_PROMOTION => {
            for index in 0..2 {
                for promotion in [11_u32, 16, 17] {
                    for existing in [0_i8, 6] {
                        common(host);
                        ram::g_player_promotion_display_entities(r)
                            .at(index)
                            .write(&mut host.context(), existing);
                        fixture(host, id, &[index as u32, promotion, 0x10]);
                    }
                }
            }
        }
        ids::BUMMER_LIFE_PROMO => {
            for index in 0..2 {
                for promotion in [11_i8, 16, 17] {
                    for duration in [0_i8, 1] {
                        common(host);
                        let display = ram::g_player_info(r).at(6);
                        display.sprite_state().write(&mut host.context(), promotion);
                        display
                            .frame_duration()
                            .write(&mut host.context(), duration);
                        display.frame().write(&mut host.context(), 12);
                        ram::g_player_promotion_display_entities(r)
                            .at(index)
                            .write(&mut host.context(), 6);
                        ram::g_random_cache_index(r).write(&mut host.context(), 58);
                        ram::g_cached_random_values(r)
                            .at(0)
                            .write(&mut host.context(), 2);
                        fixture(host, id, &[ram::g_player_info(r).at(index).address(), 0x10]);
                    }
                }
            }
        }
        ids::PRESENT_UPDATE_BURPS => {
            for index in 0..2 {
                for timer in [-128_i8, -1, 0, 1, 50] {
                    common(host);
                    for bank in 0..2 {
                        ram::g_earthling_bank_users(r)
                            .at(bank)
                            .write(&mut host.context(), 0);
                    }
                    ram::g_player_burps_left(r)
                        .at(index)
                        .write(&mut host.context(), 2);
                    ram::g_player_burp_timers(r)
                        .at(index)
                        .write(&mut host.context(), timer);
                    fixture(host, id, &[index as u32]);
                }
            }
        }
        ids::SETUP_SELECTED_PRESENT | ids::OPEN_SELECTED_PRESENT => {
            for index in 0..2 {
                let range = if id == ids::SETUP_SELECTED_PRESENT {
                    0..9
                } else {
                    0..28
                };
                for present in range {
                    common(host);
                    if id == ids::SETUP_SELECTED_PRESENT {
                        fixture(
                            host,
                            id,
                            &[
                                ram::g_player_info(r).at(index).address(),
                                index as u32,
                                present,
                                0x10,
                            ],
                        );
                    } else {
                        fixture(host, id, &[index as u32, present]);
                    }
                }
            }
        }
        ids::PLAYER_COLLECT_NEARBY_ITEMS => {
            for index in 0..2 {
                for kind in [0_i8, 64, 79, 80] {
                    for full in [false, true] {
                        common(host);
                        for slot in 0..16 {
                            ram::g_player_inventory(r)
                                .at(index)
                                .at(slot)
                                .write(&mut host.context(), if full { 27 } else { -1 });
                        }
                        ram::g_player_health(r)
                            .at(index)
                            .write(&mut host.context(), if full { 127 } else { 8 });
                        let item = ram::g_dropped_present_table(r).at(0);
                        item.kind().write(&mut host.context(), kind);
                        item.object().write(&mut host.context(), 6);
                        item.level().write(&mut host.context(), 1);
                        let p = ram::g_player_info(r).at(index);
                        let x = p.x().read(&host.context());
                        item.x().write(&mut host.context(), x);
                        item.y().write(&mut host.context(), 400);
                        fixture(host, id, &[index as u32, item.address(), 1, 1]);
                    }
                }
            }
        }
        ids::PLAYER_CHECK_LEVEL_ITEM_HAZARDS => {
            for index in 0..2 {
                common(host);
                let item = ram::g_dropped_present_table(r).at(0);
                for slot in 0..4 {
                    ram::g_dropped_present_table(r)
                        .at(slot)
                        .kind()
                        .write(&mut host.context(), -1);
                }
                let x = ram::g_player_info(r).at(index).x().read(&host.context());
                item.kind().write(&mut host.context(), 83);
                item.object().write(&mut host.context(), 6);
                item.x().write(&mut host.context(), x);
                item.y().write(&mut host.context(), 400);
                fixture(host, id, &[index as u32, item.address()]);
            }
        }
        ids::PRESENT_UPDATE_TERRAIN_EFFECTS => {
            for terrain in [0x10_u32, 0x20, 0x50, 0x80] {
                common(host);
                ram::g_player_powerups(r)
                    .at(0)
                    .write(&mut host.context(), 6);
                ram::g_player_previous_processed_input(r)
                    .at(0)
                    .write(&mut host.context(), 0);
                ram::g_player_processed_input(r)
                    .at(0)
                    .write(&mut host.context(), 0x40);
                fixture(host, id, &[ram::g_player_info(r).at(0).address(), terrain]);
            }
            common(host);
            let effect = ram::g_player_info(r).at(6);
            effect.x().write(&mut host.context(), 400);
            effect.y().write(&mut host.context(), 400);
            effect.draw_flags().write(&mut host.context(), 3);
            effect.frame_duration().write(&mut host.context(), 0);
            ram::g_player_powerups(r)
                .at(0)
                .write(&mut host.context(), 6);
            ram::g_player_processed_input(r)
                .at(0)
                .write(&mut host.context(), 0);
            ram::g_player_active_terrain_effect_entities(r)
                .at(0)
                .write(&mut host.context(), 6);
            fixture(host, id, &[ram::g_player_info(r).at(0).address(), 0x10]);
        }
        ids::PLAYER_TELEPORT_THROUGH_DOORWAY => {
            for index in 0..2 {
                common(host);
                ram::g_camera1_pending_x(r).write(&mut host.context(), -10000);
                ram::g_camera1_pending_y(r).write(&mut host.context(), -10000);
                fixture(
                    host,
                    id,
                    &[index as u32, ram::g_player_info(r).at(index).address()],
                );
            }
        }
        ids::PRESENT_UPDATE_DOORWAY => {
            for state in [0_i8, 1] {
                common(host);
                let door = ram::g_player_info(r).at(6);
                door.x().write(&mut host.context(), 400);
                door.y().write(&mut host.context(), 400);
                door.draw_flags().write(&mut host.context(), 3);
                door.sprite_state().write(&mut host.context(), state);
                ram::g_player_info(r)
                    .at(0)
                    .y()
                    .write(&mut host.context(), 400);
                ram::g_player_doorway_entities(r)
                    .at(0)
                    .write(&mut host.context(), 6);
                ram::g_doorway_player_contact_flags(r)
                    .at(0)
                    .write(&mut host.context(), 0);
                fixture(host, id, &[0]);
            }
        }
        ids::CHECK_FOR_TELEPHONE => {
            for delay in [-128_i8, 0, 1, 60] {
                common(host);
                let phone = ram::g_telephone_states(r).at(0);
                phone.level().write(&mut host.context(), 1);
                phone.x().write(&mut host.context(), 600);
                phone.y().write(&mut host.context(), 500);
                phone.object().write(&mut host.context(), 6);
                ram::g_telephone_activated(r)
                    .at(0)
                    .write(&mut host.context(), 1);
                ram::g_telephone_rings_left(r)
                    .at(0)
                    .write(&mut host.context(), 2);
                ram::g_telephone_time_to_ring(r)
                    .at(0)
                    .write(&mut host.context(), delay);
                fixture(host, id, &[0]);
            }
        }
        ids::PROCESS_PRESENTS | ids::MAP_UPDATE_ITEMS_AND_PRESENTS => {
            common(host);
            for bank in 0..2 {
                ram::g_item_bank_users(r)
                    .at(bank)
                    .write(&mut host.context(), 0);
            }
            for slot in 0..32 {
                let item = ram::g_dropped_present_table(r).at(slot);
                item.kind().write(&mut host.context(), -1);
                item.object().write(&mut host.context(), 0);
            }
            fixture(host, id, &[]);
            common(host);
            ram::g_player_burps_left(r)
                .at(0)
                .write(&mut host.context(), 1);
            ram::g_player_burp_timers(r)
                .at(0)
                .write(&mut host.context(), -1);
            fixture(host, id, &[]);
        }
        _ => panic!("no TimeAware fixture for {}", id.metadata().name),
    }
}
