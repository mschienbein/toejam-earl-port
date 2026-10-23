/// G/build/{rev02,rev00}/asm/c_terrain_movement.s:424–454 uses signed
/// BGE/BLT even though the dimension byte was zero-extended before arithmetic.
fn x_within_bounds(position: i16, width: u8) -> bool {
    i32::from(position) - i32::from(width) > 8 && i32::from(width) + i32::from(position) < 2672
}

fn y_within_bounds(position: i16, height: u8) -> bool {
    i32::from(position) - i32::from(height) > 8 && position < 1776
}

use crate::{Context, Result, call, ids};
use tje_mem::{Address, ObjectState, RenderFrame, ram};

/// G/src/earthlings/terrain_movement.c:20 (`EarthlingProbeDirectionalTerrain`).
/// Atomic ROM/RAM terrain lookup; narrow each updated argument before calling.
pub fn earthling_probe_directional_terrain(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let player = arguments[0] as i16;
    let mut x = arguments[1] as i16;
    let mut y = arguments[2] as i16;
    match arguments[3] as i16 {
        4 => y = y.wrapping_sub(8),
        5 => y = y.wrapping_add(8),
        6 => x = x.wrapping_sub(16),
        7 => x = x.wrapping_add(16),
        _ => {}
    }
    Ok(Some(u32::from(call(
        context,
        ids::GET_MAP_TERRAIN_AT_WORLD_POSITION,
        &[player as u32, x as u32, y as u32],
    )? as u8)))
}

fn map_player(context: &dyn Context, object: Address<ObjectState>) -> Option<u32> {
    let revision = context.revision();
    if object.level().read(context) == ram::g_player1_map_level(revision).read(context) {
        Some(0)
    } else if object.level().read(context) == ram::g_player2_map_level(revision).read(context) {
        Some(1)
    } else {
        None
    }
}

fn predict(
    context: &dyn Context,
    step: Address<RenderFrame>,
    object: Address<ObjectState>,
) -> (i16, u8, i16, u8) {
    let (x, fx) = if object.animation_motion_axes().read(context) & 1 != 0 {
        let distance = (i32::from(object.velocity_x().read(context))
            * i32::from(step.motion_x().read(context))) as i16;
        let fixed = i32::from(object.fraction_x().read(context))
            + (i32::from(object.x().read(context)) << 8)
            + i32::from(distance);
        ((fixed >> 8) as i16, fixed as u8)
    } else {
        (object.x().read(context), object.fraction_x().read(context))
    };
    let (y, fy) = if object.animation_motion_axes().read(context) & 2 != 0 {
        let distance = (i32::from(object.velocity_y().read(context))
            * i32::from(step.motion_y().read(context))) as i16;
        let fixed = i32::from(object.fraction_y().read(context))
            + (i32::from(object.y().read(context)) << 8)
            + i32::from(distance);
        ((fixed >> 8) as i16, fixed as u8)
    } else {
        (object.y().read(context), object.fraction_y().read(context))
    };
    (x, fx, y, fy)
}

/// G/src/earthlings/terrain_movement.c:35 (`EarthlingMoveOnAllowedTerrain`).
/// Atomic: preserve the signed-word distance truncation and int terrain result view.
pub fn earthling_move_on_allowed_terrain(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let step = Address::<RenderFrame>::new(arguments[0]);
    let object = Address::<ObjectState>::new(arguments[1]);
    let earthling = object.earthling().read(context).target();
    if call(
        context,
        ids::EARTHLING_IS_UPDATE_BLOCKED,
        &[earthling.address()],
    )? != 0
        || earthling.state().read(context) == 99
    {
        return Ok(None);
    }
    let Some(player) = map_player(context, object) else {
        return Ok(None);
    };
    let (x, fx, y, fy) = predict(context, step, object);
    if x == object.x().read(context) && y == object.y().read(context) {
        return Ok(None);
    }
    let kind = earthling.kind().read(context);
    if [5, 10, 11, 17].contains(&kind) {
        let old_x = object.x().read(context) as u32;
        let old_y = object.y().read(context) as u32;
        if call(
            context,
            ids::ENTITY_FIND_NARROW_COLLISION_AT_POSITION,
            &[object.address(), old_x, old_y, 0],
        )? == 0
            && call(
                context,
                ids::ENTITY_FIND_NARROW_COLLISION_AT_POSITION,
                &[object.address(), x as u32, y as u32, 0],
            )? != 0
        {
            object.sprite_state().write(context, 0);
            return Ok(None);
        }
    } else if kind != 18 {
        let old_x = object.x().read(context) as u32;
        let old_y = object.y().read(context) as u32;
        if call(
            context,
            ids::ENTITY_FIND_MENU_AWARE_COLLISION_AT_POSITION,
            &[object.address(), old_x, old_y],
        )? == 0
        {
            let collision = call(
                context,
                ids::ENTITY_FIND_MENU_AWARE_COLLISION_AT_POSITION,
                &[object.address(), x as u32, y as u32],
            )? as i16;
            if collision != 0 {
                object.sprite_state().write(context, 0);
                call(
                    context,
                    ids::PRESENT_REGISTER_DECOY_HIT,
                    &[(i32::from(collision) - 1) as u32],
                )?;
                return Ok(None);
            }
        }
    }
    let terrain = if kind == 14 || kind == 18 {
        let state = earthling.state().read(context) as u32;
        call(
            context,
            ids::EARTHLING_PROBE_DIRECTIONAL_TERRAIN,
            &[player, x as u32, y as u32, state],
        )? as i16
    } else {
        call(
            context,
            ids::GET_MAP_TERRAIN_AT_WORLD_POSITION,
            &[player, x as u32, y as u32],
        )? as i16
    };
    if (kind == 13 && terrain == 0x10)
        || (kind == 14 && terrain == 0x80)
        || (kind != 13 && kind != 14 && (terrain as u16) < 0x50)
    {
        object.x().write(context, x);
        object.fraction_x().write(context, fx);
        object.y().write(context, y);
        object.fraction_y().write(context, fy);
    } else {
        object.sprite_state().write(context, 0);
    }
    Ok(None)
}

/// G/src/earthlings/terrain_movement.c:122 (`EntityUpdateTerrainDrawFlag`).
/// Atomic: preserve the original consumer's full int terrain result comparison.
pub fn entity_update_terrain_draw_flag(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let object = Address::<ObjectState>::new(arguments[0]);
    let Some(player) = map_player(context, object) else {
        return Ok(None);
    };
    let x = object.x().read(context) as u32;
    let y = object.y().read(context) as u32;
    let terrain = call(
        context,
        ids::GET_MAP_TERRAIN_AT_WORLD_POSITION,
        &[player, x, y],
    )?;
    let flags = object.draw_flags().read(context);
    object.draw_flags().write(
        context,
        if terrain == 0x90 {
            flags & !0x40
        } else {
            flags | 0x40
        },
    );
    Ok(None)
}

/// G/src/earthlings/terrain_movement.c:140 (`EarthlingMoveWithinMapBounds`).
/// Atomic: collision/terrain memory only. Bounds use the emitted signed branches,
/// not the unsigned comparisons suggested by the source's dimension-byte types.
pub fn earthling_move_within_map_bounds(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let step = Address::<RenderFrame>::new(arguments[0]);
    let object = Address::<ObjectState>::new(arguments[1]);
    let earthling = object.earthling().read(context).target();
    if call(
        context,
        ids::EARTHLING_IS_UPDATE_BLOCKED,
        &[earthling.address()],
    )? != 0
        || earthling.state().read(context) == 99
        || object.behavior_flags().read(context) & 0x40 != 0
        || (earthling.kind().read(context) == 7 && earthling.state().read(context) == 3)
    {
        return Ok(None);
    }
    let Some(player) = map_player(context, object) else {
        return Ok(None);
    };
    let (mut x, fx, mut y, fy) = predict(context, step, object);
    if !x_within_bounds(x, object.width().read(context)) {
        x = object.x().read(context);
    }
    if !y_within_bounds(y, object.height_pixels().read(context)) {
        y = object.y().read(context);
    }
    let old_x = object.x().read(context) as u32;
    let old_y = object.y().read(context) as u32;
    if call(
        context,
        ids::ENTITY_FIND_MENU_AWARE_COLLISION_AT_POSITION,
        &[object.address(), old_x, old_y],
    )? == 0
    {
        let collision = call(
            context,
            ids::ENTITY_FIND_MENU_AWARE_COLLISION_AT_POSITION,
            &[object.address(), x as u32, y as u32],
        )? as i16;
        if collision != 0 {
            object.sprite_state().write(context, 0);
            call(
                context,
                ids::PRESENT_REGISTER_DECOY_HIT,
                &[(i32::from(collision) - 1) as u32],
            )?;
            return Ok(None);
        }
    }
    let terrain = call(
        context,
        ids::GET_MAP_TERRAIN_AT_WORLD_POSITION,
        &[player, x as u32, y as u32],
    )? as i16;
    let kind = earthling.kind().read(context);
    if kind == 19 {
        if ram::g_terrain_query_metatile_word(revision).read(context) & 0x7f == 34 {
            return Ok(None);
        }
        earthling.saved_x_or_status().write(context, terrain);
    } else if terrain == 0x90 {
        if kind == 6 || kind == 22 {
            let flags = object.behavior_flags().read(context);
            object.behavior_flags().write(context, flags | 0x20);
        } else {
            let flags = object.draw_flags().read(context);
            object.draw_flags().write(context, flags & !0x40);
        }
    } else if kind == 6 || kind == 22 {
        let flags = object.behavior_flags().read(context);
        object.behavior_flags().write(context, flags & !0x20);
    } else {
        let flags = object.draw_flags().read(context);
        object.draw_flags().write(context, flags | 0x40);
    }
    object.x().write(context, x);
    object.fraction_x().write(context, fx);
    object.y().write(context, y);
    object.fraction_y().write(context, fy);
    Ok(None)
}

#[cfg(test)]
mod signed_bounds_tests {
    use super::{x_within_bounds, y_within_bounds};

    fn cmp_long_bge(left: i32, right: i32) -> bool {
        let result = left.wrapping_sub(right);
        let overflow = ((left ^ right) & (left ^ result)) < 0;
        (result < 0) == overflow
    }

    fn cmp_word_blt(left: i16, right: i16) -> bool {
        let result = left.wrapping_sub(right);
        let overflow = ((left ^ right) & (left ^ result)) < 0;
        (result < 0) != overflow
    }

    #[test]
    fn every_coordinate_dimension_pair_matches_emitted_signed_flags() {
        for revision in [tje_mem::Revision::Rev02, tje_mem::Revision::Rev00] {
            for position in i16::MIN..=i16::MAX {
                for dimension in u8::MIN..=u8::MAX {
                    let difference = i32::from(position) - i32::from(dimension);
                    let sum = i32::from(position) + i32::from(dimension);
                    let lower_reject = cmp_long_bge(8, difference);
                    let upper_accept = !cmp_long_bge(sum, 2672);
                    assert_eq!(
                        x_within_bounds(position, dimension),
                        !lower_reject && upper_accept,
                        "{revision:?} x={position} width={dimension}"
                    );
                    assert_eq!(
                        y_within_bounds(position, dimension),
                        !lower_reject && cmp_word_blt(position, 1776),
                        "{revision:?} y={position} height={dimension}"
                    );
                }
            }
        }
    }
}
