//! Original execution classes include untranslated routines, independently of proof.
mod alternatives;
mod consume;
mod display;
mod drawing;
mod frame_support;
mod generation;
mod generation_stages;
mod growth;
mod land;
mod levels;
mod paths;
mod pipeline;
mod placement;
mod random_generation;
mod regions;
#[cfg(test)]
#[path = "tests.rs"]
mod regression_tests;
mod screen;
mod screen_controller;
mod terrain;
mod waits;

pub use alternatives::*;
pub use consume::*;
pub use display::*;
pub use drawing::*;
pub use generation::*;
pub use generation_stages::*;
pub use growth::*;
pub use land::*;
pub use levels::*;
pub use paths::*;
pub use pipeline::*;
pub use placement::*;
pub use random_generation::*;
pub use regions::*;
pub use screen::*;
pub use screen_controller::*;
pub use terrain::*;
pub use waits::*;

use crate::{Context, FunctionId, Result, call, execution::ExecutionClass, ids};
use tje_mem::ram;

#[inline]
fn random_products(state: i32) -> (i32, i32, i32) {
    let high = state / 127_773;
    let low = state % 127_773;
    (
        high,
        16_807_i32.wrapping_mul(low),
        2_836_i32.wrapping_mul(high),
    )
}

/// G/src/map/random_regions_and_cleanup.c:11, the Schrage-split Park-Miller step.
/// Noncanonical seeds retain the original wrapping 32-bit intermediate arithmetic.
pub fn park_miller(state: i32) -> i32 {
    let (_, low_product, high_product) = random_products(state);
    let next = low_product.wrapping_sub(high_product);
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
/// Matching c_random_regions_and_cleanup.s:14-33 and src/asm/runtime.s:157-185:
/// D1 retains the swapped quotient and new low word; A0/A1 retain the final
/// high/low products. These volatile residues are live across the Idle exit.
pub fn get_random_number(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let state = ram::g_rng_state(context.revision());
    let (high, low_product, high_product) = random_products(state.read(context));
    let (difference, borrow) = (low_product as u32).overflowing_sub(high_product as u32);
    let (next, extend) = if (difference as i32) > 0 {
        (difference, borrow)
    } else {
        difference.overflowing_add(i32::MAX as u32)
    };
    state.write(context, next as i32);
    let word = next as u16;
    context.set_data_register(1, (u32::from(high as u16) << 16) | u32::from(word));
    context.set_address_register(0, high_product as u32);
    context.set_address_register(1, low_product as u32);
    // The last MOVE.W clears V/C and sets N/Z; X is the arithmetic carry/borrow.
    let flags = (u16::from(extend) << 4)
        | (u16::from(word & 0x8000 != 0) << 3)
        | (u16::from(word == 0) << 2);
    context.set_status_register((context.status_register() & !0x1f) | flags);
    Ok(Some(u32::from(word)))
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
    crate::Translation {
        id: ids::GET_MAP_TERRAIN_AT_WORLD_POSITION,
        function: get_map_terrain_at_world_position,
    },
    crate::Translation {
        id: ids::MAP_BUILD_FILLED_TERRAIN_SHAPES,
        function: map_build_filled_terrain_shapes,
    },
    crate::Translation {
        id: ids::MAP_EXPAND_REGIONS_BESIDE_PATHS,
        function: map_expand_regions_beside_paths,
    },
    crate::Translation {
        id: ids::MAP_IS_ANCHOR_CONNECTION_MISSING,
        function: map_is_anchor_connection_missing,
    },
    crate::Translation {
        id: ids::MAP_TRACE_TRANSITION_CHAIN,
        function: map_trace_transition_chain,
    },
    crate::Translation {
        id: ids::UPDATE_LAND_AROUND_PLAYER,
        function: update_land_around_player,
    },
    crate::Translation {
        id: ids::GENERATE_LEVEL_SEEDS,
        function: generate_level_seeds,
    },
    crate::Translation {
        id: ids::MAP_CHOOSE_BACKGROUND_TILE_BUCKET,
        function: map_choose_background_tile_bucket,
    },
    crate::Translation {
        id: ids::MAP_CONSUME_PATH_CELL_DIRECTIONS,
        function: map_consume_path_cell_directions,
    },
    crate::Translation {
        id: ids::MAP_FIND_REGION_GROWTH_ORIGIN,
        function: map_find_region_growth_origin,
    },
    crate::Translation {
        id: ids::MAP_GET_RANDOM_GENERATION_PARAMETER,
        function: map_get_random_generation_parameter,
    },
    crate::Translation {
        id: ids::MAP_STABILIZE_REGION_CONNECTIONS,
        function: map_stabilize_region_connections,
    },
    crate::Translation {
        id: ids::TERRAIN_VALIDATION,
        function: terrain_validation,
    },
    crate::Translation {
        id: ids::MAP_GENERATE_SEPARATED_ANCHORS,
        function: map_generate_separated_anchors,
    },
    crate::Translation {
        id: ids::MAP_BUILD_REGION_EDGE_SHAPES,
        function: map_build_region_edge_shapes,
    },
    crate::Translation {
        id: ids::MAP_SCATTER_TERRAIN_FEATURES,
        function: map_scatter_terrain_features,
    },
    crate::Translation {
        id: ids::ALLOCATE_ENTITY_WITH_EVICTION,
        function: allocate_entity_with_eviction,
    },
    crate::Translation {
        id: ids::ENTITY_RELEASE_ONE_BY_PRIORITY,
        function: entity_release_one_by_priority,
    },
    crate::Translation {
        id: ids::ENTITY_START_LAND_PATH_ANIMATION,
        function: entity_start_land_path_animation,
    },
    crate::Translation {
        id: ids::ENTITY_RESERVE_LAND_PATH_ANIMATIONS,
        function: entity_reserve_land_path_animations,
    },
    crate::Translation {
        id: ids::MAP_DRAW_FIXED_BACKGROUND_DECORATIONS,
        function: map_draw_fixed_background_decorations,
    },
    crate::Translation {
        id: ids::DRAW_MAP_CHARACTERS,
        function: draw_map_characters,
    },
    crate::Translation {
        id: ids::MAP_QUEUE_VISIBLE_METATILE_COLUMN,
        function: map_queue_visible_metatile_column,
    },
    crate::Translation {
        id: ids::MAP_QUEUE_VISIBLE_METATILE_ROW,
        function: map_queue_visible_metatile_row,
    },
    crate::Translation {
        id: ids::MAP_DRAW_ELEVATOR,
        function: map_draw_elevator,
    },
    crate::Translation {
        id: ids::MAP_DRAW_PLAYER,
        function: map_draw_player,
    },
    crate::Translation {
        id: ids::MAP_DRAW_SHIP_PIECE,
        function: map_draw_ship_piece,
    },
    crate::Translation {
        id: ids::DISPLAY_MAP,
        function: display_map,
    },
    crate::Translation {
        id: ids::MAP_UPLOAD_COVER_ANIMATION_FRAME,
        function: map_upload_cover_animation_frame,
    },
    crate::Translation {
        id: ids::LOAD_MAP_INTO_VRAM,
        function: load_map_into_vram,
    },
    crate::Translation {
        id: ids::MAP_DRAW_BACKGROUND_AND_TERRAIN,
        function: map_draw_background_and_terrain,
    },
    crate::Translation {
        id: ids::MAP_INITIALIZE_SCREEN_SPRITES,
        function: map_initialize_screen_sprites,
    },
    crate::Translation {
        id: ids::MAP_CONNECT_ANCHOR_POSITIONS,
        function: map_connect_anchor_positions,
    },
    crate::Translation {
        id: ids::MAP_SEED_FILLED_TERRAIN_REGIONS,
        function: map_seed_filled_terrain_regions,
    },
    crate::Translation {
        id: ids::MAP_CARVE_DIRECTIONAL_PATH,
        function: map_carve_directional_path,
    },
    crate::Translation {
        id: ids::MAP_GROW_TERRAIN_REGION,
        function: map_grow_terrain_region,
    },
    crate::Translation {
        id: ids::MAP_CHOOSE_METATILE_ALTERNATIVES,
        function: map_choose_metatile_alternatives,
    },
];

/// Bodies excluded from production until dual-revision check proof is complete.
/// Their execution classes remain declared for faithful original fallback.
pub static PENDING: &[crate::FunctionId] = &[
    // Corpus REV00 frame9287: original exceeds Atomic call bound at PC006B66.
    // RAM/ROM-only classification is retained; this is not a hardware-wait waiver.
    ids::MAP_ASSIGN_TRANSITION_CHAIN,
    // Clock contract: settled original performs one additional VBlank/ISR.
    ids::MAP_ANIMATE_METATILE_PATH_CHANGE,
    // Clock contract: marked-path branch observes two additional original ISR ticks.
    ids::PROCESS_LAND,
    // Clock contract / current timed ceiling on procedural generation.
    ids::CHANGE_LEVEL,
    ids::MAP_LOAD_PLAYER_LEVEL,
    // Dependent-equivalence: failed exhaustive placement consumes real old frame outputs.
    ids::SET_POSITION_ON_NEW_LEVEL,
    // Clock contract: original marked-path animation observes additional ISR ticks.
    ids::MAP_ANIMATE_MARKED_LAND_PATHS,
    // Clock contract after owner VDP prefix fix: original 128 IRQs versus Rust 120.
    ids::DRAW_GAME_MAP,
];

/// Opt-in proof hooks only; production never registers this table.
/// Keeping body references here also exercises the ordinary library target.
pub static PENDING_TRANSLATIONS: &[crate::Translation] = &[
    crate::Translation {
        id: ids::MAP_ASSIGN_TRANSITION_CHAIN,
        function: map_assign_transition_chain,
    },
    crate::Translation {
        id: ids::MAP_ANIMATE_METATILE_PATH_CHANGE,
        function: map_animate_metatile_path_change,
    },
    crate::Translation {
        id: ids::PROCESS_LAND,
        function: process_land,
    },
    crate::Translation {
        id: ids::CHANGE_LEVEL,
        function: change_level,
    },
    crate::Translation {
        id: ids::MAP_LOAD_PLAYER_LEVEL,
        function: map_load_player_level,
    },
];

/// Bodies requiring residual/dependent-equivalence or clock contracts stay unregistered.
pub static FRAME_PENDING_TRANSLATIONS: &[crate::Translation] = &[
    crate::Translation {
        id: ids::SET_POSITION_ON_NEW_LEVEL,
        function: set_position_on_new_level,
    },
    crate::Translation {
        id: ids::MAP_ANIMATE_MARKED_LAND_PATHS,
        function: map_animate_marked_land_paths,
    },
    crate::Translation {
        id: ids::DRAW_GAME_MAP,
        function: draw_game_map,
    },
];

/// Non-atomic original contracts. Atomic originals are listed in ATOMIC_CAPABLE.
/// Functional writers observe no raster/status/busy state and use stable ROM or
/// caller/RAM sources; TimeAware entries include every clock-dependent return path.
pub static EXECUTION_CONTRACTS: &[(FunctionId, ExecutionClass)] = &[
    // G/src/map/map_drawing_and_controller.c:36: nine ordered VDP writes, FIFO stuttering only.
    (
        ids::MAP_DRAW_FIXED_BACKGROUND_DECORATIONS,
        ExecutionClass::Functional,
    ),
    // Same file:59: bounded background/terrain writes and deterministic RNG; no time reads.
    (
        ids::MAP_DRAW_BACKGROUND_AND_TERRAIN,
        ExecutionClass::Functional,
    ),
    // Same file:158: cached RNG/reveal logic plus bounded VDP writes, no time reads.
    (ids::DISPLAY_MAP, ExecutionClass::Functional),
    // Same file:214: ordered VDP writes followed by DisplayMap; no hardware observations.
    (ids::LOAD_MAP_INTO_VRAM, ExecutionClass::Functional),
    // Same file:349/375 and elevator_icon.c:12: stable ROM icon uploads, no time reads.
    (ids::MAP_DRAW_PLAYER, ExecutionClass::Functional),
    (ids::MAP_DRAW_SHIP_PIECE, ExecutionClass::Functional),
    (ids::MAP_DRAW_ELEVATOR, ExecutionClass::Functional),
    // Same file:297: allocation failure calls EntityClearViewportSprites, which waits VBlank.
    (
        ids::MAP_INITIALIZE_SCREEN_SPRITES,
        ExecutionClass::TimeAware,
    ),
    // Same file:410: WaitForNextVBlank then observes frame counter bits for marker blink.
    (ids::DRAW_MAP_CHARACTERS, ExecutionClass::TimeAware),
    // Same file:443: VBlank, DMA, queued sprite graphics upload waits.
    (
        ids::MAP_UPLOAD_COVER_ANIMATION_FRAME,
        ExecutionClass::TimeAware,
    ),
    // Same file:501: pending upload and controller press/release loops, VBlank and DMA callees.
    (ids::DRAW_GAME_MAP, ExecutionClass::TimeAware),
    // G/src/map/level_generation_and_changes.c:536: waits gSpriteGraphicsUploadPending.
    (ids::CHANGE_LEVEL, ExecutionClass::TimeAware),
    // Same file:656: ChangeLevel retry path inherits the pending-upload wait.
    (ids::MAP_LOAD_PLAYER_LEVEL, ExecutionClass::TimeAware),
    // G/src/map/land_processing.c:249/280: waits for IRQ-cleared camera transfer queue flags.
    (
        ids::MAP_QUEUE_VISIBLE_METATILE_ROW,
        ExecutionClass::TimeAware,
    ),
    (
        ids::MAP_QUEUE_VISIBLE_METATILE_COLUMN,
        ExecutionClass::TimeAware,
    ),
    // Same file:310: UpdateAndQueueEntitySprites waits gSpriteGraphicsUploadPending (its :513).
    (
        ids::ENTITY_START_LAND_PATH_ANIMATION,
        ExecutionClass::TimeAware,
    ),
    // Same file:340: animation and per-row/column camera transfer waits.
    (
        ids::MAP_ANIMATE_METATILE_PATH_CHANGE,
        ExecutionClass::TimeAware,
    ),
    // Same file:405: waits for camera queues to become idle before entity reservation.
    (
        ids::ENTITY_RESERVE_LAND_PATH_ANIMATIONS,
        ExecutionClass::TimeAware,
    ),
    // Same file:445: camera transfers, frame-duration progression and upload pending waits.
    (
        ids::MAP_ANIMATE_MARKED_LAND_PATHS,
        ExecutionClass::TimeAware,
    ),
    // Same file:527: marked-land branches inherit MapAnimateMarkedLandPaths waits.
    (ids::PROCESS_LAND, ExecutionClass::TimeAware),
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
