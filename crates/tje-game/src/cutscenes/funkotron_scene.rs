use crate::{Context, Result, call, ids};
use tje_mem::{ram, rom};

/// G/src/cutscenes/funkotron_scene.c:15 (`FunkatronCreateSceneEntity`).
/// Atomic: one pool allocation and ROM-descriptor-driven RAM initialization.
pub fn funkatron_create_scene_entity(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let placement_number = arguments[0] as i16;
    let descriptor_number = arguments[1] as i16;
    let object_number = call(context, ids::ALLOCATE_ENTITY, &[])? as i16;
    if object_number != 0 {
        ram::g_funkatron_placement_entities(revision)
            .at(i32::from(placement_number))
            .write(context, object_number as i8);
        let object = ram::g_player_info(revision)
            .byte_offset(i32::from(object_number.wrapping_mul(128)))
            .cast::<tje_mem::ObjectState>();
        let descriptor = rom::funkatron_scene_descriptors(revision)
            .byte_offset(i32::from(descriptor_number.wrapping_mul(18)))
            .cast::<tje_mem::SceneDescriptor>();
        let frames = descriptor.frames().read(context);
        object.frames().write(context, frames);
        let sequence = descriptor.idle_sequence().read(context);
        object.sequence().write(context, sequence);
        let phase = descriptor.frame_phase().read(context);
        object.frame_phase().write(context, phase);
        let width = descriptor.width().read(context);
        object.width().write(context, width);
        let height = descriptor.height().read(context);
        object.height_pixels().write(context, height);
        let sprites = descriptor.sprite_count().read(context);
        object.sprite_count().write(context, sprites as u8);
        let tiles = descriptor.tile_count().read(context);
        object.tile_count().write(context, tiles as u8);
        object.frame_duration().write(context, 1);
        object.frame().write(context, -1);
        let placement = rom::funkatron_scene_placements(revision)
            .byte_offset(i32::from(placement_number.wrapping_mul(6)))
            .cast::<tje_mem::ScenePlacement>();
        let x = placement.x().read(context);
        object.x().write(context, x);
        let y = placement.y().read(context);
        object.y().write(context, y);
        object.level().write(context, 26);
        let flags = object.draw_flags().read(context);
        object.draw_flags().write(context, flags | 8);
        if descriptor_number == 6 {
            let flags = object.draw_flags().read(context);
            object.draw_flags().write(context, flags | 0x80);
        } else if descriptor_number != 13 {
            let flags = object.behavior_flags().read(context);
            object.behavior_flags().write(context, flags | 1);
        }
    }
    Ok(None)
}

/// G/src/cutscenes/funkotron_scene.c:49 (`FunkatronUpdateSceneEntities`).
/// TimeAware: StartDialogue can reach GenerateSpeech's IRQ-cleared upload wait;
/// the visibility-only paths do not justify a weaker whole-function contract.
pub fn funkatron_update_scene_entities(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let mut descriptor_number = 0_i16;
    let mut placement_number = 0_i16;
    let mut descriptor = rom::funkatron_scene_descriptors(revision).at(0);
    while descriptor_number < 30 {
        let placement = rom::funkatron_scene_placements(revision)
            .byte_offset(i32::from(placement_number.wrapping_mul(6)))
            .cast::<tje_mem::ScenePlacement>();
        if placement.x().read(context) == 0 {
            descriptor_number = descriptor_number.wrapping_add(1);
            descriptor = descriptor.byte_offset(18);
            placement_number = placement_number.wrapping_add(1);
            continue;
        }
        if i16::from(ram::g_funkatron_update_phase(revision).read(context)) != descriptor_number & 3
            || (descriptor_number == 28 && ram::g_player_selection(revision).read(context) != 1)
            || (descriptor_number == 29 && ram::g_player_selection(revision).read(context) != 0)
        {
            placement_number = placement_number.wrapping_add(1);
            continue;
        }
        let mut half_width = (u32::from(descriptor.width().read(context)) / 2) as i16;
        half_width = half_width.wrapping_add(5);
        let mut maximum_y = i16::from(descriptor.height().read(context));
        maximum_y = maximum_y.wrapping_add(224);
        let mut screen_x = placement.x().read(context);
        screen_x = screen_x.wrapping_sub(ram::g_camera1_pending_x(revision).read(context));
        let mut screen_y = placement.y().read(context);
        screen_y = screen_y.wrapping_sub(ram::g_camera1_pending_y(revision).read(context));
        let slot = ram::g_funkatron_placement_entities(revision).at(i32::from(placement_number));
        let mut object_number = i16::from(slot.read(context));
        if i32::from(screen_x) + i32::from(half_width) > 0
            && i32::from(screen_x) < i32::from(half_width) + 320
            && i32::from(screen_y) + 5 > 0
            && screen_y < maximum_y
        {
            if object_number == 0 {
                call(
                    context,
                    ids::FUNKATRON_CREATE_SCENE_ENTITY,
                    &[
                        placement_number as i32 as u32,
                        descriptor_number as i32 as u32,
                    ],
                )?;
                object_number = i16::from(slot.read(context));
            }
            if descriptor_number >= 15 && object_number != 0 {
                let object = ram::g_player_info(revision)
                    .byte_offset(i32::from(object_number.wrapping_mul(128)))
                    .cast::<tje_mem::ObjectState>();
                let first = ram::g_player_info(revision).at(0).address();
                let second = ram::g_player_info(revision).at(1).address();
                if call(
                    context,
                    ids::ENTITY_IS_WITHIN_RANGE,
                    &[object.address(), first, 80, 50],
                )? != 0
                    || call(
                        context,
                        ids::ENTITY_IS_WITHIN_RANGE,
                        &[object.address(), second, 80, 50],
                    )? != 0
                {
                    if object.sprite_state().read(context) == 0 {
                        let sequence = descriptor.interaction_sequence().read(context);
                        object.sequence().write(context, sequence);
                        object.frame().write(context, -1);
                        object.sprite_state().write(context, 1);
                    }
                    let mut player_number = 0;
                    if object.attachment_x_or_latch().read(context) == 0 {
                        let interaction = placement.interaction().read(context);
                        if interaction != 0 {
                            let nearby = if call(
                                context,
                                ids::ENTITY_IS_WITHIN_RANGE,
                                &[object.address(), first, 30, 20],
                            )? != 0
                            {
                                true
                            } else if call(
                                context,
                                ids::ENTITY_IS_WITHIN_RANGE,
                                &[object.address(), second, 30, 20],
                            )? != 0
                            {
                                player_number = 1;
                                true
                            } else {
                                false
                            };
                            if nearby {
                                if !((interaction == 31 && player_number == 0)
                                    || (interaction == 53 && player_number == 0)
                                    || (interaction == 54 && player_number == 1))
                                {
                                    let speaker =
                                        i32::from(object.entity_index().read(context)) as u32;
                                    if call(
                                        context,
                                        ids::START_DIALOGUE,
                                        &[interaction as i32 as u32, speaker],
                                    )? as i32
                                        >= 0
                                    {
                                        object.attachment_x_or_latch().write(context, 1);
                                    }
                                }
                                if (interaction == 31 && player_number == 1)
                                    || (interaction == 54 && player_number == 0)
                                {
                                    call(context, ids::DRAW_PRESENT_DISPLAY_PANEL, &[])?;
                                }
                            }
                        }
                    }
                } else if object.sprite_state().read(context) != 0 {
                    let sequence = descriptor.idle_sequence().read(context);
                    object.sequence().write(context, sequence);
                    object.frame().write(context, -1);
                    object.sprite_state().write(context, 0);
                }
            }
        } else if object_number != 0 {
            call(context, ids::FREE_ENTITY, &[object_number as i32 as u32])?;
            slot.write(context, 0);
        }
        placement_number = placement_number.wrapping_add(1);
    }
    let phase = ram::g_funkatron_update_phase(revision);
    let current = phase.read(context);
    phase.write(context, ((i32::from(current) + 1) & 3) as i8);
    Ok(None)
}
