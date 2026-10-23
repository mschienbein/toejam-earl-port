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
];
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
