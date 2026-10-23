//! Bounded VDP patches without explicit polls; full FIFOs still need device progress.
use crate::{Context, Result};
use tje_mem::ram;

/// G/src/sprites/sprite_links_vdp.c:6, TerminateSpriteChainAtHead.
pub fn terminate_sprite_chain_at_head(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    context.write_vdp_command(0x7802_0003);
    context.write_vdp_data(79);
    Ok(None)
}

/// G/src/sprites/sprite_links_vdp.c:13, TerminatePrimarySpriteChain.
pub fn terminate_primary_sprite_chain(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    context.write_vdp_command(0x78ba_0003);
    context.write_vdp_data(79);
    Ok(None)
}

/// G/src/sprites/sprite_links_vdp.c:20, SelectPrimaryViewportSprites.
/// Sign-extend the explicitly signed char link to the word written to VRAM.
pub fn select_primary_viewport_sprites(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    context.write_vdp_command(0x78c2_0003);
    context.write_vdp_data(79);
    context.write_vdp_command(0x78ba_0003);
    let link = ram::g_primary_viewport_uploaded_link(context.revision()).read(context) as i8;
    context.write_vdp_data(link as i16 as u16);
    Ok(None)
}

/// G/src/sprites/sprite_links_vdp.c:29, SelectSecondaryViewportSprites.
pub fn select_secondary_viewport_sprites(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    context.write_vdp_command(0x78ba_0003);
    context.write_vdp_data(24);
    context.write_vdp_command(0x78c2_0003);
    let link = ram::g_secondary_viewport_uploaded_link(context.revision()).read(context) as i8;
    context.write_vdp_data(link as i16 as u16);
    Ok(None)
}

/// G/src/sprites/hide_player_sprites.c:7, HideFlaggedPlayerSprites.
/// The original forms the player-record displacement with a word-sized shift.
pub fn hide_flagged_player_sprites(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let entity = ram::g_player_info(revision)
        .byte_offset(i32::from((arguments[0] as i16).wrapping_shl(7)))
        .cast::<tje_mem::ObjectState>();
    if entity.behavior_flags().read(context) & 0x10 == 0 || entity.sprite_ready().read(context) == 0
    {
        return Ok(None);
    }
    let mut list = if entity.draw_flags().read(context) & 2 != 0 {
        entity.primary_sprites().address()
    } else if entity.draw_flags().read(context) & 4 != 0 {
        entity.secondary_sprites().address()
    } else {
        return Ok(None);
    };
    let count = i16::from(entity.sprite_count().read(context));
    for _ in 0..count {
        let sprite = context.read_i8(list);
        list = list.wrapping_add(1);
        let command = (i32::from(sprite) as u32)
            .wrapping_shl(19)
            .wrapping_add(0x7800_0003);
        context.write_vdp_command(command);
        context.write_vdp_data(1);
    }
    Ok(None)
}
