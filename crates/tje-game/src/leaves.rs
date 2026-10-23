//! W0 leaf batch: core, camera, UI, player and speech.
use crate::{Context, Result};
use tje_mem::{Address, ObjectState, ram};

/// G/src/core/startup_and_main_loop.c:122 (`DisableBlankingGraphics`).
pub fn disable_blanking_graphics(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    ram::g_v_blank_graphics_enabled(revision).write(context, 0);
    ram::g_h_blank_graphics_enabled(revision).write(context, 0);
    Ok(None)
}
/// G/src/core/startup_and_main_loop.c:129 (`EnableBlankingGraphics`).
pub fn enable_blanking_graphics(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    ram::g_v_blank_graphics_enabled(revision).write(context, 1);
    ram::g_h_blank_graphics_enabled(revision).write(context, 1);
    Ok(None)
}
/// G/src/core/startup_and_main_loop.c:136 (`GetVBlankGraphicsEnabled`).
pub fn get_vblank_graphics_enabled(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    Ok(Some(
        i32::from(ram::g_v_blank_graphics_enabled(context.revision()).read(context)) as u32,
    ))
}
/// G/src/core/startup_and_main_loop.c:65 (`ReadMappedPlayerInput`). Preserve
/// unsigned-word promotion and truth tests on signed flag bytes.
pub fn read_mapped_player_input(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let player = arguments[0] as i16;
    let swapped = ram::g_controller_ports_swapped(revision).read(context) != 0;
    let input = u32::from(if (player == 0) != swapped {
        ram::g_controller1_raw_input(revision).read(context)
    } else {
        ram::g_controller2_raw_input(revision).read(context)
    });
    let mapped = if ram::g_controller_buttons_ac_swapped(revision).read(context) != 0 {
        (input & 0x9f)
            | if input & 0x40 != 0 { 0x20 } else { 0 }
            | if input & 0x20 != 0 { 0x40 } else { 0 }
    } else {
        input
    };
    Ok(Some(mapped))
}
/// G/src/camera/toggle_camera_flag.c:6 (`SetCameraTilePriorityEnabled`).
pub fn set_camera_tile_priority_enabled(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    ram::g_camera_tile_attributes(context.revision())
        .write(context, if arguments[0] as i16 != 0 { 0x8000 } else { 0 });
    Ok(None)
}
/// G/src/ui/inventory.c:62 (`EnableTextFontAnimation`).
pub fn enable_text_font_animation(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    ram::g_text_font_animation_enabled(context.revision()).write(context, 1);
    Ok(None)
}
/// G/src/ui/inventory.c:68 (`DisableTextFontAnimationIfMenusClosed`).
pub fn disable_text_font_animation_if_menus_closed(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    if ram::g_player_inventory_open(revision).at(0).read(context) == 0
        && ram::g_player2_inventory_open(revision).read(context) == 0
    {
        ram::g_text_font_animation_enabled(revision).write(context, 0);
    }
    Ok(None)
}
/// G/src/ui/hud.c:177 (`PlayerGetMaximumHealth`). Array indexing stays signed,
/// unchecked emulated address arithmetic, including non-player arguments.
pub fn player_get_maximum_health(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let player = i32::from(arguments[0] as i16);
    let rank = i32::from(
        ram::g_player_rank(context.revision())
            .at(player)
            .read(context),
    );
    Ok(Some(
        (if player == 0 { 23_i32 } else { 31_i32 }).wrapping_add(rank.wrapping_shl(2)) as u32,
    ))
}
/// G/src/ui/hud.c:583 (`GetScoreForPromo`). Addition narrows to a signed short
/// after each iteration, not at host integer width.
pub fn get_score_for_promo(_: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let rank = arguments[0] as i16;
    let mut score = 40_i16;
    if rank >= 8 {
        score = 0;
    } else {
        for prior_rank in 0..rank {
            score = (i32::from(score) + (i32::from(prior_rank) + 3) * 20) as i16;
        }
    }
    Ok(Some(i32::from(score) as u32))
}
/// G/src/player/entity_motion_terrain.c:303 (`StopPlayer`). Preserve the original
/// interleaved velocity/acceleration store order, even on aliased addresses.
pub fn stop_player(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let state = Address::<ObjectState>::new(arguments[0]);
    state.velocity_x().write(context, 0);
    state.acceleration_x().write(context, 0);
    state.velocity_y().write(context, 0);
    state.acceleration_y().write(context, 0);
    state.velocity_height().write(context, 0);
    state.acceleration_height().write(context, 0);
    Ok(None)
}
/// G/src/player/start_motion_mode.c:5 (`PlayerInitializePhasedJumpMotion`).
pub fn player_initialize_phased_jump_motion(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let player = i32::from(arguments[0] as i16);
    ram::g_player_phased_jump_phase(revision)
        .at(player)
        .write(context, 1);
    ram::g_player_phased_jump_phase_ticks(revision)
        .at(player)
        .write(context, 16);
    ram::g_player_phased_jump_armed(revision)
        .at(player)
        .write(context, 1);
    ram::g_player_phased_jump_speech_used(revision)
        .at(player)
        .write(context, 0);
    Ok(None)
}
/// G/src/player/effect_slots.c:17 (`EntityHasFreeTimedEffectSlot`). Signed slot
/// indices use C truth, not a positive-only host predicate.
pub fn entity_has_free_timed_effect_slot(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let slots = ram::g_timed_effect_entities(context.revision());
    Ok(Some(u32::from(
        slots.at(0).read(context) == 0 || slots.at(1).read(context) == 0,
    )))
}
/// G/src/speech/speech_and_dialogue.c:63 (`SetNextSpeechLifetime`).
pub fn set_next_speech_lifetime(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    ram::g_next_speech_lifetime_override(context.revision()).write(context, arguments[0] as i16);
    Ok(None)
}
