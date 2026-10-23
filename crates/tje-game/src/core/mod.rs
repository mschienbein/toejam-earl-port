//! Core translation ownership; see docs/PORTING.md.

use crate::{Context, Result};
use tje_mem::ram;

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
];

// ErrorTrap (src/core/error_trap.c:5) never returns. WaitForNextVBlank
// (src/core/startup_and_main_loop.c:142) needs interrupt-driven frame changes.
// DrawStarfieldPlane (ibid.:88) writes 2048 data words, requiring FIFO progress.
// None can execute through the frozen-clock atomic bridge.

/// G/src/core/startup_and_main_loop.c:33 (`ResetVDP`, REV 02 0x00035C).
/// Twenty ordered register writes; there is no DMA start, status read, or wait.
pub fn reset_vdp(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
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
pub(crate) mod test_support;
