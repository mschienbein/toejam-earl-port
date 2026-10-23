use crate::{Context, Result, call, ids};
use tje_mem::{Ptr, ram, rom};

/// G/src/cutscenes/rocketship_allocate.c:10 (`CutsceneAllocateRocketshipPair`).
/// Atomic: two pool allocations followed by RAM stores; no viewport allocation.
/// Allocation failure deliberately still initializes record zero.
pub fn cutscene_allocate_rocketship_pair(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let handle = call(context, ids::ALLOCATE_ENTITY, &[])? as u8;
    ram::g_cutscene_rocketship_entity(revision).write(context, handle);
    let object = ram::g_player_info(revision).at(i32::from(handle));
    object.tile_count().write(context, 101);
    object.sprite_count().write(context, 8);
    object.tile().write(context, 0);
    let frame = rom::spr_intro_rocketship_dodging(revision)
        .at(0)
        .read(context);
    object.current_frame().write(context, frame);
    object.sprite_ready().write(context, 0);
    object.sequence().write(
        context,
        Ptr::new(
            rom::intro_held_frame_streams(revision)
                .at(0)
                .frame()
                .address(),
        ),
    );
    object.frames().write(
        context,
        Ptr::new(rom::spr_intro_rocketship_dodging(revision).address()),
    );
    object.frame_phase().write(context, 1);
    object.frame_duration().write(context, 1);
    object.frame().write(context, -1);
    let handle = call(context, ids::ALLOCATE_ENTITY, &[])? as u8;
    ram::g_cutscene_rocketship_exhaust_entity(revision).write(context, handle);
    let object = ram::g_player_info(revision).at(i32::from(handle));
    object.tile_count().write(context, 15);
    object.sprite_count().write(context, 2);
    object.tile().write(context, 0);
    let frame = rom::intro_frame_index_lists(revision).at(119).read(context) as u8;
    let frame = rom::spr_rocketship_exhaust(revision)
        .at(i32::from(frame))
        .read(context);
    object.current_frame().write(context, frame);
    object.sprite_ready().write(context, 0);
    let sequence = rom::intro_animation_pointers(revision).at(0).read(context);
    object.sequence().write(context, sequence);
    object.frames().write(
        context,
        Ptr::new(rom::spr_rocketship_exhaust(revision).address()),
    );
    object.frame_phase().write(context, 6);
    object.frame_duration().write(context, 1);
    object.frame().write(context, -1);
    Ok(None)
}
