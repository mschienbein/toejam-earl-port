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

/// Reviewed leaf entries: bounded storage accesses only, with no callees or waits.
pub static ATOMIC_CAPABLE: &[crate::FunctionId] = &[
    ids::MAP_IS_LARGE_PLACEMENT_TERRAIN_EXCLUDED,
    ids::PRESENT_CLEAR_WORLD_ENTITY_READINESS,
    ids::PRESENT_REGISTER_DECOY_HIT,
    ids::PRESENT_FIND_DOORWAY_OWNER_AT_TILE,
    ids::PLAYER_REMAP_DIRECTIONAL_INPUT,
    ids::NO_OP,
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
