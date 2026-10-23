//! Ui translation ownership; see docs/PORTING.md.

mod hud;
mod inventory;
mod main_menu;

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

use crate::{Context, Result};
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
];

/// Reviewed original entries that return without clock/interrupt progress.
/// Missing IDs are deliberately not approved for atomic checking or fallback.
///
/// W0's other entries are leaves: only finite memory copies, arithmetic,
/// sprite-shadow writes, or ordered device accesses, without ready/interrupt
/// polling. VDP writers require a live entry whose frozen FIFO accepts the writes.
/// `AreBothControllersConnected` has fixed CPU delay loops, not BUSACK waits.
/// `InventoryRefreshContentsAndWait` (G/src/ui/inventory.c:278) is deliberately
/// absent: both redraw-phase loops wait for VBlank interrupt service.
/// `HudHidePlayerSprites` (G/src/ui/hud.c:619) is also absent: its 6/12 VDP
/// data writes inevitably stall the four-slot frozen FIFO before returning.
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
