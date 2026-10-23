use crate::{Context, Result, call, ids};
use tje_mem::ram;

/// G/src/player/process_movement.c:17 (`ProcessMovement`).
/// The emitted bound comparisons are signed (BGE/BLT), including the
/// zero-extended sprite dimensions; retain the observed compiler lowering.
pub(super) fn process_movement(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let mut pointer = ram::g_entity_draw_list_head(revision).read(context);
    while pointer.address() != 0 {
        let entity = pointer.target();
        let earthling = entity.earthling().read(context).address();
        if entity.draw_flags().read(context) & 1 != 0
            && entity.draw_flags().read(context) & 8 == 0
            && call(context, ids::EARTHLING_IS_UPDATE_BLOCKED, &[earthling])? == 0
        {
            let (mut x, mut fx) = if entity.animation_motion_axes().read(context) & 1 == 0
                && (entity.velocity_x().read(context) != 0
                    || entity.acceleration_x().read(context) != 0)
            {
                entity.velocity_x().write(
                    context,
                    entity
                        .velocity_x()
                        .read(context)
                        .wrapping_add(entity.acceleration_x().read(context)),
                );
                let fixed = (i32::from(entity.x().read(context)) << 8)
                    .wrapping_add(i32::from(entity.fraction_x().read(context)))
                    .wrapping_add(i32::from(entity.velocity_x().read(context)));
                entity
                    .draw_flags()
                    .write(context, entity.draw_flags().read(context) & !0x10);
                ((fixed >> 8) as i16, fixed as u8)
            } else {
                (entity.x().read(context), entity.fraction_x().read(context))
            };
            let (mut y, mut fy) = if entity.animation_motion_axes().read(context) & 2 == 0
                && (entity.velocity_y().read(context) != 0
                    || entity.acceleration_y().read(context) != 0)
            {
                entity.velocity_y().write(
                    context,
                    entity
                        .velocity_y()
                        .read(context)
                        .wrapping_add(entity.acceleration_y().read(context)),
                );
                let fixed = (i32::from(entity.y().read(context)) << 8)
                    .wrapping_add(i32::from(entity.fraction_y().read(context)))
                    .wrapping_add(i32::from(entity.velocity_y().read(context)));
                entity
                    .draw_flags()
                    .write(context, entity.draw_flags().read(context) & !0x20);
                ((fixed >> 8) as i16, fixed as u8)
            } else {
                (entity.y().read(context), entity.fraction_y().read(context))
            };
            let (mut z, mut fz) = if entity.velocity_height().read(context) != 0
                || entity.acceleration_height().read(context) != 0
            {
                entity.velocity_height().write(
                    context,
                    entity
                        .velocity_height()
                        .read(context)
                        .wrapping_add(entity.acceleration_height().read(context)),
                );
                let fixed = (i32::from(entity.height().read(context)) << 8)
                    .wrapping_add(i32::from(entity.fraction_height().read(context)))
                    .wrapping_add(i32::from(entity.velocity_height().read(context)));
                entity
                    .draw_flags()
                    .write(context, entity.draw_flags().read(context) & !0x20);
                ((fixed >> 8) as i16, fixed as u8)
            } else {
                (
                    entity.height().read(context),
                    entity.fraction_height().read(context),
                )
            };
            if entity.entity_index().read(context) <= 1 {
                if i32::from(x) - i32::from(entity.width().read(context)) <= 8
                    || i32::from(entity.width().read(context)) + i32::from(x) >= 2652
                {
                    x = entity.x().read(context);
                }
                if i32::from(y) - i32::from(entity.height_pixels().read(context)) - i32::from(z)
                    <= 8
                    && !matches!(entity.sprite_state().read(context), 5 | 6)
                    && entity.animation_group_mask().read(context) & 0x20 == 0
                {
                    y = entity.y().read(context);
                    z = entity.height().read(context);
                } else if y >= 1736 {
                    y = 1735;
                }
            }
            if entity.entity_index().read(context) <= 1
                && entity.animation_motion_axes().read(context) == 0
            {
                let args = [
                    entity.address(),
                    entity.x().read(context) as u32,
                    entity.y().read(context) as u32,
                    entity.height().read(context) as u32,
                    0,
                ];
                if call(
                    context,
                    ids::ENTITY_FIND_VOLUME_COLLISION_AT_POSITION,
                    &args,
                )? == 0
                {
                    if entity.animation_group_mask().read(context) & 0x800_0000 != 0
                        && call(
                            context,
                            ids::PLAYER_CHECK_TERRAIN_EDGE_HOP,
                            &[entity.address(), x as u32, y as u32],
                        )? != 0
                    {
                        z = 10;
                    }
                    if x != entity.x().read(context) {
                        let args = [
                            entity.address(),
                            x as u32,
                            entity.y().read(context) as u32,
                            entity.height().read(context) as u32,
                            0,
                        ];
                        if call(
                            context,
                            ids::ENTITY_FIND_VOLUME_COLLISION_AT_POSITION,
                            &args,
                        )? != 0
                        {
                            x = entity.x().read(context);
                            fx = entity.fraction_x().read(context);
                            entity
                                .draw_flags()
                                .write(context, entity.draw_flags().read(context) | 0x10);
                        }
                    }
                    if y != entity.y().read(context) {
                        let args = [
                            entity.address(),
                            x as u32,
                            y as u32,
                            entity.height().read(context) as u32,
                            0,
                        ];
                        if call(
                            context,
                            ids::ENTITY_FIND_VOLUME_COLLISION_AT_POSITION,
                            &args,
                        )? != 0
                        {
                            y = entity.y().read(context);
                            fy = entity.fraction_y().read(context);
                            entity
                                .draw_flags()
                                .write(context, entity.draw_flags().read(context) | 0x20);
                        }
                    }
                    if z < entity.height().read(context)
                        && call(
                            context,
                            ids::ENTITY_FIND_VOLUME_COLLISION_AT_POSITION,
                            &[entity.address(), x as u32, y as u32, z as u32, 2],
                        )? != 0
                    {
                        z = entity.height().read(context);
                        fz = entity.fraction_height().read(context);
                        entity
                            .draw_flags()
                            .write(context, entity.draw_flags().read(context) | 0x20);
                        if entity.animation_group_mask().read(context) & 0x20 != 0 {
                            entity.velocity_height().write(context, 0x300);
                            entity.acceleration_height().write(context, -64);
                            y = y.wrapping_add(4);
                        }
                    }
                } else if call(
                    context,
                    ids::ELEVATOR_BLOCKS_ENTITY_MOVEMENT,
                    &[entity.address(), x as u32, y as u32],
                )? != 0
                {
                    y = entity.y().read(context);
                    fy = entity.fraction_y().read(context);
                    entity
                        .draw_flags()
                        .write(context, entity.draw_flags().read(context) | 0x20);
                }
            }
            entity.x().write(context, x);
            entity.fraction_x().write(context, fx);
            entity.y().write(context, y);
            entity.fraction_y().write(context, fy);
            entity.height().write(context, z);
            entity.fraction_height().write(context, fz);
        }
        pointer = entity.next().read(context);
    }
    Ok(None)
}
