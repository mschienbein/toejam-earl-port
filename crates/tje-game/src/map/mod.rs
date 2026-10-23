// Original execution classes include untranslated routines; see contracts.rs.
mod contracts;
pub use contracts::EXECUTION_CONTRACTS;
mod generation;
mod paths;
mod regions;
#[cfg(test)]
#[path = "tests.rs"]
mod regression_tests;
mod screen;

pub use generation::*;
pub use paths::*;
pub use regions::*;
pub use screen::*;

use crate::{Context, Result, call, ids};
use tje_mem::ram;

/// G/src/map/random_regions_and_cleanup.c:11, the Schrage-split Park-Miller step.
/// Noncanonical seeds retain the original wrapping 32-bit intermediate arithmetic.
pub fn park_miller(state: i32) -> i32 {
    let high = state / 127_773;
    let low = state % 127_773;
    let next = 16_807_i32
        .wrapping_mul(low)
        .wrapping_sub(2_836_i32.wrapping_mul(high));
    if next > 0 {
        next
    } else {
        next.wrapping_add(i32::MAX)
    }
}
/// G/src/map/random_regions_and_cleanup.c:28, including minimum-signed negation.
pub fn normalize_seed(mut seed: i32) -> i32 {
    if seed < 0 {
        seed = seed.wrapping_neg();
    }
    if seed == i32::MAX {
        seed = seed.wrapping_sub(i32::MAX);
    }
    if seed == 0 {
        seed = 1;
    }
    seed
}
/// G/src/map/random_regions_and_cleanup.c:11 (`GetRandomNumber`).
pub fn get_random_number(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let state = ram::g_rng_state(context.revision());
    let next = park_miller(state.read(context));
    state.write(context, next);
    Ok(Some(u32::from(next as u16)))
}
/// G/src/map/random_regions_and_cleanup.c:28 (`StackBasedRandom`).
pub fn stack_based_random(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let state = ram::g_rng_state(context.revision());
    let previous = state.read(context);
    state.write(context, normalize_seed(arguments[0] as i32));
    Ok(Some(previous as u32))
}
/// G/src/map/random_regions_and_cleanup.c:45 (`PreGenerateRandoms`). Rust callees
/// are dispatched by identity; a hybrid-only missing callee enters the real ROM.
pub fn pre_generate_randoms(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let saved = ram::g_rng_state(revision).read(context);
    for index in 0..59 {
        let value = call(context, ids::GET_RANDOM_NUMBER, &[])?;
        ram::g_cached_random_values(revision)
            .at(index)
            .write(context, value as u16);
    }
    ram::g_random_cache_index(revision).write(context, 0);
    ram::g_rng_state(revision).write(context, saved);
    Ok(None)
}

/// Available Rust entries owned by this subsystem.
pub static TRANSLATIONS: &[crate::Translation] = &[
    crate::Translation {
        id: crate::ids::GET_RANDOM_NUMBER,
        function: get_random_number,
    },
    crate::Translation {
        id: crate::ids::STACK_BASED_RANDOM,
        function: stack_based_random,
    },
    crate::Translation {
        id: crate::ids::PRE_GENERATE_RANDOMS,
        function: pre_generate_randoms,
    },
    crate::Translation {
        id: ids::MAP_REFINE_FILLED_SHAPE_MASKS,
        function: map_refine_filled_shape_masks,
    },
    crate::Translation {
        id: ids::MAP_BUILD_DIRECTIONAL_PATH_SHAPES,
        function: map_build_directional_path_shapes,
    },
    crate::Translation {
        id: ids::MAP_SWAP_PLAYER_BANKS,
        function: map_swap_player_banks,
    },
    crate::Translation {
        id: ids::COPY_LEVEL_BETWEEN_PLAYERS,
        function: copy_level_between_players,
    },
    crate::Translation {
        id: ids::MAP_SWAP_PLAYER_WITH_SAVED_BANK,
        function: map_swap_player_with_saved_bank,
    },
    crate::Translation {
        id: ids::LOAD_FINAL_LEVEL,
        function: load_final_level,
    },
    crate::Translation {
        id: ids::MAP_FIND_INTERIOR_GROWTH_ORIGIN,
        function: map_find_interior_growth_origin,
    },
    crate::Translation {
        id: ids::MAP_FILL_RECTANGLE_AND_MERGE_REGIONS,
        function: map_fill_rectangle_and_merge_regions,
    },
    crate::Translation {
        id: ids::MAP_WIDEN_HORIZONTAL_REGION_EDGES,
        function: map_widen_horizontal_region_edges,
    },
    crate::Translation {
        id: ids::MAP_WIDEN_VERTICAL_REGION_EDGES,
        function: map_widen_vertical_region_edges,
    },
    crate::Translation {
        id: ids::MAP_TRANSITION_CONTAINS_METATILE,
        function: map_transition_contains_metatile,
    },
    crate::Translation {
        id: ids::MAP_FIND_ROUTE_TO_SECOND_ANCHOR,
        function: map_find_route_to_second_anchor,
    },
    crate::Translation {
        id: ids::MAP_COVER_BLOCK_CONTAINS_TERRAIN,
        function: map_cover_block_contains_terrain,
    },
    crate::Translation {
        id: ids::LOAD_LEVEL1_TILES,
        function: load_level1_tiles,
    },
    crate::Translation {
        id: ids::MAP_APPEND_COVER_BLOCK_SPRITES,
        function: map_append_cover_block_sprites,
    },
    crate::Translation {
        id: ids::PLAYER_HAS_ODD_ELEVATOR_STATE,
        function: player_has_odd_elevator_state,
    },
    crate::Translation {
        id: ids::MAP_REVERSE_PATH_DIRECTION,
        function: map_reverse_path_direction,
    },
    crate::Translation {
        id: ids::MAP_FIND_PATH_SHAPE_FOR_METATILE,
        function: map_find_path_shape_for_metatile,
    },
    crate::Translation {
        id: ids::MAP_GET_BANK_PATH_DIRECTIONS,
        function: map_get_bank_path_directions,
    },
    crate::Translation {
        id: ids::PLAYER_IS_NEAR_TERRAIN_EDGE,
        function: player_is_near_terrain_edge,
    },
];

/// Reviewed original entries that return without clock/interrupt progress.
pub static ATOMIC_CAPABLE: &[crate::FunctionId] = &[
    crate::ids::GET_RANDOM_NUMBER,
    crate::ids::STACK_BASED_RANDOM,
    crate::ids::PRE_GENERATE_RANDOMS,
    // Source-reviewed RAM/ROM-only leaves; the route search recurses only into itself.
    ids::MAP_REFINE_FILLED_SHAPE_MASKS,
    ids::MAP_BUILD_DIRECTIONAL_PATH_SHAPES,
    ids::MAP_SWAP_PLAYER_BANKS,
    ids::COPY_LEVEL_BETWEEN_PLAYERS,
    ids::MAP_SWAP_PLAYER_WITH_SAVED_BANK,
    ids::LOAD_FINAL_LEVEL,
    ids::MAP_FIND_INTERIOR_GROWTH_ORIGIN,
    ids::MAP_FILL_RECTANGLE_AND_MERGE_REGIONS,
    ids::MAP_WIDEN_HORIZONTAL_REGION_EDGES,
    ids::MAP_WIDEN_VERTICAL_REGION_EDGES,
    ids::MAP_TRANSITION_CONTAINS_METATILE,
    ids::MAP_FIND_ROUTE_TO_SECOND_ANCHOR,
    ids::MAP_COVER_BLOCK_CONTAINS_TERRAIN,
    ids::LOAD_LEVEL1_TILES,
    ids::MAP_APPEND_COVER_BLOCK_SPRITES,
    ids::PLAYER_HAS_ODD_ELEVATOR_STATE,
    ids::MAP_REVERSE_PATH_DIRECTION,
    ids::MAP_FIND_PATH_SHAPE_FOR_METATILE,
    ids::MAP_GET_BANK_PATH_DIRECTIONS,
    ids::PLAYER_IS_NEAR_TERRAIN_EDGE,
    // G/src/map/level_generation_and_changes.c:86,208,292,418,435,482:
    // RAM/ROM generation and RNG only; no device/frame observations or waits.
    ids::MAP_BUILD_FILLED_TERRAIN_SHAPES,
    ids::MAP_SCATTER_TERRAIN_FEATURES,
    ids::MAP_CHOOSE_METATILE_ALTERNATIVES,
    ids::GENERATE_LEVEL_SEEDS,
    ids::MAP_GENERATE_SEPARATED_ANCHORS,
    ids::MAP_CONNECT_ANCHOR_POSITIONS,
    // G/src/map/random_regions_and_cleanup.c:60,89,182,293,433,609,630,674:
    // RAM scans, RNG, and RAM-only original callees. Original stack references in
    // growth are a translation/frame concern, not a clock-progress requirement.
    ids::MAP_GET_RANDOM_GENERATION_PARAMETER,
    ids::MAP_CARVE_DIRECTIONAL_PATH,
    ids::MAP_FIND_REGION_GROWTH_ORIGIN,
    ids::MAP_GROW_TERRAIN_REGION,
    ids::MAP_EXPAND_REGIONS_BESIDE_PATHS,
    ids::MAP_STABILIZE_REGION_CONNECTIONS,
    ids::MAP_SEED_FILLED_TERRAIN_REGIONS,
    ids::MAP_BUILD_REGION_EDGE_SHAPES,
    // G/src/map/transition_paths.c:26,128 and map_population.c:71:
    // recursive RAM/ROM connectivity and transition searches; no clock observer.
    ids::MAP_TRACE_TRANSITION_CHAIN,
    ids::MAP_ASSIGN_TRANSITION_CHAIN,
    ids::MAP_IS_ANCHOR_CONNECTION_MISSING,
    // G/src/map/map_drawing_and_controller.c:21: RNG-only bucket selection.
    ids::MAP_CHOOSE_BACKGROUND_TILE_BUCKET,
    // G/src/map/terrain_at_position.c:17,54: ROM terrain lookup, doorway RAM
    // query (present_effects.c:205), and RAM cache writes; no hardware waits.
    ids::GET_MAP_TERRAIN_AT_WORLD_POSITION,
    ids::UPDATE_LAND_AROUND_PLAYER,
    // G/src/map/terrain_placement.c:13,80: RNG/ROM search and RAM coordinates;
    // ElevatorContainsPosition is RAM/abs only. Local output addresses and
    // unsuccessful-search residuals are frame concerns, not time dependencies.
    ids::TERRAIN_VALIDATION,
    ids::SET_POSITION_ON_NEW_LEVEL,
    // G/src/map/land_processing.c:43,91: allocation/eviction is RAM-only.
    // AllocateEntity/FreeEntity (sprite_slots.c:238/259) and shared tomato
    // release (projectile_effects.c:274) never wait or access a device.
    ids::ALLOCATE_ENTITY_WITH_EVICTION,
    ids::ENTITY_RELEASE_ONE_BY_PRIORITY,
    // Same file:146: RNG/ROM shape selection and RAM-only direction updates.
    // The original uninitialized shape path is not reclassified as a time wait.
    ids::MAP_CONSUME_PATH_CELL_DIRECTIONS,
];
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_random_semantics() {
        let mut state = 1;
        for expected in [
            16_807,
            282_475_249,
            1_622_650_073,
            984_943_658,
            1_144_108_930,
        ] {
            state = park_miller(state);
            assert_eq!(state, expected);
        }
        assert_eq!(normalize_seed(i32::MIN), i32::MIN);
        assert_eq!(normalize_seed(i32::MAX), 1);
        assert_eq!(normalize_seed(-12345), 12345);
    }
}
