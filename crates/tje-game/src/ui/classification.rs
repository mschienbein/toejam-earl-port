//! Original non-atomic UI execution contracts, including untranslated entries.
//!
//! Sources: G/src/ui/inventory.c, hud.c, main_menu.c, pause.c, window_panels.c.
//! Every return path was reviewed. These contracts do not assert that a Rust body
//! exists, that missing/uninitialized caller state is modeled, or that a reset
//! branch returns normally. Those remain separate translation/proof obligations.
//! Functional writers have stable ROM/RAM sources and no status/raster reads;
//! original hardware latency only inserts transaction-stuttering steps. TimeAware
//! entries need original IRQ/input progress on at least one reachable path.

use crate::{FunctionId, execution::ExecutionClass, ids};
use ExecutionClass::{Functional, TimeAware};

/// Reviewed original execution class. Atomic entries live in ATOMIC_CAPABLE.
/// Together the two slices classify all 76 original UI catalogue entries.
pub static EXECUTION_CONTRACTS: &[(FunctionId, ExecutionClass)] = &[
    // inventory.c:75: stable font-bank DMA; no raster/status/busy observation.
    (ids::TEXT_ANIMATE_FONT_BANKS, Functional),
    // inventory.c:92: WaitForNextVBlank must run the real original ISR.
    (ids::WAIT_FOR_V_BLANK_WITH_GRAPHICS_PAUSED, TimeAware),
    // inventory.c:100: pending-upload byte wait and final VBlank wait.
    (ids::ENTITY_CLEAR_VIEWPORT_SPRITES, TimeAware),
    // inventory.c:125: stable ordered tile writes; its banner call supplies
    // already_blanked=1, so that particular call never reaches the VBlank wait.
    (ids::INVENTORY_SERVICE_REDRAW_PHASE, Functional),
    // inventory.c:197: redraw-phase IRQ wait plus viewport clear's VBlank wait.
    (ids::DRAW_INVENTORY, TimeAware),
    // inventory.c:268: services four redraw phases directly, no IRQ wait.
    (ids::INVENTORY_REDRAW_SYNCHRONOUSLY, Functional),
    // inventory.c:278: both phase loops require VBlank redraw service.
    (ids::INVENTORY_REFRESH_CONTENTS_AND_WAIT, TimeAware),
    // inventory.c:290: viewport transition/open and explicit VBlank pause.
    (ids::TOGGLE_INVENTORY, TimeAware),
    // inventory.c:327: OpenPresent/OpenSelectedPresent, speech and menu waits.
    (ids::CHECK_IF_OPEN_FOR_BOTH, TimeAware),
    // inventory.c:368: sixty redraw/sprite-service/VBlank iterations.
    (ids::RANDOMIZE_PRESENTS, TimeAware),
    // inventory.c:403: menu/redraw waits and time-aware present/dialogue paths.
    (ids::PROCESS_INVENTORY, TimeAware),
    // inventory.c:491: blocking input/VBlank loop; uninitialized frame remains
    // a call-frame prerequisite, not permission to treat the original as atomic.
    (ids::CHECK_B_BUTTON, TimeAware),
    // inventory.c:510,521: DrawInventory followed by the blocking CheckBButton.
    (ids::INVENTORY_OPEN_MAIL_ORDER_MENU, TimeAware),
    (ids::INVENTORY_OPEN_IDENTIFICATION_MENU, TimeAware),
    // inventory.c:542: OpenSelectedPresent may invoke time-dependent effects.
    (ids::OPEN_PRESENT, TimeAware),
    // inventory.c:589: GenerateSpeech/StartDialogue wait for graphics upload.
    (ids::INVENTORY_BUY_SELECTED_MAIL_ORDER_PRESENT, TimeAware),
    // inventory.c:694: stable zero-terminated input and ordered glyph writes.
    (ids::TEXT_WRITE_STRING_TILES, Functional),
    // inventory.c:706: decimal local-buffer conversion and the same tile writer.
    // Passing its stack array still requires the separate call-frame contract.
    (ids::TEXT_WRITE_DECIMAL_TILES, Functional),
    // inventory.c:735,757: stable numeric/text inputs, ordered tile writes.
    (ids::INVENTORY_DRAW_BUCKS, Functional),
    (ids::INVENTORY_DRAW_PLAYER_STATS, Functional),
    // inventory.c:878,933: stable present descriptors/font strings, no polling.
    (ids::LOAD_PRESENTS_INTO_INVENTORY, Functional),
    (ids::INVENTORY_DRAW_MAIL_ORDER_OFFERS, Functional),
    // inventory.c:972: already_blanked=0 explicitly waits for VBlank.
    (ids::INVENTORY_DRAW_ACTION_BANNER, TimeAware),
    // hud.c:24: two stable digit uploads; ROM palette reads, no polling.
    (ids::HUD_LOAD_LEVEL_NUMBER_SPRITES, Functional),
    // hud.c:103: waits on gSpriteGraphicsUploadPending when graphics are enabled.
    (ids::HUD_LOAD_ACTION_ICON_SPRITE, TimeAware),
    // hud.c:197: queued path waits for upload IRQ service; immediate path uses
    // stable buffer DMA and the finite health animator, but does not narrow class.
    (ids::HUD_INITIALIZE_HEALTH_BAR, TimeAware),
    // hud.c:321: at most six ordered pixel words and stable life-tile uploads.
    (ids::HUD_ANIMATE_HEALTH_BAR, Functional),
    // hud.c:384: finite life-tile uploads from ROM, no readiness observation.
    (ids::HUD_LOAD_LIVES_SPRITE_TILES, Functional),
    // hud.c:509: stable rank strings and sequential VDP writes.
    (ids::HUD_DRAW_RANK_LABELS, Functional),
    // hud.c:535: ordered background writes and DMA from the sprite shadow just
    // positioned by this call; no time-dependent concurrent source mutation.
    (ids::DRAW_HUD, Functional),
    // hud.c:598: explicit pending-upload wait and queued health-bar wait.
    (ids::CHECK_PROMOTION_SCORE, TimeAware),
    // hud.c:619: six/twelve ordered VDP words; no status/raster observation.
    (ids::HUD_HIDE_PLAYER_SPRITES, Functional),
    // hud.c:665: HUD constructors may wait. Preserve the original rank>=8
    // endless recomputation bug; no fabricated threshold or termination fix.
    (ids::PLAYER_RESET_RANK_AND_RESOURCES, TimeAware),
    // hud.c:689: health/action constructors include pending-upload waits.
    (ids::SETUP_THE_MAIN_GAME, TimeAware),
    // hud.c:724: promotion, health graphics and death/promotion callees may wait.
    (ids::PLAYER_UPDATE_HEALTH_AND_LIVES, TimeAware),
    // window_panels.c:107: fixed border RNG draws and ordered plane writes.
    (ids::DRAW_MENU_WINDOW_FRAME, Functional),
    // window_panels.c:135: stable ROM uploads then a fixed ordered plane fill.
    (ids::DRAW_NESTED_WINDOW_FRAME, Functional),
    // window_panels.c:176,200: blanking-service gates are plain RAM writes;
    // their stable ROM tilemaps have no clock-sensitive observation.
    (ids::DRAW_PRESENT_DISPLAY_PANEL, Functional),
    (ids::DRAW_PAUSE_CHOICE_PANEL, Functional),
    // pause.c:23: input-release/selection polling and VBlank; the reset-selected
    // branch is a nonlocal reset, not an ordinary return or fake successful call.
    (ids::RUN_FINAL_LEVEL_PAUSE_MENU, TimeAware),
    // pause.c:62: sprite readiness, VBlank, input transitions and optional reset.
    (ids::GAME_PAUSED, TimeAware),
    // main_menu.c:31: menu, intro and Jam Out loops require input/IRQs. Its
    // missing incoming argument remains a call-frame prerequisite.
    (ids::NEW_GAME_MAIN_MENU, TimeAware),
    // main_menu.c:101: input/timeout VBlank loop followed by nonlocal reset;
    // execution class is known, but no path returns through the C caller's RTS.
    (ids::SHOW_GAME_OVER_AND_RESET, TimeAware),
    // main_menu.c:131,141,151,161: GeneralMenuHandler input/VBlank servicing.
    (ids::MENU_WHO_HANDLER, TimeAware),
    (ids::MENU_CONTROLLER_OPTIONS_HANDLER, TimeAware),
    (ids::MENU_WHAT_HANDLER, TimeAware),
    (ids::MENU_JAM_OUT_HANDLER, TimeAware),
    // main_menu.c:210: input release/selection polling, VBlank and sprite waits.
    (ids::GENERAL_MENU_HANDLER, TimeAware),
    // main_menu.c:283: stable menu record/string reads and ordered VDP writes.
    (ids::MENU_DRAW_TEXT_ITEMS, Functional),
    // main_menu.c:334: stable ROM palette upload plus RAM-only entity allocation.
    (ids::ADD_MENU_HEADER, Functional),
];
