//! Presents translation ownership; see docs/PORTING.md.

use crate::{Context, Result, ids};
use tje_mem::{Address, ObjectState, ram, rom};

/// Available Rust entries owned by this subsystem.
pub static TRANSLATIONS: &[crate::Translation] = &[
    crate::Translation {
        id: ids::MAP_IS_LARGE_PLACEMENT_TERRAIN_EXCLUDED,
        function: map_is_large_placement_terrain_excluded,
    },
    crate::Translation {
        id: ids::PRESENT_CLEAR_WORLD_ENTITY_READINESS,
        function: present_clear_world_entity_readiness,
    },
    crate::Translation {
        id: ids::PRESENT_REGISTER_DECOY_HIT,
        function: present_register_decoy_hit,
    },
    crate::Translation {
        id: ids::PRESENT_FIND_DOORWAY_OWNER_AT_TILE,
        function: present_find_doorway_owner_at_tile,
    },
    crate::Translation {
        id: ids::PLAYER_REMAP_DIRECTIONAL_INPUT,
        function: player_remap_directional_input,
    },
    crate::Translation {
        id: ids::NO_OP,
        function: no_op,
    },
];

/// Original execution contracts, including untranslated entries.
///
/// These paths only read ROM/RAM and call storage-only entity, RNG, collision,
/// animation, or sound-request helpers. Random/retry loops do not require clocks;
/// the original nontermination on wholly rejected placement maps is preserved.
/// ABI defects (missing arguments/local frames) are separate translation blockers,
/// not reasons to misclassify a storage-only original as time-dependent.
pub static ATOMIC_CAPABLE: &[crate::FunctionId] = &[
    // G/src/presents/level_item_setup.c:32-477: RAM banks, RNG and terrain search.
    ids::SETUP_PRESENT_IDENTITIES,
    ids::SETUP_INITIAL_INVENTORY,
    ids::MAP_RELEASE_LEVEL_ITEM_BANK,
    ids::ELEVATOR_ALLOWS_WORLD_PLACEMENT,
    ids::MAP_IS_LARGE_PLACEMENT_TERRAIN_EXCLUDED,
    ids::MAP_FIND_ITEM_PLACEMENT_POSITION,
    ids::MAP_PLACE_LARGE_LEVEL_ITEMS,
    ids::SELECT_RANDOM_PRESENT,
    ids::LOAD_LEVEL_ITEM_BANK,
    ids::DROP_PRESENT,
    // G/src/presents/item_updates.c:40-128,343-362: entity allocation/free and RAM.
    ids::ENTITY_CREATE_LEVEL_ITEM,
    ids::ENTITY_UPDATE_LEVEL_ITEM_VISIBILITY,
    ids::PRESENT_RELEASE_DROPPED_ENTITIES,
    ids::PRESENT_CLEAR_WORLD_ENTITY_READINESS,
    // G/src/presents/present_effects.c:40-227,467-609: bounded object updates;
    // PlaySound/PlaySoundEffect enqueue requests, not device-status polling.
    ids::PRESENT_START_TRANSIENT_EFFECT,
    ids::OPEN_DECOY,
    ids::PRESENT_UPDATE_TRANSIENT_EFFECT,
    ids::PRESENT_REGISTER_DECOY_HIT,
    ids::PRESENT_UPDATE_DECOY,
    ids::PRESENT_IS_DOORWAY,
    ids::PRESENT_FIND_DOORWAY_OWNER_AT_TILE,
    ids::PRESENT_IS_RAIN_CLOUD,
    ids::PRESENT_UPDATE_RAIN_CLOUD,
    ids::OPEN_BOOM_BOX,
    ids::PRESENT_UPDATE_BOOM_BOX,
    // G/src/presents/projectile_effects.c:48-565,615-671: shared object pools,
    // RAM animation descriptors, collision/terrain tests and sound requests.
    ids::EARTHLING_LAUNCH_AIMED_PROJECTILE,
    ids::PROCESS_TOMATO_RAIN,
    ids::SET_TOMATO_RAIN_TOMATOES,
    ids::CHECK_TOMATO_RAIN_TIMER,
    ids::PRESENT_RELEASE_FIRST_SHARED_TOMATO,
    ids::EARTHLING_LAUNCH_CONTROL_REMAP_PROJECTILE,
    ids::ENTITY_FIND_PROJECTILE_TARGET_HIT,
    ids::PLAYER_APPLY_CONTROL_REMAP_EFFECT,
    ids::PLAYER_REMAP_DIRECTIONAL_INPUT,
    ids::PLAYER_UPDATE_CONTROL_REMAP_EFFECT,
    ids::EARTHLING_UPDATE_CONTROL_REMAP_PROJECTILES,
    ids::PRESENT_CREATE_ACTIVE_TERRAIN_EFFECT,
    ids::PRESENT_RETIRE_TERRAIN_EFFECT,
    ids::PRESENT_CREATE_TEMPORARY_TERRAIN_EFFECT,
    ids::EARTHLING_UPDATE_CONTROL_REMAP_ATTACK,
    ids::EARTHLING_UPDATE_AIMED_PROJECTILE_ATTACK,
    // G/src/presents/telephone_open.c:9-72,telephone.c:25-68,
    // telephone_clear.c:7-25: placement, entity state and queued sound only.
    ids::TELEPHONE_PLACE_OUTSIDE_PLAYER_VIEWS,
    ids::OPEN_TELEPHONE,
    ids::TELEPHONE_CREATE_DISPLAY_ENTITY,
    ids::TELEPHONE_START_RING,
    ids::TELEPHONE_CLEAR_ALL,
    // G/src/presents/noop.c:4: RTS.
    ids::NO_OP,
];

/// Whole-original execution classes for paths requiring interrupt progress.
/// No presents path is Functional: device latency alone cannot settle the
/// IRQ-owned sprite-upload flags, input loops or VBlank synchronization below.
pub static EXECUTION_CONTRACTS: &[(crate::FunctionId, crate::execution::ExecutionClass)] = &[
    // G/src/presents/item_updates.c:131-255: speech/dialogue (including Player_Youch)
    // can wait on gSpriteGraphicsUploadPending in speech_and_dialogue.c:135.
    (
        ids::EAT_YUMMY_FOOD,
        crate::execution::ExecutionClass::TimeAware,
    ),
    (
        ids::EAT_YUCKY_FOOD,
        crate::execution::ExecutionClass::TimeAware,
    ),
    (
        ids::EAT_FUDGE_SUNDAE,
        crate::execution::ExecutionClass::TimeAware,
    ),
    (
        ids::PLAYER_COLLECT_NEARBY_ITEMS,
        crate::execution::ExecutionClass::TimeAware,
    ),
    (
        ids::PLAYER_CHECK_LEVEL_ITEM_HAZARDS,
        crate::execution::ExecutionClass::TimeAware,
    ),
    // G/src/presents/item_updates.c:259-339: HudLoadActionIconSprite waits for
    // the IRQ-owned upload flag (src/ui/hud.c:122-125).
    (
        ids::PRESENT_TRY_END_ACTIVE_POWERUP,
        crate::execution::ExecutionClass::TimeAware,
    ),
    (
        ids::REDUCE_PRESENT_TIMER,
        crate::execution::ExecutionClass::TimeAware,
    ),
    // G/src/presents/present_effects.c:230-257: explicit VBlank then screen redraw.
    (
        ids::PLAYER_TELEPORT_THROUGH_DOORWAY,
        crate::execution::ExecutionClass::TimeAware,
    ),
    // G/src/presents/present_effects.c:260-322: calls the teleport/redraw path.
    (
        ids::PRESENT_UPDATE_DOORWAY,
        crate::execution::ExecutionClass::TimeAware,
    ),
    // G/src/presents/present_effects.c:325-464: GenerateSpeech/StartDialogue.
    (
        ids::PERFORM_PROMOTION,
        crate::execution::ExecutionClass::TimeAware,
    ),
    (
        ids::BUMMER_LIFE_PROMO,
        crate::execution::ExecutionClass::TimeAware,
    ),
    (
        ids::PRESENT_UPDATE_BURPS,
        crate::execution::ExecutionClass::TimeAware,
    ),
    // G/src/presents/projectile_effects.c:568-612: Player_Youch can speak.
    (
        ids::PRESENT_UPDATE_TERRAIN_EFFECTS,
        crate::execution::ExecutionClass::TimeAware,
    ),
    // G/src/presents/present_dispatch.c:41-188: HUD IRQ waits and speech/dialogue.
    (
        ids::OPEN_SELECTED_PRESENT,
        crate::execution::ExecutionClass::TimeAware,
    ),
    (
        ids::SETUP_SELECTED_PRESENT,
        crate::execution::ExecutionClass::TimeAware,
    ),
    // G/src/presents/telephone.c:72-157: DrawGameMap blocks on VBlank and inputs.
    (
        ids::CHECK_FOR_TELEPHONE,
        crate::execution::ExecutionClass::TimeAware,
    ),
    // G/src/presents/present_dispatch.c:23-38 and item_updates.c:366-393:
    // transitive doorway, telephone, speech and action-icon waits.
    (
        ids::PROCESS_PRESENTS,
        crate::execution::ExecutionClass::TimeAware,
    ),
    (
        ids::MAP_UPDATE_ITEMS_AND_PRESENTS,
        crate::execution::ExecutionClass::TimeAware,
    ),
];

/// G/src/presents/level_item_setup.c:139 (`MapIsLargePlacementTerrainExcluded`).
/// The exclusion bytes are signed char ROM data, not a native copied table.
pub fn map_is_large_placement_terrain_excluded(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let terrain = (arguments[0] as i16) & 0x7f;
    let table = rom::large_item_excluded_terrain_ids(context.revision());
    for entry in 0..23 {
        if i16::from(table.at(entry).read(context)) == terrain {
            return Ok(Some(1));
        }
    }
    Ok(Some(0))
}

/// G/src/presents/item_updates.c:343 (`PresentClearWorldEntityReadiness`).
/// The loaded-bank scan stops at 28, while the dropped-bank scan covers all 32.
pub fn present_clear_world_entity_readiness(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    for bank in 0..2 {
        if ram::g_item_bank_users(revision).at(bank).read(context) != 0 {
            for index in 0..28 {
                let item = ram::g_object_table(revision).at(bank).at(index);
                if item.kind().read(context) != -1
                    && item.kind().read(context) < 64
                    && item.object().read(context) != 0
                {
                    ram::g_player_info(revision)
                        .at(i32::from(item.object().read(context)))
                        .sprite_ready()
                        .write(context, 0);
                }
            }
        }
    }
    for index in 0..32 {
        let item = ram::g_dropped_present_table(revision).at(index);
        if item.kind().read(context) != -1 && item.object().read(context) != 0 {
            ram::g_player_info(revision)
                .at(i32::from(item.object().read(context)))
                .sprite_ready()
                .write(context, 0);
        }
    }
    Ok(None)
}

/// G/src/presents/present_effects.c:125 (`PresentRegisterDecoyHit`).
/// Signed-byte hit counters wrap; the original explicitly sign-extends its short D0.
pub fn present_register_decoy_hit(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let object_number = arguments[0] as i16;
    for player_number in 0..2 {
        if i16::from(
            ram::g_player_decoy_entities(revision)
                .at(player_number)
                .read(context),
        ) == object_number
        {
            let hits = ram::g_player_decoy_hit_counts(revision).at(player_number);
            let incremented = hits.read(context).wrapping_add(1);
            hits.write(context, incremented);
            return Ok(Some(1));
        }
    }
    Ok(Some(0))
}

/// G/src/presents/present_effects.c:205 (`PresentFindDoorwayOwnerAtTile`).
/// Signed world-coordinate shifts happen after promotion, before short assignment.
pub fn present_find_doorway_owner_at_tile(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let player = Address::<ObjectState>::new(arguments[0]);
    let row = i32::from(arguments[1] as i16);
    let column = i32::from(arguments[2] as i16);
    for player_number in 0..2 {
        let object_number = ram::g_player_doorway_entities(revision)
            .at(player_number)
            .read(context);
        if object_number != 0 {
            let doorway = ram::g_player_info(revision).at(i32::from(object_number));
            if player.level().read(context) == doorway.level().read(context)
                && (doorway.sprite_state().read(context) != 0
                    || player.y().read(context) >= doorway.y().read(context))
            {
                let doorway_row = (i32::from(doorway.y().read(context)) - 10) >> 3;
                let doorway_column = (i32::from(doorway.x().read(context)) - 16) >> 3;
                if row >= doorway_row
                    && row < doorway_row + 3
                    && column >= doorway_column
                    && column < doorway_column + 4
                {
                    return Ok(Some((player_number + 1) as u32));
                }
            }
        }
    }
    Ok(Some(0))
}

/// G/src/presents/projectile_effects.c:404 (`PlayerRemapDirectionalInput`).
/// The ROM row has ten entries, but input nibbles 11–15 intentionally read past it.
pub fn player_remap_directional_input(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let player_number = i32::from(arguments[0] as i16);
    let input = ram::g_player_processed_input(revision)
        .at(player_number)
        .read(context);
    if input & 15 == 0 {
        return Ok(Some(u32::from(input)));
    }
    let row = i32::from(
        ram::g_player_control_remap_indices(revision)
            .at(player_number)
            .read(context),
    );
    let direction = rom::control_remap_directions(revision)
        .at(row)
        .at(i32::from(input & 15) - 1)
        .read(context);
    Ok(Some(u32::from((input & 240) | direction)))
}

/// G/src/presents/noop.c:4 (`NoOp`). The original body is exactly RTS.
pub fn no_op(_: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    Ok(None)
}
