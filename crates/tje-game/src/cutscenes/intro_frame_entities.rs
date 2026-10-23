use super::large_player_parts::{allocate_viewport_with_d4_word, entity_at, reserve_frame_pieces};
use crate::{Context, Result, call, ids};
use tje_mem::{Ptr, RenderFrame, ram, rom};

/// G/src/cutscenes/intro_frame_entities.c:16 (`IntroCreateHiddenFrameEntities`).
/// Atomic: ten counted-ROM constructors and RAM-only viewport allocation. The
/// only established frame-table slot is IntroAndShipAnimationPointers[1].
pub fn intro_create_hidden_frame_entities(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let frames = rom::intro_and_ship_animation_pointers(revision)
        .at(1)
        .read(context)
        .target()
        .cast::<Ptr<RenderFrame>>();
    for index in 0..10 {
        let frame = frames.offset(index).read(context).target();
        let slot = call(context, ids::ALLOCATE_ENTITY, &[])? as i16;
        if slot != 0 {
            let object = entity_at(context, slot);
            let piece_count = reserve_frame_pieces(context, object, frame);
            object
                .current_frame()
                .write(context, Ptr::new(frame.address()));
            object.x().write(context, 170);
            object.y().write(context, 160);
            let tile = object.tile().read(context) | 0x8000;
            object.tile().write(context, tile);
            let flags = object.behavior_flags().read(context) | 0x40;
            object.behavior_flags().write(context, flags);
            // The matching caller still holds the completed piece counter in D4.
            allocate_viewport_with_d4_word(context, object, piece_count)?;
            object.sprite_ready().write(context, 0);
            object.sequence().write(
                context,
                Ptr::new(
                    rom::intro_held_frame_streams(revision)
                        .at(index)
                        .frame()
                        .address(),
                ),
            );
            object.frames().write(context, Ptr::new(frames.address()));
            object.frame_phase().write(context, 1);
            object.frame_duration().write(context, 1);
            object.frame().write(context, -1);
        }
        ram::g_intro_component_entities(revision)
            .at(index)
            .write(context, slot as u8);
    }
    Ok(None)
}
