//! Menus, pause, resource transitions and inventory transactions.
//! Sources are cited on each body. Original execution classes remain owned by
//! ui::classification and ui::ATOMIC_CAPABLE; these tables only track proof state.

mod draws;
mod entities;
mod main_menu;
mod pause;
mod resources;
mod transactions;

#[cfg(test)]
mod tests;

pub use draws::{menu_draw_text_items, text_write_decimal_tiles};
pub use entities::{
    add_menu_header, create_menu_cursor_entity, release_menu_cursor_entity, remove_menu_header,
};
pub use main_menu::{
    general_menu_handler, menu_controller_options_handler, menu_jam_out_handler, menu_what_handler,
    menu_who_handler, new_game_main_menu, show_game_over_and_reset,
};
pub use pause::{game_paused, run_final_level_pause_menu};
pub use resources::{
    check_promotion_score, player_reset_rank_and_resources, player_update_health_and_lives,
    setup_the_main_game,
};
pub use transactions::{
    clear_transient_entities, inventory_drop_selected_present, inventory_identify_selected_present,
    randomize_inventory, set_mail_order_presents,
};

use crate::{FunctionId, Translation, ids};

/// Only exercised, zero-mismatch proofs in both revisions enter production.
pub const PROVEN_ENTRIES: &[Translation] = &[
    Translation {
        id: ids::MENU_DRAW_TEXT_ITEMS,
        function: menu_draw_text_items,
    },
    Translation {
        id: ids::CREATE_MENU_CURSOR_ENTITY,
        function: create_menu_cursor_entity,
    },
    Translation {
        id: ids::RELEASE_MENU_CURSOR_ENTITY,
        function: release_menu_cursor_entity,
    },
    Translation {
        id: ids::ADD_MENU_HEADER,
        function: add_menu_header,
    },
    Translation {
        id: ids::REMOVE_MENU_HEADER,
        function: remove_menu_header,
    },
    Translation {
        id: ids::TEXT_WRITE_DECIMAL_TILES,
        function: text_write_decimal_tiles,
    },
    Translation {
        id: ids::SET_MAIL_ORDER_PRESENTS,
        function: set_mail_order_presents,
    },
    Translation {
        id: ids::RANDOMIZE_INVENTORY,
        function: randomize_inventory,
    },
    Translation {
        id: ids::INVENTORY_DROP_SELECTED_PRESENT,
        function: inventory_drop_selected_present,
    },
    Translation {
        id: ids::INVENTORY_IDENTIFY_SELECTED_PRESENT,
        function: inventory_identify_selected_present,
    },
    Translation {
        id: ids::CLEAR_TRANSIENT_ENTITIES,
        function: clear_transient_entities,
    },
];

/// Opt-in bodies composed with proven Rust callees by HookRegistry::proof.
/// Do not hook every pending modal root indiscriminately. Original long-RTS
/// menu calls exceed the current timed-call ceiling; the pause busy-input roots
/// additionally need source-clocked Rust effect regions, not erased poll reads.
pub const CANDIDATES: &[Translation] = &[
    // Original natural entry exceeds the host's 1,000,000-tick timed-call ceiling.
    Translation {
        id: ids::NEW_GAME_MAIN_MENU,
        function: new_game_main_menu,
    },
    // Typed nonreturning reset/exit comparison is absent from this base.
    Translation {
        id: ids::SHOW_GAME_OVER_AND_RESET,
        function: show_game_over_and_reset,
    },
    // Original natural entry exceeds the same timed-call ceiling before return.
    Translation {
        id: ids::MENU_WHO_HANDLER,
        function: menu_who_handler,
    },
    // Original GeneralMenuHandler closure needs a completed long-RTS receipt.
    Translation {
        id: ids::MENU_CONTROLLER_OPTIONS_HANDLER,
        function: menu_controller_options_handler,
    },
    // Original GeneralMenuHandler closure needs a completed long-RTS receipt.
    Translation {
        id: ids::MENU_WHAT_HANDLER,
        function: menu_what_handler,
    },
    // Original GeneralMenuHandler closure needs a completed long-RTS receipt.
    Translation {
        id: ids::MENU_JAM_OUT_HANDLER,
        function: menu_jam_out_handler,
    },
    // VBlank-paced source/input effects need a long-RTS/phase-correct receipt.
    Translation {
        id: ids::GENERAL_MENU_HANDLER,
        function: general_menu_handler,
    },
    // Needs source-clocked Rust effect regions across prefix/panels/input loops;
    // a generic IRQ wait or whole-entry original bridge is not a Rust proof.
    Translation {
        id: ids::RUN_FINAL_LEVEL_PAUSE_MENU,
        function: run_final_level_pause_menu,
    },
    // Same source-clocked Rust contract, including every controller call and IRQ.
    Translation {
        id: ids::GAME_PAUSED,
        function: game_paused,
    },
    // Positive promotion waits need a settled mainline fixture boundary.
    Translation {
        id: ids::CHECK_PROMOTION_SCORE,
        function: check_promotion_score,
    },
    // Graphics-disabled fixtures pass; settled-boundary TimeAware receipt needed.
    Translation {
        id: ids::PLAYER_RESET_RANK_AND_RESOURCES,
        function: player_reset_rank_and_resources,
    },
    // Graphics-disabled fixtures pass; settled-boundary TimeAware receipt needed.
    Translation {
        id: ids::SETUP_THE_MAIN_GAME,
        function: setup_the_main_game,
    },
    // All life-byte fixtures pass; settled-boundary TimeAware receipt needed.
    Translation {
        id: ids::PLAYER_UPDATE_HEALTH_AND_LIVES,
        function: player_update_health_and_lives,
    },
];

/// Kept outside cfg(test), so unproved translations remain inspectable tooling input.
pub const PENDING_IDS: &[FunctionId] = &[
    ids::NEW_GAME_MAIN_MENU,
    ids::SHOW_GAME_OVER_AND_RESET,
    ids::MENU_WHO_HANDLER,
    ids::MENU_CONTROLLER_OPTIONS_HANDLER,
    ids::MENU_WHAT_HANDLER,
    ids::MENU_JAM_OUT_HANDLER,
    ids::GENERAL_MENU_HANDLER,
    ids::RUN_FINAL_LEVEL_PAUSE_MENU,
    ids::GAME_PAUSED,
    ids::CHECK_PROMOTION_SCORE,
    ids::PLAYER_RESET_RANK_AND_RESOURCES,
    ids::SETUP_THE_MAIN_GAME,
    ids::PLAYER_UPDATE_HEALTH_AND_LIVES,
];
