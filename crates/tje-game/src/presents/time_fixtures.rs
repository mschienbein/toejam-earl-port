//! Opt-in, labelled live-state TimeAware fixtures; never linked into the game.
use crate::{fixture, prepare_effect_fixture};
use tje_game::{Context, FunctionId, ids};
use tje_host::Host;
use tje_mem::{Ptr, ram};

pub(super) fn labelled(host: &mut Host, id: FunctionId, arguments: &[u32], label: &str) {
    eprintln!("fixture {} {label} {arguments:08x?}", id.metadata().name);
    host.check_fixture(id.metadata().address(host.revision()), arguments)
        .unwrap_or_else(|error| panic!("{} {label} {arguments:08x?}: {error}", id.metadata().name));
}

pub(super) fn common(host: &mut Host) {
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
    crate::replay_fixtures::run(host, id);
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
                        labelled(
                            host,
                            id,
                            &[index as u32, promotion, 0x10],
                            &format!("promotion-{promotion}-existing-{existing}-player-{index}"),
                        );
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
                        labelled(
                            host,
                            id,
                            &[ram::g_player_info(r).at(index).address(), 0x10],
                            &format!(
                                "promotion-{promotion}-duration-{duration}-cache-wrap-player-{index}"
                            ),
                        );
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
                    labelled(
                        host,
                        id,
                        &[index as u32],
                        &format!("burps-timer-{timer}-player-{index}"),
                    );
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
                        labelled(
                            host,
                            id,
                            &[
                                ram::g_player_info(r).at(index).address(),
                                index as u32,
                                present,
                                0x10,
                            ],
                            &format!("setup-present-{present}-player-{index}"),
                        );
                    } else {
                        labelled(
                            host,
                            id,
                            &[index as u32, present],
                            &format!("open-present-{present}-player-{index}"),
                        );
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
                labelled(
                    host,
                    id,
                    &[ram::g_player_info(r).at(0).address(), terrain],
                    &format!("fresh-action-terrain-{terrain:02x}"),
                );
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
            labelled(
                host,
                id,
                &[ram::g_player_info(r).at(0).address(), 0x10],
                "grown-effect-owner-nearby",
            );
        }
        ids::PLAYER_TELEPORT_THROUGH_DOORWAY => {
            for index in 0..2 {
                common(host);
                ram::g_camera1_pending_x(r).write(&mut host.context(), -10000);
                ram::g_camera1_pending_y(r).write(&mut host.context(), -10000);
                labelled(
                    host,
                    id,
                    &[index as u32, ram::g_player_info(r).at(index).address()],
                    &format!("forced-redraw-player-{index}"),
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
                labelled(
                    host,
                    id,
                    &[0],
                    &format!("doorway-state-{state}-player-centered"),
                );
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
                labelled(host, id, &[0], &format!("ring-delay-{delay}"));
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
            labelled(host, id, &[], "all-effects-inactive");
            common(host);
            ram::g_player_burps_left(r)
                .at(0)
                .write(&mut host.context(), 1);
            ram::g_player_burp_timers(r)
                .at(0)
                .write(&mut host.context(), -1);
            labelled(host, id, &[], "burp-start-chain");
        }
        _ => panic!("no TimeAware fixture for {}", id.metadata().name),
    }
}

/// Strict return-state fixtures, not timing approval. The settled game supplies
/// the stack/devices; only labelled RAM inputs, entry GPRs and CCR are seeded.
/// IRQs are masked so this isolates caller/producer CPU outputs from the pending
/// main/ISR scheduling issue. The root remains TimeAware and unregistered.
pub fn expiry_registers(host: &mut Host, include_word_indexes: bool) -> usize {
    let cases = [
        ("airborne-no-flight", 1, 1, 8, 0x10, 0),
        ("airborne-falling", 1, 0x20, 8, 0x10, 0),
        ("ground-falling", 0, 0x20, 8, 0x10, 0),
        ("skates-airborne-no-flight", 1, 1, 8, 0x10, 5),
        ("skates-falling", 0, 0x20, 8, 0x10, 5),
        ("icarus-fall-north", 1, 0x8000, 60, 0x10, 0),
        ("icarus-fall-east", 1, 0x8000, 61, 0x10, 0),
        ("icarus-fall-south", 1, 0x8000, 62, 0x10, 0),
        ("icarus-fall-west", 1, 0x8000, 63, 0x10, 0),
        ("rocket-fall", 1, 0x0400_0000, 84, 0x10, 4),
        ("stationary-actual-4631-shape", 0, 0x2000_0000, 0, 0x10, 7),
        ("stationary-nonzero-state", 0, 0x2000_0000, 8, 0x10, 7),
        ("no-refresh-predicate", 0, 0, 8, 0x10, 0),
        ("inner-tube", 0, 0x0004_0000, 58, 0x10, 2),
        ("state-77-splash", 0, 0, 77, 0x10, 2),
        ("skates-water", 0, 0x0080_0000, 4, 0x80, 5),
        ("icarus-water", 0, 0x8000, 61, 0x80, 0),
        ("refresh-state-zero", 0, 1, 0, 0x10, 0),
        ("refresh-state-four-water", 0, 1, 4, 0x80, 0),
        ("refresh-state-76", 0, 1, 76, 0x10, 2),
        ("refresh-state-three", 0, 1, 3, 0x10, 0),
        ("refresh-state-seven", 0, 1, 7, 0x10, 0),
        ("refresh-social", 0, 0x0040_0000, 8, 0x10, 0),
        ("refresh-skates-dry", 0, 0x0080_0000, 8, 0x10, 5),
        ("refresh-end-high-bit", 0, 0x0800_0000, 8, 0x10, 0),
        ("refresh-end-middle-bit", 0, 0x0020_0000, 8, 0x10, 0),
        ("refresh-end-low-bit", 0, 0x4000, 8, 0x10, 0),
        ("refresh-icarus-dry", 0, 0x8000, 8, 0x10, 0),
        ("negative-height", -1, 1, 0, 0x10, 0),
    ];
    let mut checked = 0;
    for (label, height, groups, state, terrain, powerup) in cases {
        for index in 0_i16..2 {
            for seed in [0_u32, 0xa5a5_ffff, 0xdead_0001] {
                for ccr in 0_u16..32 {
                    common(host);
                    expiry_register_fixture(
                        host, label, index, height, groups, state, terrain, powerup, seed, ccr,
                    );
                    checked += 1;
                }
            }
        }
    }
    if !include_word_indexes {
        return checked;
    }
    // ASL.W #7/ADDA.W aliases the object while the scalar powerup lookup remains
    // long-indexed. Early falling avoids manufacturing any high-index HUD data.
    for index in [i16::MIN, -512, -1, 2, 255, 256, 511, 512, 1024, i16::MAX] {
        for seed in [0_u32, 0xa5a5_ffff, 0xdead_0001] {
            for ccr in 0_u16..32 {
                common(host);
                expiry_register_fixture(
                    host,
                    "word-index-early-falling",
                    index,
                    0,
                    0x20,
                    8,
                    0x10,
                    0,
                    seed,
                    ccr,
                );
                checked += 1;
            }
        }
    }
    checked
}

#[expect(
    clippy::too_many_arguments,
    reason = "the labelled source-state fixture tuple"
)]
fn expiry_register_fixture(
    host: &mut Host,
    label: &str,
    index: i16,
    height: i16,
    groups: u32,
    state: i8,
    terrain: u8,
    powerup: i8,
    seed: u32,
    ccr: u16,
) {
    let revision = host.revision();
    let player = ram::g_player_info(revision).at_word_index(index);
    let mut context = host.context();
    player.height().write(&mut context, height);
    player.sprite_state().write(&mut context, state);
    player.animation_group_mask().write(&mut context, groups);
    player.entity_index().write(&mut context, index as i8);
    ram::g_player1_terrain_underfoot(revision).write(&mut context, terrain);
    ram::g_player2_terrain_underfoot(revision).write(&mut context, terrain);
    ram::g_player_powerups(revision)
        .at(i32::from(index))
        .write(&mut context, powerup);
    ram::g_pcm_effect_cooldown_ticks(revision).write(&mut context, 0);
    ram::g_psg_effect_status(revision).write(&mut context, 0);
    for register in 0..8 {
        context.set_data_register(
            register,
            seed.rotate_left(register as u32 * 4) ^ (register as u32 * 0x1111_1111),
        );
    }
    for register in 0..7 {
        context.set_address_register(register, 0xff9000 + register as u32 * 0x100);
    }
    let status = (context.status_register() & !0x1f) | 0x0700 | ccr;
    context.set_status_register(status);
    drop(context);
    host.check_fixture(
        ids::PRESENT_TRY_END_ACTIVE_POWERUP
            .metadata()
            .address(revision),
        &[i32::from(index) as u32],
    )
    .unwrap_or_else(|error| panic!("{label} index={index} seed={seed:08x} CCR={ccr:02x}: {error}"));
}
