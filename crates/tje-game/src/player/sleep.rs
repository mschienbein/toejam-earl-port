use crate::{Context, Result, call, call_void, ids};
use tje_mem::{Address, ObjectState, Ptr, Revision, ram, rom};

/// G/src/player/sleep.c:34 (`ProcessSleep`). REV 02 alone excludes active choices
/// from idle sleep. Signed countdown branches retain SUBQ's overflow condition.
pub(super) fn process_sleep(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let state = Address::<ObjectState>::new(arguments[0]);
    let revision = context.revision();
    let player = i32::from(state.entity_index().read(context));
    let mut next_sprite = -1_i16;
    if ram::g_player_elevator_flags(revision)
        .at(player)
        .read(context)
        != 0
        || state.level().read(context) == 26
    {
        return Ok(None);
    }
    let left = ram::g_player_wake_effects_left(revision).at(player);
    let idle = ram::g_player_idle_ticks(revision).at(player);
    let handle = ram::g_player_wake_effect_entity_handles(revision).at(player);
    if state.sprite_state().read(context) == 116 {
        let delay = ram::g_player_book_sleep_delay(revision).at(player);
        let (stored, remaining) = super::counters::predecrement_byte(delay.read(context));
        delay.write(context, stored);
        if remaining <= 0 {
            next_sprite = 117;
            let random = call(context, ids::GET_RANDOM_NUMBER, &[])? as u16;
            left.write(context, ((u32::from(random) % 7) + 4) as i8);
        }
    } else if state.sprite_state().read(context) == 117 {
        let input = call(context, ids::READ_MAPPED_PLAYER_INPUT, &[player as u32])? as u8;
        let effect = handle.read(context);
        if effect != 0 {
            let wake = ram::g_player_info(revision).at(i32::from(effect));
            let finished = if left.read(context) == 0 {
                let (stored, remaining) = super::counters::predecrement_word(idle.read(context));
                idle.write(context, stored);
                remaining <= 0
            } else {
                false
            };
            if finished || wake.frame_duration().read(context) == 0 {
                call_void(context, ids::FREE_ENTITY, &[effect as u32])?;
                handle.write(context, 0);
                if left.read(context) == 0 {
                    next_sprite =
                        call(context, ids::GET_ACTIVE_PRESENT_SPRITE, &[state.address()])? as i16;
                }
            }
        } else if input != 0
            && input
                != ram::g_player_sleep_previous_input(revision)
                    .at(player)
                    .read(context)
            && u32::from(super::poses::next_random(context)) % 100 < 20
        {
            let effect = call(context, ids::ALLOCATE_ENTITY, &[])? as i16;
            if effect != 0 {
                handle.write(context, effect as i8);
                let wake = ram::g_player_info(revision).at(i32::from(effect));
                left.write(context, left.read(context).wrapping_sub(1));
                wake.x().write(context, state.x().read(context));
                wake.y().write(context, state.y().read(context));
                wake.level().write(context, state.level().read(context));
                wake.width().write(context, 64);
                wake.height_pixels().write(context, 32);
                wake.sprite_count().write(context, 2);
                wake.tile_count().write(context, 32);
                wake.tile().write(context, 0x8000);
                let animation = rom::player_wake_effect_animations(revision).at(player);
                wake.frames()
                    .write(context, animation.frames().read(context));
                wake.frame_duration().write(context, 1);
                wake.frame().write(context, -1);
                if left.read(context) != 0 {
                    wake.frame_phase().write(context, 15);
                    let sequence = rom::wake_whisper_animation_streams(revision)
                        .at(if left.read(context) & 1 != 0 { 0 } else { 2 });
                    wake.sequence().write(context, Ptr::new(sequence.address()));
                    call_void(
                        context,
                        ids::PLAY_SOUND_EFFECT,
                        &[rom::sfx_wake_up_whisper(revision).address(), 50, 0],
                    )?;
                } else {
                    wake.sequence()
                        .write(context, animation.sequence().read(context));
                    wake.frame_phase()
                        .write(context, animation.period().read(context));
                    call_void(
                        context,
                        ids::PLAY_SOUND_EFFECT,
                        &[rom::sfx_wake_up_shout(revision).address(), 88, 0],
                    )?;
                    idle.write(context, 30);
                }
            }
        } else if ram::g_pcm_effect_cooldown_ticks(revision).read(context) == 0 {
            call_void(
                context,
                ids::PLAY_SOUND_EFFECT,
                &[rom::sfx_snoring(revision).address(), 30, 0],
            )?;
        }
        ram::g_player_sleep_previous_input(revision)
            .at(player)
            .write(context, input);
    } else {
        let effect = handle.read(context);
        if effect != 0 {
            call_void(context, ids::FREE_ENTITY, &[effect as u32])?;
            handle.write(context, 0);
        }
        if state.sprite_state().read(context) == 0
            && (revision == Revision::Rev00
                || ram::g_player_interaction_choice_active(revision)
                    .at(player)
                    .read(context)
                    == 0)
        {
            idle.write(context, idle.read(context).wrapping_add(1));
            if idle.read(context) > 300 {
                next_sprite = 117;
                let random = call(context, ids::GET_RANDOM_NUMBER, &[])? as u16;
                left.write(context, ((u32::from(random) % 7) + 4) as i8);
            }
        } else {
            idle.write(context, 0);
        }
    }
    if next_sprite != -1 {
        call_void(
            context,
            ids::SET_SPRITE,
            &[state.address(), next_sprite as u32],
        )?;
    }
    Ok(None)
}
