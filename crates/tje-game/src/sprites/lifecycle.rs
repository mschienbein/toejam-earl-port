//! Original pool ownership and per-view residency; emulated pointers stay numeric.
use crate::{
    Context, Result, call, call_at,
    call_frame::{CallSite, FrameLayout, LocalFrame},
    call_void, ids,
};
use tje_mem::{Address, ObjectState, ram};

/// G/src/sprites/sprite_slots.c:238, AllocateEntity.
/// The reset precedes the active-byte store and clears only the original 47 words.
pub fn allocate_entity(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let count = if ram::g_player1_map_level(revision).read(context) == 26 {
        34
    } else {
        20
    };
    let allocated = ram::g_dynamic_entity_slot_allocated(revision);
    for index in 0..count {
        if allocated.at(index).read(context) == 0 {
            allocated.at(index).write(context, 1);
            let entity_number = index + 6;
            let entity = ram::g_player_info(revision).at(entity_number);
            call_void(
                context,
                ids::RESET_ENTITY_RUNTIME_STATE,
                &[entity.address()],
            )?;
            entity.draw_flags().write(context, 1);
            return Ok(Some(entity_number as u32));
        }
    }
    Ok(Some(0))
}

/// G/src/sprites/sprite_slots.c:148, ReleaseEntityShadows.
pub fn release_entity_shadows(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let entity = Address::<ObjectState>::new(arguments[0]);
    for shadow in [entity.primary_shadow(), entity.secondary_shadow()] {
        if shadow.read(context) != 0 {
            let handle = shadow.read(context) as i32 as u32;
            if call(context, ids::IS_SPRITE_SLOT_LOCKED, &[handle])? == 0 {
                let handle = shadow.read(context) as i32 as u32;
                call_void(context, ids::FREE_SPRITE_SLOT, &[handle])?;
                shadow.write(context, 0);
            }
        }
    }
    Ok(None)
}

/// G/src/sprites/sprite_slots.c:162, AllocateEntityViewportSprites.
/// ORIGINAL BUG: on shared-tiles allocation failure, tile is the entry low D4 word,
/// not an initialized zero (G/build/rev02/asm/c_sprite_slots.s:390-456).
pub fn allocate_entity_viewport_sprites(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    // G c_sprite_slots.s:390: D2-D5/A2; selector JSR returns at
    // REV02 D7E6 / REV00 D7C6, both caller offset 0x70.
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x3c, 0x04))?;
    let result = allocate_viewport_inner(context, arguments);
    let result = result.map_err(|error| context.protect_timing_failure(error));
    if result
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        return result;
    }
    let leave = frame.leave(context);
    match (result, leave) {
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(value), Ok(())) => Ok(value),
    }
}

fn allocate_viewport_inner(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let entity = Address::<ObjectState>::new(arguments[0]);
    let viewport = arguments[1] as i16;
    let tile_count = i16::from(entity.tile_count().read(context));
    context.set_address_register(2, entity.address());
    context.set_data_register(
        2,
        (context.data_register(2) & 0xffff_0000) | u32::from(viewport as u16),
    );
    context.set_data_register(
        5,
        (context.data_register(5) & 0xffff_0000) | u32::from(tile_count as u16),
    );
    if entity.draw_flags().read(context) & 6 == 0 && entity.behavior_flags().read(context) & 4 == 0
    {
        let tile = call(
            context,
            ids::ALLOCATE_SPRITE_TILE_RUN,
            &[tile_count as i32 as u32],
        )? as i16;
        context.set_data_register(
            4,
            (context.data_register(4) & 0xffff_0000) | u32::from(tile as u16),
        );
        if tile == 0 {
            return Ok(None);
        }
        entity
            .tile()
            .write(context, entity.tile().read(context) & 0xf800);
        entity
            .tile()
            .write(context, entity.tile().read(context) | tile as u16);
    }
    let (viewport_bit, slots) = if viewport == 0 {
        (2, entity.primary_sprites())
    } else {
        (4, entity.secondary_sprites())
    };
    context.set_data_register(3, viewport_bit);
    let sprite_count = u32::from(entity.sprite_count().read(context));
    let site = CallSite::relative(
        ids::ALLOCATE_ENTITY_VIEWPORT_SPRITES,
        context.revision(),
        0x70,
    );
    let status = call_at(
        context,
        ids::ALLOCATE_SPRITE_SLOTS,
        &[sprite_count, viewport as i32 as u32, 1, slots.address()],
        site,
    )?;
    if status == 0 {
        entity.draw_flags().write(
            context,
            entity.draw_flags().read(context) | context.data_register(3) as u8,
        );
    } else if entity.draw_flags().read(context) & 6 == 0 {
        let tile = context.data_register(4) as i16;
        let tile_count = context.data_register(5) as i16;
        call_void(
            context,
            ids::FREE_SPRITE_TILE_RUN,
            &[tile as i32 as u32, tile_count as i32 as u32],
        )?;
    }
    Ok(None)
}

/// G/src/sprites/sprite_slots.c:194, ReleaseEntityViewportSprites.
pub fn release_entity_viewport_sprites(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let entity = Address::<ObjectState>::new(arguments[0]);
    if entity.draw_flags().read(context) & 1 == 0 {
        return Ok(None);
    }
    let (slots, shadow) = if arguments[1] as i16 == 0 {
        entity
            .draw_flags()
            .write(context, entity.draw_flags().read(context) & !2);
        (entity.primary_sprites(), entity.primary_shadow())
    } else {
        entity
            .draw_flags()
            .write(context, entity.draw_flags().read(context) & !4);
        (entity.secondary_sprites(), entity.secondary_shadow())
    };
    if shadow.read(context) != 0 {
        let handle = shadow.read(context) as i32 as u32;
        if call(context, ids::IS_SPRITE_SLOT_LOCKED, &[handle])? == 0 {
            let handle = shadow.read(context) as i32 as u32;
            call_void(context, ids::FREE_SPRITE_SLOT, &[handle])?;
            shadow.write(context, 0);
        }
    }
    let sprite_count = u32::from(entity.sprite_count().read(context));
    call_void(
        context,
        ids::FREE_SPRITE_SLOTS,
        &[sprite_count, slots.address()],
    )?;
    if entity.draw_flags().read(context) & 6 == 0 && entity.behavior_flags().read(context) & 4 == 0
    {
        let tile_count = i16::from(entity.tile_count().read(context));
        let tile = (entity.tile().read(context) & 0x07ff) as i16;
        call_void(
            context,
            ids::FREE_SPRITE_TILE_RUN,
            &[tile as i32 as u32, tile_count as i32 as u32],
        )?;
    }
    Ok(None)
}
