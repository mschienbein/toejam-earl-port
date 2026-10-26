//! Original non-atomic UI execution contracts, including untranslated entries.
//!
//! Sources: G/src/ui/inventory.c, hud.c, main_menu.c, pause.c, window_panels.c.
//! Every return path was reviewed. These contracts do not assert that a Rust body
//! exists, that missing/uninitialized caller state is modeled, or that a reset
//! branch returns normally. Those remain separate translation/proof obligations.
//! Functional writers have stable ROM/RAM sources and no status/raster reads;
//! original hardware latency only inserts transaction-stuttering steps. TimeAware
//! entries need original IRQ/input progress on at least one reachable path.

use crate::{execution::ExecutionClass, ids};
use ExecutionClass::{Functional, TimeAware};

/// Reviewed original execution class. Atomic entries live in ATOMIC_CAPABLE.
/// Together the two slices classify all 76 original UI catalogue entries.
pub static EXECUTION_CONTRACTS: &[crate::execution::ExecutionContract] = &[
    // inventory.c:75: stable font-bank DMA; no raster/status/busy observation.
    crate::execution::ExecutionContract {
        function: ids::TEXT_ANIMATE_FONT_BANKS,
        class: Functional,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // inventory.c:92: WaitForNextVBlank must run the real original ISR.
    crate::execution::ExecutionContract {
        function: ids::WAIT_FOR_V_BLANK_WITH_GRAPHICS_PAUSED,
        class: TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // inventory.c:100: pending-upload byte wait and final VBlank wait.
    crate::execution::ExecutionContract {
        function: ids::ENTITY_CLEAR_VIEWPORT_SPRITES,
        class: TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // inventory.c:125: already_blanked=1 excludes its banner wait, but the
    // dispatch contract conservatively inherits the callee's general TimeAware
    // class rather than silently refining execution class from an argument.
    crate::execution::ExecutionContract {
        function: ids::INVENTORY_SERVICE_REDRAW_PHASE,
        class: TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // inventory.c:197: redraw-phase IRQ wait plus viewport clear's VBlank wait.
    crate::execution::ExecutionContract {
        function: ids::DRAW_INVENTORY,
        class: TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // inventory.c:268: directly services four phases; inherits that callee class.
    crate::execution::ExecutionContract {
        function: ids::INVENTORY_REDRAW_SYNCHRONOUSLY,
        class: TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // inventory.c:278: both phase loops require VBlank redraw service.
    crate::execution::ExecutionContract {
        function: ids::INVENTORY_REFRESH_CONTENTS_AND_WAIT,
        class: TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // inventory.c:290: viewport transition/open and explicit VBlank pause.
    crate::execution::ExecutionContract {
        function: ids::TOGGLE_INVENTORY,
        class: TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // inventory.c:327: OpenPresent/OpenSelectedPresent, speech and menu waits.
    crate::execution::ExecutionContract {
        function: ids::CHECK_IF_OPEN_FOR_BOTH,
        class: TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ReplayedEnvironment,
        operational_ceiling: None,
        evidence: None,
    },
    // inventory.c:368: sixty redraw/sprite-service/VBlank iterations.
    crate::execution::ExecutionContract {
        function: ids::RANDOMIZE_PRESENTS,
        class: TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // inventory.c:403: menu/redraw waits and time-aware present/dialogue paths.
    crate::execution::ExecutionContract {
        function: ids::PROCESS_INVENTORY,
        class: TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ReplayedEnvironment,
        operational_ceiling: None,
        evidence: None,
    },
    // inventory.c:491: blocking input/VBlank loop; uninitialized frame remains
    // a call-frame prerequisite, not permission to treat the original as atomic.
    crate::execution::ExecutionContract {
        function: ids::CHECK_B_BUTTON,
        class: TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ReplayedEnvironment,
        operational_ceiling: None,
        evidence: None,
    },
    // inventory.c:510,521: DrawInventory followed by the blocking CheckBButton.
    crate::execution::ExecutionContract {
        function: ids::INVENTORY_OPEN_MAIL_ORDER_MENU,
        class: TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    crate::execution::ExecutionContract {
        function: ids::INVENTORY_OPEN_IDENTIFICATION_MENU,
        class: TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // inventory.c:542: OpenSelectedPresent may invoke time-dependent effects.
    crate::execution::ExecutionContract {
        function: ids::OPEN_PRESENT,
        class: TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // inventory.c:589: GenerateSpeech/StartDialogue wait for graphics upload.
    crate::execution::ExecutionContract {
        function: ids::INVENTORY_BUY_SELECTED_MAIL_ORDER_PRESENT,
        class: TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // inventory.c:694: stable zero-terminated input and ordered glyph writes.
    crate::execution::ExecutionContract {
        function: ids::TEXT_WRITE_STRING_TILES,
        class: Functional,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // inventory.c:706: decimal local-buffer conversion and the same tile writer.
    // Passing its stack array still requires the separate call-frame contract.
    crate::execution::ExecutionContract {
        function: ids::TEXT_WRITE_DECIMAL_TILES,
        class: Functional,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // inventory.c:735,757: stable numeric/text inputs, ordered tile writes.
    crate::execution::ExecutionContract {
        function: ids::INVENTORY_DRAW_BUCKS,
        class: Functional,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    crate::execution::ExecutionContract {
        function: ids::INVENTORY_DRAW_PLAYER_STATS,
        class: Functional,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // inventory.c:878,933: stable present descriptors/font strings, no polling.
    crate::execution::ExecutionContract {
        function: ids::LOAD_PRESENTS_INTO_INVENTORY,
        class: Functional,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    crate::execution::ExecutionContract {
        function: ids::INVENTORY_DRAW_MAIL_ORDER_OFFERS,
        class: Functional,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // inventory.c:972: already_blanked=0 explicitly waits for VBlank.
    crate::execution::ExecutionContract {
        function: ids::INVENTORY_DRAW_ACTION_BANNER,
        class: TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // hud.c:24: two stable digit uploads; ROM palette reads, no polling.
    crate::execution::ExecutionContract {
        function: ids::HUD_LOAD_LEVEL_NUMBER_SPRITES,
        class: Functional,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // hud.c:103: waits on gSpriteGraphicsUploadPending when graphics are enabled.
    crate::execution::ExecutionContract {
        function: ids::HUD_LOAD_ACTION_ICON_SPRITE,
        class: TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ReplayedEnvironment,
        operational_ceiling: None,
        evidence: None,
    },
    // hud.c:197: queued path waits for upload IRQ service; immediate path uses
    // stable buffer DMA and the finite health animator, but does not narrow class.
    crate::execution::ExecutionContract {
        function: ids::HUD_INITIALIZE_HEALTH_BAR,
        class: TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // hud.c:321: at most six ordered pixel words and stable life-tile uploads.
    crate::execution::ExecutionContract {
        function: ids::HUD_ANIMATE_HEALTH_BAR,
        class: Functional,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // hud.c:384: finite life-tile uploads from ROM, no readiness observation.
    crate::execution::ExecutionContract {
        function: ids::HUD_LOAD_LIVES_SPRITE_TILES,
        class: Functional,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // hud.c:509: stable rank strings and sequential VDP writes.
    crate::execution::ExecutionContract {
        function: ids::HUD_DRAW_RANK_LABELS,
        class: Functional,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // hud.c:535: ordered background writes and DMA from the sprite shadow just
    // positioned by this call; no time-dependent concurrent source mutation.
    crate::execution::ExecutionContract {
        function: ids::DRAW_HUD,
        class: Functional,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // hud.c:598: explicit pending-upload wait and queued health-bar wait.
    crate::execution::ExecutionContract {
        function: ids::CHECK_PROMOTION_SCORE,
        class: TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // hud.c:619: six/twelve ordered VDP words; no status/raster observation.
    crate::execution::ExecutionContract {
        function: ids::HUD_HIDE_PLAYER_SPRITES,
        class: Functional,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // hud.c:665: HUD constructors may wait. Preserve the original rank>=8
    // endless recomputation bug; no fabricated threshold or termination fix.
    crate::execution::ExecutionContract {
        function: ids::PLAYER_RESET_RANK_AND_RESOURCES,
        class: TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // hud.c:689: health/action constructors include pending-upload waits.
    crate::execution::ExecutionContract {
        function: ids::SETUP_THE_MAIN_GAME,
        class: TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // hud.c:724: promotion, health graphics and death/promotion callees may wait.
    crate::execution::ExecutionContract {
        function: ids::PLAYER_UPDATE_HEALTH_AND_LIVES,
        class: TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // window_panels.c:107: fixed border RNG draws and ordered plane writes.
    crate::execution::ExecutionContract {
        function: ids::DRAW_MENU_WINDOW_FRAME,
        class: Functional,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // window_panels.c:135: stable ROM uploads then a fixed ordered plane fill.
    crate::execution::ExecutionContract {
        function: ids::DRAW_NESTED_WINDOW_FRAME,
        class: Functional,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // window_panels.c:176,200: blanking-service gates are plain RAM writes;
    // their stable ROM tilemaps have no clock-sensitive observation.
    crate::execution::ExecutionContract {
        function: ids::DRAW_PRESENT_DISPLAY_PANEL,
        class: Functional,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    crate::execution::ExecutionContract {
        function: ids::DRAW_PAUSE_CHOICE_PANEL,
        class: Functional,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // pause.c:23: input-release/selection polling and VBlank; the reset-selected
    // branch is a nonlocal reset, not an ordinary return or fake successful call.
    crate::execution::ExecutionContract {
        function: ids::RUN_FINAL_LEVEL_PAUSE_MENU,
        class: TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // pause.c:62: sprite readiness, VBlank, input transitions and optional reset.
    crate::execution::ExecutionContract {
        function: ids::GAME_PAUSED,
        class: TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // main_menu.c:31: menu, intro and Jam Out loops require input/IRQs. Its
    // missing incoming argument remains a call-frame prerequisite.
    crate::execution::ExecutionContract {
        function: ids::NEW_GAME_MAIN_MENU,
        class: TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // main_menu.c:101: input/timeout VBlank loop followed by nonlocal reset;
    // execution class is known, but no path returns through the C caller's RTS.
    crate::execution::ExecutionContract {
        function: ids::SHOW_GAME_OVER_AND_RESET,
        class: TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // main_menu.c:131,141,151,161: GeneralMenuHandler input/VBlank servicing.
    crate::execution::ExecutionContract {
        function: ids::MENU_WHO_HANDLER,
        class: TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    crate::execution::ExecutionContract {
        function: ids::MENU_CONTROLLER_OPTIONS_HANDLER,
        class: TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    crate::execution::ExecutionContract {
        function: ids::MENU_WHAT_HANDLER,
        class: TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    crate::execution::ExecutionContract {
        function: ids::MENU_JAM_OUT_HANDLER,
        class: TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // main_menu.c:210: input release/selection polling, VBlank and sprite waits.
    crate::execution::ExecutionContract {
        function: ids::GENERAL_MENU_HANDLER,
        class: TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // main_menu.c:283: stable menu record/string reads and ordered VDP writes.
    crate::execution::ExecutionContract {
        function: ids::MENU_DRAW_TEXT_ITEMS,
        class: Functional,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // main_menu.c:334: stable ROM palette upload plus RAM-only entity allocation.
    crate::execution::ExecutionContract {
        function: ids::ADD_MENU_HEADER,
        class: Functional,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
];
