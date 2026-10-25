use crate::{Context, Result, call, call_void, ids};
use tje_mem::{Address, ObjectState, ram, rom};

/// G/src/presents/present_effects.c:40 (`PresentStartTransientEffect`).
/// Atomic: RAM allocation/restart, ROM animation reads and a queued PSG request.
pub fn present_start_transient_effect(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let index = i32::from(arguments[0] as i16);
    let handle = ram::g_player_transient_effect_entities(revision).at(index);
    if handle.read(context) == 0 {
        let number = call(context, ids::ALLOCATE_ENTITY, &[])? as i8;
        handle.write(context, number);
    }
    let number = handle.read(context);
    if number == 0 {
        return Ok(Some(1));
    }
    let effect = ram::g_player_info(revision).at(i32::from(number));
    effect.x().write(context, arguments[1] as i16);
    effect.y().write(context, arguments[2] as i16);
    effect.height().write(context, arguments[3] as i16);
    effect.level().write(context, arguments[4] as i8);
    effect.width().write(context, 32);
    effect.height_pixels().write(context, 32);
    effect.sprite_count().write(context, 1);
    effect.tile_count().write(context, 16);
    effect
        .tile()
        .write(context, effect.tile().read(context) | 0x8000);
    let animation = rom::transient_effect_animation(revision);
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
    call_void(context, ids::PLAY_SOUND, &[16, 50])?;
    Ok(Some(0))
}

/// G/src/presents/present_effects.c:107 (`PresentUpdateTransientEffect`).
/// Atomic: frame counters are ordinary RAM; only completed objects are released.
pub fn present_update_transient_effect(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let handle = ram::g_player_transient_effect_entities(revision).at(arguments[0] as i32);
    let number = handle.read(context);
    if number != 0 {
        let effect = ram::g_player_info(revision).at(i32::from(number));
        if effect.frame_duration().read(context) == 0 {
            call_void(context, ids::FREE_ENTITY, &[number as u32])?;
            handle.write(context, 0);
        } else if effect.frame().read(context) <= 2 {
            return Ok(Some(0));
        }
    }
    Ok(Some(1))
}

/// G/src/presents/present_effects.c:499 (`PresentUpdateRainCloud`).
/// Atomic: RAM animation state/timers, RNG and queued PSG; no animation wait.
pub fn present_update_rain_cloud(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let index = i32::from(arguments[0] as i16);
    let handle = ram::g_player_rain_cloud_states(revision).at(index);
    let number = handle.read(context);
    if number == 0 {
        return Ok(None);
    }
    let cloud = ram::g_player_info(revision).at(i32::from(number));
    let player = super::word_indexed_player(revision, index);
    cloud.level().write(context, player.level().read(context));
    let mut animation = None;
    let animations = rom::player_rain_cloud_animations(revision);
    if ram::g_player_elevator_flags(revision)
        .at(index)
        .read(context)
        != 0
        || ram::g_player_inventory_open(revision)
            .at(index)
            .read(context)
            != 0
    {
        if cloud.sprite_state().read(context) != 0 {
            animation = Some(animations.at(0));
            cloud.sprite_state().write(context, 0);
            let health = ram::g_player_health(revision).at(index);
            health.write(context, health.read(context).wrapping_sub(2));
        }
    } else {
        let timer = ram::g_player_rain_cloud_timers(revision).at(index);
        let remaining = super::predecrement_i16(context, timer);
        if remaining <= 0
            || ram::g_player_lives(revision).at(index).read(context) < 0
            || matches!(player.sprite_state().read(context), 5 | 6)
        {
            call_void(context, ids::FREE_ENTITY, &[number as u32])?;
            handle.write(context, 0);
        } else if cloud.sprite_state().read(context) != 0 {
            if cloud.frame_duration().read(context) == 0 {
                animation = Some(animations.at(0));
                cloud.sprite_state().write(context, 0);
                let groups = player.animation_group_mask().read(context);
                if ram::g_player_fall_states(revision).at(index).read(context) == 0
                    && (matches!(player.sprite_state().read(context), 0 | 4 | 76)
                        || groups & (0x400000 | 0x20000000 | 0x0834d06c) != 0)
                {
                    ram::g_player_pending_hurt_damage(revision)
                        .at(index)
                        .write(context, 2);
                    call_void(context, ids::SET_SPRITE, &[player.address(), 68])?;
                    player.animation_motion_axes().write(context, 3);
                    player.velocity_height().write(context, 0);
                    player.acceleration_height().write(context, 0);
                    call_void(context, ids::PLAY_SOUND, &[18, 40])?;
                } else if groups & (0x80 | 0x100 | 0x200) != 0 {
                    let health = ram::g_player_health(revision).at(index);
                    health.write(context, health.read(context).wrapping_sub(2));
                }
            }
        } else if call(context, ids::GET_RANDOM_NUMBER, &[])? & 63 == 1 {
            animation = Some(animations.at(1));
            cloud.sprite_state().write(context, 1);
        }
    }
    if let Some(animation) = animation {
        cloud
            .sequence()
            .write(context, animation.sequence().read(context));
        cloud
            .frames()
            .write(context, animation.frames().read(context));
        cloud
            .frame_phase()
            .write(context, animation.period().read(context));
        cloud.frame().write(context, -1);
        cloud.frame_duration().write(context, 1);
    }
    Ok(None)
}

/// G/src/presents/present_effects.c:140 (`PresentUpdateDecoy`).
/// Atomic: RAM readiness/hit tests and storage-only release/effect creation.
/// Level-change and three-hit branches are independent, including a double free.
pub fn present_update_decoy(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let index = i32::from(arguments[0] as i16);
    let handle = ram::g_player_decoy_entities(revision).at(index);
    let number = handle.read(context);
    if number != 0 {
        if call(
            context,
            ids::PRESENT_UPDATE_TRANSIENT_EFFECT,
            &[index as u32],
        )? == 0
        {
            return Ok(None);
        }
        let decoy = ram::g_player_info(revision).at(i32::from(number));
        decoy
            .behavior_flags()
            .write(context, decoy.behavior_flags().read(context) & !0x40);
        if super::word_indexed_player(revision, index)
            .level()
            .read(context)
            != decoy.level().read(context)
        {
            call_void(context, ids::FREE_ENTITY, &[number as u32])?;
            handle.write(context, 0);
        }
        if ram::g_player_decoy_hit_counts(revision)
            .at(index)
            .read(context)
            >= 3
        {
            call_void(context, ids::FREE_ENTITY, &[number as u32])?;
            let effect_arguments = [
                decoy.x().read(context) as u32,
                decoy.y().read(context) as u32,
                decoy.level().read(context) as u32,
            ];
            call(context, ids::ENTITY_SPAWN_TIMED_EFFECT, &effect_arguments)?;
            handle.write(context, 0);
        }
    }
    Ok(None)
}

/// G/src/presents/present_effects.c:73 (`OpenDecoy`).
/// Atomic: allocation, RAM placement/retry and a nonblocking reveal effect.
pub fn open_decoy(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let index = i32::from(arguments[0] as i16);
    let handle = ram::g_player_decoy_entities(revision).at(index);
    if handle.read(context) == 0 {
        let number = call(context, ids::ALLOCATE_ENTITY_WITH_EVICTION, &[0])? as i8;
        handle.write(context, number);
    }
    if handle.read(context) != 0 {
        let decoy = ram::g_player_info(revision).at(i32::from(handle.read(context)));
        let player = super::word_indexed_player(revision, index);
        decoy.x().write(context, player.x().read(context));
        decoy
            .y()
            .write(context, player.y().read(context).wrapping_add(1));
        decoy.level().write(context, player.level().read(context));
        loop {
            let args = [
                decoy.x().read(context) as u32,
                decoy.y().read(context) as u32,
                decoy.level().read(context) as u32,
            ];
            if call(context, ids::ELEVATOR_CONTAINS_POSITION, &args)? == 0 {
                break;
            }
            decoy
                .x()
                .write(context, decoy.x().read(context).wrapping_add(8));
        }
        decoy.width().write(context, 16);
        decoy.height_pixels().write(context, 32);
        decoy.sprite_count().write(context, 2);
        decoy.tile_count().write(context, 24);
        decoy
            .tile()
            .write(context, decoy.tile().read(context) | 0x8000);
        let animation = rom::player_decoy_animations(revision).at(index);
        decoy
            .sequence()
            .write(context, animation.sequence().read(context));
        decoy
            .frames()
            .write(context, animation.frames().read(context));
        decoy
            .frame_phase()
            .write(context, animation.period().read(context));
        decoy.frame_duration().write(context, 1);
        decoy
            .behavior_flags()
            .write(context, decoy.behavior_flags().read(context) | 1);
        ram::g_player_decoy_hit_counts(revision)
            .at(index)
            .write(context, 0);
        let args = [
            index as u32,
            decoy.x().read(context) as u32,
            (i32::from(decoy.y().read(context)) + 1) as u32,
            5,
            decoy.level().read(context) as u32,
        ];
        if call(context, ids::PRESENT_START_TRANSIENT_EFFECT, &args)? == 0 {
            decoy
                .behavior_flags()
                .write(context, decoy.behavior_flags().read(context) | 0x40);
        }
    }
    Ok(None)
}

/// G/src/presents/present_effects.c:164 (`PresentIsDoorway`).
/// Atomic: RAM placement and collision checks; this does not teleport or wait.
pub fn present_is_doorway(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let index = i32::from(arguments[0] as i16);
    let handle = ram::g_player_doorway_entities(revision).at(index);
    if handle.read(context) == 0 {
        let number = call(context, ids::ALLOCATE_ENTITY_WITH_EVICTION, &[0])? as i8;
        handle.write(context, number);
    }
    if handle.read(context) != 0 {
        let doorway = ram::g_player_info(revision).at(i32::from(handle.read(context)));
        let player = super::word_indexed_player(revision, index);
        doorway.x().write(context, player.x().read(context) & !7);
        let delta = if call(context, ids::GET_RANDOM_NUMBER, &[])? & 1 != 0 {
            -32
        } else {
            32
        };
        doorway
            .x()
            .write(context, doorway.x().read(context).wrapping_add(delta));
        doorway.y().write(context, player.y().read(context) & !7);
        doorway
            .x()
            .write(context, doorway.x().read(context).wrapping_add(16));
        doorway
            .y()
            .write(context, doorway.y().read(context).wrapping_sub(14));
        doorway.level().write(context, player.level().read(context));
        loop {
            let args = [
                doorway.x().read(context) as u32,
                doorway.y().read(context) as u32,
                doorway.level().read(context) as u32,
            ];
            if call(context, ids::ELEVATOR_CONTAINS_POSITION, &args)? == 0 {
                break;
            }
            doorway
                .x()
                .write(context, doorway.x().read(context).wrapping_add(8));
        }
        doorway.width().write(context, 24);
        doorway.height_pixels().write(context, 48);
        doorway.sprite_count().write(context, 2);
        doorway.tile_count().write(context, 28);
        doorway
            .tile()
            .write(context, doorway.tile().read(context) | 0x8000);
        let animation = rom::doorway_animations(revision).at(0);
        doorway
            .sequence()
            .write(context, animation.sequence().read(context));
        doorway
            .frames()
            .write(context, animation.frames().read(context));
        doorway
            .frame_phase()
            .write(context, animation.period().read(context));
        doorway.frame_duration().write(context, 1);
        doorway.frame().write(context, -1);
        doorway.sprite_state().write(context, 0);
        doorway
            .behavior_flags()
            .write(context, doorway.behavior_flags().read(context) | 1);
        let args = [
            index as u32,
            doorway.x().read(context) as u32,
            (i32::from(doorway.y().read(context)) + 1) as u32,
            5,
            doorway.level().read(context) as u32,
        ];
        if call(context, ids::PRESENT_START_TRANSIENT_EFFECT, &args)? == 0 {
            doorway
                .behavior_flags()
                .write(context, doorway.behavior_flags().read(context) | 0x40);
        }
    }
    Ok(None)
}

/// G/src/presents/present_effects.c:467 (`PresentIsRainCloud`).
/// Atomic: RAM allocation, ROM animation and a random timer store.
pub fn present_is_rain_cloud(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let index = i32::from(arguments[0] as i16);
    let handle = ram::g_player_rain_cloud_states(revision).at(index);
    if handle.read(context) == 0 {
        let number = call(context, ids::ALLOCATE_ENTITY_WITH_EVICTION, &[0])? as i8;
        handle.write(context, number);
        if number != 0 {
            let cloud = ram::g_player_info(revision).at(i32::from(number));
            let player = super::word_indexed_player(revision, index);
            cloud.level().write(context, player.level().read(context));
            cloud
                .behavior_flags()
                .write(context, cloud.behavior_flags().read(context) | 8);
            cloud.owner().write(context, index as i8);
            cloud.attachment_y_or_latch().write(context, 10);
            cloud.attachment_height().write(context, 10);
            cloud
                .tile()
                .write(context, cloud.tile().read(context) | 0x8000);
            cloud.width().write(context, 32);
            cloud.height_pixels().write(context, 32);
            cloud.sprite_count().write(context, 1);
            cloud.tile_count().write(context, 16);
            let animation = rom::player_rain_cloud_animations(revision).at(0);
            cloud
                .sequence()
                .write(context, animation.sequence().read(context));
            cloud
                .frames()
                .write(context, animation.frames().read(context));
            cloud
                .frame_phase()
                .write(context, animation.period().read(context));
            cloud.frame().write(context, -1);
            cloud.frame_duration().write(context, 1);
        }
    }
    let timer = (call(context, ids::GET_RANDOM_NUMBER, &[])? as u16) % 101 + 400;
    ram::g_player_rain_cloud_timers(revision)
        .at(index)
        .write(context, timer as i16);
    Ok(None)
}

/// G/src/presents/present_effects.c:554 (`OpenBoomBox`).
/// Atomic: RAM allocation/reveal and music-request flags, not hardware playback.
pub fn open_boom_box(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let index = i32::from(arguments[0] as i16);
    let handle = ram::g_player_boom_box_entities(revision).at(index);
    if handle.read(context) == 0 {
        let number = call(context, ids::ALLOCATE_ENTITY_WITH_EVICTION, &[0])? as i8;
        handle.write(context, number);
    }
    if handle.read(context) != 0 {
        let object = ram::g_player_info(revision).at(i32::from(handle.read(context)));
        let player = super::word_indexed_player(revision, index);
        object.x().write(context, player.x().read(context));
        object
            .y()
            .write(context, player.y().read(context).wrapping_add(4));
        object.level().write(context, player.level().read(context));
        object.width().write(context, 32);
        object.height_pixels().write(context, 24);
        object.sprite_count().write(context, 1);
        object.tile_count().write(context, 16);
        object
            .tile()
            .write(context, object.tile().read(context) | 0x8000);
        let animation = rom::boom_box_animations(revision).at(0);
        object
            .sequence()
            .write(context, animation.sequence().read(context));
        object
            .frames()
            .write(context, animation.frames().read(context));
        object.frame_phase().write(context, 2);
        object.frame_duration().write(context, 1);
        object
            .behavior_flags()
            .write(context, object.behavior_flags().read(context) | 1);
        let args = [
            index as u32,
            object.x().read(context) as u32,
            (i32::from(object.y().read(context)) + 1) as u32,
            1,
            object.level().read(context) as u32,
        ];
        call(context, ids::PRESENT_START_TRANSIENT_EFFECT, &args)?;
        ram::g_player_boom_box_timers(revision)
            .at(index)
            .write(context, 300);
        ram::g_requested_music_selection(revision).write(context, 14);
        ram::g_boom_box_music_template_hold(revision).write(context, 1);
    }
    Ok(None)
}

/// G/src/presents/present_effects.c:587 (`PresentUpdateBoomBox`).
/// Atomic: timer/object/music RAM and nonblocking entity/RNG helpers.
pub fn present_update_boom_box(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let index = i32::from(arguments[0] as i16);
    let handle = ram::g_player_boom_box_entities(revision).at(index);
    if handle.read(context) != 0 {
        let object = ram::g_player_info(revision).at(i32::from(handle.read(context)));
        let timer = ram::g_player_boom_box_timers(revision).at(index);
        let remaining = super::predecrement_i16(context, timer);
        if remaining <= 0 {
            let level = object.level().read(context);
            call_void(context, ids::EARTHLING_WAKE_LEVEL, &[level as u32])?;
            let args = [
                index as u32,
                object.x().read(context) as u32,
                (i32::from(object.y().read(context)) + 1) as u32,
                1,
                object.level().read(context) as u32,
            ];
            call(context, ids::PRESENT_START_TRANSIENT_EFFECT, &args)?;
            let number = handle.read(context);
            call_void(context, ids::FREE_ENTITY, &[number as u32])?;
            handle.write(context, 0);
            if ram::g_player_boom_box_entities(revision)
                .at(index ^ 1)
                .read(context)
                == 0
            {
                let highest = ram::g_highest_level(revision).read(context);
                let music = call(context, ids::GET_LEVEL_MUSIC_INDEX, &[highest as u32])?;
                ram::g_requested_music_selection(revision).write(context, music as i8);
                ram::g_boom_box_music_template_hold(revision).write(context, 0);
            }
        } else if ram::g_music_animation_event(revision).read(context) != 0 {
            object.frame().write(context, -1);
            object.frame_duration().write(context, 1);
            ram::g_music_animation_event(revision).write(context, 0);
        }
    }
    Ok(None)
}

/// G/src/presents/present_effects.c:325 (`PerformPromotion`).
/// TimeAware: the immediate fallback can enter GenerateSpeech's IRQ wait.
/// Its rank fallback passes the object pointer as speaker, exactly as the bug does.
pub fn perform_promotion(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let index = i32::from(arguments[0] as i16);
    let promotion = arguments[1] as i16;
    let terrain = arguments[2] as u16;
    let handle = ram::g_player_promotion_display_entities(revision).at(index);
    let allocated = if handle.read(context) == 0 {
        let number = call(context, ids::ALLOCATE_ENTITY_WITH_EVICTION, &[0])? as i8;
        handle.write(context, number);
        number != 0
    } else {
        false
    };
    if allocated {
        let display = ram::g_player_info(revision).at(i32::from(handle.read(context)));
        let player = super::word_indexed_player(revision, index);
        display
            .behavior_flags()
            .write(context, display.behavior_flags().read(context) | 8);
        display.owner().write(context, index as i8);
        display.level().write(context, player.level().read(context));
        display
            .tile()
            .write(context, display.tile().read(context) | 0x8000);
        display.width().write(context, 32);
        display.height_pixels().write(context, 32);
        let animation = if promotion == 11 {
            display.attachment_y_or_latch().write(context, 8);
            display.sprite_count().write(context, 2);
            display.tile_count().write(context, 32);
            rom::rank_promotion_display_animation(revision).at(0)
        } else {
            display.sprite_count().write(context, 1);
            display.tile_count().write(context, 16);
            let animation = if promotion == 16 {
                rom::health_drain_display_animation(revision).at(0)
            } else {
                rom::extra_life_display_animation(revision).at(0)
            };
            display.attachment_y_or_latch().write(context, 6);
            display.attachment_height().write(context, 1);
            animation
        };
        display
            .sequence()
            .write(context, animation.sequence().read(context));
        display
            .frames()
            .write(context, animation.frames().read(context));
        display
            .frame_phase()
            .write(context, animation.period().read(context));
        display.frame().write(context, -1);
        display.frame_duration().write(context, 1);
        display.sprite_state().write(context, promotion as i8);
        if promotion != 11
            && terrain < 0x80
            && player.height().read(context) <= 0
            && player.sprite_state().read(context) != 5
            && player.sprite_state().read(context) != 6
            && player.sprite_state().read(context) != 79
            && player.sprite_state().read(context) != 78
            && player.sprite_state().read(context) != 77
            && player.animation_group_mask().read(context) & 0x7785bf60 == 0
        {
            let sprite = if promotion == 17 { 71 } else { 70 };
            call_void(context, ids::SET_SPRITE, &[player.address(), sprite])?;
            player.animation_motion_axes().write(context, 3);
            call_void(context, ids::STOP_PLAYER, &[player.address()])?;
        }
    } else {
        if promotion == 17
            || (promotion == 11
                && (i32::from(ram::g_player_rank(revision).at(index).read(context)) - 2) % 3 == 0)
        {
            let lives = ram::g_player_lives(revision).at(index);
            if lives.read(context) < 8 {
                lives.write(context, lives.read(context).wrapping_add(1));
                ram::g_hud_redraw_pending(revision).write(context, 1);
            }
        } else if promotion == 16 {
            call_void(context, ids::PLAY_SOUND, &[18, 40])?;
            context.discard_u8(ram::g_player_health(revision).at(index).address())?; // CLR.B.
            ram::g_player_health(revision).at(index).write(context, 0);
        }
        if promotion == 11 {
            let text = rom::rank_name_str_table(revision)
                .at(i32::from(
                    ram::g_player_rank(revision).at(index).read(context),
                ))
                .read(context)
                .address();
            let speaker = super::word_indexed_player(revision, index).address();
            call(context, ids::GENERATE_SPEECH, &[speaker, text, 1])?;
            call_void(
                context,
                ids::PLAY_SOUND_EFFECT,
                &[rom::sfx_yeah_all_right(revision).address(), 90, 0],
            )?;
        }
    }
    if promotion == 17 {
        call_void(
            context,
            ids::PLAY_SOUND_EFFECT,
            &[rom::sfx_extra_life_glitz(revision).address(), 87, 0],
        )?;
    }
    Ok(None)
}

/// G/src/presents/present_effects.c:447 (`PresentUpdateBurps`).
/// TimeAware: dialogue/speech may wait for IRQ-owned graphics uploads.
pub fn present_update_burps(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let index = i32::from(arguments[0] as i16);
    let burps = ram::g_player_burps_left(revision).at(index);
    if burps.read(context) != 0 {
        let timer = ram::g_player_burp_timers(revision).at(index);
        if timer.read(context) == -1 {
            call(context, ids::START_DIALOGUE, &[24, index as u32])?;
            call_void(context, ids::AUDIO_REQUEST_CHANNEL_TEMPLATE_EFFECT, &[14])?;
            timer.write(context, 50);
        } else {
            let remaining = super::predecrement_i8(context, timer);
            if remaining < 0 {
                let mask = super::word_indexed_player(revision, index)
                    .draw_flags()
                    .read(context)
                    & 6;
                call(
                    context,
                    ids::EARTHLING_RESET_STATE_TWO_BY_DRAW_MASK,
                    &[u32::from(mask)],
                )?;
                let text = tje_mem::rom_symbol(revision, "GameText_0169DC")
                    .expect("catalogued burp text")
                    .address;
                call(context, ids::GENERATE_SPEECH, &[index as u32, text, 1])?;
                burps.write(context, burps.read(context).wrapping_sub(1));
                let delay = 20 + (call(context, ids::GET_RANDOM_NUMBER, &[])? as u16) % 81;
                timer.write(context, delay as i8);
                if ram::g_pcm_effect_cooldown_ticks(revision).read(context) == 0 {
                    call_void(
                        context,
                        ids::PLAY_SOUND_EFFECT,
                        &[rom::sfx_burp(revision).address(), 50, 0],
                    )?;
                }
            }
        }
    }
    Ok(None)
}

/// G/src/presents/present_effects.c:395 (`BummerLifePromo`).
/// TimeAware: completed displays can generate speech or another promotion display.
pub fn bummer_life_promo(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let player = Address::<ObjectState>::new(arguments[0]);
    let terrain = arguments[1] as u16;
    let index = i32::from(player.entity_index().read(context));
    let handle = ram::g_player_promotion_display_entities(revision).at(index);
    if handle.read(context) == 0 {
        return Ok(None);
    }
    let display = ram::g_player_info(revision).at(i32::from(handle.read(context)));
    display.level().write(context, player.level().read(context));
    if display.frame_duration().read(context) != 0 {
        if display.sprite_state().read(context) == 16 {
            let health = ram::g_player_health(revision).at(index);
            if health.read(context) > 1 {
                health.write(context, health.read(context).wrapping_sub(1));
            }
            call_void(context, ids::PLAY_SOUND, &[18, 40])?;
        } else if display.sprite_state().read(context) == 17
            && ram::g_pcm_effect_cooldown_ticks(revision).read(context) < 4
        {
            call_void(
                context,
                ids::PLAY_SOUND_EFFECT,
                &[rom::sfx_extra_life_glitz(revision).address(), 87, 0],
            )?;
        } else if display.sprite_state().read(context) == 11 {
            let cache_index = ram::g_random_cache_index(revision);
            let next = if cache_index.read(context) == 58 {
                cache_index.write(context, 0);
                0
            } else {
                cache_index.write(context, cache_index.read(context).wrapping_add(1));
                cache_index.read(context)
            };
            // The compiler uses BTST.B on the low byte of this cached word.
            let random_low = ram::g_cached_random_values(revision)
                .at(i32::from(next))
                .address()
                + 1;
            if context.read_u8(random_low) & 2 != 0 {
                let phase = i16::from(display.frame().read(context)) % 14;
                if phase == 12 || (phase > 1 && phase < 11 && phase != 7) {
                    call_void(
                        context,
                        ids::PLAY_SOUND_EFFECT,
                        &[rom::sfx_jam_snare_drum(revision).address(), 90, 0],
                    )?;
                }
            }
        }
    } else {
        let mut promotion_complete = false;
        if display.sprite_state().read(context) == 17 {
            let lives = ram::g_player_lives(revision).at(index);
            if lives.read(context) < 8 {
                lives.write(context, lives.read(context).wrapping_add(1));
                ram::g_hud_redraw_pending(revision).write(context, 1);
            }
        } else if display.sprite_state().read(context) == 16 {
            if player.sprite_state().read(context) != 5 && player.sprite_state().read(context) != 6
            {
                context.discard_u8(ram::g_player_health(revision).at(index).address())?; // CLR.B.
                ram::g_player_health(revision).at(index).write(context, 0);
                let text = tje_mem::rom_symbol(revision, "GameText_0168DC")
                    .expect("catalogued promotion text")
                    .address;
                call(context, ids::GENERATE_SPEECH, &[index as u32, text, 0])?;
            }
        } else if display.sprite_state().read(context) == 11 {
            let text = rom::rank_name_str_table(revision)
                .at(i32::from(
                    ram::g_player_rank(revision).at(index).read(context),
                ))
                .read(context)
                .address();
            call(context, ids::GENERATE_SPEECH, &[index as u32, text, 1])?;
            promotion_complete = true;
            call_void(
                context,
                ids::PLAY_SOUND_EFFECT,
                &[rom::sfx_yeah_all_right(revision).address(), 90, 0],
            )?;
        }
        if player.sprite_state().read(context) == 71 || player.sprite_state().read(context) == 70 {
            let sprite = call(context, ids::GET_ACTIVE_PRESENT_SPRITE, &[player.address()])?;
            call_void(context, ids::SET_SPRITE, &[player.address(), sprite])?;
        }
        let number = handle.read(context);
        call_void(context, ids::FREE_ENTITY, &[number as u32])?;
        context.discard_u8(handle.address())?; // CLR.B after FreeEntity.
        handle.write(context, 0);
        if promotion_complete
            && (i32::from(ram::g_player_rank(revision).at(index).read(context)) - 2) % 3 == 0
        {
            call_void(
                context,
                ids::PERFORM_PROMOTION,
                &[index as u32, 17, u32::from(terrain)],
            )?;
        }
    }
    Ok(None)
}

/// G/src/presents/present_effects.c:230 (`PlayerTeleportThroughDoorway`).
/// TimeAware: explicit graphics-paused VBlank and screen-split redraw transactions.
pub fn player_teleport_through_doorway(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let index = i32::from(arguments[0] as i16);
    let player = Address::<ObjectState>::new(arguments[1]);
    // Both camera-one words are fetched before the alternate-view predicate.
    let mut camera_x = ram::g_camera1_pending_x(revision).read(context);
    let mut camera_y = ram::g_camera1_pending_y(revision).read(context);
    if ram::g_player_lives(revision).at(0).read(context) >= 0
        && index == 1
        && ram::g_split_screen_enabled(revision).read(context) != 0
    {
        camera_x = ram::g_camera2_pending_x(revision).read(context);
        camera_y = ram::g_camera2_pending_y(revision).read(context);
    }
    loop {
        call_void(context, ids::SET_POSITION_ON_NEW_LEVEL, &[index as u32])?;
        let y = player.y().read(context);
        let x = player.x().read(context);
        let args = [player.address(), x as u32, y as u32, 2];
        if call(
            context,
            ids::ENTITY_FIND_NARROW_COLLISION_AT_POSITION,
            &args,
        )? == 0
        {
            break;
        }
    }
    if ram::g_split_screen_enabled(revision).read(context) == 0
        && i32::from(player.x().read(context)) > i32::from(camera_x) + 20
        && i32::from(player.x().read(context)) < i32::from(camera_x) + 320 - 20
        && i32::from(player.y().read(context)) > i32::from(camera_y) + 40
        && i32::from(player.y().read(context)) < i32::from(camera_y) + 224 - 40
    {
        return Ok(None);
    }
    call_void(context, ids::WAIT_FOR_V_BLANK_WITH_GRAPHICS_PAUSED, &[])?;
    call_void(context, ids::DISABLE_BLANKING_GRAPHICS, &[])?;
    let mode = ram::g_mode2_reg_softcopy(revision);
    mode.write(context, mode.read(context) & 0x3c);
    context.write_vdp_control(0x8104 | u16::from(mode.read(context)));
    call_void(context, ids::UPDATE_SCREEN_SPLIT_MODE, &[1])?;
    mode.write(context, mode.read(context) | 0x40);
    context.write_vdp_control(0x8104 | u16::from(mode.read(context)));
    call_void(context, ids::ENABLE_BLANKING_GRAPHICS, &[])?;
    Ok(None)
}

/// G/src/presents/present_effects.c:260 (`PresentUpdateDoorway`).
/// TimeAware: a centered crossing can teleport and redraw through VBlank.
pub fn present_update_doorway(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let owner = i32::from(arguments[0] as i16);
    let handle = ram::g_player_doorway_entities(revision).at(owner);
    let number = handle.read(context);
    if number == 0 {
        return Ok(None);
    }
    if call(
        context,
        ids::PRESENT_UPDATE_TRANSIENT_EFFECT,
        &[owner as u32],
    )? == 0
    {
        return Ok(None);
    }
    let object = ram::g_player_info(revision).at(i32::from(number));
    if object.behavior_flags().read(context) & 0x40 != 0 {
        object
            .behavior_flags()
            .write(context, object.behavior_flags().read(context) & !0x40);
    }
    if object.draw_flags().read(context) & 6 == 0
        && ram::g_player_inventory_open(revision).at(0).read(context) == 0
        && ram::g_player_inventory_open(revision).at(1).read(context) == 0
    {
        call_void(context, ids::FREE_ENTITY, &[number as u32])?;
        context.discard_u8(handle.address())?; // CLR.B after FreeEntity.
        handle.write(context, 0);
        return Ok(None);
    }
    let contact = ram::g_doorway_player_contact_flags(revision).at(owner);
    if contact.read(context) & 4 != 0 {
        let index = i32::from((contact.read(context) & 3).wrapping_sub(1));
        let player = ram::g_player_info(revision).at(index);
        let doorway_y = object.y().read(context);
        if doorway_y > player.y().read(context) {
            call_void(
                context,
                ids::PLAYER_TELEPORT_THROUGH_DOORWAY,
                &[index as u32, player.address()],
            )?;
            let level = player.level().read(context);
            let y = i32::from(player.y().read(context)) + 1;
            let x = player.x().read(context);
            let args = [index as u32, x as u32, y as u32, 5, level as u32];
            call(context, ids::PRESENT_START_TRANSIENT_EFFECT, &args)?;
        }
    }
    let mut active = 0_i8;
    context.discard_u8(contact.address())?; // CLR.B before rebuilding contact flags.
    contact.write(context, 0);
    for index in 0..2 {
        let player = ram::g_player_info(revision).at(index);
        let column = i32::from(player.x().read(context)) >> 3;
        let row = i32::from(player.y().read(context)) >> 3;
        let args = [player.address(), row as u32, column as u32];
        if call(context, ids::PRESENT_FIND_DOORWAY_OWNER_AT_TILE, &args)? as i32 == owner + 1 {
            contact.write(context, (index + 1) as i8);
            if i32::from(player.x().read(context)) > i32::from(object.x().read(context)) - 6
                && i32::from(player.x().read(context)) < i32::from(object.x().read(context)) + 6
            {
                active = 1;
                contact.write(context, contact.read(context) | 4);
            }
        }
    }
    if active != object.sprite_state().read(context) {
        let animation = rom::doorway_animations(revision).at(i32::from(active));
        object
            .sequence()
            .write(context, animation.sequence().read(context));
        object
            .frames()
            .write(context, animation.frames().read(context));
        object
            .frame_phase()
            .write(context, animation.period().read(context));
        object.frame_duration().write(context, 1);
        object.frame().write(context, -1);
        object.sprite_state().write(context, active);
        if active != 0 {
            object
                .behavior_flags()
                .write(context, object.behavior_flags().read(context) & !1);
        } else {
            object
                .behavior_flags()
                .write(context, object.behavior_flags().read(context) | 1);
            if contact.read(context) != 0 {
                let index = i32::from((contact.read(context) & 3).wrapping_sub(1));
                let player = ram::g_player_info(revision).at(index);
                if i32::from(contact.read(context) & 3) == index + 1
                    && i32::from(player.y().read(context)) - i32::from(object.y().read(context)) < 8
                {
                    player
                        .y()
                        .write(context, object.y().read(context).wrapping_add(8));
                }
            }
        }
    }
    Ok(None)
}
