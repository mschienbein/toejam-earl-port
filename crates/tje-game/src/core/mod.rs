//! Core translation ownership; see docs/PORTING.md.

use crate::execution::ExecutionClass;
use crate::{Context, Result};
use tje_mem::ram;

mod control;
mod input;
mod main_loop;
mod remaining;
pub mod timed_wait;
mod vblank;
mod views;

/// Available Rust entries owned by this subsystem.
pub static TRANSLATIONS: &[crate::Translation] = &[
    crate::Translation {
        id: crate::ids::DISABLE_BLANKING_GRAPHICS,
        function: disable_blanking_graphics,
    },
    crate::Translation {
        id: crate::ids::ENABLE_BLANKING_GRAPHICS,
        function: enable_blanking_graphics,
    },
    crate::Translation {
        id: crate::ids::GET_V_BLANK_GRAPHICS_ENABLED,
        function: get_vblank_graphics_enabled,
    },
    crate::Translation {
        id: crate::ids::READ_MAPPED_PLAYER_INPUT,
        function: read_mapped_player_input,
    },
    crate::Translation {
        id: crate::ids::GET_LEVEL_MUSIC_INDEX,
        function: music_hook,
    },
    crate::Translation {
        id: crate::ids::RESET_VDP,
        function: reset_vdp,
    },
    crate::Translation {
        id: crate::ids::DRAW_LEVEL_GRAPHICS,
        function: draw_level_graphics,
    },
    crate::Translation {
        id: crate::ids::PLAYERS_REQUIRE_SEPARATE_VIEWS,
        function: players_require_separate_views,
    },
    crate::Translation {
        id: crate::ids::DELAY_ITERATIONS,
        function: delay_iterations,
    },
    crate::Translation {
        id: crate::ids::GET_PLAYER_INPUT,
        function: remaining::get_player_input,
    },
    crate::Translation {
        id: crate::ids::INITIALISE_THE_Z80,
        function: remaining::initialise_the_z80,
    },
    crate::Translation {
        id: crate::ids::DRAW_STARFIELD_PLANE,
        function: remaining::draw_starfield_plane,
    },
    crate::Translation {
        id: crate::ids::HANDLE_H_BLANK,
        function: remaining::handle_hblank,
    },
    crate::Translation {
        id: crate::ids::ERROR_TRAP,
        function: control::error_trap,
    },
    crate::Translation {
        id: crate::ids::VDP_INITIALISATION,
        function: control::vdp_initialisation,
    },
    crate::Translation {
        id: crate::ids::RELOAD_ACTIVE_CAMERA_VIEWS,
        function: views::reload_active_camera_views,
    },
];

/// Reviewed original entries that return without clock/interrupt progress.
/// Missing IDs are deliberately not approved for atomic checking or fallback.
pub static ATOMIC_CAPABLE: &[crate::FunctionId] = &[
    crate::ids::DISABLE_BLANKING_GRAPHICS,
    crate::ids::ENABLE_BLANKING_GRAPHICS,
    crate::ids::GET_V_BLANK_GRAPHICS_ENABLED,
    crate::ids::READ_MAPPED_PLAYER_INPUT,
    crate::ids::GET_LEVEL_MUSIC_INDEX,
    crate::ids::RESET_VDP,
    crate::ids::DRAW_LEVEL_GRAPHICS,
    crate::ids::PLAYERS_REQUIRE_SEPARATE_VIEWS,
    crate::ids::DELAY_ITERATIONS,
    // ibid.:282: two RAM records and ReadMappedPlayerInput, no device accesses.
    crate::ids::GET_PLAYER_INPUT,
    // ibid.:572: fixed Z80 reset/bus/RAM writes and six finite DelayIterations
    // calls. No BUSACK/mailbox polling and no clock-dependent return condition.
    crate::ids::INITIALISE_THE_Z80,
];

/// Reviewed original non-atomic execution contracts, including untranslated
/// entries. These are logic classifications, not raster/cycle equivalence claims.
pub static EXECUTION_CONTRACTS: &[crate::execution::ExecutionContract] = &[
    // src/core/error_trap.c:5: a declared terminal, never an RTS. The scheduler
    // must retain real device clocks and IRQ service after the terminal branch.
    crate::execution::ExecutionContract {
        function: crate::ids::ERROR_TRAP,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // src/core/startup_and_main_loop.c:58,607: initialization transfers into the
    // clocked persistent main loop; these declarations require typed exits.
    crate::execution::ExecutionContract {
        function: crate::ids::VDP_INITIALISATION,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    crate::execution::ExecutionContract {
        function: crate::ids::FURTHER_INITIALISATION,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // src/core/startup_and_main_loop.c:88: 2048 ordered VDP data writes, no
    // status/HV reads, unstable source, interrupt wait, or concurrent observation.
    crate::execution::ExecutionContract {
        function: crate::ids::DRAW_STARFIELD_PLANE,
        class: ExecutionClass::Functional,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // ibid.:157 and src/sprites/sprite_links_vdp.c:13-35: bounded RAM
    // selection plus ordered VDP writes, no status/HV read or polling loop.
    // Genuine entries with three occupied FIFO slots require transaction
    // progress for the selected two-word writer; frozen atomic is insufficient.
    crate::execution::ExecutionContract {
        function: crate::ids::SELECT_VIEWPORT_SPRITE_CHAIN,
        class: ExecutionClass::Functional,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // ibid.:142: waits for an IRQ-driven RAM frame-counter change.
    crate::execution::ExecutionContract {
        function: crate::ids::WAIT_FOR_NEXT_V_BLANK,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // ibid.:175: installed callbacks and music/controller service include Z80
    // mailbox/BUSY waits; IRQ execution must retain real device progress.
    crate::execution::ExecutionContract {
        function: crate::ids::HANDLE_V_BLANK,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // ibid.:258: at most six data writes including the selected sprite-chain
    // callee. No raster/status read; precise IRQ4 publication remains a separate
    // scheduler obligation documented in docs/TIMING.md.
    crate::execution::ExecutionContract {
        function: crate::ids::HANDLE_H_BLANK,
        class: ExecutionClass::Functional,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // ibid.:307: GamePaused can wait for controller/frame changes.
    crate::execution::ExecutionContract {
        function: crate::ids::CHECK_START_BUTTON,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // ibid.:321: pause, inventory, and map callees have interactive frame waits.
    crate::execution::ExecutionContract {
        function: crate::ids::PROCESS_INPUT,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // ibid.:368: SetCameraPosition -> SetCameraViewOrigin -> bounded RAM row
    // building and VDP uploads; stable per-row source and no hardware reads/waits.
    crate::execution::ExecutionContract {
        function: crate::ids::RELOAD_ACTIVE_CAMERA_VIEWS,
        class: ExecutionClass::Functional,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // ibid.:376: waits on the IRQ-cleared sprite-upload byte and optional wipes.
    crate::execution::ExecutionContract {
        function: crate::ids::ENTER_SPLIT_SCREEN,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // ibid.:415: WaitForVBlankWithGraphicsPaused plus ordered camera/HUD uploads.
    crate::execution::ExecutionContract {
        function: crate::ids::ENTER_SHARED_SCREEN,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // ibid.:460: WaitForVBlankWithGraphicsPaused when display is enabled.
    crate::execution::ExecutionContract {
        function: crate::ids::CENTER_SHARED_CAMERA_ON_PLAYER,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // ibid.:514: invokes the split/shared/centering waits above.
    crate::execution::ExecutionContract {
        function: crate::ids::UPDATE_SCREEN_SPLIT_MODE,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
];

// FurtherInitialisation retains its original twenty-byte frame across modal
// call transfers and Begin/Idle continuations; its complete staged body remains
// unregistered until every finite checkpoint is proven in both revisions.
/// Complete bodies not approved for production replacement.
pub static PENDING: &[crate::FunctionId] = &[
    // Full corpus: REV 00 frame 74603 and REV 02 frame 31361 enter with only
    // one FIFO slot free; the two-word sprite writer requires transaction
    // progress. Functional classification is not joint/current-corpus proof.
    crate::ids::SELECT_VIEWPORT_SPRITE_CHAIN,
    // Full corpus: content-later REV 00 and content-effects REV 02 expose
    // live FF8033 differences at frame 837; earlier narrow sessions were
    // insufficient. Keep the body unregistered until that effect is proven.
    crate::ids::WAIT_FOR_NEXT_V_BLANK,
    // Full corpus: cooperation frame 943/971 changes real HBlank/VBlank
    // publication order versus main-thread stores. No timing waiver applies.
    crate::ids::PROCESS_INPUT,
    // Full corpus: cooperation pause paths at frames 1640/2180 change real
    // HBlank/VDP ordering. Held/no-new-press proof does not approve these paths.
    crate::ids::CHECK_START_BUTTON,
    // Real display-on paths observe different IRQ placement/frame counts
    // with zero-time translated work; quiet finite fixtures are not a waiver.
    crate::ids::ENTER_SPLIT_SCREEN,
    crate::ids::ENTER_SHARED_SCREEN,
    crate::ids::CENTER_SHARED_CAMERA_ON_PLAYER,
    crate::ids::UPDATE_SCREEN_SPLIT_MODE,
    // The ISR root has no nested Enter marker for the YM-busy projection;
    // its natural frame-17 raw A04002 read differs (0 versus 128).
    crate::ids::HANDLE_V_BLANK,
    // Source-backed non-RTS initialization and all continuation bodies await
    // prepared-entry/stage proof, never an invented RTS or fixed RNG policy.
    crate::ids::FURTHER_INITIALISATION,
];

/// Unregistered bodies retained for explicit proof harnesses.
pub static PENDING_TRANSLATIONS: &[crate::Translation] = &[
    crate::Translation {
        id: crate::ids::SELECT_VIEWPORT_SPRITE_CHAIN,
        function: remaining::select_viewport_sprite_chain,
    },
    crate::Translation {
        id: crate::ids::WAIT_FOR_NEXT_V_BLANK,
        function: wait_for_next_vblank,
    },
    crate::Translation {
        id: crate::ids::PROCESS_INPUT,
        function: input::process_input,
    },
    crate::Translation {
        id: crate::ids::CHECK_START_BUTTON,
        function: input::check_start_button,
    },
    crate::Translation {
        id: crate::ids::ENTER_SPLIT_SCREEN,
        function: views::enter_split_screen,
    },
    crate::Translation {
        id: crate::ids::ENTER_SHARED_SCREEN,
        function: views::enter_shared_screen,
    },
    crate::Translation {
        id: crate::ids::CENTER_SHARED_CAMERA_ON_PLAYER,
        function: views::center_shared_camera_on_player,
    },
    crate::Translation {
        id: crate::ids::UPDATE_SCREEN_SPLIT_MODE,
        function: views::update_screen_split_mode,
    },
    crate::Translation {
        id: crate::ids::HANDLE_V_BLANK,
        function: vblank::handle_vblank,
    },
    crate::Translation {
        id: crate::ids::FURTHER_INITIALISATION,
        function: main_loop::further_initialisation,
    },
];

/// Source-backed native frame continuations; only the scheduler consumes the
/// typed stages. This bundle is not a production function registration.
pub static MAIN_LOOP_BODIES: crate::exits::MainLoopBodies = main_loop::BODIES;

/// G/src/core/startup_and_main_loop.c:142 (`WaitForNextVBlank`, REV 02 0x000578).
/// TimeAware: suspend until the actual IRQ-updated counter changes. This wait
/// does not perform the separate idle RNG draws in FurtherInitialisation.
pub fn wait_for_next_vblank(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    if ram::g_mode2_reg_softcopy(revision).read(context) & 0x40 != 0 {
        let mut count = arguments[0] as i16;
        while count > 0 {
            let counter = ram::g_frame_counter(revision);
            let previous = counter.read(context);
            context.wait_vblank_frame(counter.address(), previous)?;
            count -= 1;
        }
    }
    Ok(None)
}

/// G/src/core/startup_and_main_loop.c:142 (`WaitForNextVBlank`).
/// Canonical clocked, prepared-entry candidate for the same source function.
/// It is deliberately not registered until value callers migrate atomically.
pub fn wait_for_next_vblank_framed(context: &mut dyn Context) -> Result<Option<u32>> {
    let entry = crate::ids::WAIT_FOR_NEXT_V_BLANK;
    let frame = context
        .frame_state()
        .ok_or(crate::call_frame::FrameError::Unavailable)?;
    if !frame.is_prepared_for(entry) || context.stack_pointer()? != frame.entry_sp {
        return Err(crate::call_frame::FrameError::PreparedEntryRequired(entry).into());
    }
    let mut source = timed_wait::WaitSource::new(context.revision());
    Ok(context.execute_timing_program(entry, &mut source)?.value)
}
/// G/src/core/startup_and_main_loop.c:33 (`ResetVDP`, REV 02 0x00035C).
/// Twenty ordered register writes; there is no DMA start, status read, or wait.
pub fn reset_vdp(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    // Every original literal MOVE.W is negative: N=1, Z/V/C=0, X unchanged.
    // Preserve this defined callee result for non-RTS transfer boundaries too.
    let status = context.status_register();
    context.set_status_register((status & !0x000f) | 0x0008);
    for value in [
        0x8004, 0x8144, 0x8228, 0x8334, 0x8407, 0x857c, 0x8700, 0x8a00, 0x8b00, 0x8c89, 0x8d3f,
        0x8f02, 0x9001, 0x9100, 0x9200, 0x9302, 0x9400, 0x9500, 0x9600, 0x9700,
    ] {
        context.write_vdp_control(value);
    }
    Ok(None)
}

/// G/src/core/startup_and_main_loop.c:105 (`DrawLevelGraphics`, REV 02 0x0004DC).
/// Four CRAM writes at most: atomic checking requires four free FIFO entries.
/// MCC68K tests SUBQ.B's signed flags before its overflowed byte is reloaded.
pub fn draw_level_graphics(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let delay = ram::g_level_palette_animation_delay(revision);
    let previous_delay = delay.read(context);
    delay.write(context, previous_delay.wrapping_sub(1));
    if previous_delay <= 0 {
        delay.write(context, 3);
        let frame = ram::g_level_palette_animation_frame(revision);
        let next_frame = ((i32::from(frame.read(context)) + 1) & 3) as i8;
        frame.write(context, next_frame);
        context.write_vdp_command(0xc0340000);
        for colour in 0..4 {
            let palette = if ram::g_player1_map_level(revision).read(context) < 26 {
                tje_mem::rom::palette_animation_frames_a(revision)
            } else {
                tje_mem::rom::palette_animation_frames_b(revision)
            };
            let value = palette
                .at(i32::from(frame.read(context)))
                .at(colour)
                .read(context);
            context.write_vdp_data(value as u16);
        }
    }
    Ok(None)
}

/// G/src/core/startup_and_main_loop.c:489 (`PlayersRequireSeparateViews`,
/// REV 02 0x000FB6). Keep word subtraction/addition and MCC68K's SUB.W/BGE
/// predicate, including signed overflow before narrowing the absolute distance.
pub fn players_require_separate_views(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    if ram::g_player_lives(revision).at(0).read(context) < 0
        || ram::g_player2_lives(revision).read(context) < 0
    {
        return Ok(Some(0));
    }
    let toejam = ram::g_player_info(revision).at(0);
    let earl = ram::g_player_info(revision).at(1);
    if toejam.level().read(context) != earl.level().read(context) {
        return Ok(Some(1));
    }
    let toejam_y = toejam.y().read(context);
    let earl_y = earl.y().read(context);
    let mut distance_y = toejam_y.wrapping_sub(earl_y);
    if toejam_y < earl_y {
        distance_y = distance_y.wrapping_neg();
    }
    let toejam_x = toejam.x().read(context);
    let earl_x = earl.x().read(context);
    let mut distance_x = toejam_x.wrapping_sub(earl_x);
    if toejam_x < earl_x {
        distance_x = distance_x.wrapping_neg();
    }
    if toejam.height().read(context) > 0
        && toejam.animation_group_mask().read(context) & 0x04008000 != 0
        && toejam.y().read(context) < earl.y().read(context)
    {
        distance_y = distance_y.wrapping_add(toejam.height().read(context));
    } else if earl.height().read(context) > 0
        && earl.animation_group_mask().read(context) & 0x04008000 != 0
        && earl.y().read(context) < toejam.y().read(context)
    {
        distance_y = distance_y.wrapping_add(earl.height().read(context));
    }
    Ok(Some(u32::from(distance_y > 124 || distance_x > 220)))
}

/// G/src/core/startup_and_main_loop.c:564 (`DelayIterations`, REV 02 0x0011CA).
/// A finite wrapping word countdown, not a peripheral/interrupt wait. Negative
/// counts reach zero after word wraparound, exactly as SUBQ.W/TST.W/BNE does.
pub fn delay_iterations(_: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let mut iterations = arguments[0] as i16;
    while iterations != 0 {
        iterations = iterations.wrapping_sub(1);
    }
    Ok(None)
}
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
    // Emitted c_startup_and_main_loop.s:61-77 leaves the full zero-extended
    // result in D1 before copying it to D0. Non-RTS callers retain that residue.
    context.set_data_register(1, mapped);
    Ok(Some(mapped))
}
/// G/src/core/startup_and_main_loop.c: GetLevelMusicIndex (REV 02 0x001262).
/// The subtraction is promoted to 32-bit int before the signed remainder.
pub fn get_level_music_index(selection: i16) -> i16 {
    ((i32::from(selection) - 1) % 6) as i16
}
/// G/src/core/startup_and_main_loop.c: GetLevelMusicIndex, MCC68K entry bridge.
pub fn music_hook(_: &mut dyn crate::Context, arguments: &[u32]) -> crate::Result<Option<u32>> {
    Ok(Some(
        i32::from(get_level_music_index(arguments[0] as i16)) as u32
    ))
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod remaining_tests;

#[cfg(test)]
mod caller_tests;

#[cfg(test)]
pub(crate) mod test_support;
