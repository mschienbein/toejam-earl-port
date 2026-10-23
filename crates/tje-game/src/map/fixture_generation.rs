use crate::{Result, check, fill_map};
use tje_game::{Context, ids};
use tje_host::Host;
use tje_mem::{ram, rom};

fn seed(host: &mut Host) {
    let r = host.revision();
    ram::g_requested_map_level(r).write(&mut host.context(), 2);
    ram::g_rng_state(r).write(&mut host.context(), 12345);
}
fn masks(host: &mut Host, value: u8) {
    let r = host.revision();
    let map = ram::g_map_path_direction_banks(r);
    for row in 0..26 {
        for column in 0..19 {
            map.at(row).at(column).write(&mut host.context(), value);
        }
    }
}

pub fn run(host: &mut Host) -> Result {
    let r = host.revision();
    let map = ram::g_player1_level_map(r);
    let anchors = ram::g_level_anchor_positions(r);
    for world in [0, 1] {
        seed(host);
        ram::g_world_type(r).write(&mut host.context(), world);
        check(
            host,
            ids::GENERATE_LEVEL_SEEDS,
            &[],
            "random/fixed world seed generation and cache identities",
        )?;
    }
    seed(host);
    for index in 0..8 {
        check(
            host,
            ids::MAP_GET_RANDOM_GENERATION_PARAMETER,
            &[index],
            "each observed inclusive range pair",
        )?;
    }
    for state in [1, 12345, 65535] {
        ram::g_rng_state(r).write(&mut host.context(), state);
        check(
            host,
            ids::MAP_CHOOSE_BACKGROUND_TILE_BUCKET,
            &[],
            "ten-bit RNG buckets",
        )?;
    }
    for state in [1, 12345, 65535] {
        for turn in [0_i32, 60, 100, -1, i32::from(i16::MIN), i32::from(i16::MAX)] {
            seed(host);
            ram::g_rng_state(r).write(&mut host.context(), state);
            fill_map(host, 0);
            masks(host, 0xa0);
            check(
                host,
                ids::MAP_CARVE_DIRECTIONAL_PATH,
                &[5, 5, 10, 10, 20, turn as u32],
                "seeded reciprocal path, mutable turn slot, signed remainder branches",
            )?;
        }
    }
    let rp = ram::g_saved_level_map(r).cast::<i16>();
    let cp = rp.offset(1);
    for direction in [1, 2, 4, 8] {
        for (contents, join) in [
            (0, 0),
            (0x1100, 0),
            (0x1200, 0),
            (0x1200, 100),
            (0x1200, u32::MAX),
            (0x1200, 0xffff8000),
        ] {
            seed(host);
            fill_map(host, contents);
            rp.write(&mut host.context(), 5);
            cp.write(&mut host.context(), 5);
            check(
                host,
                ids::MAP_FIND_REGION_GROWTH_ORIGIN,
                &[rp.address(), cp.address(), direction, 0x1100, join],
                "growth origin: empty, matching, overlap rejected/joined",
            )?;
        }
    }
    seed(host);
    fill_map(host, 0);
    map.at(5).at(5).write(&mut host.context(), 0x2001);
    map.at(4).at(5).write(&mut host.context(), 0x1100);
    check(
        host,
        ids::MAP_STABILIZE_REGION_CONNECTIONS,
        &[],
        "widening plus expansion to fixed point",
    )?;
    fill_map(host, 34);
    check(
        host,
        ids::MAP_ASSIGN_TRANSITION_CHAIN,
        &[],
        "clear transition bits and exhaust both search passes",
    )?;
    for bank in [0, 1] {
        for shape in [1, 2, 4, 8, 15] {
            seed(host);
            fill_map(host, 0);
            masks(host, 0);
            let base = rom::map_generation_lookup_tables(r)
                .tiles()
                .at(shape)
                .read(&host.context());
            let alternatives = rom::tile_alternative_pointers(r)
                .at(i32::from(base))
                .read(&host.context())
                .target();
            let tile = alternatives.offset(1).read(&host.context());
            let active = if bank == 0 {
                ram::g_player1_level_map(r)
            } else {
                ram::g_player2_level_map(r)
            };
            active
                .at(5)
                .at(5)
                .write(&mut host.context(), 0x4000 | u16::from(tile));
            ram::g_map_path_direction_banks(r).at(5).at(5).write(
                &mut host.context(),
                if bank == 0 {
                    shape as u8
                } else {
                    (shape as u8) << 4
                },
            );
            ram::g_land_path_traversal_direction(r).write(&mut host.context(), -1);
            check(
                host,
                ids::MAP_CONSUME_PATH_CELL_DIRECTIONS,
                &[bank, 5, 5],
                "pending path directions and real counted alternatives",
            )?;
        }
        for residual in [0, 16, 19] {
            seed(host);
            fill_map(host, 0);
            masks(host, 0);
            let active = if bank == 0 {
                ram::g_player1_level_map(r)
            } else {
                ram::g_player2_level_map(r)
            };
            active.at(5).at(5).write(&mut host.context(), 0x4000);
            ram::g_land_path_traversal_direction(r).write(&mut host.context(), -1);
            host.context().set_data_register(6, residual);
            check(
                host,
                ids::MAP_CONSUME_PATH_CELL_DIRECTIONS,
                &[bank, 5, 5],
                "original no-neighbor bug consumes residual D6.B",
            )?;
        }
    }
    for (tile, width, height, flags) in [
        (30, 16, 8, 1),
        (75, 16, 8, 8),
        (53, 16, 8, 2),
        (34, 4, 2, 3),
        (30, 4, 2, 3),
        (0x11e, 4, 2, 1),
        (0x11e, 4, 2, 5),
        (0x9e, 4, 2, 3),
    ] {
        seed(host);
        check(
            host,
            ids::TERRAIN_VALIDATION,
            &[tile, width, height, rp.address(), cp.address(), flags],
            "full/subrect terrain, mirroring, overlay gate, output slots",
        )?;
    }
    seed(host);
    check(
        host,
        ids::MAP_GENERATE_SEPARATED_ANCHORS,
        &[],
        "seeded level-two separated anchor generation",
    )?;
    seed(host);
    let mut c = host.context();
    for (i, (x, y)) in [(5, 8), (13, 17), (10, 10)].into_iter().enumerate() {
        anchors.at(i as i32).x().write(&mut c, x);
        anchors.at(i as i32).y().write(&mut c, y);
    }
    fill_map(host, 0);
    masks(host, 0);
    check(
        host,
        ids::MAP_CONNECT_ANCHOR_POSITIONS,
        &[3],
        "three authored anchors; ordered RNG path extension",
    )?;
    seed(host);
    fill_map(host, 0x1000);
    check(
        host,
        ids::MAP_BUILD_REGION_EDGE_SHAPES,
        &[1],
        "one boundary label, complete-neighborhood edge masks",
    )?;
    seed(host);
    fill_map(host, 0x1100);
    for i in 0..3 {
        anchors.at(i).x().write(&mut host.context(), -1);
        anchors.at(i).y().write(&mut host.context(), -1);
    }
    check(
        host,
        ids::MAP_SEED_FILLED_TERRAIN_REGIONS,
        &[],
        "eligible uniform regions and original stack-backed growth callee",
    )?;
    seed(host);
    masks(host, 0);
    for row in 0..26 {
        for column in 0..19 {
            let choices = [0, 0x1000, 0x9000, 27, 45, 46, 75, 76, 80];
            map.at(row).at(column).write(
                &mut host.context(),
                choices[((row * 19 + column) % 9) as usize],
            );
        }
    }
    check(
        host,
        ids::MAP_SCATTER_TERRAIN_FEATURES,
        &[],
        "neighbor classes/features; parameter-before-RNG comparison order",
    )?;
    for state in [1, 12345, 65535] {
        for (kind, background, minimum) in [(0x1000, 0, 0), (0x1000, 0, 12), (0x3000, 0x1100, 12)] {
            seed(host);
            ram::g_rng_state(r).write(&mut host.context(), state);
            fill_map(host, background);
            for index in 0..3 {
                anchors.at(index).x().write(&mut host.context(), -1);
                anchors.at(index).y().write(&mut host.context(), -1);
            }
            if kind == 0x1000 {
                map.at(5).at(6).write(&mut host.context(), 0x1200);
                map.at(1).at(1).write(&mut host.context(), 0x1200);
            }
            check(
                host,
                ids::MAP_GROW_TERRAIN_REGION,
                &[kind, 1, minimum, 5, 5],
                "real writable origin slots; filled/boundary growth, merging and minimum count",
            )?;
        }
        seed(host);
        ram::g_rng_state(r).write(&mut host.context(), state);
        fill_map(host, 30);
        let mut cursor = 1;
        for index in 0..16 {
            let descriptor = rom::map_transition_descriptors(r).at(index);
            let shapes = descriptor.shapes().read(&host.context()).target();
            let count = i32::from(shapes.read(&host.context()));
            if count != 0 {
                for choice in [0, count / 2, count - 1] {
                    let shape = shapes.offset(choice + 1).read(&host.context());
                    let flags = if cursor & 1 == 0 { 0x4000 } else { 0x8000 };
                    map.at(cursor / 19).at(cursor % 19).write(
                        &mut host.context(),
                        ((index as u16 + 1) << 8) | u16::from(shape) | flags,
                    );
                    cursor += 1;
                }
            }
        }
        check(
            host,
            ids::MAP_CHOOSE_METATILE_ALTERNATIVES,
            &[],
            "ROM-driven first/middle/last transition shapes, bank changes, ordinary alternatives and flags",
        )?;
    }
    for source in 0..4 {
        let mut c = host.context();
        for i in 0..6 {
            ram::g_mole_effect_entities(r).at(i).write(&mut c, 0);
        }
        for i in 0..4 {
            ram::g_speech_bubble_entities(r).at(i).write(&mut c, 0);
        }
        for i in 4..12 {
            ram::g_shared_projectile_entities(r).at(i).write(&mut c, 0);
        }
        for bank in 0..2 {
            ram::g_earthling_bank_users(r).at(bank).write(&mut c, 0);
            for entry in 0..32 {
                ram::g_object_table(r)
                    .at(bank)
                    .at(entry)
                    .object()
                    .write(&mut c, 0);
            }
        }
        ram::g_dynamic_entity_slot_allocated(r)
            .at(0)
            .write(&mut c, 1);
        ram::g_player_info(r).at(6).draw_flags().write(&mut c, 1);
        match source {
            0 => ram::g_mole_effect_entities(r).at(0).write(&mut c, 6),
            1 => ram::g_speech_bubble_entities(r).at(0).write(&mut c, 6),
            2 => ram::g_shared_projectile_entities(r).at(4).write(&mut c, 6),
            _ => ram::g_object_table(r).at(0).at(0).object().write(&mut c, 6),
        }
        check(
            host,
            ids::ALLOCATE_ENTITY_WITH_EVICTION,
            &[1],
            "each early eviction priority uses real allocator/free callees",
        )?;
        check(
            host,
            ids::ENTITY_RELEASE_ONE_BY_PRIORITY,
            &[],
            "release through identical eviction and allocation priority",
        )?;
    }
    check(
        host,
        ids::ALLOCATE_ENTITY_WITH_EVICTION,
        &[0],
        "initial free-slot attempt",
    )?;
    Ok(())
}
