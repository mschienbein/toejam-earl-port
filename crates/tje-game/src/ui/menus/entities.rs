use crate::{Context, Result, call, ids, ui::entities::entity};
use tje_mem::{Ptr, ram, rom};

/// G/src/ui/main_menu.c:295 (`CreateMenuCursorEntity`).
/// Entity displacement and allocation-result truth both use the stored short.
pub fn create_menu_cursor_entity(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let handle = call(context, ids::ALLOCATE_ENTITY, &[])? as i16;
    ram::g_menu_cursor_entity(revision).write(context, handle);
    if handle != 0 {
        let cursor = entity(context, ram::g_menu_cursor_entity(revision).read(context));
        cursor.tile_count().write(context, 12);
        cursor.sprite_count().write(context, 1);
        cursor.x().write(context, 20);
        cursor.y().write(context, -50);
        cursor.sequence().write(
            context,
            Ptr::new(rom::menu_cursor_animation(revision).address()),
        );
        cursor
            .frames()
            .write(context, Ptr::new(rom::spr_menu_cursor(revision).address()));
        cursor.frame_phase().write(context, 8);
        cursor.frame_duration().write(context, 1);
        cursor.frame().write(context, -1);
        cursor.sprite_ready().write(context, 0);
        call(
            context,
            ids::ALLOCATE_ENTITY_VIEWPORT_SPRITES,
            &[cursor.address(), 0],
        )?;
    }
    Ok(None)
}

/// G/src/ui/main_menu.c:315 (`ReleaseMenuCursorEntity`).
pub fn release_menu_cursor_entity(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let handle = ram::g_menu_cursor_entity(context.revision());
    if handle.read(context) != 0 {
        let argument = i32::from(handle.read(context)) as u32;
        call(context, ids::FREE_ENTITY, &[argument])?;
    }
    Ok(None)
}

/// G/src/ui/main_menu.c:334 (`AddMenuHeader`).
/// Animation indexing retains ADD.W/ADDA.W signed displacement wrap.
pub fn add_menu_header(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let header_number = arguments[0] as i16;
    call(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::menu_header_palette(revision).address(), 1],
    )?;
    let handle = call(context, ids::ALLOCATE_ENTITY, &[])? as i16;
    ram::g_menu_header_entity(revision).write(context, handle);
    if handle != 0 {
        let header = entity(context, ram::g_menu_header_entity(revision).read(context));
        header.tile_count().write(context, 0x80);
        header.sprite_count().write(context, 11);
        header.x().write(context, 160);
        header.y().write(context, 40);
        let sequence = rom::menu_header_animations(revision)
            .cast::<i8>()
            .byte_offset(i32::from(header_number.wrapping_mul(2)));
        header
            .sequence()
            .write(context, Ptr::new(sequence.address()));
        header
            .frames()
            .write(context, Ptr::new(rom::spr_menu_header(revision).address()));
        header.frame_phase().write(context, 8);
        header.frame_duration().write(context, 1);
        header.frame().write(context, -1);
        header.sprite_ready().write(context, 0);
        call(
            context,
            ids::ALLOCATE_ENTITY_VIEWPORT_SPRITES,
            &[header.address(), 0],
        )?;
    }
    Ok(None)
}

/// G/src/ui/main_menu.c:356 (`RemoveMenuHeader`).
pub fn remove_menu_header(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let handle = ram::g_menu_header_entity(context.revision());
    if handle.read(context) != 0 {
        let argument = i32::from(handle.read(context)) as u32;
        call(context, ids::FREE_ENTITY, &[argument])?;
    }
    Ok(None)
}
