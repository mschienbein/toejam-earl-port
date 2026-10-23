//! Execution classes of original map routines, including untranslated entries.
//! Classification is independent of translation approval and call-frame support.
use crate::{FunctionId, execution::ExecutionClass, ids};

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
