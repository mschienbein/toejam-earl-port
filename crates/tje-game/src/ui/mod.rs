//! Ui translation ownership; see docs/PORTING.md.

mod classification;
pub use classification::EXECUTION_CONTRACTS;

pub(crate) mod animations;
mod control;
pub(crate) mod draws;
pub(crate) mod entities;
mod hud;
pub mod hud_all;
mod input;
mod inventory;
mod inventory_animations;
mod inventory_draws;
mod inventory_entities;
mod main_menu;
pub mod menus;
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

/// Both-revision-proven entries; this literal registry is also traceability input.
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
    crate::Translation {
        id: ids::MENU_DRAW_TEXT_ITEMS,
        function: menus::menu_draw_text_items,
    },
    crate::Translation {
        id: ids::CREATE_MENU_CURSOR_ENTITY,
        function: menus::create_menu_cursor_entity,
    },
    crate::Translation {
        id: ids::RELEASE_MENU_CURSOR_ENTITY,
        function: menus::release_menu_cursor_entity,
    },
    crate::Translation {
        id: ids::ADD_MENU_HEADER,
        function: menus::add_menu_header,
    },
    crate::Translation {
        id: ids::REMOVE_MENU_HEADER,
        function: menus::remove_menu_header,
    },
    crate::Translation {
        id: ids::TEXT_WRITE_DECIMAL_TILES,
        function: menus::text_write_decimal_tiles,
    },
    crate::Translation {
        id: ids::SET_MAIL_ORDER_PRESENTS,
        function: menus::set_mail_order_presents,
    },
    crate::Translation {
        id: ids::RANDOMIZE_INVENTORY,
        function: menus::randomize_inventory,
    },
    crate::Translation {
        id: ids::INVENTORY_DROP_SELECTED_PRESENT,
        function: menus::inventory_drop_selected_present,
    },
    crate::Translation {
        id: ids::INVENTORY_IDENTIFY_SELECTED_PRESENT,
        function: menus::inventory_identify_selected_present,
    },
    crate::Translation {
        id: ids::CLEAR_TRANSIENT_ENTITIES,
        function: menus::clear_transient_entities,
    },
    crate::Translation {
        id: ids::HUD_ANIMATE_HEALTH_BAR,
        function: hud_all::hud_animate_health_bar,
    },
    crate::Translation {
        id: ids::HUD_DRAW_RANK_LABELS,
        function: hud_all::hud_draw_rank_labels,
    },
    crate::Translation {
        id: ids::DRAW_HUD,
        function: hud_all::draw_hud,
    },
    crate::Translation {
        id: ids::INVENTORY_DRAW_BUCKS,
        function: hud_all::inventory_draw_bucks,
    },
    crate::Translation {
        id: ids::INVENTORY_DRAW_PLAYER_STATS,
        function: hud_all::inventory_draw_player_stats,
    },
    crate::Translation {
        id: ids::INVENTORY_DRAW_MAIL_ORDER_OFFERS,
        function: hud_all::inventory_draw_mail_order_offers,
    },
    crate::Translation {
        id: ids::DRAW_MENU_WINDOW_FRAME,
        function: hud_all::draw_menu_window_frame,
    },
    crate::Translation {
        id: ids::INVENTORY_DEACTIVATE_DISPLAY_ENTITIES,
        function: inventory_entities::inventory_deactivate_display_entities,
    },
    crate::Translation {
        id: ids::INVENTORY_DRAW_ACTION_BANNER,
        function: inventory_draws::inventory_draw_action_banner,
    },
    crate::Translation {
        id: ids::LOAD_PRESENTS_INTO_INVENTORY,
        function: inventory_draws::load_presents_into_inventory,
    },
    crate::Translation {
        id: ids::INVENTORY_INITIALIZE_DISPLAY_ENTITIES,
        function: control::inventory_initialize_display_entities,
    },
    crate::Translation {
        id: ids::INVENTORY_SERVICE_REDRAW_PHASE,
        function: control::inventory_service_redraw_phase,
    },
];

/// Opt-in inventory candidates; not registered until both-revision proof.
pub static PENDING_TRANSLATIONS: &[crate::Translation] = &[
    // Isolated prefix passes; late integrated CheckIfOpenForBoth effect traces
    // differ at REV02 frame4808/REV00 frame4810 even with actual caller frames.
    crate::Translation {
        id: ids::OPEN_PRESENT,
        function: control::open_present,
    },
    crate::Translation {
        id: ids::CHECK_IF_OPEN_FOR_BOTH,
        function: control::check_if_open_for_both,
    },
    // Valid clock/IRQ entry and completed natural viewport-clear receipt needed.
    crate::Translation {
        id: ids::ENTITY_CLEAR_VIEWPORT_SPRITES,
        function: inventory_animations::entity_clear_viewport_sprites,
    },
    // Unreached by the current controller scripts; needs a labelled valid entry.
    crate::Translation {
        id: ids::INVENTORY_REDRAW_SYNCHRONOUSLY,
        function: control::inventory_redraw_synchronously,
    },
    // Two natural entries pass; complete proof through the split-mode path needed.
    crate::Translation {
        id: ids::DRAW_INVENTORY,
        function: control::draw_inventory,
    },
    // Both revisions frame2103 observe different semantic YM2612 BUSY operands.
    crate::Translation {
        id: ids::TOGGLE_INVENTORY,
        function: control::toggle_inventory,
    },
    // Needs the recorded mailbox acquisition path and completed speech waits.
    crate::Translation {
        id: ids::INVENTORY_BUY_SELECTED_MAIL_ORDER_PRESENT,
        function: control::inventory_buy_selected_mail_order_present,
    },
    // Sixty redraw/VBlank iterations need a complete real-input timed receipt.
    crate::Translation {
        id: ids::RANDOMIZE_PRESENTS,
        function: input::randomize_presents,
    },
    // Semantic YM2612 status differs at frame1901 REV02/frame2100 REV00.
    crate::Translation {
        id: ids::PROCESS_INVENTORY,
        function: input::process_inventory,
    },
    // Incoming D2.low residue is retained; long modal/input timed receipt needed.
    crate::Translation {
        id: ids::CHECK_B_BUTTON,
        function: input::check_b_button,
    },
    // DrawInventory plus modal CheckBButton input/wait closure still pending.
    crate::Translation {
        id: ids::INVENTORY_OPEN_MAIL_ORDER_MENU,
        function: input::inventory_open_mail_order_menu,
    },
    // DrawInventory plus modal CheckBButton input/wait closure still pending.
    crate::Translation {
        id: ids::INVENTORY_OPEN_IDENTIFICATION_MENU,
        function: input::inventory_open_identification_menu,
    },
    // Original modal calls exceed the host's timed-call ceiling; source-clocked
    // controller polling and typed reset exits are separate pending contracts.
    crate::Translation {
        id: ids::NEW_GAME_MAIN_MENU,
        function: menus::new_game_main_menu,
    },
    crate::Translation {
        id: ids::SHOW_GAME_OVER_AND_RESET,
        function: menus::show_game_over_and_reset,
    },
    crate::Translation {
        id: ids::MENU_WHO_HANDLER,
        function: menus::menu_who_handler,
    },
    crate::Translation {
        id: ids::MENU_CONTROLLER_OPTIONS_HANDLER,
        function: menus::menu_controller_options_handler,
    },
    crate::Translation {
        id: ids::MENU_WHAT_HANDLER,
        function: menus::menu_what_handler,
    },
    crate::Translation {
        id: ids::MENU_JAM_OUT_HANDLER,
        function: menus::menu_jam_out_handler,
    },
    crate::Translation {
        id: ids::GENERAL_MENU_HANDLER,
        function: menus::general_menu_handler,
    },
    crate::Translation {
        id: ids::RUN_FINAL_LEVEL_PAUSE_MENU,
        function: menus::run_final_level_pause_menu,
    },
    crate::Translation {
        id: ids::GAME_PAUSED,
        function: menus::game_paused,
    },
    // Positive resource transitions need settled mainline TimeAware receipts.
    crate::Translation {
        id: ids::CHECK_PROMOTION_SCORE,
        function: menus::check_promotion_score,
    },
    crate::Translation {
        id: ids::PLAYER_RESET_RANK_AND_RESOURCES,
        function: menus::player_reset_rank_and_resources,
    },
    crate::Translation {
        id: ids::SETUP_THE_MAIN_GAME,
        function: menus::setup_the_main_game,
    },
    crate::Translation {
        id: ids::PLAYER_UPDATE_HEALTH_AND_LIVES,
        function: menus::player_update_health_and_lives,
    },
    // Extra original ISR mutates observed frame/graphics state; no masking.
    crate::Translation {
        id: ids::HUD_LOAD_ACTION_ICON_SPRITE,
        function: hud_all::hud_load_action_icon_sprite,
    },
    crate::Translation {
        id: ids::HUD_INITIALIZE_HEALTH_BAR,
        function: hud_all::hud_initialize_health_bar,
    },
];

/// Readable bodies are written; exact-clock/IRQ projection and source-backed
/// call-frame composition are still proof prerequisites, never assumed passed.
pub static PENDING: &[crate::FunctionId] = &[
    // Exact per-entry blockers are on the body registry above, not approval.
    ids::OPEN_PRESENT,
    ids::CHECK_IF_OPEN_FOR_BOTH,
    // Live viewport/clock entry and direct redraw fixture coverage prerequisites.
    ids::ENTITY_CLEAR_VIEWPORT_SPRITES,
    ids::INVENTORY_REDRAW_SYNCHRONOUSLY,
    ids::DRAW_INVENTORY,
    // Semantic device operands and long modal/input closure must match exactly.
    ids::TOGGLE_INVENTORY,
    ids::INVENTORY_BUY_SELECTED_MAIL_ORDER_PRESENT,
    ids::RANDOMIZE_PRESENTS,
    ids::PROCESS_INVENTORY,
    ids::CHECK_B_BUTTON,
    ids::INVENTORY_OPEN_MAIL_ORDER_MENU,
    ids::INVENTORY_OPEN_IDENTIFICATION_MENU,
    // Original long-RTS ceiling, source-clocked input, and typed exits.
    ids::NEW_GAME_MAIN_MENU,
    ids::SHOW_GAME_OVER_AND_RESET,
    ids::MENU_WHO_HANDLER,
    ids::MENU_CONTROLLER_OPTIONS_HANDLER,
    ids::MENU_WHAT_HANDLER,
    ids::MENU_JAM_OUT_HANDLER,
    ids::GENERAL_MENU_HANDLER,
    ids::RUN_FINAL_LEVEL_PAUSE_MENU,
    ids::GAME_PAUSED,
    // Settled mainline positive resource transitions, not fixture-only approval.
    ids::CHECK_PROMOTION_SCORE,
    ids::PLAYER_RESET_RANK_AND_RESOURCES,
    ids::SETUP_THE_MAIN_GAME,
    ids::PLAYER_UPDATE_HEALTH_AND_LIVES,
    // Extra original ISR changes observed frame/graphics state.
    ids::HUD_LOAD_ACTION_ICON_SPRITE,
    ids::HUD_INITIALIZE_HEALTH_BAR,
];

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
