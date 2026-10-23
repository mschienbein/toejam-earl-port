use crate::{Context, Result, call, ids};
use tje_mem::{ram, rom};

/// G/src/core/startup_and_main_loop.c:88 (`DrawStarfieldPlane`, REV 02 0x00046A).
/// Functional: preserve signed-char extension and both repeated word writes;
/// transaction completion is a platform contract, not a fabricated FIFO-ready bit.
pub fn draw_starfield_plane(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    context.write_vdp_control(0x8f40);
    for row in 0..32 {
        for column in 0..32 {
            let source = rom::starfield_byte_map(revision)
                .at(row)
                .at(column)
                .read(context);
            let tile = ((i32::from(source) & 0x087f) + 0x0200) as u16;
            let command = 0x60000003_u32
                .wrapping_add((row as u32) << 23)
                .wrapping_add((column as u32) << 17);
            context.write_vdp_command(command);
            context.write_vdp_data(tile);
            context.write_vdp_data(tile);
        }
    }
    context.write_vdp_control(0x8f02);
    Ok(None)
}

/// G/src/core/startup_and_main_loop.c:157 (`SelectViewportSpriteChain`,
/// REV 02 0x00059E). Keep the original short-circuit inventory/player tests.
pub fn select_viewport_sprite_chain(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let player = arguments[0] as i16;
    let selected = if ram::g_split_screen_enabled(revision).read(context) != 0 {
        if player == 0 {
            ids::SELECT_PRIMARY_VIEWPORT_SPRITES
        } else {
            ids::SELECT_SECONDARY_VIEWPORT_SPRITES
        }
    } else if (player == 0 && ram::g_player_inventory_open(revision).at(0).read(context) != 0)
        || (player == 1 && ram::g_player_inventory_open(revision).at(1).read(context) != 0)
    {
        ids::SELECT_SECONDARY_VIEWPORT_SPRITES
    } else if player == 0 || ram::g_player_inventory_open(revision).at(0).read(context) != 0 {
        ids::SELECT_PRIMARY_VIEWPORT_SPRITES
    } else {
        ids::TERMINATE_PRIMARY_SPRITE_CHAIN
    };
    call(context, selected, &[])?;
    Ok(None)
}

/// G/src/core/startup_and_main_loop.c:258 (`HandleHBlank`, REV 02 0x000858).
/// Functional logic proof only: TIMING.md's line-112/113 publication remains a
/// timed scheduler obligation. Signed divisions truncate toward zero.
pub fn handle_hblank(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    if ram::g_h_blank_graphics_enabled(revision).read(context) != 0
        && ram::g_player1_map_level(revision).read(context) < 26
    {
        call(context, ids::SELECT_VIEWPORT_SPRITE_CHAIN, &[1])?;
        if ram::g_split_screen_enabled(revision).read(context) != 0
            || ram::g_player2_inventory_open(revision).read(context) != 0
        {
            if ram::g_player2_inventory_open(revision).read(context) != 0 {
                context.write_vdp_control(0x928e);
            } else {
                context.write_vdp_control(0x8230);
                context.write_vdp_command(0x40000010);
                let y = i32::from(ram::g_camera2_committed_y(revision).read(context)) - 112;
                context.write_vdp_data(y as u16);
                context.write_vdp_command(0x7c000003);
                let x = -i32::from(ram::g_camera2_committed_x(revision).read(context));
                context.write_vdp_data(x as u16);
                context.write_vdp_command(0x40020010);
                let y = i32::from(ram::g_camera2_committed_y(revision).read(context)) / 2 - 112;
                context.write_vdp_data(y as u16);
                context.write_vdp_command(0x7c020003);
                let x = -i32::from(ram::g_camera2_committed_x(revision).read(context)) / 2;
                context.write_vdp_data(x as u16);
            }
        }
        context.write_vdp_control(0x8aff);
    }
    Ok(None)
}

/// G/src/core/startup_and_main_loop.c:282 (`GetPlayerInput`, REV 02 0x000938).
/// Copy previous input before mapping each living player's controller, then
/// apply the reaction and mutually exclusive movement-group masks in order.
pub fn get_player_input(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    for player in 0..2 {
        let entity = ram::g_player_info(revision).at(player);
        let input = ram::g_player_processed_input(revision).at(player);
        context.set_data_register(0, player as u32);
        context.set_data_register(1, player as u32);
        context.set_address_register(
            0,
            ram::g_player_previous_processed_input(revision).address(),
        );
        let previous = input.read(context);
        ram::g_player_previous_processed_input(revision)
            .at(player)
            .write(context, previous);
        context.set_address_register(0, ram::g_player_lives(revision).address());
        if ram::g_player_lives(revision).at(player).read(context) >= 0 {
            let mapped = call(context, ids::READ_MAPPED_PLAYER_INPUT, &[player as u32])?;
            input.write(context, mapped as u8);
            let reaction = entity.animation_group_mask().read(context) & 0x00020000;
            context.set_data_register(0, reaction);
            if reaction != 0 || matches!(entity.sprite_state().read(context), 79 | 78 | 5 | 6) {
                context.set_data_register(0, player as u32);
                let masked = input.read(context) & 0x80;
                input.write(context, masked);
            }
        } else {
            context.set_data_register(0, player as u32);
            input.write(context, 0);
        }
        let stationary = entity.animation_group_mask().read(context) & 0x20000000;
        context.set_data_register(0, stationary);
        if stationary != 0 {
            context.set_data_register(0, player as u32);
            let masked = input.read(context) & 0x8f;
            input.write(context, masked);
        } else {
            let falling = entity.animation_group_mask().read(context) & 0x60;
            context.set_data_register(1, 0x60);
            context.set_data_register(0, falling);
            if falling != 0 {
                context.set_data_register(0, player as u32);
                let masked = input.read(context) & 0xcf;
                input.write(context, masked);
            } else {
                let skates = entity.animation_group_mask().read(context) & 0x07800000;
                context.set_data_register(0, skates);
                if skates != 0 || entity.sprite_state().read(context) == 2 {
                    context.set_data_register(0, player as u32);
                    let masked = input.read(context) & 0xef;
                    input.write(context, masked);
                }
            }
        }
    }
    // Final ADDQ.W makes the local two, then CMPI.W #1 leaves NZVC clear.
    // The addition also clears X; MOVEM and RTS preserve those flags. These
    // emitted scratch outputs survive into FurtherInitialisation's typed exit.
    let status = context.status_register() & !0x001f;
    context.set_status_register(status);
    Ok(None)
}

/// G/src/core/startup_and_main_loop.c:572 (`InitialiseTheZ80`, REV 02 0x0011D8).
/// Preserve reset/bus ownership and the six explicit bootstrap byte writes from
/// C. DelayIterations is the original finite countdown, not a BUSACK poll.
pub fn initialise_the_z80(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    context.write_z80_reset(0);
    call(context, ids::DELAY_ITERATIONS, &[10])?;
    context.write_z80_bus_request(0x100);
    call(context, ids::DELAY_ITERATIONS, &[10])?;
    context.write_z80_reset(0x100);
    call(context, ids::DELAY_ITERATIONS, &[10])?;
    for (offset, value) in [0xed, 0x56, 0xf3, 0xc3, 3, 0].into_iter().enumerate() {
        context.write_z80_ram(offset as u16, value);
    }
    context.write_z80_reset(0);
    call(context, ids::DELAY_ITERATIONS, &[10])?;
    context.write_z80_bus_request(0);
    call(context, ids::DELAY_ITERATIONS, &[10])?;
    context.write_z80_reset(0x100);
    call(context, ids::DELAY_ITERATIONS, &[10])?;
    Ok(None)
}
