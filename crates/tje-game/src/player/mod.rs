//! Player translation ownership; see docs/PORTING.md.
//!
//! Original execution contracts are reviewed independently of translation status.
//! Speech and graphics-upload waits require real interrupt progress; queuing an
//! audio request alone is RAM-only (`src/audio/play_sound_effect.c:15`).

use crate::execution::ExecutionClass;
use crate::{Context, Result, ids};
use tje_mem::{Address, LevelItem, ObjectState, Ptr, ram, rom};

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
    ids::STOP_PLAYER, // entity_motion_terrain.c:303: six RAM stores.
    ids::PLAYER1_PROCESS_TERRAIN_MOTION, // entity_motion_terrain.c:315: atomic forwarding.
    ids::PLAYER2_PROCESS_TERRAIN_MOTION, // entity_motion_terrain.c:322: atomic forwarding.
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
    ids::OPEN_ROCKET_SKATES, // open_rocket_skates.c:11: StopPlayer, SetSprite, RAM.
    ids::PLAYER_UPDATE_ROCKET_SKATES_SPRITE, // rocket_motion.c:16: StopPlayer, SetSprite, RAM.
    ids::PLAYER_APPLY_ROCKET_SKATES_INPUT, // rocket_motion.c:70: local arithmetic and RAM.
    ids::PLAYER_UPDATE_ROCKET_SKATES_MOTION, // rocket_motion.c:176: cached RNG and atomic input/sprite helpers.
    ids::PLAYER_INITIALIZE_PHASED_JUMP_MOTION, // start_motion_mode.c:5: four RAM stores.
    ids::PLAYER_CHECK_TERRAIN_EDGE_HOP, // edge_hop_and_clap.c:15: cache query, RNG, sprite/audio requests.
    ids::PLAYER_UPDATE_HEALTH_SHARING_CLAP, // edge_hop_and_clap.c:60: health arithmetic, sprite/audio requests.
    ids::OPEN_SCHOOL_BOOK, // sleep.c:21: sprite/motion and RAM countdown.
    ids::PROCESS_SLEEP, // sleep.c:34: RAM input, allocation, ROM animation, queued audio; no speech wait.
    ids::PLAYER_CAN_RECEIVE_EARTHLING_EFFECT, // earthling_damage_reactions.c:8: RAM predicate.
    ids::PLAYER_SQUISH, // earthling_damage_reactions.c:152: damage and queued audio.
    ids::EARTHLING_START_HORIZONTAL_ATTACK, // earthling_damage_reactions.c:226: abs and SetEarthlingState.
    ids::CUPID_ABILITY, // earthling_damage_reactions.c:245: bounded attack/state selection.
    ids::EARTHLING_UPDATE_BOUNCING_DAMAGE_STATE, // earthling_damage_reactions.c:283: terrain/damage; residual map index is an ABI issue, not a time wait.
    ids::WHIRL_WIND_ABILITY, // earthling_damage_reactions.c:315: sprite/RAM and FreeEntity.
    ids::EARTHLING_CHECK_DECOY_ATTACK_RANGE, // earthling_damage_reactions.c:412: two RAM targets and atomic attack helper.
    ids::ENTITY_HAS_FREE_TIMED_EFFECT_SLOT, // effect_slots.c:17: two RAM reads.
    ids::ENTITY_SPAWN_TIMED_EFFECT, // effect_slots.c:26: allocation, ROM animation, queued audio.
    ids::ENTITY_RELEASE_FINISHED_TIMED_EFFECT, // effect_slots.c:63: FreeEntity and RAM.
    ids::PLAYER_LAUNCH_PRESENT_PROJECTILE, // projectiles.c:24: allocation, ROM placement, queued FM request.
    ids::PLAYER_UPDATE_PRESENT_SHOOTING, // projectiles.c:105: atomic launch/pose helpers.
    ids::DO_MOLE_ABILITY, // mole.c:23: bounded inventory scan, cached RNG and allocation.
    ids::MOLE_ABILITY, // mole.c:77: inventory/state/collision helpers; no waits.
    ids::INITIALIZE_LEVEL_ZERO_FIXED_ITEMS, // ability_helpers.c:20: fixed RAM stores.
    ids::GET_LEVEL_ZERO_EARTHLING_POSITION, // ability_helpers.c:40: switch and two RAM stores.
];

/// Non-atomic ORIGINAL contracts, including still-untranslated callers.
/// `GenerateSpeech` can wait for an IRQ-cleared graphics upload
/// (G/src/speech/speech_and_dialogue.c:135); those callers are TimeAware even
/// when their ordinary entry state happens to bypass speech.
pub static EXECUTION_CONTRACTS: &[(crate::FunctionId, ExecutionClass)] = &[
    // active_present_falling_and_respawn.c:71: bounded VRAM read/write stream,
    // stable sources, no raster/status reads; FIFO transaction completion needed.
    (ids::MAP_CLEAR_TILE_PRIORITY_NEAR_PLAYER, ExecutionClass::Functional),
    // active_present_falling_and_respawn.c:134: IRQ-cleared terrain flag and speech.
    (ids::PLAYER_START_FALLING, ExecutionClass::TimeAware),
    // active_present_falling_and_respawn.c:250,407: MakePlayerFall redraws through
    // UpdateScreenSplitMode, whose shared/split transitions can wait for VBlank.
    (ids::CHECK_PLAYER_FALLING, ExecutionClass::TimeAware),
    (ids::MAKE_PLAYER_FALL, ExecutionClass::TimeAware),
    // active_present_falling_and_respawn.c:580: GenerateSpeech.
    (ids::PLAYER_UPDATE_TERRAIN_SINKING, ExecutionClass::TimeAware),
    // active_present_falling_and_respawn.c:636,750: PlayerStartFalling.
    (ids::PLAYER_UPDATE_HURT_RECOVERY_AND_IDLE, ExecutionClass::TimeAware),
    (ids::PLAYER_RESOLVE_DESCENDING_TERRAIN, ExecutionClass::TimeAware),
    // active_present_falling_and_respawn.c:811: explicit WaitForNextVBlank loop.
    (ids::PLAYER_ADVANCE_TO_NEXT_LEVEL, ExecutionClass::TimeAware),
    // active_present_falling_and_respawn.c:844: explicit paused VBlank wait,
    // plus PlayerAdvanceToNextLevel and screen transitions.
    (ids::PLAYER_RELOCATE_TO_PARTNER_OR_NEXT_LEVEL, ExecutionClass::TimeAware),
    // active_present_falling_and_respawn.c:923: HudLoadActionIconSprite waits for
    // the IRQ-cleared graphics upload (G/src/ui/hud.c:122-124).
    (ids::KILL_THE_PLAYER, ExecutionClass::TimeAware),
    // active_present_falling_and_respawn.c:956: screen redraw and game-over
    // input/VBlank loop; the reset branch does not normally return.
    (ids::PLAYER_FINALIZE_LAST_LIFE_EXIT, ExecutionClass::TimeAware),
    // active_present_falling_and_respawn.c:990,1052: falling/death/speech callees.
    (ids::PLAYER_UPDATE_DEATH_ANIMATION, ExecutionClass::TimeAware),
    (ids::PROCESS_PLAYERS, ExecutionClass::TimeAware),
    // jump_motion.c:21,117: PlayerStartFalling and GenerateSpeech.
    (ids::PLAYER_UPDATE_PHASED_JUMP_MOTION, ExecutionClass::TimeAware),
    (ids::PLAYER_APPLY_PHASED_JUMP_INPUT, ExecutionClass::TimeAware),
    // earthling_damage_reactions.c:40,177: GenerateSpeech.
    (ids::PLAYER_YOUCH, ExecutionClass::TimeAware),
    (ids::HULA_ABILITY, ExecutionClass::TimeAware),
    // earthling_damage_reactions.c:347: PlayerStartFalling and GenerateSpeech.
    (ids::PLAYER_UPDATE_WHIRLWIND_AND_RECOVERY, ExecutionClass::TimeAware),
    // earthling_damage_reactions.c:435,467: Player_Youch's speech path.
    (ids::CHICKEN_ABILITY, ExecutionClass::TimeAware),
    (ids::AI_BOOGIE_TOUCH, ExecutionClass::TimeAware),
    // ability_helpers.c:61: GenerateSpeech and general OpenSelectedPresent.
    (ids::LEMONADE_ABILITY, ExecutionClass::TimeAware),
    // hottub_ability.c:19: GenerateSpeech.
    (ids::HOTTUB_ABILITY, ExecutionClass::TimeAware),
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
