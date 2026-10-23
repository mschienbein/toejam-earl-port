use super::large_player_parts::entity_at;
use crate::{Context, Result, call, ids};
use tje_mem::{Ptr, ram, rom};

/// G/src/cutscenes/sega_logo.c:24 (`SegaLogoClearVideoPlanes`).
/// Functional: a fixed, stable-ROM VDP transaction stream, with no status or
/// raster observations; its 4,176 data words cannot fit a frozen hardware FIFO.
pub fn sega_logo_clear_video_planes(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    call(context, ids::CLEAR_COLOUR_RAM, &[])?;
    context.write_vdp_command(0x7fe0_0001);
    call(
        context,
        ids::MOVE_DATA_TO_VRAM,
        &[rom::vram_clearing_data(revision).address(), 16],
    )?;
    context.write_vdp_command(0x6000_0002);
    call(context, ids::FILL_VDP_DATA_WORDS, &[0xe3ff, 2048])?;
    context.write_vdp_command(0x6000_0003);
    call(context, ids::FILL_VDP_DATA_WORDS, &[0x63ff, 2048])?;
    Ok(None)
}

/// G/src/cutscenes/sega_logo.c:125 (`SegaLogoCreateTrademarkEntity`).
/// Atomic: fixed RAM stores and RAM-only viewport sprite allocation.
pub fn sega_logo_create_trademark_entity(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let selector = ram::g_sega_trademark_entity(revision);
    let allocated = call(context, ids::ALLOCATE_ENTITY, &[])? as u16;
    selector.write(context, allocated);
    if selector.read(context) != 0 {
        let trademark = entity_at(context, selector.read(context) as i16);
        trademark.tile().write(context, 0x8000);
        let frame = rom::spr_trademark(revision).at(0).read(context);
        trademark.current_frame().write(context, frame);
        trademark
            .frames()
            .write(context, Ptr::new(rom::spr_trademark(revision).address()));
        trademark.tile_count().write(context, 108);
        trademark.sprite_count().write(context, 10);
        trademark.x().write(context, 160);
        trademark.y().write(context, 166);
        trademark.sprite_ready().write(context, 0);
        trademark.sequence().write(
            context,
            Ptr::new(
                rom::intro_held_frame_streams(revision)
                    .at(0)
                    .frame()
                    .address(),
            ),
        );
        trademark.frame_duration().write(context, 1);
        trademark.frame().write(context, -1);
        trademark.frame_phase().write(context, 6);
        call(
            context,
            ids::ALLOCATE_ENTITY_VIEWPORT_SPRITES,
            &[trademark.address(), 0],
        )?;
    }
    Ok(None)
}

/// G/src/cutscenes/sega_logo.c:148 (`SegaLogoReleaseTrademarkEntity`).
/// Atomic: conditional RAM-only release, then clear the word-sized selector.
pub fn sega_logo_release_trademark_entity(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let selector = ram::g_sega_trademark_entity(context.revision());
    let slot = selector.read(context);
    if slot != 0 {
        call(context, ids::FREE_ENTITY, &[u32::from(slot)])?;
    }
    selector.write(context, 0);
    Ok(None)
}

/// G/src/cutscenes/sega_logo.c:65-73,104-112: the identical palette-advance
/// blocks use unsigned-preserving word promotion and a 16-colour pointer step.
fn advance_palette(context: &mut dyn Context, palette_time: &mut u16, palette_frame: &mut i16) {
    let revision = context.revision();
    if u32::from(ram::g_frame_counter(revision).read(context)) >= u32::from(*palette_time) + 3 {
        *palette_time = ram::g_frame_counter(revision).read(context);
        let source = ram::g_sega_palette_upload_source(revision);
        let next = source.read(context).address().wrapping_add(32);
        source.write(context, Ptr::new(next));
        *palette_frame = palette_frame.wrapping_add(1);
        if *palette_frame >= 20 {
            source.write(context, Ptr::new(rom::sega_logo_pal(revision).address()));
            *palette_frame = 0;
        }
        ram::g_sega_palette_upload_pending(revision).write(context, 1);
    }
}

/// G/src/cutscenes/sega_logo.c:36 (`SEGAVideo`).
/// TimeAware: inline frame-counter polling suspends on real IRQ completion, with
/// SegaLogoVBlank consuming palette/sprite publications. REV 02 alone resets the
/// counter before the unsigned palette deadline; no idle RNG draws are added.
pub fn sega_video(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let counter = ram::g_frame_counter(revision);
    ram::g_sega_palette_upload_pending(revision).write(context, 0);
    ram::g_sega_v_blank_sprite_updates_enabled(revision).write(context, 0);
    ram::g_v_blank_func(revision).write(
        context,
        Ptr::new(ids::SEGA_LOGO_V_BLANK.metadata().address(revision)),
    );
    call(context, ids::SEGA_LOGO_CLEAR_VIDEO_PLANES, &[])?;
    call(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_title_screen_bank0(revision).address(), 0],
    )?;
    call(context, ids::DRAW_STARFIELD_PLANE, &[])?;
    context.write_vdp_command(0x4000_0000);
    call(
        context,
        ids::MOVE_DATA_TO_VRAM,
        &[rom::splash_sega_logo(revision).address(), 784],
    )?;
    call(
        context,
        ids::UPLOAD_VRAM_WORD_RECTANGLE,
        &[
            0xa51c,
            rom::sega_logo_tile_map(revision).address(),
            12,
            4,
            64,
        ],
    )?;
    ram::g_sega_palette_upload_source(revision)
        .write(context, Ptr::new(rom::sega_logo_pal(revision).address()));
    ram::g_sega_palette_upload_pending(revision).write(context, 1);
    ram::g_sega_v_blank_sprite_updates_enabled(revision).write(context, 1);
    let frame = counter.read(context);
    context.wait_vblank_frame(counter.address(), frame)?;
    let mode = ram::g_mode2_reg_softcopy(revision).read(context) | 0x40;
    ram::g_mode2_reg_softcopy(revision).write(context, mode);
    let mode = ram::g_mode2_reg_softcopy(revision).read(context);
    context.write_device_u16(0xc00004, u16::from(mode) | 0x8104);
    if revision == tje_mem::Revision::Rev02 {
        counter.write(context, 0);
    }
    let mut palette_time = counter.read(context);
    let start_time = counter.read(context);
    let mut palette_frame = 0_i16;
    while u32::from(counter.read(context)) < u32::from(start_time) + 120 {
        advance_palette(context, &mut palette_time, &mut palette_frame);
        let frame = counter.read(context);
        context.wait_vblank_frame(counter.address(), frame)?;
    }
    if ram::g_sega_rocket_pass_shown(revision).read(context) == 0 {
        let frame = counter.read(context);
        context.wait_vblank_frame(counter.address(), frame)?;
        ram::g_sega_v_blank_sprite_updates_enabled(revision).write(context, 0);
        ram::g_sega_palette_upload_pending(revision).write(context, 0);
        call(context, ids::SEGA_LOGO_CREATE_TRADEMARK_ENTITY, &[])?;
        call(context, ids::CUTSCENE_ALLOCATE_ROCKETSHIP_PAIR, &[])?;
        call(
            context,
            ids::COPY_PALETTE_TO_CRAM_SLOT,
            &[rom::palette_space_screen_bank2(revision).address(), 2],
        )?;
        let mut character_x = 280_i16;
        let mut character_y = 150_i16;
        call(
            context,
            ids::CUTSCENE_SET_ROCKETSHIP_PAIR_POSE,
            &[0, character_x as i32 as u32, character_y as i32 as u32],
        )?;
        ram::g_sega_v_blank_sprite_updates_enabled(revision).write(context, 1);
        palette_time = counter.read(context);
        while character_x > -50 {
            call(context, ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, &[])?;
            let frame = counter.read(context);
            context.wait_vblank_frame(counter.address(), frame)?;
            if character_x == 257 {
                call(
                    context,
                    ids::PLAY_SOUND_EFFECT,
                    &[rom::sfx_achoo(revision).address(), 50, 0],
                )?;
            }
            if character_x < 57 && character_x > 44 {
                character_y = character_y.wrapping_add(2);
            } else if character_x > 178 && character_x < 191 {
                character_y = character_y.wrapping_sub(2);
                if character_x == 190 {
                    call(context, ids::PLAY_SOUND, &[21, 99])?;
                }
            }
            character_x = character_x.wrapping_sub(1);
            call(
                context,
                ids::CUTSCENE_POSITION_ROCKETSHIP_PAIR,
                &[character_x as i32 as u32, character_y as i32 as u32],
            )?;
            advance_palette(context, &mut palette_time, &mut palette_frame);
        }
        call(context, ids::CUTSCENE_RELEASE_ROCKETSHIP_PAIR, &[])?;
        call(context, ids::SEGA_LOGO_RELEASE_TRADEMARK_ENTITY, &[])?;
        ram::g_sega_rocket_pass_shown(revision).write(context, 1);
    }
    let frame = counter.read(context);
    context.wait_vblank_frame(counter.address(), frame)?;
    ram::g_v_blank_func(revision).write(context, Ptr::new(0));
    Ok(None)
}
