use crate::{Context, Result, call, ids};
use tje_mem::{Address, ObjectState, Ptr, RenderFrame, RenderFramePieceView, ram, rom};

/// G/src/cutscenes/large_player_parts.c:27, lowered with ASL.W #7 / ADDA.W.
pub(super) fn entity_at(context: &dyn Context, slot: i16) -> Address<ObjectState> {
    Address::new(
        ram::g_player_info(context.revision())
            .address()
            .wrapping_add(i32::from(slot.wrapping_shl(7)) as u32),
    )
}

/// G/src/cutscenes/large_player_parts.c:80,207; matching callers leave their Y
/// argument in D4.low. AllocateEntityViewportSprites reads that word as an
/// uninitialized tile when shared-tile sprite allocation fails. Restore the
/// Rust caller's preserved D4 even on error; never invent a zero residue.
pub(super) fn allocate_viewport_with_d4_word(
    context: &mut dyn Context,
    entity: Address<ObjectState>,
    word: i16,
) -> Result<()> {
    let saved = context.data_register(4);
    context.set_data_register(4, (saved & 0xffff_0000) | u32::from(word as u16));
    let result = call(
        context,
        ids::ALLOCATE_ENTITY_VIEWPORT_SPRITES,
        &[entity.address(), 0],
    );
    context.set_data_register(4, saved);
    result.map(|_| ())
}

/// G/src/cutscenes/large_player_parts.c:28-32 and the identical counted-piece
/// loops in drifting_entities.c:27-34 and intro_frame_entities.c:28-34.
/// MCC68K reads height before width, then adds only the product's low byte.
pub(super) fn reserve_frame_pieces(
    context: &mut dyn Context,
    entity: Address<ObjectState>,
    frame: Address<RenderFrame>,
) -> i16 {
    let count = frame.piece_count().read(context);
    entity.sprite_count().write(context, count);
    entity.tile_count().write(context, 0);
    let mut piece = 0_i16;
    while (i32::from(piece) as u32) < u32::from(frame.piece_count().read(context)) {
        let dimensions = frame
            .cast::<RenderFramePieceView>()
            .pieces()
            .at(i32::from(piece));
        let height = u32::from(dimensions.height().read(context));
        let width = u32::from(dimensions.width().read(context));
        let count = entity
            .tile_count()
            .read(context)
            .wrapping_add((height * width) as u8);
        entity.tile_count().write(context, count);
        piece = piece.wrapping_add(1);
    }
    piece
}

/// G/src/cutscenes/large_player_parts.c:18 (`IntroAllocateLargePlayerParts`).
/// Atomic: counted ROM dimensions and RAM-only allocation, without waits.
pub fn intro_allocate_large_player_parts(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    for index in 0..8 {
        let initial = rom::intro_large_player_part_initial_frame_indices(revision)
            .at(index)
            .read(context);
        let frame = rom::spr_title_big_players(revision)
            .at(i32::from(initial))
            .read(context)
            .target();
        let slot = call(context, ids::ALLOCATE_ENTITY, &[])? as i16;
        if slot != 0 {
            let entity = entity_at(context, slot);
            reserve_frame_pieces(context, entity, frame);
            entity.tile().write(context, 0x8000);
            entity
                .current_frame()
                .write(context, Ptr::new(frame.address()));
        }
        ram::g_intro_large_player_part_entities(revision)
            .at(index)
            .write(context, slot as u8);
    }
    Ok(None)
}

/// G/src/cutscenes/large_player_parts.c:53 (`IntroPositionLargePlayerPart`).
/// Atomic: RAM-only viewport allocation; negative selectors remain unchecked.
pub fn intro_position_large_player_part(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let index = arguments[0] as i16;
    if index >= 8 {
        return Ok(None);
    }
    let offset = i32::from(index.wrapping_shl(2)) as u32;
    let _unused_frame = context.read_u32(
        rom::spr_title_big_players(revision)
            .address()
            .wrapping_add(offset),
    );
    let slot = ram::g_intro_large_player_part_entities(revision)
        .at(i32::from(index))
        .read(context);
    let entity = entity_at(context, i16::from(slot));
    entity.x().write(context, arguments[1] as i16);
    entity.y().write(context, arguments[2] as i16);
    if index == 3 {
        let y = entity.y().read(context).wrapping_add(5);
        entity.y().write(context, y);
        entity.height().write(context, 5);
    }
    let sequence = rom::intro_large_player_part_held_frame_streams(revision)
        .address()
        .wrapping_add(i32::from(index.wrapping_mul(2)) as u32);
    entity.sequence().write(context, Ptr::new(sequence));
    entity.frames().write(
        context,
        Ptr::new(rom::spr_title_big_players(revision).address()),
    );
    entity.frame_phase().write(context, 10);
    entity.frame_duration().write(context, 1);
    entity.frame().write(context, -1);
    allocate_viewport_with_d4_word(context, entity, arguments[2] as i16)?;
    entity.sprite_ready().write(context, 0);
    Ok(None)
}

/// G/src/cutscenes/large_player_parts.c:100 (`IntroReleaseLargePlayerPartSprites`).
/// Atomic: conditional RAM-only sprite release; selector guard is signed.
pub fn intro_release_large_player_part_sprites(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let index = arguments[0] as i16;
    if index >= 8 {
        return Ok(None);
    }
    let slot = ram::g_intro_large_player_part_entities(context.revision())
        .at(i32::from(index))
        .read(context);
    let entity = entity_at(context, i16::from(slot));
    if entity.draw_flags().read(context) & 1 != 0 {
        call(
            context,
            ids::RELEASE_ENTITY_VIEWPORT_SPRITES,
            &[entity.address(), 0],
        )?;
    }
    Ok(None)
}

/// G/src/cutscenes/large_player_parts.c:144 (`IntroAllocateDrivingParts`).
/// Atomic: counted ROM dimensions and RAM-only allocation, without waits.
pub fn intro_allocate_driving_parts(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    for index in 0..3 {
        let initial = rom::intro_driving_initial_frame_indices(revision)
            .at(index)
            .read(context);
        let frame = rom::spr_intro_earl_driving(revision)
            .at(i32::from(initial))
            .read(context)
            .target();
        let slot = call(context, ids::ALLOCATE_ENTITY, &[])? as i16;
        if slot != 0 {
            let entity = entity_at(context, slot);
            reserve_frame_pieces(context, entity, frame);
            entity.tile().write(context, 0x8000);
            entity
                .current_frame()
                .write(context, Ptr::new(frame.address()));
        }
        ram::g_intro_driving_part_entities(revision)
            .at(index)
            .write(context, slot as u8);
    }
    Ok(None)
}

/// G/src/cutscenes/large_player_parts.c:179 (`IntroPositionDrivingPart`).
/// Atomic: RAM-only sprite allocation; repeated placement accumulates height.
pub fn intro_position_driving_part(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let index = arguments[0] as i16;
    if index >= 3 {
        return Ok(None);
    }
    let offset = i32::from(index.wrapping_shl(2)) as u32;
    let _unused_frame = context.read_u32(
        rom::spr_intro_earl_driving(revision)
            .address()
            .wrapping_add(offset),
    );
    let slot = ram::g_intro_driving_part_entities(revision)
        .at(i32::from(index))
        .read(context);
    let entity = entity_at(context, i16::from(slot));
    entity.x().write(context, arguments[1] as i16);
    entity.y().write(context, arguments[2] as i16);
    if index == 1 {
        let y = entity.y().read(context).wrapping_add(100);
        entity.y().write(context, y);
        let height = entity.height().read(context).wrapping_add(100);
        entity.height().write(context, height);
    } else if index == 2 {
        let y = entity.y().read(context).wrapping_add(101);
        entity.y().write(context, y);
        let height = entity.height().read(context).wrapping_add(101);
        entity.height().write(context, height);
    }
    let sequence = rom::intro_driving_part_held_frame_streams(revision)
        .address()
        .wrapping_add(i32::from(index.wrapping_mul(2)) as u32);
    entity.sequence().write(context, Ptr::new(sequence));
    entity.frames().write(
        context,
        Ptr::new(rom::spr_intro_earl_driving(revision).address()),
    );
    entity.frame_phase().write(context, 10);
    entity.frame_duration().write(context, 1);
    entity.frame().write(context, -1);
    allocate_viewport_with_d4_word(context, entity, arguments[2] as i16)?;
    entity.sprite_ready().write(context, 0);
    Ok(None)
}

/// G/src/cutscenes/large_player_parts.c:227 (`IntroReleaseDrivingPartSprites`).
/// Atomic: the original tests either low flag bit, not only primary visibility.
pub fn intro_release_driving_part_sprites(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let index = arguments[0] as i16;
    if index >= 3 {
        return Ok(None);
    }
    let slot = ram::g_intro_driving_part_entities(context.revision())
        .at(i32::from(index))
        .read(context);
    let entity = entity_at(context, i16::from(slot));
    if entity.draw_flags().read(context) & 3 != 0 {
        call(
            context,
            ids::RELEASE_ENTITY_VIEWPORT_SPRITES,
            &[entity.address(), 0],
        )?;
    }
    Ok(None)
}

/// G/src/cutscenes/large_player_parts.c:241 (`IntroCreateAsteroidEntity`).
/// Atomic: fixed constructor and RAM-only viewport sprite allocation.
pub fn intro_create_asteroid_entity(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let selector = ram::g_intro_asteroid_entity(revision);
    let allocated = call(context, ids::ALLOCATE_ENTITY, &[])? as u8;
    selector.write(context, allocated);
    if selector.read(context) != 0 {
        let entity = entity_at(context, i16::from(selector.read(context)));
        entity.tile().write(context, 0);
        let frame = rom::spr_intro_asteroid(revision).at(0).read(context);
        entity.current_frame().write(context, frame);
        entity.frames().write(
            context,
            Ptr::new(rom::spr_intro_asteroid(revision).address()),
        );
        entity.tile_count().write(context, 160);
        entity.sprite_count().write(context, 12);
        entity.x().write(context, 104);
        entity.y().write(context, 151);
        entity.sprite_ready().write(context, 0);
        entity.sequence().write(
            context,
            Ptr::new(rom::intro_asteroid_loop_animation(revision).address()),
        );
        entity.frame_duration().write(context, 1);
        entity.frame().write(context, -1);
        entity.frame_phase().write(context, 6);
        call(
            context,
            ids::ALLOCATE_ENTITY_VIEWPORT_SPRITES,
            &[entity.address(), 0],
        )?;
    }
    Ok(None)
}

/// G/src/cutscenes/large_player_parts.c:41 (`IntroReleaseLargePlayerParts`).
/// Atomic: release before clearing each of eight RAM selector bytes.
pub fn intro_release_large_player_parts(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    for index in 0..8 {
        let selector = ram::g_intro_large_player_part_entities(revision).at(index);
        let slot = selector.read(context);
        if slot != 0 {
            call(context, ids::FREE_ENTITY, &[u32::from(slot)])?;
        }
        selector.write(context, 0);
    }
    Ok(None)
}

/// G/src/cutscenes/large_player_parts.c:114 (`IntroPositionLargePlayerParts`).
/// Atomic: seven RAM-only positioner calls in the original selector order.
pub fn intro_position_large_player_parts(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    for index in [0, 4, 1, 2, 5, 6, 7] {
        call(
            context,
            ids::INTRO_POSITION_LARGE_PLAYER_PART,
            &[index, 148, 112],
        )?;
    }
    Ok(None)
}

/// G/src/cutscenes/large_player_parts.c:128 (`IntroReleaseLargePlayerSpritesAndRefresh`).
/// TimeAware: queue graphics, wait for a real VBlank, then upload.
pub fn intro_release_large_player_sprites_and_refresh(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    for index in [0, 4, 1, 2, 5, 6, 7] {
        call(
            context,
            ids::INTRO_RELEASE_LARGE_PLAYER_PART_SPRITES,
            &[index],
        )?;
    }
    context.discard_u8(ram::g_sprite_graphics_upload_pending(context.revision()).address())?;
    ram::g_sprite_graphics_upload_pending(context.revision()).write(context, 0);
    call(context, ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, &[])?;
    call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    call(context, ids::UPLOAD_QUEUED_SPRITE_GRAPHICS, &[0, 0])?;
    Ok(None)
}

/// G/src/cutscenes/large_player_parts.c:167 (`IntroReleaseDrivingParts`).
/// Atomic: release before clearing each of three RAM selector bytes.
pub fn intro_release_driving_parts(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    for index in 0..3 {
        let selector = ram::g_intro_driving_part_entities(revision).at(index);
        let slot = selector.read(context);
        if slot != 0 {
            call(context, ids::FREE_ENTITY, &[u32::from(slot)])?;
        }
        selector.write(context, 0);
    }
    Ok(None)
}

/// G/src/cutscenes/large_player_parts.c:264 (`IntroReleaseAsteroidEntity`).
/// Atomic: RAM-only entity release followed by clearing its selector.
pub fn intro_release_asteroid_entity(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let selector = ram::g_intro_asteroid_entity(context.revision());
    let slot = selector.read(context);
    if slot != 0 {
        call(context, ids::FREE_ENTITY, &[u32::from(slot)])?;
    }
    selector.write(context, 0);
    Ok(None)
}
