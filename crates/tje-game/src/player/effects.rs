use crate::{Context, Result, call, ids};
use tje_mem::{ram, rom};

/// G/src/player/effect_slots.c:26 (`EntitySpawnTimedEffect`).
/// Preserve byte-slot narrowing of the compiler's int-view allocation result.
pub(super) fn entity_spawn_timed_effect(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let slots = ram::g_timed_effect_entities(revision);
    let slot = if slots.at(0).read(context) == 0 {
        0
    } else if slots.at(1).read(context) == 0 {
        1
    } else {
        return Ok(Some(1));
    };
    let allocated = call(context, ids::ALLOCATE_ENTITY, &[])? as i8;
    slots.at(slot).write(context, allocated);
    if allocated == 0 {
        return Ok(Some(1));
    }
    let effect = ram::g_player_info(revision).at(i32::from(slots.at(slot).read(context)));
    effect.x().write(context, arguments[0] as i16);
    effect.y().write(context, arguments[1] as i16);
    effect.height().write(context, 5);
    effect.level().write(context, arguments[2] as i8);
    effect.width().write(context, 32);
    effect.height_pixels().write(context, 32);
    effect.sprite_count().write(context, 1);
    effect.tile_count().write(context, 16);
    effect
        .tile()
        .write(context, effect.tile().read(context) | 0x8000);
    let animation = rom::timed_entity_effect_animation(revision);
    effect
        .sequence()
        .write(context, animation.sequence().read(context));
    effect
        .frames()
        .write(context, animation.frames().read(context));
    effect
        .frame_phase()
        .write(context, animation.period().read(context));
    effect.frame_duration().write(context, 1);
    effect.frame().write(context, -1);
    call(context, ids::PLAY_SOUND, &[14, 50])?;
    Ok(Some(0))
}

/// G/src/player/effect_slots.c:63 (`EntityReleaseFinishedTimedEffect`).
pub(super) fn entity_release_finished_timed_effect(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let slot = ram::g_timed_effect_entities(revision).at(i32::from(arguments[0] as i16));
    if slot.read(context) != 0
        && ram::g_player_info(revision)
            .at(i32::from(slot.read(context)))
            .frame_duration()
            .read(context)
            == 0
    {
        let entity = slot.read(context) as u32;
        call(context, ids::FREE_ENTITY, &[entity])?;
        slot.write(context, 0);
    }
    Ok(None)
}
