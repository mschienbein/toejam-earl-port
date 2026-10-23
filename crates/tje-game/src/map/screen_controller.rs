use crate::call_frame::{CallSite, FrameLayout, LocalFrame, OutgoingArguments};
use crate::{Context, FunctionId, Result, call_at, ids};
use tje_mem::{HardwareSprite, Revision, ram, rom};

// Matching c_map_drawing_and_controller.s:1558-1802 (REV 02). REV 00
// omits the sixteen-byte idle-counter reset; every call below follows that
// branch, so its linked return PC is sixteen bytes earlier.
fn screen_site(c: &dyn Context, rev02_return_pc: u32) -> CallSite {
    CallSite::new(
        rev02_return_pc
            - if c.revision() == Revision::Rev00 {
                16
            } else {
                0
            },
    )
}

fn screen_call(
    c: &mut dyn Context,
    id: FunctionId,
    args: &[u32],
    rev02_return_pc: u32,
) -> Result<u32> {
    let site = screen_site(c, rev02_return_pc);
    call_at(c, id, args, site)
}

// MCC68K batches outgoing areas across adjacent calls. Keep each area live until
// the matching ADDQ/LEA cleanup; later pointer-taking callees see the real SP.
fn retain_call(
    c: &mut dyn Context,
    id: FunctionId,
    args: &[u32],
    rev02_return_pc: u32,
) -> Result<u32> {
    let outgoing = OutgoingArguments::push(c, args)?;
    let site = screen_site(c, rev02_return_pc);
    let dispatcher = c.dispatcher();
    outgoing.call(c, dispatcher, id, site)
}

fn release_arguments(c: &mut dyn Context, slots: u32) -> Result<()> {
    OutgoingArguments::existing(c, slots)?.release(c)
}

/// G/src/map/map_drawing_and_controller.c:501 (`DrawGameMap`).
/// The 80-sprite table is emulated RAM at A6-640, not a host array: the matching
/// prologue is LINK A6,#-640 then MOVEM D2-D3/A2-A4 in both revisions. All
/// pointer/DMA calls retain the original SP and linked return PC. Input waits
/// use the real VBlank/input services, never an invented dismissal or ready bit.
pub fn draw_game_map(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(c, FrameLayout::link(640, 0x0c, 0x1c))?;
    let r = c.revision();
    let player = i32::from(args[0] as i16);
    let sprites = frame.local::<HardwareSprite>(-640);
    let mut map_number = i16::from(player == 1 && ram::g_split_screen_enabled(r).read(c) != 0);
    c.set_data_register(
        2,
        (c.data_register(2) & 0xffff_0000) | u32::from(player as u16),
    );
    c.set_address_register(2, ids::WAIT_FOR_NEXT_V_BLANK.metadata().address(r));
    c.set_address_register(3, ram::g_mode2_reg_softcopy(r).address());
    c.set_address_register(4, ids::COPY_PALETTE_TO_CRAM_SLOT.metadata().address(r));
    if r == Revision::Rev02 {
        ram::g_player_idle_ticks(r).at(player).write(c, 0);
    }
    let pending = ram::g_sprite_graphics_upload_pending(r);
    if pending.read(c) != 0 {
        super::waits::wait_pending_byte(c, pending.cast())?;
    } else {
        screen_call(c, ids::WAIT_FOR_NEXT_V_BLANK, &[1], 0x888e)?;
    }
    screen_call(c, ids::DISABLE_BLANKING_GRAPHICS, &[], 0x8896)?;
    let mode = ram::g_mode2_reg_softcopy(r);
    mode.write(c, mode.read(c) & 0x3c);
    c.write_vdp_control(0x8104 | u16::from(mode.read(c)));
    c.write_vdp_command(0x4020_0000);
    let count = i32::from(rom::map_screen_tile_count(r).read(c)) << 4;
    retain_call(
        c,
        ids::MOVE_DATA_TO_VRAM,
        &[rom::map_screen_tiles(r).address(), count as u32],
        0x88ce,
    )?;
    retain_call(
        c,
        ids::MAP_INITIALIZE_SCREEN_SPRITES,
        &[sprites.address()],
        0x88d6,
    )?;
    retain_call(
        c,
        ids::MAP_DRAW_BACKGROUND_AND_TERRAIN,
        &[i32::from(map_number) as u32],
        0x88e0,
    )?;
    retain_call(
        c,
        ids::LOAD_MAP_INTO_VRAM,
        &[i32::from(map_number) as u32, sprites.address()],
        0x88ee,
    )?;
    release_arguments(c, 6)?;
    c.write_vdp_command(0x4002_0010);
    c.write_vdp_data(8);
    c.write_vdp_command(0x7c02_0003);
    c.write_vdp_data((-8_i16) as u16);
    c.write_vdp_control(0x911e);
    c.write_vdp_control(0x921c);
    for (palette, slot, pc) in [
        (rom::palette_map_line1(r).address(), 0, 0x8930),
        (rom::palette_map_line2(r).address(), 1, 0x893c),
        (rom::palette_map_line3(r).address(), 2, 0x8948),
    ] {
        retain_call(c, ids::COPY_PALETTE_TO_CRAM_SLOT, &[palette, slot], pc)?;
    }
    // The original uses a word-width player*128 address before reading level.
    map_number = i16::from(
        ram::g_player_info(r)
            .at(0)
            .byte_offset(i32::from((player as i16).wrapping_mul(128)))
            .level()
            .read(c),
    );
    retain_call(
        c,
        ids::MAP_DRAW_PLAYER,
        &[i32::from(map_number) as u32, sprites.address()],
        0x8966,
    )?;
    retain_call(
        c,
        ids::MAP_DRAW_SHIP_PIECE,
        &[i32::from(map_number) as u32, sprites.offset(2).address()],
        0x8974,
    )?;
    retain_call(
        c,
        ids::MAP_DRAW_ELEVATOR,
        &[i32::from(map_number) as u32, sprites.offset(6).address()],
        0x8984,
    )?;
    retain_call(
        c,
        ids::MAP_UPLOAD_COVER_ANIMATION_FRAME,
        &[sprites.address(), 11, 1],
        0x8994,
    )?;
    retain_call(
        c,
        ids::MAP_UPLOAD_COVER_ANIMATION_FRAME,
        &[sprites.address(), 0, 0],
        0x89a0,
    )?;
    retain_call(
        c,
        ids::DMA_COPY_WORDS_TO_VRAM,
        &[sprites.address(), 0xf800, 320],
        0x89b4,
    )?;
    mode.write(c, mode.read(c) | 0x40);
    release_arguments(c, 21)?;
    c.write_vdp_control(0x8104 | u16::from(mode.read(c)));
    if ram::g_map_reveal_mode(r).read(c) != 0 {
        for animation in 1..12 {
            retain_call(c, ids::WAIT_FOR_NEXT_V_BLANK, &[4], 0x89de)?;
            retain_call(
                c,
                ids::MAP_UPLOAD_COVER_ANIMATION_FRAME,
                &[sprites.address(), animation, 0],
                0x89ee,
            )?;
            release_arguments(c, 4)?;
        }
    }
    screen_call(c, ids::PRE_GENERATE_RANDOMS, &[], 0x8a00)?;
    while screen_call(c, ids::READ_MAPPED_PLAYER_INPUT, &[player as u32], 0x8a16)? & 0x20 != 0 {
        screen_call(c, ids::WAIT_FOR_NEXT_V_BLANK, &[1], 0x8a08)?;
    }
    let buttons = loop {
        let buttons =
            screen_call(c, ids::READ_MAPPED_PLAYER_INPUT, &[player as u32], 0x8a36)? as i16;
        if buttons & 0x70 != 0 {
            break buttons & 0x70;
        }
        screen_call(c, ids::DRAW_MAP_CHARACTERS, &[sprites.address()], 0x8a28)?;
    };
    while screen_call(c, ids::READ_MAPPED_PLAYER_INPUT, &[player as u32], 0x8a5e)?
        & i32::from(buttons) as u32
        != 0
    {
        screen_call(c, ids::WAIT_FOR_NEXT_V_BLANK, &[1], 0x8a50)?;
    }
    mode.write(c, mode.read(c) & 0x3c);
    c.write_vdp_control(0x8104 | u16::from(mode.read(c)));
    screen_call(c, ids::DRAW_STARFIELD_PLANE, &[], 0x8a84)?;
    screen_call(
        c,
        ids::DMA_COPY_WORDS_TO_VRAM,
        &[ram::g_hardware_sprite_table(r).address(), 0xf800, 320],
        0x8a9a,
    )?;
    if ram::g_map_reveal_mode(r).read(c) != 0 || buttons & 0x20 != 0 {
        c.write_vdp_command(0x4000_0000);
        let count = (i32::from(rom::map_screen_tile_count(r).read(c)) + 41) << 4;
        screen_call(
            c,
            ids::MOVE_DATA_TO_VRAM,
            &[rom::level_tiles(r).address(), count as u32],
            0x8ad4,
        )?;
        c.write_vdp_control(0x9100);
        c.write_vdp_control(0x9200);
        screen_call(c, ids::DRAW_HUD, &[], 0x8aec)?;
        for (palette, slot, pc) in [
            (rom::palette_game_line1(r).address(), 0, 0x8af6),
            (rom::palette_game_line2(r).address(), 1, 0x8b02),
            (rom::palette_game_line3(r).address(), 2, 0x8b0e),
        ] {
            retain_call(c, ids::COPY_PALETTE_TO_CRAM_SLOT, &[palette, slot], pc)?;
        }
        let partner_open = ram::g_player_inventory_open(r).at(player ^ 1).read(c) != 0;
        release_arguments(c, 6)?;
        if partner_open {
            screen_call(
                c,
                ids::INVENTORY_REDRAW_SYNCHRONOUSLY,
                &[(player ^ 1) as u32],
                0x8b36,
            )?;
        }
    } else {
        screen_call(
            c,
            ids::SHOW_SHIP_PIECES,
            &[(-1_i32) as u32, player as u32],
            0x8b4a,
        )?;
    }
    mode.write(c, mode.read(c) | 0x40);
    c.write_vdp_control(0x8104 | u16::from(mode.read(c)));
    screen_call(c, ids::ENABLE_BLANKING_GRAPHICS, &[], 0x8b68)?;
    frame.leave(c)?;
    Ok(None)
}
