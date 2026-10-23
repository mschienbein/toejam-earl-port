use crate::{Context, Result, call, ids};
use tje_mem::{Ptr, ram, rom};

/// G/src/cutscenes/cutscene_text.c:62 (`CutsceneQueueCenteredRemappedTextLine`).
/// Atomic: bounded font copies and the character mapper touch RAM/ROM only.
pub fn cutscene_queue_centered_remapped_text_line(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    queue_centered_line(context, arguments, ids::CUTSCENE_APPEND_REMAPPED_FONT_TILES)
}

/// G/src/cutscenes/cutscene_text.c:91 (`CutsceneQueueCenteredOriginalPaletteTextLine`).
/// Atomic: bounded font copies and the character mapper touch RAM/ROM only.
pub fn cutscene_queue_centered_original_palette_text_line(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    queue_centered_line(context, arguments, ids::APPEND_SPRITE_TILES_TO_BUFFER)
}

fn queue_centered_line(
    context: &mut dyn Context,
    arguments: &[u32],
    append: crate::FunctionId,
) -> Result<Option<u32>> {
    let revision = context.revision();
    let text = arguments[0];
    let length = arguments[1] as i16;
    let tile_base = arguments[2] as i16;
    let font = rom::runtime_text_font_tiles(revision).address();
    let free_tiles = 32_i32.wrapping_sub(i32::from(length));
    let mut padding = (free_tiles / 2) as i16;
    let mut index = 0_i16;
    while index < padding {
        call(context, ids::APPEND_SPRITE_TILES_TO_BUFFER, &[font, 1])?;
        index = index.wrapping_add(1);
    }
    index = 0;
    while index < length {
        let character = i32::from(context.read_u8(text.wrapping_add(index as i32 as u32)) as i8);
        let mut glyph = call(
            context,
            ids::TEXT_MAP_CHARACTER_TO_GLYPH,
            &[character as u32],
        )? as i8;
        if glyph == 27 {
            glyph = glyph.wrapping_add(1);
        }
        let source = font.wrapping_add(i32::from(glyph).wrapping_shl(5) as u32);
        call(context, append, &[source, 1])?;
        index = index.wrapping_add(1);
    }
    if length & 1 != 0 {
        padding = padding.wrapping_add(1);
    }
    index = 0;
    while index < padding {
        call(context, ids::APPEND_SPRITE_TILES_TO_BUFFER, &[font, 1])?;
        index = index.wrapping_add(1);
    }
    let count = ram::g_sprite_tile_upload_queue_word_count(revision);
    let slot = count.read(context);
    count.write(context, slot.wrapping_add(1));
    ram::g_sprite_tile_upload_runs(revision)
        .at(i32::from(slot))
        .write(context, tile_base as u16);
    let slot = count.read(context);
    count.write(context, slot.wrapping_add(1));
    ram::g_sprite_tile_upload_runs(revision)
        .at(i32::from(slot))
        .write(context, 32);
    Ok(None)
}

/// G/src/cutscenes/cutscene_text.c:164 (`CutsceneLoadSecondTextLine`).
/// Atomic: scans and queues text into RAM, without a device transfer or wait.
pub fn cutscene_load_second_text_line(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let slot = ram::g_cutscene_second_text_entity(context.revision());
    load_text_entity(context, arguments, slot, false)
}

/// G/src/cutscenes/cutscene_text.c:194 (`CutsceneLoadOriginalPaletteTextLine`).
/// Atomic: ordered text release, scan, allocation and RAM-only font preparation.
pub fn cutscene_load_original_palette_text_line(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    call(context, ids::CUTSCENE_RELEASE_TEXT_ENTITIES, &[])?;
    let slot = ram::g_cutscene_first_text_entity(context.revision());
    load_text_entity(context, arguments, slot, true)
}

/// G/src/cutscenes/cutscene_text.c:133 (`CutsceneTextLoad`).
/// Atomic: ordered text release, scan, allocation and RAM-only font preparation.
pub fn cutscene_text_load(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    call(context, ids::CUTSCENE_RELEASE_TEXT_ENTITIES, &[])?;
    let slot = ram::g_cutscene_first_text_entity(context.revision());
    load_text_entity(context, arguments, slot, false)
}

fn load_text_entity(
    context: &mut dyn Context,
    arguments: &[u32],
    slot: tje_mem::Address<i8>,
    original_palette: bool,
) -> Result<Option<u32>> {
    let revision = context.revision();
    let text = arguments[0];
    let mut length = 0_i16;
    while context.read_u8(text.wrapping_add(length as i32 as u32)) != 0 {
        length = length.wrapping_add(1);
    }
    if length > 32 {
        length = 32;
    }
    let handle = call(context, ids::ALLOCATE_ENTITY, &[])? as i8;
    slot.write(context, handle);
    if handle != 0 {
        let entity = ram::g_player_info(revision).at(i32::from(handle));
        entity.x().write(context, arguments[1] as i16);
        let elevation = if original_palette { 0 } else { 100 };
        entity
            .y()
            .write(context, (arguments[2] as i16).wrapping_add(elevation));
        entity.height().write(context, elevation);
        entity.width().write(context, length.wrapping_shl(3) as u8);
        entity.height_pixels().write(context, 8);
        entity.sprite_count().write(context, 8);
        entity.tile_count().write(context, 32);
        entity.tile().write(context, 0x8000);
        let frame = if original_palette {
            rom::cutscene_original_palette_text_frame(revision).address()
        } else {
            rom::cutscene_remapped_text_frame(revision).address()
        };
        entity.current_frame().write(context, Ptr::new(frame));
        // All three matching caption callers retain Y in D4.W.
        super::allocate_viewport_with_d4(context, entity.address(), arguments[2] as u16)?;
        let tile = entity.tile().read(context) & 0x7ff;
        call(
            context,
            if original_palette {
                ids::CUTSCENE_QUEUE_CENTERED_ORIGINAL_PALETTE_TEXT_LINE
            } else {
                ids::CUTSCENE_QUEUE_CENTERED_REMAPPED_TEXT_LINE
            },
            &[text, length as i32 as u32, u32::from(tile)],
        )?;
        call(
            context,
            ids::PREPARE_ENTITY_SPRITE_FRAME,
            &[entity.address(), frame, 1, 0],
        )?;
        entity.sprite_ready().write(context, 1);
    }
    Ok(None)
}

/// G/src/cutscenes/cutscene_text.c:120 (`CutsceneReleaseTextEntities`).
/// Atomic: signed-byte handles are sign-extended for each RAM-only release.
pub fn cutscene_release_text_entities(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let first = ram::g_cutscene_first_text_entity(revision);
    if first.read(context) != 0 {
        let handle = first.read(context);
        call(context, ids::FREE_ENTITY, &[i32::from(handle) as u32])?;
        first.write(context, 0);
    }
    let second = ram::g_cutscene_second_text_entity(revision);
    if second.read(context) != 0 {
        let handle = second.read(context);
        call(context, ids::FREE_ENTITY, &[i32::from(handle) as u32])?;
        second.write(context, 0);
    }
    Ok(None)
}
