//! HUD construction and inventory statistics, traced to G/src/ui.
//!
//! Differential evidence is reproduced by the opt-in `check/` harness. Pending
//! entries never enter production dispatch until both revisions are exercised.

mod animations;
mod draws;
pub(crate) mod frame;
mod health;
mod statistics;

pub(crate) use animations::{hud_animate_health_bar, hud_load_action_icon_sprite};
pub(crate) use draws::{draw_menu_window_frame, hud_draw_rank_labels};
pub(crate) use health::{draw_hud, hud_initialize_health_bar};
pub(crate) use statistics::{
    inventory_draw_bucks, inventory_draw_mail_order_offers, inventory_draw_player_stats,
};

use crate::{FramedTranslation, FunctionId, Translation, ids};

/// Entries with exercised, zero-mismatch proof in both revisions.
pub static PROVEN_ENTRIES: &[Translation] = &[
    Translation {
        id: ids::HUD_ANIMATE_HEALTH_BAR,
        function: hud_animate_health_bar,
    },
    Translation {
        id: ids::HUD_DRAW_RANK_LABELS,
        function: hud_draw_rank_labels,
    },
    Translation {
        id: ids::DRAW_HUD,
        function: draw_hud,
    },
    Translation {
        id: ids::INVENTORY_DRAW_BUCKS,
        function: inventory_draw_bucks,
    },
    Translation {
        id: ids::INVENTORY_DRAW_PLAYER_STATS,
        function: inventory_draw_player_stats,
    },
    Translation {
        id: ids::INVENTORY_DRAW_MAIL_ORDER_OFFERS,
        function: inventory_draw_mail_order_offers,
    },
    Translation {
        id: ids::DRAW_MENU_WINDOW_FRAME,
        function: draw_menu_window_frame,
    },
];

/// TimeAware candidates, not production hooks: a source-clock contract is missing.
/// Natural CPU-prefix paths accept one original ISR and no translated ISR;
/// gFrameCounter FF800F differs (action icon frame 917, initialization frame 940).
/// The ISR projection cannot erase a different invocation count. No cycle padding.
pub static FRAME_CANDIDATES: &[FramedTranslation] = &[
    FramedTranslation {
        id: ids::HUD_LOAD_ACTION_ICON_SPRITE,
        function: hud_load_action_icon_sprite,
    },
    FramedTranslation {
        id: ids::HUD_INITIALIZE_HEALTH_BAR,
        function: hud_initialize_health_bar,
    },
];

/// Exact blocker: missing source CPU clocks; unequal TimeAware ISR invocations.
pub static PENDING_IDS: &[FunctionId] = &[
    ids::HUD_LOAD_ACTION_ICON_SPRITE,
    ids::HUD_INITIALIZE_HEALTH_BAR,
];
