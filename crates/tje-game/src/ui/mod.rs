//! Ui translation ownership; see docs/PORTING.md.

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
];

/// Reviewed original entries that return without clock/interrupt progress.
/// Missing IDs are deliberately not approved for atomic checking or fallback.
pub static ATOMIC_CAPABLE: &[crate::FunctionId] = &[
    crate::ids::ENABLE_TEXT_FONT_ANIMATION,
    crate::ids::DISABLE_TEXT_FONT_ANIMATION_IF_MENUS_CLOSED,
    crate::ids::PLAYER_GET_MAXIMUM_HEALTH,
    crate::ids::GET_SCORE_FOR_PROMO,
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
