use crate::{
    Context, Result, call_at,
    call_frame::{CallSite, FrameLayout, LocalFrame, OutgoingArguments},
    ids,
};
use tje_mem::{Address, ObjectState, arithmetic, ram};

/// G/src/camera/split_views.c:12 (`AnimatePlayersIntoSplitViews`).
/// Real VBlank waits retain original interrupt work; 26.6 intermediates and the
/// original zero-distance divide trap are not replaced with a special case.
pub fn animate_players_into_split_views(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    // Preserve the real call-frame depth and caller-owned argument areas.
    // The non-address-taken scalar locals may remain readable host values.
    let frame = LocalFrame::enter(context, FrameLayout::link(36, 0xfc, 0x1c))?;
    let result = split_players_body(context);
    let result = result.map_err(|error| context.protect_timing_failure(error));
    if result
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        return result.map(|()| None);
    }
    let leave = frame.leave(context);
    result?;
    leave?;
    Ok(None)
}

fn split_target_y(context: &dyn Context, entity: Address<ObjectState>, bias: i32) -> i32 {
    // The compiler reads height_pixels before camera Y, then signed height.
    let pixels = i32::from(entity.height_pixels().read(context)) / 2;
    let camera = i32::from(ram::g_camera1_pending_y(context.revision()).read(context));
    let height = i32::from(entity.height().read(context)) / 2;
    pixels
        .wrapping_add(camera.wrapping_add(bias))
        .wrapping_add(height)
}

fn split_divide(context: &mut dyn Context, numerator: i32, divisor: i32, site: u32) -> Result<i32> {
    context.set_data_register(0, numerator as u32);
    context.set_data_register(1, divisor as u32);
    let value = arithmetic::div_i32(numerator, divisor)?;
    if i16::try_from(divisor).is_ok() && i16::try_from(value).is_ok() {
        context.set_data_register(0, value as u32);
        Ok(value)
    } else {
        // The emitted DIVS fast path falls back to the actual register helper.
        let revision = context.revision();
        Ok(call_at(
            context,
            ids::LDIVT,
            &[],
            CallSite::relative(ids::ANIMATE_PLAYERS_INTO_SPLIT_VIEWS, revision, site),
        )? as i32)
    }
}

fn split_players_body(context: &mut dyn Context) -> Result<()> {
    let revision = context.revision();
    let caller = ids::ANIMATE_PLAYERS_INTO_SPLIT_VIEWS;
    let dispatcher = context.dispatcher();
    let first = ram::g_player_info(revision).at(0);
    let second = first.offset(1);
    context.set_address_register(2, first.address());
    let coordinates = [first.x(), first.y(), second.x(), second.y()];
    let saved = coordinates.map(|coordinate| coordinate.read(context));
    let mut positions =
        coordinates.map(|coordinate| i32::from(coordinate.read(context)).wrapping_shl(6));
    let targets = [
        i32::from(ram::g_camera1_pending_x(revision).read(context))
            .wrapping_add(160)
            .wrapping_shl(6),
        split_target_y(context, first, 56).wrapping_shl(6),
        i32::from(ram::g_camera1_pending_x(revision).read(context))
            .wrapping_add(160)
            .wrapping_shl(6),
        split_target_y(context, second, 168).wrapping_shl(6),
    ];
    let mut deltas =
        std::array::from_fn::<_, 4, _>(|index| targets[index].wrapping_sub(positions[index]));
    let mut distance = call_at(
        context,
        ids::ABS,
        &[deltas[0] as u32],
        CallSite::relative(caller, revision, 0x12e),
    )? as i32;
    for (index, sites) in [
        (1, [0x13a, 0x148]),
        (2, [0x156, 0x166]),
        (3, [0x174, 0x184]),
    ] {
        let magnitude = call_at(
            context,
            ids::ABS,
            &[deltas[index] as u32],
            CallSite::relative(caller, revision, sites[0]),
        )? as i32;
        if distance < magnitude {
            distance = call_at(
                context,
                ids::ABS,
                &[deltas[index] as u32],
                CallSite::relative(caller, revision, sites[1]),
            )? as i32;
        }
    }
    let divisor = (distance >> 7).wrapping_add(192);
    distance = split_divide(context, distance, divisor, 0x1ac)?;
    for (delta, site) in deltas.iter_mut().zip([0x1d2, 0x1f8, 0x21e, 0x246]) {
        *delta = split_divide(context, *delta, distance, site)?;
    }
    loop {
        let mut finished = true;
        let head = ram::g_entity_draw_list_head(revision)
            .read(context)
            .address();
        let position_args = OutgoingArguments::push(context, &[head])?;
        position_args.call(
            context,
            dispatcher,
            ids::POSITION_ENTITY_SPRITES,
            CallSite::relative(caller, revision, 0x264),
        )?;
        let wait = OutgoingArguments::push(context, &[1])?;
        wait.call(
            context,
            dispatcher,
            ids::WAIT_FOR_NEXT_V_BLANK,
            CallSite::relative(caller, revision, 0x26e),
        )?;
        let upload = OutgoingArguments::push(context, &[1, 0])?;
        upload.call(
            context,
            dispatcher,
            ids::UPLOAD_QUEUED_SPRITE_GRAPHICS,
            CallSite::relative(caller, revision, 0x27a),
        )?;
        upload.release(context)?;
        wait.release(context)?;
        position_args.release(context)?;
        for index in 0..4 {
            let remaining = targets[index].wrapping_sub(positions[index]);
            if deltas[index] != 0 && remaining != 0 && (remaining ^ deltas[index]) >= 0 {
                finished = false;
                positions[index] = positions[index].wrapping_add(deltas[index]);
                coordinates[index].write(context, (positions[index] >> 6) as i16);
            }
        }
        if finished {
            break;
        }
    }
    for (coordinate, value) in coordinates.into_iter().zip(saved) {
        coordinate.write(context, value);
    }
    Ok(())
}

/// G/src/camera/split_screen_wipe.c:12 (`AnimateSplitScreenWipe`).
/// Both two-pass loops retain their final lower-row command, including pass two.
/// The allocation loop preserves entry D4: c_split_screen_wipe.s:4-46 does not
/// initialize it before AllocateEntityViewportSprites's residual-tile bug path.
pub fn animate_split_screen_wipe(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x3c, 0x0c))?;
    let result = split_wipe_body(context);
    let result = result.map_err(|error| context.protect_timing_failure(error));
    if result
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        return result.map(|()| None);
    }
    let leave = frame.leave(context);
    result?;
    leave?;
    Ok(None)
}

fn set_word(context: &mut dyn Context, register: usize, value: i16) {
    let high = context.data_register(register) & 0xffff0000;
    context.set_data_register(register, high | u32::from(value as u16));
}

fn split_wipe_body(context: &mut dyn Context) -> Result<()> {
    let revision = context.revision();
    let caller = ids::ANIMATE_SPLIT_SCREEN_WIPE;
    let dispatcher = context.dispatcher();
    context.set_address_register(3, ram::g_player_info(revision).address());
    set_word(context, 3, 0);
    context.set_address_register(2, context.address_register(3));
    while (context.data_register(3) as i16) < 2 {
        while Address::<ObjectState>::new(context.address_register(2))
            .draw_flags()
            .read(context)
            & 2
            == 0
        {
            let entity = Address::<ObjectState>::new(context.address_register(2));
            context.discard_u8(entity.sprite_ready().address())?;
            entity.sprite_ready().write(context, 0);
            let allocation = OutgoingArguments::push(context, &[entity.address(), 0])?;
            allocation.call(
                context,
                dispatcher,
                ids::ALLOCATE_ENTITY_VIEWPORT_SPRITES,
                CallSite::relative(caller, revision, 0x20),
            )?;
            let resident = Address::<ObjectState>::new(context.address_register(2))
                .draw_flags()
                .read(context)
                & 2
                != 0;
            allocation.release(context)?;
            if !resident {
                call_at(
                    context,
                    ids::ENTITY_RELEASE_ONE_BY_PRIORITY,
                    &[],
                    CallSite::relative(caller, revision, 0x30),
                )?;
            }
            // Compound allocation poll: retain the address reload, CLR,
            // allocation and conditional release before its next BTST.
            context.poll_yield()?;
        }
        let next = (context.data_register(3) as i16).wrapping_add(1);
        set_word(context, 3, next);
        context.set_address_register(2, context.address_register(2).wrapping_add(128));
    }
    while {
        let first = Address::<ObjectState>::new(context.address_register(3));
        first.sprite_ready().read(context) == 0 || first.offset(1).sprite_ready().read(context) == 0
    } {
        call_at(
            context,
            ids::UPDATE_AND_QUEUE_ENTITY_SPRITES,
            &[],
            CallSite::relative(caller, revision, 0x4c),
        )?;
        let wait = OutgoingArguments::push(context, &[1])?;
        wait.call(
            context,
            dispatcher,
            ids::WAIT_FOR_NEXT_V_BLANK,
            CallSite::relative(caller, revision, 0x56),
        )?;
        let upload = OutgoingArguments::push(context, &[0, 0])?;
        upload.call(
            context,
            dispatcher,
            ids::UPLOAD_QUEUED_SPRITE_GRAPHICS,
            CallSite::relative(caller, revision, 0x60),
        )?;
        upload.release(context)?;
        wait.release(context)?;
        // Compound readiness poll, not an aligned/frame-counter proxy. The
        // next iteration still owns both original short-circuited byte reads.
        context.poll_yield()?;
    }
    let wait = OutgoingArguments::push(context, &[1])?;
    wait.call(
        context,
        dispatcher,
        ids::WAIT_FOR_NEXT_V_BLANK,
        CallSite::relative(caller, revision, 0x7a),
    )?;
    call_at(
        context,
        ids::LINK_PLAYER_SPRITES_FOR_SPLIT_WIPE,
        &[],
        CallSite::relative(caller, revision, 0x80),
    )?;
    let cursor = ram::g_camera1_tile_row_cursor(revision);
    let mut upper = cursor.read(context) & 31;
    set_word(context, 5, upper);
    let mut lower = cursor.read(context).wrapping_add(29) & 31;
    set_word(context, 4, lower);
    wait.release(context)?;
    for step in 0..15 {
        set_word(context, 3, step);
        let wait = OutgoingArguments::push(context, &[1])?;
        wait.call(
            context,
            dispatcher,
            ids::WAIT_FOR_NEXT_V_BLANK,
            CallSite::relative(caller, revision, 0xa6),
        )?;
        wait.release(context)?;
        context.write_vdp_command((upper as u32).wrapping_shl(23).wrapping_add(0x60000002));
        for pass in 0..2 {
            set_word(context, 2, pass);
            for _ in 0..64 {
                context.write_vdp_data(0x25b4);
            }
            context.write_vdp_command((lower as u32).wrapping_shl(23).wrapping_add(0x60000002));
            set_word(context, 2, pass + 1);
        }
        if step == 14 {
            break;
        }
        upper = upper.wrapping_add(1) & 31;
        lower = lower.wrapping_sub(1) & 31;
        set_word(context, 5, upper);
        set_word(context, 4, lower);
        context.write_vdp_command((upper as u32).wrapping_shl(23).wrapping_add(0x60000002));
        for pass in 0..2 {
            set_word(context, 2, pass);
            for _ in 0..64 {
                context.write_vdp_data(0x05ab);
            }
            context.write_vdp_command((lower as u32).wrapping_shl(23).wrapping_add(0x60000002));
            set_word(context, 2, pass + 1);
        }
    }
    call_at(
        context,
        ids::SELECT_SPRITE_CHAIN_START,
        &[1],
        CallSite::relative(caller, revision, 0x160),
    )?;
    for row in 0..28 {
        set_word(context, 2, row as i16);
        context.write_vdp_command((row as u32).wrapping_shl(23).wrapping_add(0x50000003));
        for _ in 0..40 {
            context.write_vdp_data(0xa5b4);
        }
        set_word(context, 2, (row + 1) as i16);
    }
    context.write_vdp_control(0x921c);
    Ok(())
}

#[cfg(test)]
#[path = "animation_tests.rs"]
mod tests;
