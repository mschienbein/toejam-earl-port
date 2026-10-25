use crate::{Context, Result, call, call_void, ids};
use tje_mem::{Address, ObjectState, Ptr, ram, rom};

/// G/src/cutscenes/collision_scene_entities.c:10 (`IntroCreateCollisionSceneEntities`).
/// Atomic: four bounded RAM allocations. The ORIGINAL BUG uses entry A3 if the
/// parent allocation fails but an attached allocation succeeds (matching listing
/// c_collision_scene_entities.s:17,81); no fabricated parent pointer is supplied.
pub fn intro_create_collision_scene_entities(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let mut parent: Address<ObjectState> = Address::new(context.address_register(3));
    let handle = call(context, ids::ALLOCATE_ENTITY, &[])? as u8;
    ram::g_intro_collision_parent_entity(revision).write(context, handle);
    if handle != 0 {
        parent = ram::g_player_info(revision).at(i32::from(handle));
        let tile = parent.tile().read(context);
        parent.tile().write(context, tile | 0x8000);
        let frame = rom::spr_intro_collision(revision).at(0).read(context);
        parent.current_frame().write(context, frame);
        parent.frames().write(
            context,
            Ptr::new(rom::spr_intro_collision(revision).address()),
        );
        parent.tile_count().write(context, 160);
        parent.sprite_count().write(context, 12);
        parent.x().write(context, 16);
        parent.y().write(context, 97);
        parent.height().write(context, 1);
        parent.sequence().write(
            context,
            Ptr::new(rom::intro_held_frame_streams(revision).at(1).address()),
        );
        parent.frame_duration().write(context, 1);
        parent.frame().write(context, -1);
        parent.frame_phase().write(context, 6);
        parent.sprite_ready().write(context, 0);
        super::allocate_viewport_with_d4(
            context,
            parent.address(),
            ram::g_intro_collision_parent_entity(revision).address() as u16,
        )?;
    }
    for index in 0..3 {
        let handle = call(context, ids::ALLOCATE_ENTITY, &[])? as u8;
        ram::g_intro_collision_attached_entities(revision)
            .at(index)
            .write(context, handle);
        if handle != 0 {
            let object = ram::g_player_info(revision).at(i32::from(handle));
            let tile = object.tile().read(context);
            object.tile().write(context, tile | 0x8000);
            let frame = rom::spr_intro_collision(revision).at(0).read(context);
            object.current_frame().write(context, frame);
            object.frames().write(
                context,
                Ptr::new(rom::spr_intro_collision(revision).address()),
            );
            object.tile_count().write(context, 160);
            object.sprite_count().write(context, 12);
            if index == 0 {
                object.x().write(context, 16);
                object.y().write(context, 208);
            } else if index == 1 {
                object.x().write(context, 176);
                object.y().write(context, 96);
            } else {
                object.x().write(context, 176);
                object.y().write(context, 208);
            }
            let y = object.y().read(context);
            object.y().write(context, y.wrapping_add(1));
            object.height().write(context, 1);
            object.sequence().write(
                context,
                Ptr::new(rom::intro_held_frame_streams(revision).at(1).address()),
            );
            object.frame_duration().write(context, 1);
            object.frame().write(context, -1);
            object.frame_phase().write(context, 6);
            let flags = object.behavior_flags().read(context);
            object.behavior_flags().write(context, flags | 4);
            let tile = parent.tile().read(context);
            object.tile().write(context, tile);
            let owner = ram::g_intro_collision_parent_entity(revision).read(context);
            object.owner().write(context, owner as i8);
            object.sprite_ready().write(context, 0);
            // Matching caller retains &gIntroCollisionParentEntity in D4.
            super::allocate_viewport_with_d4(
                context,
                object.address(),
                ram::g_intro_collision_parent_entity(revision).address() as u16,
            )?;
        }
    }
    Ok(None)
}

/// G/src/cutscenes/collision_scene_entities.c:68 (`IntroReleaseCollisionSceneEntities`).
/// Atomic: parent, then auxiliary/attached pairs are freed and cleared in order.
pub fn intro_release_collision_scene_entities(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let parent = ram::g_intro_collision_parent_entity(revision);
    if parent.read(context) != 0 {
        let handle = parent.read(context);
        call_void(context, ids::FREE_ENTITY, &[u32::from(handle)])?;
    }
    parent.write(context, 0);
    for index in 0..3 {
        let auxiliary = ram::g_intro_collision_auxiliary_entities(revision).at(index);
        if auxiliary.read(context) != 0 {
            let handle = auxiliary.read(context);
            call_void(context, ids::FREE_ENTITY, &[u32::from(handle)])?;
        }
        auxiliary.write(context, 0);
        let attached = ram::g_intro_collision_attached_entities(revision).at(index);
        if attached.read(context) != 0 {
            let handle = attached.read(context);
            call_void(context, ids::FREE_ENTITY, &[u32::from(handle)])?;
        }
        attached.write(context, 0);
    }
    Ok(None)
}
