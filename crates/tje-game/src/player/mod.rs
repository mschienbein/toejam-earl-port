//! Player translation ownership; see docs/PORTING.md.
//!
//! Original execution contracts are reviewed independently of translation status.
//! Speech and graphics-upload waits require real interrupt progress; queuing an
//! audio request alone is RAM-only (`src/audio/play_sound_effect.c:15`).

use crate::execution::ExecutionClass;
use crate::{Context, Result, ids};
use tje_mem::{Address, LevelItem, ObjectState, Ptr, ram, rom};

mod cooperation;
mod counters;
mod damage;
mod directional;
mod directional_animation;
mod effects;
mod falling;
mod falling_check;
mod finalize;
mod frames;
mod hottub;
mod indexing;
mod jump;
mod levels;
mod mole;
mod movement;
mod poses;
mod priority;
mod projectiles;
mod reactions;
mod rocket;
mod sleep;
mod state;
mod terrain;
mod terrain_motion;
mod update;

/// Available Rust entries owned by this subsystem.
pub static TRANSLATIONS: &[crate::Translation] = &[
    crate::Translation {
        id: ids::STOP_PLAYER,
        function: stop_player,
    },
    crate::Translation {
        id: ids::PLAYER_INITIALIZE_PHASED_JUMP_MOTION,
        function: player_initialize_phased_jump_motion,
    },
    crate::Translation {
        id: ids::ENTITY_HAS_FREE_TIMED_EFFECT_SLOT,
        function: entity_has_free_timed_effect_slot,
    },
    crate::Translation {
        id: ids::SETUP_PLAYER,
        function: setup_player,
    },
    crate::Translation {
        id: ids::PLAYER_CAN_RECEIVE_EARTHLING_EFFECT,
        function: player_can_receive_earthling_effect,
    },
    crate::Translation {
        id: ids::INITIALIZE_LEVEL_ZERO_FIXED_ITEMS,
        function: initialize_level_zero_fixed_items,
    },
    crate::Translation {
        id: ids::GET_LEVEL_ZERO_EARTHLING_POSITION,
        function: get_level_zero_earthling_position,
    },
    crate::Translation {
        id: ids::MAP_CLEAR_TILE_PRIORITY_NEAR_PLAYER,
        function: priority::map_clear_tile_priority_near_player,
    },
    crate::Translation {
        id: ids::ENTITY_IS_NEAR_POSITION,
        function: terrain::entity_is_near_position,
    },
    crate::Translation {
        id: ids::ENTITY_IS_WITHIN_RANGE,
        function: terrain::entity_is_within_range,
    },
    crate::Translation {
        id: ids::OPEN_ROCKET_SKATES,
        function: rocket::open_rocket_skates,
    },
    crate::Translation {
        id: ids::OPEN_SCHOOL_BOOK,
        function: state::open_school_book,
    },
    crate::Translation {
        id: ids::PLAYER_APPLY_ROCKET_SKATES_INPUT,
        function: rocket::player_apply_rocket_skates_input,
    },
    crate::Translation {
        id: ids::PLAYER_APPLY_TERRAIN_DESTINATION,
        function: terrain::player_apply_terrain_destination,
    },
    crate::Translation {
        id: ids::PLAYER_START_TEETERING_FALL,
        function: state::player_start_teetering_fall,
    },
    crate::Translation {
        id: ids::PLAYER_UPDATE_ROCKET_SKATES_SPRITE,
        function: rocket::player_update_rocket_skates_sprite,
    },
    crate::Translation {
        id: ids::DO_MOLE_ABILITY,
        function: mole::do_mole_ability,
    },
    crate::Translation {
        id: ids::ENTITY_SPAWN_TIMED_EFFECT,
        function: effects::entity_spawn_timed_effect,
    },
    crate::Translation {
        id: ids::GET_ACTIVE_PRESENT_SPRITE,
        function: poses::get_active_present_sprite,
    },
    crate::Translation {
        id: ids::PLAYER_CHECK_TERRAIN_EDGE_HOP,
        function: cooperation::player_check_terrain_edge_hop,
    },
    crate::Translation {
        id: ids::PLAYER_LAUNCH_PRESENT_PROJECTILE,
        function: projectiles::player_launch_present_projectile,
    },
    crate::Translation {
        id: ids::PLAYER_UPDATE_HEALTH_SHARING_CLAP,
        function: cooperation::player_update_health_sharing_clap,
    },
    crate::Translation {
        id: ids::PLAYER_UPDATE_JUMP_LANDING_ANIMATION,
        function: poses::player_update_jump_landing_animation,
    },
    crate::Translation {
        id: ids::PLAYER_UPDATE_ROCKET_SKATES_MOTION,
        function: rocket::player_update_rocket_skates_motion,
    },
    crate::Translation {
        id: ids::PLAYER_UPDATE_SPRING_SHOE_JUMP,
        function: poses::player_update_spring_shoe_jump,
    },
    crate::Translation {
        id: ids::PLAYER_UPDATE_TEETERING,
        function: poses::player_update_teetering,
    },
    crate::Translation {
        id: ids::PLAYER_SQUISH,
        function: damage::player_squish,
    },
    crate::Translation {
        id: ids::ENTITY_RELEASE_FINISHED_TIMED_EFFECT,
        function: effects::entity_release_finished_timed_effect,
    },
    crate::Translation {
        id: ids::PLAYER1_PROCESS_TERRAIN_MOTION,
        function: terrain::player1_process_terrain_motion,
    },
    crate::Translation {
        id: ids::PLAYER2_PROCESS_TERRAIN_MOTION,
        function: terrain::player2_process_terrain_motion,
    },
    crate::Translation {
        id: ids::PLAYER_UPDATE_PRESENT_COMPLETION_AND_TERRAIN_POSE,
        function: poses::player_update_present_completion_and_terrain_pose,
    },
    crate::Translation {
        id: ids::PLAYER_UPDATE_PRESENT_SHOOTING,
        function: projectiles::player_update_present_shooting,
    },
    crate::Translation {
        id: ids::PROCESS_MOVEMENT,
        function: movement::process_movement,
    },
    crate::Translation {
        id: ids::PROCESS_SLEEP,
        function: sleep::process_sleep,
    },
    crate::Translation {
        id: ids::WHIRL_WIND_ABILITY,
        function: damage::whirl_wind_ability,
    },
    crate::Translation {
        id: ids::MOLE_ABILITY,
        function: mole::mole_ability,
    },
    crate::Translation {
        id: ids::EARTHLING_START_HORIZONTAL_ATTACK,
        function: damage::earthling_start_horizontal_attack,
    },
    crate::Translation {
        id: ids::CUPID_ABILITY,
        function: damage::cupid_ability,
    },
    crate::Translation {
        id: ids::EARTHLING_UPDATE_BOUNCING_DAMAGE_STATE,
        function: damage::earthling_update_bouncing_damage_state,
    },
    crate::Translation {
        id: ids::EARTHLING_CHECK_DECOY_ATTACK_RANGE,
        function: damage::earthling_check_decoy_attack_range,
    },
    crate::Translation {
        id: ids::PLAYER_PROCESS_TERRAIN_MOTION,
        function: terrain_motion::player_process_terrain_motion,
    },
    crate::Translation {
        id: ids::PLAYER_APPLY_DIRECTIONAL_MOVEMENT_ANIMATION,
        function: directional_animation::player_apply_directional_movement_animation,
    },
];

/// Frame migrations awaiting a checker prerequisite, not missing frame bodies.
/// Chooser/filter retain the physical killed-high-byte read until differential
/// perturbation can discharge it. Falling passes its real local array to the
/// still-original MakePlayerFall and must wait for nested-original read auditing.
pub static FRAME_BLOCKED: &[crate::FunctionId] = &[
    ids::PLAYER_CHOOSE_DIRECTIONAL_MOVEMENT,
    ids::SPRITE_CHECKS,
    ids::CHECK_PLAYER_FALLING,
];

/// The full final-life body now propagates typed reset exits. Its UI game-over
/// callee is still pending separate admission; both returning and reset paths
/// are checked by the joint lane harness before production promotion.
pub static TERMINAL_BLOCKED: &[crate::FunctionId] = &[ids::PLAYER_FINALIZE_LAST_LIFE_EXIT];

/// No ctx-only frame target is published before its complete caller cohort is
/// prepared and jointly proven; value-only dispatch must never preload it.
pub static FRAMED_TRANSLATIONS: &[crate::FramedTranslation] = &[];

/// Canonical ctx-only frame candidates load only the source-used slots.
pub static PENDING_FRAMED_TRANSLATIONS: &[crate::FramedTranslation] = &[
    crate::FramedTranslation {
        id: ids::PLAYER_CHOOSE_DIRECTIONAL_MOVEMENT,
        function: directional::player_choose_directional_movement,
    },
    crate::FramedTranslation {
        id: ids::SPRITE_CHECKS,
        function: directional::sprite_checks,
    },
];

/// Complete bodies not yet admitted to production: both-revision proofs are
/// still required. These entries are never part of the production dispatcher.
pub static PENDING: &[crate::FunctionId] = &[
    ids::KILL_THE_PLAYER, // Both revisions: ordered device-write trace mismatch.
    ids::PLAYER_CHOOSE_DIRECTIONAL_MOVEMENT, // Framed; killed-high-byte perturbation proof pending.
    ids::SPRITE_CHECKS,   // Framed; transitive chooser perturbation proof pending.
    ids::CHECK_PLAYER_FALLING, // Framed; nested-original local-read audit and timing proof pending.
    ids::PLAYER_START_FALLING, // Both revisions: original terrain-priority wait did not return.
    ids::PLAYER_UPDATE_TERRAIN_SINKING, // Both revisions: ordered device-write trace mismatch.
    ids::PLAYER_UPDATE_HURT_RECOVERY_AND_IDLE, // Both revisions: device-write trace mismatch.
    ids::PLAYER_RESOLVE_DESCENDING_TERRAIN, // Both revisions: device-write trace mismatch.
    ids::PLAYER_APPLY_PHASED_JUMP_INPUT, // Both revisions: device-write trace mismatch.
    ids::PLAYER_UPDATE_PHASED_JUMP_MOTION, // Both revisions: device-write trace mismatch.
    ids::PLAYER_YOUCH,    // Both revisions: device-write trace mismatch.
    ids::HULA_ABILITY,    // Both revisions: device-write trace mismatch.
    ids::HOTTUB_ABILITY,  // Both revisions: device-write trace mismatch.
    ids::CHICKEN_ABILITY, // Both revisions: device-write trace mismatch.
    ids::AI_BOOGIE_TOUCH, // Both revisions: device-write trace mismatch.
    ids::PLAYER_UPDATE_WHIRLWIND_AND_RECOVERY, // Both revisions: device-write trace mismatch.
    ids::LEMONADE_ABILITY, // Both revisions: device-write trace mismatch.
    ids::PLAYER_ADVANCE_TO_NEXT_LEVEL, // Both revisions: original call did not return.
    ids::PLAYER_RELOCATE_TO_PARTNER_OR_NEXT_LEVEL, // Both revisions: original call did not return.
    ids::MAKE_PLAYER_FALL, // Both revisions: logical VDP state mismatch.
    ids::PLAYER_UPDATE_DEATH_ANIMATION, // Both revisions: device-write trace mismatch.
    ids::PROCESS_PLAYERS, // Both revisions: device-write trace mismatch.
    ids::PLAYER_FINALIZE_LAST_LIFE_EXIT, // Full typed-reset body; joint callee proof/admission pending.
];

/// Non-registered pending bodies retained for the opt-in lane proof harness.
pub static PENDING_TRANSLATIONS: &[crate::Translation] = &[
    crate::Translation {
        id: ids::KILL_THE_PLAYER,
        function: state::kill_the_player,
    },
    crate::Translation {
        id: ids::CHECK_PLAYER_FALLING,
        function: falling_check::check_player_falling,
    },
    crate::Translation {
        id: ids::PLAYER_START_FALLING,
        function: falling::player_start_falling,
    },
    crate::Translation {
        id: ids::PLAYER_UPDATE_TERRAIN_SINKING,
        function: falling::player_update_terrain_sinking,
    },
    crate::Translation {
        id: ids::PLAYER_UPDATE_HURT_RECOVERY_AND_IDLE,
        function: falling::player_update_hurt_recovery_and_idle,
    },
    crate::Translation {
        id: ids::PLAYER_RESOLVE_DESCENDING_TERRAIN,
        function: falling::player_resolve_descending_terrain,
    },
    crate::Translation {
        id: ids::PLAYER_APPLY_PHASED_JUMP_INPUT,
        function: jump::player_apply_phased_jump_input,
    },
    crate::Translation {
        id: ids::PLAYER_UPDATE_PHASED_JUMP_MOTION,
        function: jump::player_update_phased_jump_motion,
    },
    crate::Translation {
        id: ids::PLAYER_YOUCH,
        function: reactions::player_youch,
    },
    crate::Translation {
        id: ids::HULA_ABILITY,
        function: reactions::hula_ability,
    },
    crate::Translation {
        id: ids::HOTTUB_ABILITY,
        function: hottub::hottub_ability,
    },
    crate::Translation {
        id: ids::CHICKEN_ABILITY,
        function: reactions::chicken_ability,
    },
    crate::Translation {
        id: ids::AI_BOOGIE_TOUCH,
        function: reactions::ai_boogie_touch,
    },
    crate::Translation {
        id: ids::PLAYER_UPDATE_WHIRLWIND_AND_RECOVERY,
        function: reactions::player_update_whirlwind_and_recovery,
    },
    crate::Translation {
        id: ids::LEMONADE_ABILITY,
        function: levels::lemonade_ability,
    },
    crate::Translation {
        id: ids::PLAYER_ADVANCE_TO_NEXT_LEVEL,
        function: levels::player_advance_to_next_level,
    },
    crate::Translation {
        id: ids::PLAYER_RELOCATE_TO_PARTNER_OR_NEXT_LEVEL,
        function: levels::player_relocate_to_partner_or_next_level,
    },
    crate::Translation {
        id: ids::MAKE_PLAYER_FALL,
        function: levels::make_player_fall,
    },
    crate::Translation {
        id: ids::PLAYER_UPDATE_DEATH_ANIMATION,
        function: update::player_update_death_animation,
    },
    crate::Translation {
        id: ids::PROCESS_PLAYERS,
        function: update::process_players,
    },
    crate::Translation {
        id: ids::PLAYER_FINALIZE_LAST_LIFE_EXIT,
        function: finalize::player_finalize_last_life_exit,
    },
];

/// Reviewed original entries that need no clock or interrupt progress.
/// Source locations below are relative to G/src/player. Arithmetic, collision,
/// allocation, sprite selection, and audio-request callees are bounded RAM/ROM
/// operations; no pending-upload, raster, DMA-status, or sound-mailbox polling
/// occurs in their reachable paths.
pub static ATOMIC_CAPABLE: &[crate::FunctionId] = &[
    ids::SETUP_PLAYER, // setup_player.c:13: fixed RAM sprite/object initialization.
    ids::ENTITY_IS_NEAR_POSITION, // entity_motion_terrain.c:22: coordinates and abs.
    ids::ENTITY_IS_WITHIN_RANGE, // entity_motion_terrain.c:35: coordinates and abs.
    ids::PLAYER_PROCESS_TERRAIN_MOTION, // entity_motion_terrain.c:71: proposed motion and collision queries.
    ids::PLAYER_APPLY_TERRAIN_DESTINATION, // entity_motion_terrain.c:196: terrain/cache and SetSprite.
    ids::STOP_PLAYER,                      // entity_motion_terrain.c:303: six RAM stores.
    ids::PLAYER1_PROCESS_TERRAIN_MOTION,   // entity_motion_terrain.c:315: atomic forwarding.
    ids::PLAYER2_PROCESS_TERRAIN_MOTION,   // entity_motion_terrain.c:322: atomic forwarding.
    ids::PROCESS_MOVEMENT, // process_movement.c:17: entity-list integration and collision queries.
    ids::SPRITE_CHECKS, // sprite_checks_and_movement.c:14: input filtering; ProcessElevator is a RAM query.
    ids::PLAYER_CHOOSE_DIRECTIONAL_MOVEMENT, // sprite_checks_and_movement.c:103: velocity selection.
    ids::PLAYER_APPLY_DIRECTIONAL_MOVEMENT_ANIMATION, // sprite_checks_and_movement.c:205: animation selection and queued audio.
    ids::GET_ACTIVE_PRESENT_SPRITE, // active_present_falling_and_respawn.c:14: pose selection and OpenRocketSkates.
    ids::PLAYER_START_TEETERING_FALL, // active_present_falling_and_respawn.c:165: motion stores and SetSprite.
    ids::PLAYER_UPDATE_TEETERING, // active_present_falling_and_respawn.c:193: counters, cached RNG, queued audio.
    ids::PLAYER_UPDATE_JUMP_LANDING_ANIMATION, // active_present_falling_and_respawn.c:499: motion/sprite and queued audio.
    ids::PLAYER_UPDATE_PRESENT_COMPLETION_AND_TERRAIN_POSE, // active_present_falling_and_respawn.c:541: bounded terrain scan.
    ids::PLAYER_UPDATE_SPRING_SHOE_JUMP, // active_present_falling_and_respawn.c:699: motion/sprite and queued audio.
    ids::OPEN_ROCKET_SKATES,             // open_rocket_skates.c:11: StopPlayer, SetSprite, RAM.
    ids::PLAYER_UPDATE_ROCKET_SKATES_SPRITE, // rocket_motion.c:16: StopPlayer, SetSprite, RAM.
    ids::PLAYER_APPLY_ROCKET_SKATES_INPUT, // rocket_motion.c:70: local arithmetic and RAM.
    ids::PLAYER_UPDATE_ROCKET_SKATES_MOTION, // rocket_motion.c:176: cached RNG and atomic input/sprite helpers.
    ids::PLAYER_INITIALIZE_PHASED_JUMP_MOTION, // start_motion_mode.c:5: four RAM stores.
    ids::PLAYER_CHECK_TERRAIN_EDGE_HOP, // edge_hop_and_clap.c:15: cache query, RNG, sprite/audio requests.
    ids::PLAYER_UPDATE_HEALTH_SHARING_CLAP, // edge_hop_and_clap.c:60: health arithmetic, sprite/audio requests.
    ids::OPEN_SCHOOL_BOOK,                  // sleep.c:21: sprite/motion and RAM countdown.
    ids::PROCESS_SLEEP, // sleep.c:34: RAM input, allocation, ROM animation, queued audio; no speech wait.
    ids::PLAYER_CAN_RECEIVE_EARTHLING_EFFECT, // earthling_damage_reactions.c:8: RAM predicate.
    ids::PLAYER_SQUISH, // earthling_damage_reactions.c:152: damage and queued audio.
    ids::EARTHLING_START_HORIZONTAL_ATTACK, // earthling_damage_reactions.c:226: abs and SetEarthlingState.
    ids::CUPID_ABILITY, // earthling_damage_reactions.c:245: bounded attack/state selection.
    ids::EARTHLING_UPDATE_BOUNCING_DAMAGE_STATE, // earthling_damage_reactions.c:283: terrain/damage; residual map index is an ABI issue, not a time wait.
    ids::WHIRL_WIND_ABILITY, // earthling_damage_reactions.c:315: sprite/RAM and FreeEntity.
    ids::EARTHLING_CHECK_DECOY_ATTACK_RANGE, // earthling_damage_reactions.c:412: two RAM targets and atomic attack helper.
    ids::ENTITY_HAS_FREE_TIMED_EFFECT_SLOT,  // effect_slots.c:17: two RAM reads.
    ids::ENTITY_SPAWN_TIMED_EFFECT, // effect_slots.c:26: allocation, ROM animation, queued audio.
    ids::ENTITY_RELEASE_FINISHED_TIMED_EFFECT, // effect_slots.c:63: FreeEntity and RAM.
    ids::PLAYER_LAUNCH_PRESENT_PROJECTILE, // projectiles.c:24: allocation, ROM placement, queued FM request.
    ids::PLAYER_UPDATE_PRESENT_SHOOTING,   // projectiles.c:105: atomic launch/pose helpers.
    ids::DO_MOLE_ABILITY, // mole.c:23: bounded inventory scan, cached RNG and allocation.
    ids::MOLE_ABILITY,    // mole.c:77: inventory/state/collision helpers; no waits.
    ids::INITIALIZE_LEVEL_ZERO_FIXED_ITEMS, // ability_helpers.c:20: fixed RAM stores.
    ids::GET_LEVEL_ZERO_EARTHLING_POSITION, // ability_helpers.c:40: switch and two RAM stores.
];

/// Non-atomic ORIGINAL contracts, including still-untranslated callers.
/// `GenerateSpeech` can wait for an IRQ-cleared graphics upload
/// (G/src/speech/speech_and_dialogue.c:135); those callers are TimeAware even
/// when their ordinary entry state happens to bypass speech.
pub static EXECUTION_CONTRACTS: &[crate::execution::ExecutionContract] = &[
    // active_present_falling_and_respawn.c:71: bounded VRAM read/write stream,
    // stable sources, no raster/status reads; FIFO transaction completion needed.
    crate::execution::ExecutionContract {
        function: ids::MAP_CLEAR_TILE_PRIORITY_NEAR_PLAYER,
        class: ExecutionClass::Functional,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // active_present_falling_and_respawn.c:134: IRQ-cleared terrain flag and speech.
    crate::execution::ExecutionContract {
        function: ids::PLAYER_START_FALLING,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // active_present_falling_and_respawn.c:250,407: MakePlayerFall redraws through
    // UpdateScreenSplitMode, whose shared/split transitions can wait for VBlank.
    crate::execution::ExecutionContract {
        function: ids::CHECK_PLAYER_FALLING,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    crate::execution::ExecutionContract {
        function: ids::MAKE_PLAYER_FALL,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // active_present_falling_and_respawn.c:580: GenerateSpeech.
    crate::execution::ExecutionContract {
        function: ids::PLAYER_UPDATE_TERRAIN_SINKING,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // active_present_falling_and_respawn.c:636,750: PlayerStartFalling.
    crate::execution::ExecutionContract {
        function: ids::PLAYER_UPDATE_HURT_RECOVERY_AND_IDLE,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    crate::execution::ExecutionContract {
        function: ids::PLAYER_RESOLVE_DESCENDING_TERRAIN,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // active_present_falling_and_respawn.c:811: explicit WaitForNextVBlank loop.
    crate::execution::ExecutionContract {
        function: ids::PLAYER_ADVANCE_TO_NEXT_LEVEL,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // active_present_falling_and_respawn.c:844: explicit paused VBlank wait,
    // plus PlayerAdvanceToNextLevel and screen transitions.
    crate::execution::ExecutionContract {
        function: ids::PLAYER_RELOCATE_TO_PARTNER_OR_NEXT_LEVEL,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // active_present_falling_and_respawn.c:923: HudLoadActionIconSprite waits for
    // the IRQ-cleared graphics upload (G/src/ui/hud.c:122-124).
    crate::execution::ExecutionContract {
        function: ids::KILL_THE_PLAYER,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // active_present_falling_and_respawn.c:956: screen redraw and game-over
    // input/VBlank loop; the reset branch does not normally return.
    crate::execution::ExecutionContract {
        function: ids::PLAYER_FINALIZE_LAST_LIFE_EXIT,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // active_present_falling_and_respawn.c:990,1052: falling/death/speech callees.
    crate::execution::ExecutionContract {
        function: ids::PLAYER_UPDATE_DEATH_ANIMATION,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    crate::execution::ExecutionContract {
        function: ids::PROCESS_PLAYERS,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // jump_motion.c:21,117: PlayerStartFalling and GenerateSpeech.
    crate::execution::ExecutionContract {
        function: ids::PLAYER_UPDATE_PHASED_JUMP_MOTION,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    crate::execution::ExecutionContract {
        function: ids::PLAYER_APPLY_PHASED_JUMP_INPUT,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // earthling_damage_reactions.c:40,177: GenerateSpeech.
    crate::execution::ExecutionContract {
        function: ids::PLAYER_YOUCH,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    crate::execution::ExecutionContract {
        function: ids::HULA_ABILITY,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // earthling_damage_reactions.c:347: PlayerStartFalling and GenerateSpeech.
    crate::execution::ExecutionContract {
        function: ids::PLAYER_UPDATE_WHIRLWIND_AND_RECOVERY,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // earthling_damage_reactions.c:435,467: Player_Youch's speech path.
    crate::execution::ExecutionContract {
        function: ids::CHICKEN_ABILITY,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    crate::execution::ExecutionContract {
        function: ids::AI_BOOGIE_TOUCH,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // ability_helpers.c:61: GenerateSpeech and general OpenSelectedPresent.
    crate::execution::ExecutionContract {
        function: ids::LEMONADE_ABILITY,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // hottub_ability.c:19: GenerateSpeech.
    crate::execution::ExecutionContract {
        function: ids::HOTTUB_ABILITY,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
];
/// G/src/player/entity_motion_terrain.c:303 (`StopPlayer`). Preserve the original
/// interleaved velocity/acceleration store order, even on aliased addresses.
pub fn stop_player(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let state = Address::<ObjectState>::new(arguments[0]);
    state.velocity_x().write(context, 0);
    state.acceleration_x().write(context, 0);
    state.velocity_y().write(context, 0);
    state.acceleration_y().write(context, 0);
    state.velocity_height().write(context, 0);
    state.acceleration_height().write(context, 0);
    Ok(None)
}
/// G/src/player/start_motion_mode.c:5 (`PlayerInitializePhasedJumpMotion`).
pub fn player_initialize_phased_jump_motion(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let player = i32::from(arguments[0] as i16);
    ram::g_player_phased_jump_phase(revision)
        .at(player)
        .write(context, 1);
    ram::g_player_phased_jump_phase_ticks(revision)
        .at(player)
        .write(context, 16);
    ram::g_player_phased_jump_armed(revision)
        .at(player)
        .write(context, 1);
    ram::g_player_phased_jump_speech_used(revision)
        .at(player)
        .write(context, 0);
    Ok(None)
}
/// G/src/player/effect_slots.c:17 (`EntityHasFreeTimedEffectSlot`). Signed slot
/// indices use C truth, not a positive-only host predicate.
pub fn entity_has_free_timed_effect_slot(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let slots = ram::g_timed_effect_entities(context.revision());
    Ok(Some(u32::from(
        slots.at(0).read(context) == 0 || slots.at(1).read(context) == 0,
    )))
}

/// G/src/player/setup_player.c:13 (`SetupPlayer`).
/// Sprite-chain setup changes the RAM shadow, not the VDP; ROM pointers remain emulated.
pub fn setup_player(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let sprites = ram::g_hardware_sprite_table(revision);
    sprites.at(0).link().write(context, 11);
    for sprite_number in 0..10 {
        let mut tile = 0x85a9;
        if sprite_number & 1 != 0 {
            tile |= 0x1800;
        }
        let sprite = sprites.at(sprite_number + 1);
        sprite
            .x()
            .write(context, ((sprite_number << 5) + 128) as i16);
        sprite.y().write(context, 236);
        sprite.size().write(context, 12);
        sprite.link().write(context, (sprite_number + 2) as u8);
        sprite.tile().write(context, tile);
    }
    for sprite_number in 11..23 {
        sprites
            .at(sprite_number)
            .link()
            .write(context, (sprite_number + 1) as u8);
    }
    sprites.at(23).link().write(context, 23);
    sprites.at(23).link().write(context, 79);
    ram::g_primary_viewport_uploaded_link(revision).write(context, 79);
    sprites.at(24).link().write(context, 79);
    ram::g_secondary_viewport_uploaded_link(revision).write(context, 79);
    sprites.at(79).link().write(context, 0);
    let mut entity = ram::g_player_info(revision).at(0);
    entity.process().write(
        context,
        Ptr::new(
            ids::PLAYER1_PROCESS_TERRAIN_MOTION
                .metadata()
                .address(revision),
        ),
    );
    let animation = rom::player_animation_descriptors(revision).at(0);
    let sequence = animation.sequence().read(context);
    entity.sequence().write(context, sequence);
    let frames = animation.frames().read(context);
    entity.frames().write(context, frames);
    entity.frame_phase().write(context, 10);
    entity.frame_duration().write(context, 1);
    entity.width().write(context, 24);
    entity.height_pixels().write(context, 32);
    entity.sprite_count().write(context, 3);
    entity.tile_count().write(context, 40);
    entity.tile().write(context, 0x8000);
    entity.draw_flags().write(context, 1);
    entity.behavior_flags().write(context, 1);
    entity = ram::g_player_info(revision).at(1);
    entity.process().write(
        context,
        Ptr::new(
            ids::PLAYER2_PROCESS_TERRAIN_MOTION
                .metadata()
                .address(revision),
        ),
    );
    let animation = rom::player2_animation_descriptors(revision).at(0);
    let sequence = animation.sequence().read(context);
    entity.sequence().write(context, sequence);
    let frames = animation.frames().read(context);
    entity.frames().write(context, frames);
    entity.frame_phase().write(context, 10);
    entity.frame_duration().write(context, 1);
    entity.width().write(context, 30);
    entity.height_pixels().write(context, 40);
    entity.sprite_count().write(context, 4);
    entity.tile_count().write(context, 40);
    entity.tile().write(context, 0x8000);
    entity.draw_flags().write(context, 1);
    entity.behavior_flags().write(context, 1);
    entity = ram::g_player_info(revision).at(26);
    ram::g_inventory_draw_list_head(revision).write(context, Ptr::new(entity.address()));
    for _ in 0..2 {
        entity.frames().write(
            context,
            Ptr::new(rom::spr_present_selector(revision).address()),
        );
        entity.sequence().write(
            context,
            Ptr::new(rom::single_frame_hold_animation(revision).address()),
        );
        entity.width().write(context, 32);
        entity.height_pixels().write(context, 24);
        entity.sprite_count().write(context, 1);
        entity.tile_count().write(context, 12);
        entity.tile().write(context, 0x8000);
        entity
            .next()
            .write(context, Ptr::new(entity.offset(1).address()));
        entity = entity.next().read(context).target();
        for _ in 0..6 {
            entity.frames().write(
                context,
                Ptr::new(rom::present_sprite_table(revision).address()),
            );
            entity.sequence().write(
                context,
                Ptr::new(rom::single_frame_hold_animation(revision).address()),
            );
            entity.width().write(context, 24);
            entity.height_pixels().write(context, 16);
            entity.sprite_count().write(context, 1);
            entity.tile_count().write(context, 6);
            entity.tile().write(context, 0x8000);
            entity
                .next()
                .write(context, Ptr::new(entity.offset(1).address()));
            entity = entity.next().read(context).target();
        }
    }
    entity.offset(-1).next().write(context, Ptr::new(0));
    Ok(None)
}

/// G/src/player/earthling_damage_reactions.c:8 (`PlayerCanReceiveEarthlingEffect`).
pub fn player_can_receive_earthling_effect(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let player = Address::<ObjectState>::new(arguments[0]);
    let player_number = i32::from(arguments[1] as i16);
    let allowed = ram::g_player_interaction_choice_active(context.revision())
        .at(player_number)
        .read(context)
        == 0
        && (player.sprite_state().read(context) == 0
            || player.sprite_state().read(context) == 4
            || player.animation_group_mask().read(context) & 0x2040_0000 != 0
            || player.sprite_state().read(context) == 76
            || player.animation_group_mask().read(context) & 0x0830_409c != 0);
    Ok(Some(u32::from(allowed)))
}

/// G/src/player/ability_helpers.c:20 (`InitializeLevelZeroFixedItems`).
/// Coordinates are C code literals; object handles and padding are left untouched.
pub fn initialize_level_zero_fixed_items(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let item = Address::<LevelItem>::new(arguments[0]);
    item.kind().write(context, 82);
    item.level().write(context, 0);
    item.x().write(context, 1464);
    item.y().write(context, 854);
    let item = item.offset(1);
    item.kind().write(context, 82);
    item.level().write(context, 0);
    item.x().write(context, 1536);
    item.y().write(context, 928);
    let item = item.offset(1);
    item.kind().write(context, 82);
    item.level().write(context, 0);
    item.x().write(context, 1336);
    item.y().write(context, 1110);
    Ok(None)
}

/// G/src/player/ability_helpers.c:40 (`GetLevelZeroEarthlingPosition`).
/// The switch default performs no stores; matching cases store X before Y.
pub fn get_level_zero_earthling_position(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let (x, y) = match arguments[0] as i16 {
        23 => (1536, 1024),
        24 => (1398, 980),
        5 => (1536, 944),
        _ => return Ok(None),
    };
    Address::<i16>::new(arguments[1]).write(context, x);
    Address::<i16>::new(arguments[2]).write(context, y);
    Ok(None)
}
