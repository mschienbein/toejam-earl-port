//! Cutscenes translation ownership; see docs/PORTING.md.

use crate::{Context, Result, ids};
use tje_mem::{Ptr, ram, rom};

pub mod drifting_entities;
pub mod ending_player_release;
pub mod flying_entities;
pub mod impact_and_planet_entities;
pub mod intro;
pub mod intro_frame_entities;
pub mod intro_part11;
pub mod large_player_parts;
pub mod rocketship_pose;
pub mod sega_logo;

/// Complete bodies withheld from production until both-revision check evidence.
pub static PENDING: &[crate::FunctionId] = &[
    // TimeAware checks did not return from original IRQ/frame waits in both
    // revisions (proof-class-rev02/rev00); no successful checked entries yet.
    ids::INTRO_RELEASE_LARGE_PLAYER_SPRITES_AND_REFRESH,
    ids::INTRO_BEGIN_LARGE_PLAYER_SCENE,
    ids::INTRO_PART5,
    ids::INTRO_PART6,
    ids::INTRO_PART7,
    ids::INTRO_PART8,
    ids::INTRO_PART9,
    ids::SEGA_VIDEO,
    ids::INTRO_PART1,
    ids::INTRO_PART10,
    ids::INTRO_PART11,
    ids::INTRO_PART2,
    ids::INTRO_PART3,
    ids::INTRO_PART4,
    ids::INTRO_VIDEO,
    // A: settled both-revision fixtures: clears still mismatch logical VDP.
    ids::CLEAR_LOWER_VRAM_TILEMAP,
    ids::CLEAR_UPPER_VRAM_TILEMAP,
    // Narrow settled fixtures pass; broader/natural timed proof remains pending.
    ids::ANIMATE_TITLE_TRANSITION_BACKGROUND,
    ids::CUTSCENE_DRAW_RANDOMIZED_RECTANGLE_BORDER,
    // Both border modes and selected logo mismatch the IRQ-owned frame counter.
    ids::CUTSCENE_DRAW_RANDOMIZED_SCREEN_BORDER,
    ids::TITLE_DRAW_SELECTED_LOGO,
    // Cadence/refresh fixtures pass; initialization mismatches the frame counter.
    ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE,
    ids::INITIALIZE_TITLE_TRANSITION_TILEMAP,
    ids::INTRO_RELEASE_FRAME_ENTITIES_AND_REFRESH,
    // W4-W7 bodies await dependent-equivalence or readable-layer timed proof.
    // Ending scene bodies: whole-scene timed original exceeds the 1,000,000-tick
    // call bound (all six, both revisions; Ending lane's labelled/natural attempts).
    ids::ENDING_PLAY_DESCENDING_ROCKETSHIP_SCENE,
    // Both revisions still mismatch live RAM in a settled title fixture.
    ids::TITLE_SCREEN_VIDEO,
    ids::ENDING_PLAY_DIALOGUE_AND_DEPARTURE,
    ids::ENDING_PLAY_SCROLLING_CREDITS,
    ids::ENDING_PLAY_UPWARD_ROCKETSHIP_DEPARTURE,
    // Narrow fixtures pass; scene-interaction/IRQ paths still need natural proof.
    ids::FUNKATRON_UPDATE_SCENE_ENTITIES,
    ids::SEGA_LOGO_V_BLANK,
    // Whole attract invocation exceeds the original-call instruction bound.
    ids::SEGA_AND_INTRO_VIDEO_ROUTINE,
    // Whole-scene timed original exceeds 1,000,000-tick call bound; additionally
    // bouncing: dependent residual: awaiting dependent-equivalence proof.
    // PlayEndingSequence inherits both blockers.
    // Prepared-original caller reads are audited; whole-scene budget and
    // genuine dependent-residual obligations remain, rather than being waived.
    ids::ENDING_PLAY_BOUNCING_ROCKETSHIP_SCENE,
    ids::PLAY_ENDING_SEQUENCE,
];

/// Opt-in fixture roots only. The production dispatcher never consumes this list.
pub static PENDING_TRANSLATIONS: &[crate::Translation] = &[
    crate::Translation {
        id: ids::INTRO_RELEASE_LARGE_PLAYER_SPRITES_AND_REFRESH,
        function: large_player_parts::intro_release_large_player_sprites_and_refresh,
    },
    crate::Translation {
        id: ids::INTRO_BEGIN_LARGE_PLAYER_SCENE,
        function: intro::intro_begin_large_player_scene,
    },
    crate::Translation {
        id: ids::INTRO_PART5,
        function: intro::intro_part5,
    },
    crate::Translation {
        id: ids::INTRO_PART6,
        function: intro::intro_part6,
    },
    crate::Translation {
        id: ids::INTRO_PART7,
        function: intro::intro_part7,
    },
    crate::Translation {
        id: ids::INTRO_PART8,
        function: intro::intro_part8,
    },
    crate::Translation {
        id: ids::INTRO_PART9,
        function: intro::intro_part9,
    },
    crate::Translation {
        id: ids::SEGA_VIDEO,
        function: sega_logo::sega_video,
    },
    crate::Translation {
        id: ids::INTRO_PART1,
        function: intro::intro_part1,
    },
    crate::Translation {
        id: ids::INTRO_PART10,
        function: intro::intro_part10,
    },
    crate::Translation {
        id: ids::INTRO_PART11,
        function: intro_part11::intro_part11,
    },
    crate::Translation {
        id: ids::INTRO_PART2,
        function: intro::intro_part2,
    },
    crate::Translation {
        id: ids::INTRO_PART3,
        function: intro::intro_part3,
    },
    crate::Translation {
        id: ids::INTRO_PART4,
        function: intro::intro_part4,
    },
    crate::Translation {
        id: ids::INTRO_VIDEO,
        function: intro::intro_video,
    },
    crate::Translation {
        id: ids::CLEAR_LOWER_VRAM_TILEMAP,
        function: title_and_transition::clear_lower_vram_tilemap,
    },
    crate::Translation {
        id: ids::CLEAR_UPPER_VRAM_TILEMAP,
        function: title_and_transition::clear_upper_vram_tilemap,
    },
    crate::Translation {
        id: ids::ANIMATE_TITLE_TRANSITION_BACKGROUND,
        function: title_and_transition::animate_title_transition_background,
    },
    crate::Translation {
        id: ids::CUTSCENE_DRAW_RANDOMIZED_RECTANGLE_BORDER,
        function: title_and_transition::cutscene_draw_randomized_rectangle_border,
    },
    crate::Translation {
        id: ids::CUTSCENE_DRAW_RANDOMIZED_SCREEN_BORDER,
        function: title_and_transition::cutscene_draw_randomized_screen_border,
    },
    crate::Translation {
        id: ids::TITLE_DRAW_SELECTED_LOGO,
        function: title_and_transition::title_draw_selected_logo,
    },
    crate::Translation {
        id: ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE,
        function: title_and_transition::cutscene_step_frame_and_music_cadence,
    },
    crate::Translation {
        id: ids::INITIALIZE_TITLE_TRANSITION_TILEMAP,
        function: title_and_transition::initialize_title_transition_tilemap,
    },
    crate::Translation {
        id: ids::INTRO_RELEASE_FRAME_ENTITIES_AND_REFRESH,
        function: intro_frame_release::intro_release_frame_entities_and_refresh,
    },
    crate::Translation {
        id: ids::ENDING_PLAY_DESCENDING_ROCKETSHIP_SCENE,
        function: ending::ending_play_descending_rocketship_scene,
    },
    crate::Translation {
        id: ids::TITLE_SCREEN_VIDEO,
        function: title_and_transition::title_screen_video,
    },
    crate::Translation {
        id: ids::ENDING_PLAY_DIALOGUE_AND_DEPARTURE,
        function: ending::ending_play_dialogue_and_departure,
    },
    crate::Translation {
        id: ids::ENDING_PLAY_SCROLLING_CREDITS,
        function: ending::ending_play_scrolling_credits,
    },
    crate::Translation {
        id: ids::ENDING_PLAY_UPWARD_ROCKETSHIP_DEPARTURE,
        function: ending::ending_play_upward_rocketship_departure,
    },
    crate::Translation {
        id: ids::FUNKATRON_UPDATE_SCENE_ENTITIES,
        function: funkotron_scene::funkatron_update_scene_entities,
    },
    crate::Translation {
        id: ids::SEGA_LOGO_V_BLANK,
        function: sega_logo_vblank::sega_logo_vblank,
    },
    crate::Translation {
        id: ids::SEGA_AND_INTRO_VIDEO_ROUTINE,
        function: title_and_transition::sega_and_intro_video_routine,
    },
    crate::Translation {
        id: ids::ENDING_PLAY_BOUNCING_ROCKETSHIP_SCENE,
        function: ending::ending_play_bouncing_rocketship_scene,
    },
    crate::Translation {
        id: ids::PLAY_ENDING_SEQUENCE,
        function: ending::play_ending_sequence,
    },
];
mod collision_scene_entities;
mod composite_entities;
mod cutscene_text;
mod ending;
mod ending_player_entities;
mod funkotron_scene;
mod intro_frame_release;
mod rocketship_allocate;
mod rocketship_foreground;
mod rocketship_release;
mod sega_logo_vblank;
mod title_and_transition;

/// Available Rust entries owned by this subsystem.
pub static TRANSLATIONS: &[crate::Translation] = &[
    crate::Translation {
        id: ids::SEGA_LOGO_CLEAR_VIDEO_PLANES,
        function: sega_logo::sega_logo_clear_video_planes,
    },
    crate::Translation {
        id: ids::CUTSCENE_CREATE_DRIFTING_ENTITIES,
        function: drifting_entities::cutscene_create_drifting_entities,
    },
    crate::Translation {
        id: ids::CUTSCENE_CREATE_PLANET_ENTITY,
        function: impact_and_planet_entities::cutscene_create_planet_entity,
    },
    crate::Translation {
        id: ids::CUTSCENE_SET_ROCKETSHIP_PAIR_POSE,
        function: rocketship_pose::cutscene_set_rocketship_pair_pose,
    },
    crate::Translation {
        id: ids::CUTSCENE_UPDATE_DRIFTING_ENTITIES,
        function: drifting_entities::cutscene_update_drifting_entities,
    },
    crate::Translation {
        id: ids::ENDING_CREATE_STATIONARY_COMPOSITE,
        function: impact_and_planet_entities::ending_create_stationary_composite,
    },
    crate::Translation {
        id: ids::INTRO_ALLOCATE_DRIVING_PARTS,
        function: large_player_parts::intro_allocate_driving_parts,
    },
    crate::Translation {
        id: ids::INTRO_ALLOCATE_FLYING_ENTITY_SLOTS,
        function: drifting_entities::intro_allocate_flying_entity_slots,
    },
    crate::Translation {
        id: ids::INTRO_ALLOCATE_LARGE_PLAYER_PARTS,
        function: large_player_parts::intro_allocate_large_player_parts,
    },
    crate::Translation {
        id: ids::INTRO_CREATE_ASTEROID_ENTITY,
        function: large_player_parts::intro_create_asteroid_entity,
    },
    crate::Translation {
        id: ids::INTRO_CREATE_HIDDEN_FRAME_ENTITIES,
        function: intro_frame_entities::intro_create_hidden_frame_entities,
    },
    crate::Translation {
        id: ids::INTRO_CREATE_ROCKETSHIP_IMPACT,
        function: impact_and_planet_entities::intro_create_rocketship_impact,
    },
    crate::Translation {
        id: ids::INTRO_POSITION_DRIVING_PART,
        function: large_player_parts::intro_position_driving_part,
    },
    crate::Translation {
        id: ids::INTRO_POSITION_LARGE_PLAYER_PART,
        function: large_player_parts::intro_position_large_player_part,
    },
    crate::Translation {
        id: ids::INTRO_RELEASE_DRIVING_PART_SPRITES,
        function: large_player_parts::intro_release_driving_part_sprites,
    },
    crate::Translation {
        id: ids::INTRO_RELEASE_LARGE_PLAYER_PART_SPRITES,
        function: large_player_parts::intro_release_large_player_part_sprites,
    },
    crate::Translation {
        id: ids::INTRO_UPDATE_FLYING_ENTITY_MOTION,
        function: flying_entities::intro_update_flying_entity_motion,
    },
    crate::Translation {
        id: ids::INTRO_UPDATE_RIGHTWARD_FLYING_ENTITY,
        function: flying_entities::intro_update_rightward_flying_entity,
    },
    crate::Translation {
        id: ids::SEGA_LOGO_CREATE_TRADEMARK_ENTITY,
        function: sega_logo::sega_logo_create_trademark_entity,
    },
    crate::Translation {
        id: ids::CUTSCENE_RELEASE_DRIFTING_ENTITIES,
        function: drifting_entities::cutscene_release_drifting_entities,
    },
    crate::Translation {
        id: ids::CUTSCENE_RELEASE_PLANET_ENTITY,
        function: impact_and_planet_entities::cutscene_release_planet_entity,
    },
    crate::Translation {
        id: ids::ENDING_RELEASE_ACTIVE_PLAYER_ENTITIES,
        function: ending_player_release::ending_release_active_player_entities,
    },
    crate::Translation {
        id: ids::ENDING_RELEASE_STATIONARY_COMPOSITE,
        function: impact_and_planet_entities::ending_release_stationary_composite,
    },
    crate::Translation {
        id: ids::INTRO_POSITION_LARGE_PLAYER_PARTS,
        function: large_player_parts::intro_position_large_player_parts,
    },
    crate::Translation {
        id: ids::INTRO_RELEASE_ASTEROID_ENTITY,
        function: large_player_parts::intro_release_asteroid_entity,
    },
    crate::Translation {
        id: ids::INTRO_RELEASE_DRIVING_PARTS,
        function: large_player_parts::intro_release_driving_parts,
    },
    crate::Translation {
        id: ids::INTRO_RELEASE_FLYING_ENTITY_SLOTS,
        function: drifting_entities::intro_release_flying_entity_slots,
    },
    crate::Translation {
        id: ids::INTRO_RELEASE_LARGE_PLAYER_PARTS,
        function: large_player_parts::intro_release_large_player_parts,
    },
    crate::Translation {
        id: ids::INTRO_RELEASE_ROCKETSHIP_IMPACT,
        function: impact_and_planet_entities::intro_release_rocketship_impact,
    },
    crate::Translation {
        id: ids::SEGA_LOGO_RELEASE_TRADEMARK_ENTITY,
        function: sega_logo::sega_logo_release_trademark_entity,
    },
    crate::Translation {
        id: ids::CUTSCENE_APPEND_REMAPPED_FONT_TILES,
        function: cutscene_append_remapped_font_tiles,
    },
    crate::Translation {
        id: ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
        function: intro_set_large_player_part_sequence,
    },
    crate::Translation {
        id: ids::INTRO_SET_DRIVING_PART_SEQUENCE,
        function: intro_set_driving_part_sequence,
    },
    crate::Translation {
        id: ids::ENDING_GET_FIRST_PLAYER_ANIMATION_FRAME,
        function: ending_get_first_player_animation_frame,
    },
    crate::Translation {
        id: ids::INTRO_REVEAL_FRAME_ENTITIES,
        function: intro_reveal_frame_entities,
    },
    crate::Translation {
        id: ids::CUTSCENE_MOVE_ROCKETSHIP_PAIR_LEFT,
        function: cutscene_move_rocketship_pair_left,
    },
    crate::Translation {
        id: ids::CUTSCENE_POSITION_ROCKETSHIP_PAIR,
        function: cutscene_position_rocketship_pair,
    },
    crate::Translation {
        id: ids::ENDING_SET_POSE_COMPOSITE,
        function: ending_set_pose_composite,
    },
    crate::Translation {
        id: ids::ENDING_POSITION_SIX_PIECE_COMPOSITE,
        function: ending_position_six_piece_composite,
    },
    crate::Translation {
        id: ids::ENDING_POSITION_RIGHT_OFFSET_COMPOSITE,
        function: ending_position_right_offset_composite,
    },
    crate::Translation {
        id: ids::INTRO_POSITION_FALLING_ROCKETSHIP,
        function: intro_position_falling_rocketship,
    },
    crate::Translation {
        id: ids::ENDING_RESTART_ELEVATED_COMPOSITE_SEQUENCE,
        function: ending_restart_elevated_composite_sequence,
    },
    crate::Translation {
        id: ids::CUTSCENE_DRAW_METATILE_BLOCK,
        function: title_and_transition::cutscene_draw_metatile_block,
    },
    crate::Translation {
        id: ids::CUTSCENE_QUEUE_CENTERED_ORIGINAL_PALETTE_TEXT_LINE,
        function: cutscene_text::cutscene_queue_centered_original_palette_text_line,
    },
    crate::Translation {
        id: ids::CUTSCENE_QUEUE_CENTERED_REMAPPED_TEXT_LINE,
        function: cutscene_text::cutscene_queue_centered_remapped_text_line,
    },
    crate::Translation {
        id: ids::CUTSCENE_ALLOCATE_ROCKETSHIP_PAIR,
        function: rocketship_allocate::cutscene_allocate_rocketship_pair,
    },
    crate::Translation {
        id: ids::ENDING_CREATE_ELEVATED_THREE_PIECE_COMPOSITE,
        function: composite_entities::ending_create_elevated_three_piece_composite,
    },
    crate::Translation {
        id: ids::ENDING_CREATE_POSE_COMPOSITE,
        function: composite_entities::ending_create_pose_composite,
    },
    crate::Translation {
        id: ids::ENDING_CREATE_RIGHT_OFFSET_COMPOSITE,
        function: composite_entities::ending_create_right_offset_composite,
    },
    crate::Translation {
        id: ids::ENDING_CREATE_SIX_PIECE_COMPOSITE,
        function: composite_entities::ending_create_six_piece_composite,
    },
    crate::Translation {
        id: ids::ENDING_DRAW_NINE_METATILE_BACKGROUND,
        function: title_and_transition::ending_draw_nine_metatile_background,
    },
    crate::Translation {
        id: ids::ENDING_FILL_REPEATED_METATILE_BACKGROUND,
        function: title_and_transition::ending_fill_repeated_metatile_background,
    },
    crate::Translation {
        id: ids::FUNKATRON_CREATE_SCENE_ENTITY,
        function: funkotron_scene::funkatron_create_scene_entity,
    },
    crate::Translation {
        id: ids::INTRO_CREATE_FALLING_ROCKETSHIP,
        function: composite_entities::intro_create_falling_rocketship,
    },
    crate::Translation {
        id: ids::CUTSCENE_RELEASE_FLAGGED_ENTITY_RANGE,
        function: title_and_transition::cutscene_release_flagged_entity_range,
    },
    crate::Translation {
        id: ids::CUTSCENE_RELEASE_ROCKETSHIP_PAIR,
        function: rocketship_release::cutscene_release_rocketship_pair,
    },
    crate::Translation {
        id: ids::CUTSCENE_RELEASE_TEXT_ENTITIES,
        function: cutscene_text::cutscene_release_text_entities,
    },
    crate::Translation {
        id: ids::ENDING_RELEASE_ELEVATED_THREE_PIECE_COMPOSITE,
        function: composite_entities::ending_release_elevated_three_piece_composite,
    },
    crate::Translation {
        id: ids::ENDING_RELEASE_POSE_COMPOSITE,
        function: composite_entities::ending_release_pose_composite,
    },
    crate::Translation {
        id: ids::ENDING_RELEASE_RIGHT_OFFSET_COMPOSITE,
        function: composite_entities::ending_release_right_offset_composite,
    },
    crate::Translation {
        id: ids::ENDING_RELEASE_SIX_PIECE_COMPOSITE,
        function: composite_entities::ending_release_six_piece_composite,
    },
    crate::Translation {
        id: ids::INTRO_RELEASE_COLLISION_SCENE_ENTITIES,
        function: collision_scene_entities::intro_release_collision_scene_entities,
    },
    crate::Translation {
        id: ids::INTRO_RELEASE_FALLING_ROCKETSHIP,
        function: composite_entities::intro_release_falling_rocketship,
    },
    crate::Translation {
        id: ids::CUTSCENE_SET_FOREGROUND_ROCKETSHIP_POSE,
        function: rocketship_foreground::cutscene_set_foreground_rocketship_pose,
    },
    crate::Translation {
        id: ids::ENDING_CREATE_ACTIVE_PLAYER_ENTITIES,
        function: ending_player_entities::ending_create_active_player_entities,
    },
    crate::Translation {
        id: ids::INTRO_CREATE_COLLISION_SCENE_ENTITIES,
        function: collision_scene_entities::intro_create_collision_scene_entities,
    },
    crate::Translation {
        id: ids::CUTSCENE_LOAD_SECOND_TEXT_LINE,
        function: cutscene_text::cutscene_load_second_text_line,
    },
    crate::Translation {
        id: ids::CUTSCENE_LOAD_ORIGINAL_PALETTE_TEXT_LINE,
        function: cutscene_text::cutscene_load_original_palette_text_line,
    },
    crate::Translation {
        id: ids::CUTSCENE_TEXT_LOAD,
        function: cutscene_text::cutscene_text_load,
    },
];

// AllocateEntityViewportSprites' shared-tile/failure bug reads entry D4.W.
// Forward the matching compiled caller's low word, preserving its C ABI on
// both success and error. Unmodified-D4 callers need no extra forwarding.
fn allocate_viewport_with_d4(context: &mut dyn Context, object: u32, low: u16) -> Result<u32> {
    let saved = context.data_register(4);
    context.set_data_register(4, (saved & 0xffff_0000) | u32::from(low));
    let result = crate::call(context, ids::ALLOCATE_ENTITY_VIEWPORT_SPRITES, &[object, 0]);
    context.set_data_register(4, saved);
    result
}

/// Reviewed non-atomic original entries, whether or not translated yet.
/// Classes describe required execution, not approval of an unexercised body.
pub static EXECUTION_CONTRACTS: &[(crate::FunctionId, crate::execution::ExecutionClass)] = &[
    // G/src/cutscenes/sega_logo.c:24: fixed stable-ROM VDP writes, no status,
    // raster, interrupt or concurrent-source observations. Requires completed
    // transactions because the 4,176-word stream exceeds the frozen FIFO.
    (
        ids::SEGA_LOGO_CLEAR_VIDEO_PLANES,
        crate::execution::ExecutionClass::Functional,
    ),
    // G/src/cutscenes/large_player_parts.c:128: explicitly waits one VBlank
    // between queueing RAM sprite graphics and uploading them.
    (
        ids::INTRO_RELEASE_LARGE_PLAYER_SPRITES_AND_REFRESH,
        crate::execution::ExecutionClass::TimeAware,
    ),
    // G/src/cutscenes/intro.c:85: AnimateTitleTransitionBackground waits on IRQ.
    (
        ids::INTRO_BEGIN_LARGE_PLAYER_SCENE,
        crate::execution::ExecutionClass::TimeAware,
    ),
    // G/src/cutscenes/intro.c:524,628,711,819,866: every state-machine path
    // services real VBlank waits; controller and music state change over time.
    (
        ids::INTRO_PART5,
        crate::execution::ExecutionClass::TimeAware,
    ),
    (
        ids::INTRO_PART6,
        crate::execution::ExecutionClass::TimeAware,
    ),
    (
        ids::INTRO_PART7,
        crate::execution::ExecutionClass::TimeAware,
    ),
    (
        ids::INTRO_PART8,
        crate::execution::ExecutionClass::TimeAware,
    ),
    (
        ids::INTRO_PART9,
        crate::execution::ExecutionClass::TimeAware,
    ),
    // G/src/cutscenes/sega_logo.c:36: busy loops observe the IRQ-owned frame
    // counter and SegaLogoVBlank consumes queued palette/sprite publications.
    (ids::SEGA_VIDEO, crate::execution::ExecutionClass::TimeAware),
    // G/src/cutscenes/intro.c:97,912,314,383,455 and intro_part11.c:16:
    // controller-release loops and timed states call shared VBlank service.
    (
        ids::INTRO_PART1,
        crate::execution::ExecutionClass::TimeAware,
    ),
    (
        ids::INTRO_PART10,
        crate::execution::ExecutionClass::TimeAware,
    ),
    (
        ids::INTRO_PART11,
        crate::execution::ExecutionClass::TimeAware,
    ),
    (
        ids::INTRO_PART2,
        crate::execution::ExecutionClass::TimeAware,
    ),
    (
        ids::INTRO_PART3,
        crate::execution::ExecutionClass::TimeAware,
    ),
    (
        ids::INTRO_PART4,
        crate::execution::ExecutionClass::TimeAware,
    ),
    // G/src/cutscenes/intro.c:26: explicit waits plus ordered timed scene calls.
    (
        ids::INTRO_VIDEO,
        crate::execution::ExecutionClass::TimeAware,
    ),
    // CutscenesA entries (appended).
    // G/src/cutscenes/title_and_transition.c:87,99: explicit VBlank, then plane fill.
    (
        ids::CLEAR_LOWER_VRAM_TILEMAP,
        crate::execution::ExecutionClass::TimeAware,
    ),
    (
        ids::CLEAR_UPPER_VRAM_TILEMAP,
        crate::execution::ExecutionClass::TimeAware,
    ),
    // :200: 128 ordered VDP words, stable ROM metatiles, no status/raster reads.
    (
        ids::CUTSCENE_DRAW_METATILE_BLOCK,
        crate::execution::ExecutionClass::Functional,
    ),
    // :24: every path waits VBlank; changed frames additionally upload graphics.
    (
        ids::ANIMATE_TITLE_TRANSITION_BACKGROUND,
        crate::execution::ExecutionClass::TimeAware,
    ),
    // :178,142: explicit VBlank before ordered border writes and RNG draws.
    (
        ids::CUTSCENE_DRAW_RANDOMIZED_RECTANGLE_BORDER,
        crate::execution::ExecutionClass::TimeAware,
    ),
    (
        ids::CUTSCENE_DRAW_RANDOMIZED_SCREEN_BORDER,
        crate::execution::ExecutionClass::TimeAware,
    ),
    // :212,226: fixed metatile writers, no status/raster or unstable DMA source.
    (
        ids::ENDING_DRAW_NINE_METATILE_BACKGROUND,
        crate::execution::ExecutionClass::Functional,
    ),
    (
        ids::ENDING_FILL_REPEATED_METATILE_BACKGROUND,
        crate::execution::ExecutionClass::Functional,
    ),
    // :111: calls the VBlank-waiting clear before loading the selected logo.
    (
        ids::TITLE_DRAW_SELECTED_LOGO,
        crate::execution::ExecutionClass::TimeAware,
    ),
    // :245,69 and intro_frame_release.c:8: frame waits and interrupt-owned counters.
    (
        ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE,
        crate::execution::ExecutionClass::TimeAware,
    ),
    (
        ids::INITIALIZE_TITLE_TRANSITION_TILEMAP,
        crate::execution::ExecutionClass::TimeAware,
    ),
    (
        ids::INTRO_RELEASE_FRAME_ENTITIES_AND_REFRESH,
        crate::execution::ExecutionClass::TimeAware,
    ),
    // ending.c:466,405: explicit frame waits; bouncing also has a residual local.
    (
        ids::ENDING_PLAY_BOUNCING_ROCKETSHIP_SCENE,
        crate::execution::ExecutionClass::TimeAware,
    ),
    (
        ids::ENDING_PLAY_DESCENDING_ROCKETSHIP_SCENE,
        crate::execution::ExecutionClass::TimeAware,
    ),
    // title_and_transition.c:282: frame waits, controller timeline and music events.
    (
        ids::TITLE_SCREEN_VIDEO,
        crate::execution::ExecutionClass::TimeAware,
    ),
    // ending.c:68,287,221: script loops progress through real VBlank services.
    (
        ids::ENDING_PLAY_DIALOGUE_AND_DEPARTURE,
        crate::execution::ExecutionClass::TimeAware,
    ),
    (
        ids::ENDING_PLAY_SCROLLING_CREDITS,
        crate::execution::ExecutionClass::TimeAware,
    ),
    (
        ids::ENDING_PLAY_UPWARD_ROCKETSHIP_DEPARTURE,
        crate::execution::ExecutionClass::TimeAware,
    ),
    // funkatron_scene.c:49: StartDialogue reaches GenerateSpeech's explicit
    // IRQ-cleared upload wait (speech_and_dialogue.c:135-136), as well as panel VDP.
    (
        ids::FUNKATRON_UPDATE_SCENE_ENTITIES,
        crate::execution::ExecutionClass::TimeAware,
    ),
    // sega_logo_vblank.c:11: DMA completion and music's YM/Z80 transactions.
    (
        ids::SEGA_LOGO_V_BLANK,
        crate::execution::ExecutionClass::TimeAware,
    ),
    // ending.c:28 and title_and_transition.c:337: call full time-aware scenes.
    (
        ids::PLAY_ENDING_SEQUENCE,
        crate::execution::ExecutionClass::TimeAware,
    ),
    (
        ids::SEGA_AND_INTRO_VIDEO_ROUTINE,
        crate::execution::ExecutionClass::TimeAware,
    ),
    // cutscene_text.c:62,91: arbitrary embedded controls reach
    // inventory.c:666-682's VDP writes; repeated controls exceed a frozen FIFO.
    (
        ids::CUTSCENE_QUEUE_CENTERED_ORIGINAL_PALETTE_TEXT_LINE,
        crate::execution::ExecutionClass::Functional,
    ),
    (
        ids::CUTSCENE_QUEUE_CENTERED_REMAPPED_TEXT_LINE,
        crate::execution::ExecutionClass::Functional,
    ),
    // cutscene_text.c:164,194,133: inherit those ordered control-code VDP writes.
    (
        ids::CUTSCENE_LOAD_SECOND_TEXT_LINE,
        crate::execution::ExecutionClass::Functional,
    ),
    (
        ids::CUTSCENE_LOAD_ORIGINAL_PALETTE_TEXT_LINE,
        crate::execution::ExecutionClass::Functional,
    ),
    (
        ids::CUTSCENE_TEXT_LOAD,
        crate::execution::ExecutionClass::Functional,
    ),
];

/// Reviewed atomic original entries and translations. Constructors, positioners
/// and releases below only access RAM/ROM and RAM-only sprite allocation; their
/// loops do not need peripheral clocks or interrupts. RNG rejection loops depend
/// only on the RAM seed, never on asynchronous progress.
///
/// Reviewed originals with no clock/interrupt dependency. The W0 leaves below
/// are call-free; later entity constructors and releases make bounded
/// RAM/ROM calls only (G/src/cutscenes/{cutscene_text,composite_entities,
/// rocketship_allocate,rocketship_foreground,rocketship_release,
/// ending_player_entities,collision_scene_entities,title_and_transition,
/// funkotron_scene}.c). Classification does not waive residual-frame bugs.
pub static ATOMIC_CAPABLE: &[crate::FunctionId] = &[
    // G/src/cutscenes/drifting_entities.c:17,68,94: RAM-only entity allocation,
    // ROM frame dimensions, RNG state and per-object counters/positions.
    ids::CUTSCENE_CREATE_DRIFTING_ENTITIES,
    ids::CUTSCENE_UPDATE_DRIFTING_ENTITIES,
    ids::INTRO_ALLOCATE_FLYING_ENTITY_SLOTS,
    // G/src/cutscenes/impact_and_planet_entities.c:15,46,77: fixed constructor
    // stores followed by RAM-only viewport sprite allocation.
    ids::INTRO_CREATE_ROCKETSHIP_IMPACT,
    ids::CUTSCENE_CREATE_PLANET_ENTITY,
    ids::ENDING_CREATE_STATIONARY_COMPOSITE,
    // G/src/cutscenes/rocketship_pose.c:10: RAM sprite release/reallocation.
    ids::CUTSCENE_SET_ROCKETSHIP_PAIR_POSE,
    // G/src/cutscenes/large_player_parts.c:18,53,100,144,179,227,241:
    // counted ROM pieces and RAM-only allocation, placement or release.
    ids::INTRO_ALLOCATE_LARGE_PLAYER_PARTS,
    ids::INTRO_POSITION_LARGE_PLAYER_PART,
    ids::INTRO_RELEASE_LARGE_PLAYER_PART_SPRITES,
    ids::INTRO_ALLOCATE_DRIVING_PARTS,
    ids::INTRO_POSITION_DRIVING_PART,
    ids::INTRO_RELEASE_DRIVING_PART_SPRITES,
    ids::INTRO_CREATE_ASTEROID_ENTITY,
    // G/src/cutscenes/intro_frame_entities.c:16: ten counted-ROM constructors.
    ids::INTRO_CREATE_HIDDEN_FRAME_ENTITIES,
    // G/src/cutscenes/flying_entities.c:15,50: scene-dependent RAM motion, RNG,
    // and RAM-only sprite lifecycle; no waits/status/HV reads.
    ids::INTRO_UPDATE_RIGHTWARD_FLYING_ENTITY,
    ids::INTRO_UPDATE_FLYING_ENTITY_MOTION,
    // G/src/cutscenes/sega_logo.c:125: fixed RAM-only trademark construction.
    ids::SEGA_LOGO_CREATE_TRADEMARK_ENTITY,
    // G/src/cutscenes/drifting_entities.c:55,111: bounded RAM-only releases.
    ids::CUTSCENE_RELEASE_DRIFTING_ENTITIES,
    ids::INTRO_RELEASE_FLYING_ENTITY_SLOTS,
    // G/src/cutscenes/impact_and_planet_entities.c:39,70,103: conditional
    // RAM-only FreeEntity; intentionally retain the published selector.
    ids::INTRO_RELEASE_ROCKETSHIP_IMPACT,
    ids::CUTSCENE_RELEASE_PLANET_ENTITY,
    ids::ENDING_RELEASE_STATIONARY_COMPOSITE,
    // G/src/cutscenes/ending_player_release.c:8: two RAM-only releases.
    ids::ENDING_RELEASE_ACTIVE_PLAYER_ENTITIES,
    // G/src/cutscenes/large_player_parts.c:41,114,167,264: bounded RAM-only
    // releases or seven calls to the atomic positioner, in source order.
    ids::INTRO_RELEASE_LARGE_PLAYER_PARTS,
    ids::INTRO_POSITION_LARGE_PLAYER_PARTS,
    ids::INTRO_RELEASE_DRIVING_PARTS,
    ids::INTRO_RELEASE_ASTEROID_ENTITY,
    // G/src/cutscenes/sega_logo.c:148: RAM-only release and selector clear.
    ids::SEGA_LOGO_RELEASE_TRADEMARK_ENTITY,
    // W0 entries below are call-free RAM/ROM operations with local counters.
    ids::CUTSCENE_APPEND_REMAPPED_FONT_TILES,
    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
    ids::INTRO_SET_DRIVING_PART_SEQUENCE,
    ids::ENDING_GET_FIRST_PLAYER_ANIMATION_FRAME,
    ids::INTRO_REVEAL_FRAME_ENTITIES,
    ids::CUTSCENE_MOVE_ROCKETSHIP_PAIR_LEFT,
    ids::CUTSCENE_POSITION_ROCKETSHIP_PAIR,
    ids::ENDING_SET_POSE_COMPOSITE,
    ids::ENDING_POSITION_SIX_PIECE_COMPOSITE,
    ids::ENDING_POSITION_RIGHT_OFFSET_COMPOSITE,
    ids::INTRO_POSITION_FALLING_ROCKETSHIP,
    ids::ENDING_RESTART_ELEVATED_COMPOSITE_SEQUENCE,
    // W2: allocation, initialization, and viewport bookkeeping are RAM-only.
    ids::CUTSCENE_ALLOCATE_ROCKETSHIP_PAIR,
    ids::CUTSCENE_SET_FOREGROUND_ROCKETSHIP_POSE,
    ids::ENDING_CREATE_ACTIVE_PLAYER_ENTITIES,
    ids::ENDING_CREATE_ELEVATED_THREE_PIECE_COMPOSITE,
    ids::ENDING_CREATE_POSE_COMPOSITE,
    ids::ENDING_CREATE_RIGHT_OFFSET_COMPOSITE,
    ids::ENDING_CREATE_SIX_PIECE_COMPOSITE,
    ids::FUNKATRON_CREATE_SCENE_ENTITY,
    ids::INTRO_CREATE_COLLISION_SCENE_ENTITIES,
    ids::INTRO_CREATE_FALLING_ROCKETSHIP,
    // W3: bounded release loops, no rendering transfers or waits.
    ids::CUTSCENE_RELEASE_FLAGGED_ENTITY_RANGE,
    ids::CUTSCENE_RELEASE_ROCKETSHIP_PAIR,
    ids::CUTSCENE_RELEASE_TEXT_ENTITIES,
    ids::ENDING_RELEASE_ELEVATED_THREE_PIECE_COMPOSITE,
    ids::ENDING_RELEASE_POSE_COMPOSITE,
    ids::ENDING_RELEASE_RIGHT_OFFSET_COMPOSITE,
    ids::ENDING_RELEASE_SIX_PIECE_COMPOSITE,
    ids::INTRO_RELEASE_COLLISION_SCENE_ENTITIES,
    ids::INTRO_RELEASE_FALLING_ROCKETSHIP,
];

/// G/src/cutscenes/cutscene_text.c:26 (`CutsceneAppendRemappedFontTiles`).
/// The signed word byte count can wrap independently of the final offset update.
pub fn cutscene_append_remapped_font_tiles(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let mut source = arguments[0];
    let tile_count = arguments[1] as i16;
    let offset = ram::g_sprite_tile_buffer_offset(revision);
    let mut destination = ram::g_sprite_tile_upload_buffer(revision)
        .at(i32::from(offset.read(context)))
        .address();
    let byte_count = (i32::from(tile_count) << 5) as i16;
    let mut remaining = byte_count;
    while remaining > 0 {
        let mut pixels = match context.read_u8(source) & 15 {
            11 => 1,
            10 | 9 => 2,
            8 => 15,
            _ => 0,
        };
        // The matching code reads the source again before writing the destination.
        pixels |= match context.read_u8(source) & 0xf0 {
            0xb0 => 0x10,
            0xa0 | 0x90 => 0x20,
            0x80 => 0xf0,
            _ => 0,
        };
        context.write_u8(destination, pixels);
        destination = destination.wrapping_add(1);
        source = source.wrapping_add(1);
        remaining = remaining.wrapping_sub(1);
    }
    let current_offset = offset.read(context);
    offset.write(context, current_offset.wrapping_add(byte_count as u16));
    Ok(None)
}

/// G/src/cutscenes/large_player_parts.c:85 (`IntroSetLargePlayerPartSequence`).
/// Selector indexing is deliberately unchecked; period uses the low signed byte.
pub fn intro_set_large_player_part_sequence(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let index = arguments[0] as i16;
    let handle = ram::g_intro_large_player_part_entities(revision)
        .at(i32::from(index))
        .read(context);
    let entity = ram::g_player_info(revision).at(i32::from(handle));
    entity.sequence().write(context, Ptr::new(arguments[1]));
    entity.frame_phase().write(context, arguments[2] as i8);
    entity.frame_duration().write(context, 1);
    entity.frame().write(context, -1);
    Ok(None)
}

/// G/src/cutscenes/large_player_parts.c:212 (`IntroSetDrivingPartSequence`).
pub fn intro_set_driving_part_sequence(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let index = arguments[0] as i16;
    let handle = ram::g_intro_driving_part_entities(revision)
        .at(i32::from(index))
        .read(context);
    let entity = ram::g_player_info(revision).at(i32::from(handle));
    entity.sequence().write(context, Ptr::new(arguments[1]));
    entity.frame_phase().write(context, arguments[2] as i8);
    entity.frame_duration().write(context, 1);
    entity.frame().write(context, -1);
    Ok(None)
}

/// G/src/cutscenes/ending_player_release.c:22 (`EndingGetFirstPlayerAnimationFrame`).
/// Preserve the original first-handle lookup even when only player two exists.
pub fn ending_get_first_player_animation_frame(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let first = ram::g_ending_player1_entity(revision);
    if first.read(context) == 0 && ram::g_ending_player2_entity(revision).read(context) == 0 {
        return Ok(Some(0));
    }
    let entity = ram::g_player_info(revision).at(i32::from(first.read(context)));
    Ok(Some(i32::from(entity.frame().read(context)) as u32))
}

/// G/src/cutscenes/intro_frame_reveal.c:7 (`IntroRevealFrameEntities`).
/// Zero selectors still address entity zero, as in the original ten-slot loop.
pub fn intro_reveal_frame_entities(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    for index in 0..10 {
        let handle = ram::g_intro_component_entities(revision)
            .at(index)
            .read(context);
        let entity = ram::g_player_info(revision).at(i32::from(handle));
        let flags = entity.behavior_flags().read(context);
        entity.behavior_flags().write(context, flags & !0x40);
    }
    Ok(None)
}

/// G/src/cutscenes/rocketship_move.c:7 (`CutsceneMoveRocketshipPairLeft`).
/// The ship uses distance, the exhaust always one, and each word wraps before testing.
pub fn cutscene_move_rocketship_pair_left(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let handle = ram::g_cutscene_rocketship_entity(revision).read(context);
    let entity = ram::g_player_info(revision).at(i32::from(handle));
    let x = entity.x().read(context).wrapping_sub(arguments[0] as i16);
    entity.x().write(context, x);
    if entity.x().read(context) < -60 {
        entity.x().write(context, 320);
    }
    let handle = ram::g_cutscene_rocketship_exhaust_entity(revision).read(context);
    let entity = ram::g_player_info(revision).at(i32::from(handle));
    let x = entity.x().read(context).wrapping_sub(1);
    entity.x().write(context, x);
    if entity.x().read(context) < -60 {
        entity.x().write(context, 320);
    }
    Ok(None)
}

/// G/src/cutscenes/composite_entities.c:19 (`CutscenePositionRocketshipPair`).
/// Complete the ship stores before reading the exhaust selector (which may alias).
pub fn cutscene_position_rocketship_pair(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let handle = ram::g_cutscene_rocketship_entity(revision).read(context);
    let entity = ram::g_player_info(revision).at(i32::from(handle));
    entity.x().write(context, arguments[0] as i16);
    entity.y().write(context, arguments[1] as i16);
    let handle = ram::g_cutscene_rocketship_exhaust_entity(revision).read(context);
    let entity = ram::g_player_info(revision).at(i32::from(handle));
    entity.x().write(context, arguments[0] as i16);
    entity.y().write(context, arguments[1] as i16);
    Ok(None)
}

/// G/src/cutscenes/composite_entities.c:58 (`EndingSetPoseComposite`).
/// MCC68K doubles the frame index as a word before signed address addition.
pub fn ending_set_pose_composite(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let selector = ram::g_ending_pose_composite_entity(revision);
    if selector.read(context) != 0 {
        let entity = ram::g_player_info(revision).at(i32::from(selector.read(context)));
        entity
            .x()
            .write(context, (i32::from(arguments[1] as i16) - 5) as i16);
        entity
            .y()
            .write(context, (i32::from(arguments[2] as i16) - 5) as i16);
        let tile = entity.tile().read(context);
        entity.tile().write(context, tile | 0x8000);
        let flags = entity.draw_flags().read(context);
        entity.draw_flags().write(context, flags | 0x80);
        let frame_offset = (arguments[0] as i16).wrapping_mul(2);
        let sequence = rom::intro_held_frame_streams(revision)
            .byte_offset(i32::from(frame_offset))
            .address();
        entity.sequence().write(context, Ptr::new(sequence));
        entity.frame_duration().write(context, 1);
        entity.frame().write(context, -1);
        entity.frame_phase().write(context, 6);
    }
    Ok(None)
}

/// G/src/cutscenes/composite_entities.c:108 (`EndingPositionSixPieceComposite`).
pub fn ending_position_six_piece_composite(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let selector = ram::g_ending_six_piece_composite_entity(revision);
    if selector.read(context) != 0 {
        let entity = ram::g_player_info(revision).at(i32::from(selector.read(context)));
        entity.x().write(context, arguments[0] as i16);
        entity
            .y()
            .write(context, (i32::from(arguments[1] as i16) + 100) as i16);
        entity.height().write(context, 100);
    }
    Ok(None)
}

/// G/src/cutscenes/composite_entities.c:154 (`EndingPositionRightOffsetComposite`).
pub fn ending_position_right_offset_composite(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let selector = ram::g_ending_right_offset_composite_entity(revision);
    if selector.read(context) != 0 {
        let entity = ram::g_player_info(revision).at(i32::from(selector.read(context)));
        entity
            .x()
            .write(context, (i32::from(arguments[0] as i16) + 50) as i16);
        entity.y().write(context, arguments[1] as i16);
    }
    Ok(None)
}

/// G/src/cutscenes/composite_entities.c:206 (`IntroPositionFallingRocketship`).
pub fn intro_position_falling_rocketship(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let selector = ram::g_intro_falling_rocketship_entity(revision);
    if selector.read(context) != 0 {
        let entity = ram::g_player_info(revision).at(i32::from(selector.read(context)));
        entity.x().write(context, arguments[0] as i16);
        entity.y().write(context, arguments[1] as i16);
    }
    Ok(None)
}

/// G/src/cutscenes/composite_entities.c:253 (`EndingRestartElevatedCompositeSequence`).
pub fn ending_restart_elevated_composite_sequence(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let selector = ram::g_ending_elevated_three_piece_composite_entity(revision);
    if selector.read(context) != 0 {
        let entity = ram::g_player_info(revision).at(i32::from(selector.read(context)));
        entity.sprite_ready().write(context, 0);
        let sequence = rom::ending_frame_index_streams(revision).at(249).address();
        entity.sequence().write(context, Ptr::new(sequence));
        entity.frame_duration().write(context, 1);
        entity.frame().write(context, -1);
        entity.frame_phase().write(context, 4);
    }
    Ok(None)
}

#[cfg(test)]
mod tests;
