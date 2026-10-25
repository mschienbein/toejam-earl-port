use crate::{Context, Result, call, call_void, ids};
use tje_mem::{Address, ObjectState, ram, rom};

/// G/src/earthlings/mole_santa.c:33 (`EarthlingSpawnMoleEffect`).
/// Atomic: six-slot scan, object allocation and ROM animation descriptor reads.
pub fn earthling_spawn_mole_effect(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let mole = Address::<ObjectState>::new(arguments[1]);
    let slots = ram::g_mole_effect_entities(revision);
    for slot in 0..6 {
        if slots.at(slot).read(context) == 0 {
            let index = call(context, ids::ALLOCATE_ENTITY, &[])? as i8;
            slots.at(slot).write(context, index);
            if index != 0 {
                let effect =
                    ram::g_player_info(revision).at(i32::from(slots.at(slot).read(context)));
                effect.x().write(context, mole.x().read(context));
                effect.y().write(context, mole.y().read(context));
                effect.level().write(context, mole.level().read(context));
                effect.width().write(context, 16);
                effect.height_pixels().write(context, 8);
                effect.sprite_count().write(context, 1);
                effect.tile_count().write(context, 8);
                effect.tile().write(context, 0x8000);
                let animation = rom::mole_effect_animations(revision).at(0);
                effect
                    .sequence()
                    .write(context, animation.sequence().read(context));
                effect
                    .frames()
                    .write(context, animation.frames().read(context));
                effect.frame_phase().write(context, 6);
                effect.frame().write(context, -1);
                effect.frame_duration().write(context, 1);
                let flags = effect.draw_flags().read(context);
                effect.draw_flags().write(context, flags | 0x80);
            }
            return Ok(None);
        }
    }
    Ok(None)
}

/// G/src/earthlings/mole_santa.c:68 (`ReleaseFinishedMoleEffectsAndStolenPresents`).
/// Atomic: bounded RAM effect lists and object releases, no device progress.
pub fn release_finished_mole_effects_and_stolen_presents(
    context: &mut dyn Context,
    _arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    for slot in 0..6 {
        let entry = ram::g_mole_effect_entities(revision).at(slot);
        let object = entry.read(context);
        if object != 0
            && ram::g_player_info(revision)
                .at(i32::from(object))
                .frame_duration()
                .read(context)
                == 0
        {
            call_void(context, ids::FREE_ENTITY, &[object as u32])?;
            entry.write(context, 0);
        }
    }
    for slot in 0..2 {
        let entry = ram::g_player_stolen_present_entities(revision).at(slot);
        let object = entry.read(context);
        if object != 0
            && ram::g_player_info(revision)
                .at(i32::from(object))
                .height()
                .read(context)
                <= 0
        {
            call_void(context, ids::FREE_ENTITY, &[object as u32])?;
            entry.write(context, 0);
        }
    }
    Ok(None)
}

/// G/src/earthlings/mole_santa.c:205 (`DropPresentsAbility`).
/// Atomic: preserve the 25-byte land cache and coordinates across each drop.
pub fn drop_presents_ability(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let player_number = arguments[0] as i16;
    let earthling = Address::<tje_mem::EarthlingState>::new(arguments[1]);
    if earthling.state().read(context) > 1 {
        return Ok(None);
    }
    call_void(context, ids::SET_EARTHLING_STATE, &[earthling.address(), 3])?;
    let object = ram::g_player_info(revision).at(i32::from(earthling.object().read(context)));
    object.frame().write(context, -1);
    let player = super::word_object(revision, player_number);
    let saved_x = player.x().read(context);
    let saved_y = player.y().read(context);
    player.x().write(context, earthling.x().read(context));
    player.y().write(context, earthling.y().read(context));
    let nearby = if player_number == 0 {
        ram::g_player1_nearby_terrain(revision).cast::<u8>()
    } else {
        ram::g_player2_nearby_terrain(revision).cast::<u8>()
    };
    let mut saved_land = [0_u8; 25];
    for (index, value) in saved_land.iter_mut().enumerate() {
        *value = nearby.offset(index as i32).read(context)
    }
    call_void(
        context,
        ids::UPDATE_LAND_AROUND_PLAYER,
        &[player_number as u32],
    )?;
    let mut count = ((call(context, ids::GET_RANDOM_NUMBER, &[])? as i16) & 1) + 2;
    while count > 0 {
        let present = call(context, ids::SELECT_RANDOM_PRESENT, &[])?;
        call_void(context, ids::DROP_PRESENT, &[player_number as u32, present])?;
        count -= 1;
    }
    for (index, value) in saved_land.into_iter().enumerate() {
        nearby.offset(index as i32).write(context, value)
    }
    player.x().write(context, saved_x);
    player.y().write(context, saved_y);
    Ok(None)
}
