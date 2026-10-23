//! Jam Out leaf translations; see docs/PORTING.md.

use crate::execution::ExecutionClass;
use crate::{Context, FunctionId, Result, Translation, ids};
use tje_mem::{Address, ObjectState, Ptr, ram, rom};

mod audio_callbacks;
mod entities;
mod layouts;
mod lifecycle;
mod scene_layouts;

/// Available Rust entries owned by this subsystem.
pub static TRANSLATIONS: &[Translation] = &[
    Translation {
        id: ids::JAM_OUT_UPDATE_LEAD_ANIMATIONS,
        function: update_lead_animations,
    },
    Translation {
        id: ids::JAM_OUT_SYNCHRONIZE_ENTITY_COPIES,
        function: synchronize_entity_copies,
    },
    Translation {
        id: ids::JAM_OUT_POSITION_TOEJAM_COPY,
        function: position_toejam_copy,
    },
    Translation {
        id: ids::JAM_OUT_POSITION_EARL_COPY,
        function: position_earl_copy,
    },
    Translation {
        id: ids::JAM_OUT_HIDE_TOEJAM_COPY,
        function: hide_toejam_copy,
    },
    Translation {
        id: ids::JAM_OUT_HIDE_EARL_COPY,
        function: hide_earl_copy,
    },
    Translation {
        id: ids::JAM_OUT_SET_TOEJAM_SEQUENCE,
        function: set_toejam_sequence,
    },
    Translation {
        id: ids::JAM_OUT_SET_EARL_SEQUENCE,
        function: set_earl_sequence,
    },
    Translation {
        id: ids::JAM_OUT_RELEASE_CHARACTER_COPIES,
        function: entities::release_character_copies,
    },
    Translation {
        id: ids::JAM_OUT_DRAW_PACKED_BACKGROUND_BLOCK,
        function: layouts::draw_packed_background_block,
    },
];

/// Reviewed RAM-only originals: bounded loops and callees confined to sprite
/// allocation/release RAM. No clocks, interrupts, DMA, or FIFO progress.
pub static ATOMIC_CAPABLE: &[FunctionId] = &[
    ids::JAM_OUT_UPDATE_LEAD_ANIMATIONS,
    ids::JAM_OUT_SYNCHRONIZE_ENTITY_COPIES,
    ids::JAM_OUT_POSITION_TOEJAM_COPY,
    ids::JAM_OUT_POSITION_EARL_COPY,
    ids::JAM_OUT_HIDE_TOEJAM_COPY,
    ids::JAM_OUT_HIDE_EARL_COPY,
    ids::JAM_OUT_SET_TOEJAM_SEQUENCE,
    ids::JAM_OUT_SET_EARL_SEQUENCE,
    // src/jamout/jamout.c:442: two nine-entry allocation loops. AllocateEntity
    // and AllocateEntityViewportSprites only touch RAM. Original uninitialized
    // lead/allocator failure state still needs a call-frame ABI, not a clock.
    ids::JAM_OUT_CREATE_CHARACTER_COPIES,
    // ibid.:509: bounded handle loop; FreeEntity/viewport release only touch RAM.
    ids::JAM_OUT_RELEASE_CHARACTER_COPIES,
];

/// Reviewed original contracts, including untranslated entries.
pub static EXECUTION_CONTRACTS: &[(FunctionId, ExecutionClass)] = &[
    // src/jamout/jamout.c:30: controller-release and 51-update exit loops,
    // explicit VBlank waits, per-frame music, and IRQ-updated animation inputs.
    (ids::START_JAM_OUT, ExecutionClass::TimeAware),
    // ibid.:149: music sequencing can retry Z80 mailboxes / wait on YM BUSY;
    // controller reads coordinate with the continuously running Z80 driver.
    (ids::JAM_OUT_V_BLANK, ExecutionClass::TimeAware),
    // ibid.:223: same music/controller clocks, plus ordered PCM requests.
    (
        ids::JAM_OUT_RANDOM_SAMPLE_V_BLANK,
        ExecutionClass::TimeAware,
    ),
    // ibid.:352: selected layouts call the time-aware helpers below; the
    // no-transition path remains the same class, not a branch-dependent waiver.
    (
        ids::JAM_OUT_UPDATE_LAYOUTS_AND_SCROLL,
        ExecutionClass::TimeAware,
    ),
    // ibid.:609: InitializeTitleTransitionTilemap -> animation VBlank wait and
    // ClearUpperVramTilemap -> WaitForNextVBlank before their uploads.
    (
        ids::JAM_OUT_SHOW_LEAD_PAIR_LAYOUT,
        ExecutionClass::TimeAware,
    ),
    // ibid.:635: 280 ordered ROM-to-VDP data writes; no status/HV reads, timing
    // observer, mutable DMA source, controller service, or interrupt wait.
    (
        ids::JAM_OUT_DRAW_PACKED_BACKGROUND_BLOCK,
        ExecutionClass::Functional,
    ),
    // ibid.:654: ClearUpperVramTilemap waits for VBlank before four uploads.
    (
        ids::JAM_OUT_SHOW_FOUR_PAIR_LAYOUT,
        ExecutionClass::TimeAware,
    ),
    // ibid.:677: both tilemap helpers explicitly wait for VBlank.
    (
        ids::JAM_OUT_SHOW_NINE_PAIR_LAYOUT,
        ExecutionClass::TimeAware,
    ),
];

/// Complete bodies not approved for production replacement.
pub static PENDING: &[FunctionId] = &[
    // Complete TimeAware callers await both-revision proof. StartJamOut also
    // retains the original creation call's uninitialized-local failure path
    // until a call-frame ABI is available.
    ids::JAM_OUT_V_BLANK,
    ids::JAM_OUT_RANDOM_SAMPLE_V_BLANK,
    ids::JAM_OUT_UPDATE_LAYOUTS_AND_SCROLL,
    ids::JAM_OUT_SHOW_LEAD_PAIR_LAYOUT,
    ids::JAM_OUT_SHOW_FOUR_PAIR_LAYOUT,
    ids::JAM_OUT_SHOW_NINE_PAIR_LAYOUT,
    ids::START_JAM_OUT,
];

/// Unregistered bodies retained for explicit proof harnesses.
pub static PENDING_TRANSLATIONS: &[Translation] = &[
    Translation {
        id: ids::JAM_OUT_V_BLANK,
        function: audio_callbacks::vblank,
    },
    Translation {
        id: ids::JAM_OUT_RANDOM_SAMPLE_V_BLANK,
        function: audio_callbacks::random_sample_vblank,
    },
    Translation {
        id: ids::JAM_OUT_UPDATE_LAYOUTS_AND_SCROLL,
        function: scene_layouts::update_layouts_and_scroll,
    },
    Translation {
        id: ids::JAM_OUT_SHOW_LEAD_PAIR_LAYOUT,
        function: scene_layouts::show_lead_pair_layout,
    },
    Translation {
        id: ids::JAM_OUT_SHOW_FOUR_PAIR_LAYOUT,
        function: scene_layouts::show_four_pair_layout,
    },
    Translation {
        id: ids::JAM_OUT_SHOW_NINE_PAIR_LAYOUT,
        function: scene_layouts::show_nine_pair_layout,
    },
    Translation {
        id: ids::START_JAM_OUT,
        function: lifecycle::start_jam_out,
    },
];

// JamOutCreateCharacterCopies (G/src/jamout/jamout.c:442) has no Rust body yet:
// MCC68K's -2(A6) lead-tile spill and residual D3 parent byte are read if a
// nonlead allocation succeeds after slot zero failed. Faithful failure paths,
// including AllocateEntityViewportSprites' uninitialized tile, require the
// reviewed call-frame API. Do not initialize those values or repair the layout.

// MCC68K lowers every Jam Out handle-to-object index with ASL.W #7 / ADDA.W.
// Keep the word-sized byte displacement even for out-of-range signed handles.
fn object_from_handle(context: &dyn Context, handle: i16) -> Address<ObjectState> {
    ram::g_player_info(context.revision())
        .cast::<ObjectState>()
        .byte_offset(i32::from(handle.wrapping_shl(7)))
}

/// G/src/jamout/jamout.c:310 (`JamOutUpdateLeadAnimations`, REV 02 0x03B27A).
/// Preserve the partner-frame stores that are immediately overwritten by -1,
/// and finish ToeJam before Earl reads ToeJam's updated sequence/frame caches.
pub fn update_lead_animations(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let transition = Ptr::new(rom::jam_lead_idle_transition_sequence(revision).address());
    let idle = Ptr::new(rom::jam_lead_idle_loop_sequence(revision).address());
    let toejam_sequence = ram::g_jam_toejam_animation_sequence(revision);
    let earl_sequence = ram::g_jam_earl_animation_sequence(revision);
    if ram::g_jam_toejam_entities(revision).at(0).read(context) != 0 {
        let object = object_from_handle(
            context,
            ram::g_jam_toejam_entities(revision).at(0).read(context),
        );
        if object.frame_duration().read(context) == 0 {
            if toejam_sequence.read(context) == transition {
                if earl_sequence.read(context) == idle {
                    let frame = ram::g_jam_earl_animation_frame(revision).read(context);
                    object.frame().write(context, frame as i8);
                }
                object.sequence().write(context, idle);
            } else {
                object.sequence().write(context, transition);
            }
            object.frame_duration().write(context, 1);
            object.frame().write(context, -1);
            object.frame_phase().write(context, 6);
            let sequence = object.sequence().read(context);
            toejam_sequence.write(context, sequence);
        }
        let frame = object.frame().read(context);
        ram::g_jam_toejam_animation_frame(revision).write(context, i16::from(frame));
    }
    if ram::g_jam_earl_entities(revision).at(0).read(context) != 0 {
        let object = object_from_handle(
            context,
            ram::g_jam_earl_entities(revision).at(0).read(context),
        );
        if object.frame_duration().read(context) == 0 {
            if earl_sequence.read(context) == transition {
                if toejam_sequence.read(context) == idle {
                    let frame = ram::g_jam_toejam_animation_frame(revision).read(context);
                    object.frame().write(context, frame as i8);
                }
                object.sequence().write(context, idle);
            } else {
                object.sequence().write(context, transition);
            }
            object.frame_duration().write(context, 1);
            object.frame().write(context, -1);
            object.frame_phase().write(context, 6);
            let sequence = object.sequence().read(context);
            earl_sequence.write(context, sequence);
        }
        let frame = object.frame().read(context);
        ram::g_jam_earl_animation_frame(revision).write(context, i16::from(frame));
    }
    if toejam_sequence.read(context) == transition || toejam_sequence.read(context) == idle {
        ram::g_jam_toejam_animation_locked(revision).write(context, 0);
    }
    if earl_sequence.read(context) == transition || earl_sequence.read(context) == idle {
        ram::g_jam_earl_animation_locked(revision).write(context, 0);
    }
    Ok(None)
}

/// G/src/jamout/jamout.c:418 (`JamOutSynchronizeEntityCopies`, REV 02 0x03B4C4).
/// Walk all forty records in order. Owner and sprite indices are signed bytes;
/// piece_count is unsigned and is not clamped to the declared access-view bounds.
pub fn synchronize_entity_copies(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    for object_number in 0..40 {
        let object = ram::g_player_info(revision).at(object_number);
        if object.draw_flags().read(context) & 1 != 0
            && object.behavior_flags().read(context) & 4 != 0
        {
            let parent = object_from_handle(context, i16::from(object.owner().read(context)));
            let frame = parent.frame().read(context);
            object.frame().write(context, frame);
            let ready = parent.sprite_ready().read(context);
            object.sprite_ready().write(context, ready);
            let current_frame = parent.current_frame().read(context);
            object.current_frame().write(context, current_frame);
            let piece_count = object
                .current_frame()
                .read(context)
                .target()
                .piece_count()
                .read(context);
            for piece in 0..i32::from(piece_count) {
                let parent_index = parent.primary_sprites().at(piece).read(context);
                let parent_sprite =
                    ram::g_hardware_sprite_table(revision).at(i32::from(parent_index));
                let index = object.primary_sprites().at(piece).read(context);
                let sprite = ram::g_hardware_sprite_table(revision).at(i32::from(index));
                let size = parent_sprite.size().read(context);
                sprite.size().write(context, size);
                let tile = parent_sprite.tile().read(context);
                sprite.tile().write(context, tile);
            }
        }
    }
    Ok(None)
}

/// G/src/jamout/jamout.c:525 (`JamOutPositionToejamCopy`, REV 02 0x03B73E).
/// The slot is a full int; coordinates narrow only at their word stores.
pub fn position_toejam_copy(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let handles = ram::g_jam_toejam_entities(context.revision());
    let slot = arguments[0] as i32;
    if handles.at(slot).read(context) != 0 {
        let object = object_from_handle(context, handles.at(slot).read(context));
        object.x().write(context, arguments[1] as i16);
        object.y().write(context, arguments[2] as i16);
    }
    Ok(None)
}

/// G/src/jamout/jamout.c:537 (`JamOutPositionEarlCopy`, REV 02 0x03B782).
pub fn position_earl_copy(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let handles = ram::g_jam_earl_entities(context.revision());
    let slot = arguments[0] as i32;
    if handles.at(slot).read(context) != 0 {
        let object = object_from_handle(context, handles.at(slot).read(context));
        object.x().write(context, arguments[1] as i16);
        object.y().write(context, arguments[2] as i16);
    }
    Ok(None)
}

/// G/src/jamout/jamout.c:549 (`JamOutHideToejamCopy`, REV 02 0x03B7C6).
/// A missing handle does nothing; an allocated handle is moved, not released.
pub fn hide_toejam_copy(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let handles = ram::g_jam_toejam_entities(context.revision());
    let slot = arguments[0] as i32;
    if handles.at(slot).read(context) != 0 {
        let object = object_from_handle(context, handles.at(slot).read(context));
        object.x().write(context, 160);
        object.y().write(context, -50);
    }
    Ok(None)
}

/// G/src/jamout/jamout.c:561 (`JamOutHideEarlCopy`, REV 02 0x03B7FE).
pub fn hide_earl_copy(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let handles = ram::g_jam_earl_entities(context.revision());
    let slot = arguments[0] as i32;
    if handles.at(slot).read(context) != 0 {
        let object = object_from_handle(context, handles.at(slot).read(context));
        object.x().write(context, 160);
        object.y().write(context, -50);
    }
    Ok(None)
}

/// G/src/jamout/jamout.c:573 (`JamOutSetToejamSequence`, REV 02 0x03B836).
/// A lock blocks every store; otherwise the cache changes even without a lead.
pub fn set_toejam_sequence(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    if ram::g_jam_toejam_animation_locked(revision).read(context) != 0 {
        return Ok(None);
    }
    let sequence = Ptr::new(arguments[0]);
    let handles = ram::g_jam_toejam_entities(revision);
    if handles.at(0).read(context) != 0 {
        let object = object_from_handle(context, handles.at(0).read(context));
        object.sequence().write(context, sequence);
        object.frame_duration().write(context, 1);
        object.frame().write(context, -1);
        object
            .frame_phase()
            .write(context, arguments[1] as i16 as i8);
    }
    ram::g_jam_toejam_animation_sequence(revision).write(context, sequence);
    Ok(None)
}

/// G/src/jamout/jamout.c:591 (`JamOutSetEarlSequence`, REV 02 0x03B880).
pub fn set_earl_sequence(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    if ram::g_jam_earl_animation_locked(revision).read(context) != 0 {
        return Ok(None);
    }
    let sequence = Ptr::new(arguments[0]);
    let handles = ram::g_jam_earl_entities(revision);
    if handles.at(0).read(context) != 0 {
        let object = object_from_handle(context, handles.at(0).read(context));
        object.sequence().write(context, sequence);
        object.frame_duration().write(context, 1);
        object.frame().write(context, -1);
        object
            .frame_phase()
            .write(context, arguments[1] as i16 as i8);
    }
    ram::g_jam_earl_animation_sequence(revision).write(context, sequence);
    Ok(None)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod scene_tests;
