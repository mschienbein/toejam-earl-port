//! Ui translation ownership; see docs/PORTING.md.

mod classification;
pub use classification::EXECUTION_CONTRACTS;

mod hud;
mod inventory;
mod main_menu;
mod rendering;
mod waiting;

#[cfg(test)]
mod tests;

pub use hud::{
    hud_position_action_icon_sprite, hud_position_health_bar_sprites,
    hud_position_level_number_sprites, hud_position_lives_sprite,
};
pub use inventory::{
    force_text_background_color_to_black, inventory_get_selected_present,
    inventory_set_camera_suspension, inventory_update_selector_entity, text_copy_font_banks_to_ram,
    text_map_character_to_glyph,
};
pub use main_menu::{are_both_controllers_connected, position_menu_cursor_entity};
pub use rendering::{
    draw_nested_window_frame, draw_pause_choice_panel, draw_present_display_panel,
    hud_hide_player_sprites, hud_load_level_number_sprites, hud_load_lives_sprite_tiles,
    text_animate_font_banks, text_write_string_tiles,
};
pub use waiting::{inventory_refresh_contents_and_wait, wait_for_vblank_with_graphics_paused};

use crate::{Context, Result, ids};
use tje_mem::ram;

/// Available Rust entries owned by this subsystem.
pub static TRANSLATIONS: &[crate::Translation] = &[
    crate::Translation {
        id: crate::ids::ENABLE_TEXT_FONT_ANIMATION,
        function: enable_text_font_animation,
    },
    crate::Translation {
        id: crate::ids::DISABLE_TEXT_FONT_ANIMATION_IF_MENUS_CLOSED,
        function: disable_text_font_animation_if_menus_closed,
    },
    crate::Translation {
        id: crate::ids::PLAYER_GET_MAXIMUM_HEALTH,
        function: player_get_maximum_health,
    },
    crate::Translation {
        id: crate::ids::GET_SCORE_FOR_PROMO,
        function: get_score_for_promo,
    },
    crate::Translation {
        id: crate::ids::FORCE_TEXT_BACKGROUND_COLOR_TO_BLACK,
        function: force_text_background_color_to_black,
    },
    crate::Translation {
        id: crate::ids::TEXT_COPY_FONT_BANKS_TO_RAM,
        function: text_copy_font_banks_to_ram,
    },
    crate::Translation {
        id: crate::ids::INVENTORY_SET_CAMERA_SUSPENSION,
        function: inventory_set_camera_suspension,
    },
    crate::Translation {
        id: crate::ids::INVENTORY_GET_SELECTED_PRESENT,
        function: inventory_get_selected_present,
    },
    crate::Translation {
        id: crate::ids::TEXT_MAP_CHARACTER_TO_GLYPH,
        function: text_map_character_to_glyph,
    },
    crate::Translation {
        id: crate::ids::INVENTORY_UPDATE_SELECTOR_ENTITY,
        function: inventory_update_selector_entity,
    },
    crate::Translation {
        id: crate::ids::HUD_POSITION_LEVEL_NUMBER_SPRITES,
        function: hud_position_level_number_sprites,
    },
    crate::Translation {
        id: crate::ids::HUD_POSITION_ACTION_ICON_SPRITE,
        function: hud_position_action_icon_sprite,
    },
    crate::Translation {
        id: crate::ids::HUD_POSITION_HEALTH_BAR_SPRITES,
        function: hud_position_health_bar_sprites,
    },
    crate::Translation {
        id: crate::ids::HUD_POSITION_LIVES_SPRITE,
        function: hud_position_lives_sprite,
    },
    crate::Translation {
        id: crate::ids::ARE_BOTH_CONTROLLERS_CONNECTED,
        function: are_both_controllers_connected,
    },
    crate::Translation {
        id: crate::ids::POSITION_MENU_CURSOR_ENTITY,
        function: position_menu_cursor_entity,
    },
    crate::Translation {
        id: crate::ids::HUD_HIDE_PLAYER_SPRITES,
        function: hud_hide_player_sprites,
    },
    crate::Translation {
        id: crate::ids::INVENTORY_REFRESH_CONTENTS_AND_WAIT,
        function: inventory_refresh_contents_and_wait,
    },
    crate::Translation {
        id: crate::ids::DRAW_NESTED_WINDOW_FRAME,
        function: draw_nested_window_frame,
    },
    crate::Translation {
        id: crate::ids::DRAW_PAUSE_CHOICE_PANEL,
        function: draw_pause_choice_panel,
    },
    crate::Translation {
        id: crate::ids::DRAW_PRESENT_DISPLAY_PANEL,
        function: draw_present_display_panel,
    },
    crate::Translation {
        id: crate::ids::HUD_LOAD_LEVEL_NUMBER_SPRITES,
        function: hud_load_level_number_sprites,
    },
    crate::Translation {
        id: crate::ids::HUD_LOAD_LIVES_SPRITE_TILES,
        function: hud_load_lives_sprite_tiles,
    },
    crate::Translation {
        id: crate::ids::TEXT_ANIMATE_FONT_BANKS,
        function: text_animate_font_banks,
    },
    crate::Translation {
        id: crate::ids::TEXT_WRITE_STRING_TILES,
        function: text_write_string_tiles,
    },
    crate::Translation {
        id: ids::WAIT_FOR_V_BLANK_WITH_GRAPHICS_PAUSED,
        function: wait_for_vblank_with_graphics_paused,
    },
];

/// Opt-in unproven bodies, never part of production dispatch.
pub static PENDING_TRANSLATIONS: &[crate::Translation] = &[];
/// No unproven entries in the completed W0/W1 batch.
pub static PENDING: &[crate::FunctionId] = &[];

/// Reviewed ORIGINAL atomic entries, translated or still residual.
/// Classification describes clock/interrupt requirements, not translation readiness:
/// the sprite allocator's original uninitialized-local bug still needs a call frame.
///
/// G/src/ui/inventory.c:32,48,62,68,238,531,645,784 and hud.c:56,133,177,284,
/// 465,583 are finite RAM/ROM operations or at most two VDP data writes.
/// G/src/ui/main_menu.c:171 has fixed CPU delay loops, not BUSACK polling.
/// VDP writers still require an entry whose frozen FIFO accepts the accesses.
/// Remaining atomic callers below cite their reviewed RAM-only closure.
pub static ATOMIC_CAPABLE: &[crate::FunctionId] = &[
    crate::ids::ENABLE_TEXT_FONT_ANIMATION,
    crate::ids::DISABLE_TEXT_FONT_ANIMATION_IF_MENUS_CLOSED,
    crate::ids::PLAYER_GET_MAXIMUM_HEALTH,
    crate::ids::GET_SCORE_FOR_PROMO,
    crate::ids::FORCE_TEXT_BACKGROUND_COLOR_TO_BLACK,
    crate::ids::TEXT_COPY_FONT_BANKS_TO_RAM,
    crate::ids::INVENTORY_SET_CAMERA_SUSPENSION,
    crate::ids::INVENTORY_GET_SELECTED_PRESENT,
    crate::ids::TEXT_MAP_CHARACTER_TO_GLYPH,
    crate::ids::INVENTORY_UPDATE_SELECTOR_ENTITY,
    crate::ids::HUD_POSITION_LEVEL_NUMBER_SPRITES,
    crate::ids::HUD_POSITION_ACTION_ICON_SPRITE,
    crate::ids::HUD_POSITION_HEALTH_BAR_SPRITES,
    crate::ids::HUD_POSITION_LIVES_SPRITE,
    crate::ids::ARE_BOTH_CONTROLLERS_CONNECTED,
    crate::ids::POSITION_MENU_CURSOR_ENTITY,
    // inventory.c:176: seeded RAM RNG and SelectRandomPresent; no clock-based retry.
    crate::ids::SET_MAIL_ORDER_PRESENTS,
    // inventory.c:356: finite inventory rerolls through the same RAM-only selector.
    crate::ids::RANDOMIZE_INVENTORY,
    // inventory.c:567: RAM compaction, DropPresent's RAM search, atomic PlaySound.
    crate::ids::INVENTORY_DROP_SELECTED_PRESENT,
    // inventory.c:611: RAM selection/identification, RAM RNG, atomic PlaySound.
    crate::ids::INVENTORY_IDENTIFY_SELECTED_PRESENT,
    // inventory.c:821: allocation/eviction retries change RAM, not hardware time.
    ids::INVENTORY_INITIALIZE_DISPLAY_ENTITIES,
    // inventory.c:855: seven RAM-only ReleaseEntityViewportSprites calls.
    ids::INVENTORY_DEACTIVATE_DISPLAY_ENTITIES,
    // window_panels.c:16: FreeEntity, dropped-present and telephone RAM cleanup.
    crate::ids::CLEAR_TRANSIENT_ENTITIES,
    // main_menu.c:295: AllocateEntity/AllocateEntityViewportSprites are RAM-only.
    crate::ids::CREATE_MENU_CURSOR_ENTITY,
    // main_menu.c:315,356: conditional FreeEntity; no wait or hardware access.
    crate::ids::RELEASE_MENU_CURSOR_ENTITY,
    crate::ids::REMOVE_MENU_HEADER,
];
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
