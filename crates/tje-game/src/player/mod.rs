//! Player translation ownership; see docs/PORTING.md.
//!
//! `MapClearTilePriorityNearPlayer` (G/src/player/active_present_falling_and_respawn.c:71)
//! remains original: its alternating VDP reads/writes wait for FIFO clock progress.

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

/// Reviewed original entries that return without clock/interrupt progress.
/// Missing IDs are deliberately not approved for atomic checking or fallback.
pub static ATOMIC_CAPABLE: &[crate::FunctionId] = &[
    ids::STOP_PLAYER,
    ids::PLAYER_INITIALIZE_PHASED_JUMP_MOTION,
    ids::ENTITY_HAS_FREE_TIMED_EFFECT_SLOT,
    // These four leaves perform bounded RAM/ROM accesses only, without callees.
    ids::SETUP_PLAYER,
    ids::PLAYER_CAN_RECEIVE_EARTHLING_EFFECT,
    ids::INITIALIZE_LEVEL_ZERO_FIXED_ITEMS,
    ids::GET_LEVEL_ZERO_EARTHLING_POSITION,
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
