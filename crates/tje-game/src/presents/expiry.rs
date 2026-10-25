use crate::call_frame::{CallSite, FrameLayout, LocalFrame, OutgoingArguments};
use crate::{Context, FunctionId, Result, call_at, call_at_void, ids};
use tje_mem::{Address, ObjectState, ram};

/// G/src/presents/item_updates.c:259 (`PresentTryEndActivePowerup`).
/// TimeAware: the action-icon upload can require the VBlank interrupt.
/// G/build/rev{02,00}/asm/c_item_updates.s:594-724 also defines the observable
/// MOVEM frame, live register residues and final CMPI.B, including the no-sprite
/// return's Z flag. Object offsets below are relative to function start 0x67c.
pub fn present_try_end_active_powerup(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x1c, 0x04))?;
    let result = expiry_body(context, arguments[0] as i16);
    if result
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        return result;
    }
    frame.leave(context)?;
    result
}

fn expiry_body(context: &mut dyn Context, index: i16) -> Result<Option<u32>> {
    let revision = context.revision();
    move_word(context, 2, index as u16);
    move_long(context, 3, u32::MAX);
    move_word(context, 0, index as u16);
    let old = context.data_register(0) as u16;
    let shifted = old.wrapping_shl(7);
    context.set_data_register(
        0,
        (context.data_register(0) & 0xffff_0000) | u32::from(shifted),
    );
    let carry = u16::from(old & 0x0200 != 0);
    let overflow = u16::from(old & 0xff00 != 0 && old & 0xff00 != 0xff00);
    context.set_status_register(
        nz(context.status_register() & !0x1f, u32::from(shifted), 16)
            | (carry * 0x11)
            | (overflow << 1),
    );
    let player = super::word_indexed_player(revision, i32::from(index));
    context.set_address_register(0, player.address());
    context.set_address_register(2, player.address());
    logical(context, u32::from(index as u16), 16);
    move_long(context, 0, 0);
    let terrain = if index == 0 {
        ram::g_player1_terrain_underfoot(revision).read(context)
    } else {
        ram::g_player2_terrain_underfoot(revision).read(context)
    };
    move_byte(context, 0, terrain);
    move_word(context, 4, u16::from(terrain));
    move_word(context, 4, u16::from(terrain & 0xf0));
    move_index(context, index);
    context.set_address_register(0, ram::g_player_powerups(revision).address());
    let powerup = ram::g_player_powerups(revision)
        .at(i32::from(index))
        .read(context);
    compare(context, powerup as u8 as u32, 5, 8);
    if powerup == 5 {
        source_call_void(context, ids::PLAY_SOUND, &[6, 99], 0x54)?;
    }
    let height = player.height().read(context);
    logical(context, u32::from(height as u16), 16);
    if height > 0 && masked_group(context, player, 0x0400_8000) {
        logical(context, player.address(), 32); // MOVE.L A2,-(SP)
        source_call_void(context, ids::STOP_PLAYER, &[player.address()], 0x70)?;
        player.acceleration_height().write(context, -32);
        logical(context, 0xffe0, 16);
        direction_sprite(context, player, 20);
        move_index(context, index);
        context.set_address_register(0, ram::g_player_landing_counter_seed(revision).address());
        ram::g_player_landing_counter_seed(revision)
            .at(i32::from(index))
            .write(context, 2);
        logical(context, 2, 8);
    } else {
        let falling = player
            .animation_group_mask()
            .byte_offset(3)
            .cast::<u8>()
            .read(context)
            & 0x20
            != 0;
        context.set_status_register((context.status_register() & !4) | if falling { 0 } else { 4 }); // BTST changes only Z.
        if !falling {
            let height = player.height().read(context);
            logical(context, u32::from(height as u16), 16);
            if height <= 0 {
                ground_expiry(context, player, index)?;
            }
        }
    }
    let sprite = context.data_register(3) as u8;
    compare(context, u32::from(sprite), 0xff, 8);
    if sprite != 0xff {
        move_byte(context, 0, sprite);
        move_word(context, 0, i16::from(sprite as i8) as u16);
        let sprite_argument = i32::from(sprite as i8) as u32;
        move_long(context, 0, sprite_argument);
        // The second push (A2) supplies the flags seen by SetSprite.
        logical(context, player.address(), 32);
        source_call_void(
            context,
            ids::SET_SPRITE,
            &[player.address(), sprite_argument],
            0x1a2,
        )?;
    }
    Ok(None)
}

fn ground_expiry(
    context: &mut dyn Context,
    player: Address<ObjectState>,
    index: i16,
) -> Result<()> {
    let revision = context.revision();
    logical(context, player.address(), 32);
    let stop = retained_call(context, ids::STOP_PLAYER, &[player.address()], 0xb2)?;
    move_index(context, index);
    context.set_address_register(0, ram::g_player_powerups(revision).address());
    stop.release(context)?;
    ram::g_player_powerups(revision)
        .at(i32::from(index))
        .write(context, -1);
    logical(context, 0xff, 8);
    move_index(context, index);
    // The first argument remains live under the second call's argument until
    // after the animation-group AND. Releasing each call immediately is wrong.
    let load = retained_call(
        context,
        ids::HUD_LOAD_ACTION_ICON_SPRITE,
        &[i32::from(index) as u32],
        0xd0,
    )?;
    move_index(context, index);
    let position = retained_call(
        context,
        ids::HUD_POSITION_ACTION_ICON_SPRITE,
        &[i32::from(index) as u32],
        0xdc,
    );
    let position = match position {
        Ok(position) => position,
        Err(error) => {
            if !error.preserves_call_frame() {
                load.release(context)?;
            }
            return Err(error);
        }
    };
    let inner_tube = masked_group(context, player, 0x0004_0000);
    position.release(context)?;
    load.release(context)?;
    let splash = if inner_tube {
        true
    } else {
        let state = player.sprite_state().read(context);
        compare(context, u32::from(state as u8), 77, 8);
        if state == 77 {
            true
        } else if masked_group(context, player, 0x0780_0000)
            || masked_group(context, player, 0x0000_8000)
        {
            let terrain = context.data_register(4) as u16;
            compare(context, u32::from(terrain), 0x80, 16);
            terrain == 0x80
        } else {
            false
        }
    };
    if splash {
        direction_sprite(context, player, 64);
        clear_splash_motion(context, player)?;
    } else {
        if !refresh_eligible(context, player) || masked_group(context, player, 0x2000_0000) {
            return Ok(());
        }
        logical(context, player.address(), 32);
        let sprite = source_call(
            context,
            ids::GET_ACTIVE_PRESENT_SPRITE,
            &[player.address()],
            0x182,
        )? as u8;
        move_byte(context, 3, sprite);
    }
    let flags = player.draw_flags().read(context) & !0x40;
    player.draw_flags().write(context, flags);
    logical(context, u32::from(flags), 8);
    Ok(())
}

fn refresh_eligible(context: &mut dyn Context, player: Address<ObjectState>) -> bool {
    let state = player.sprite_state().read(context);
    logical(context, u32::from(state as u8), 8);
    if state == 0 {
        return true;
    }
    let state = player.sprite_state().read(context);
    compare(context, u32::from(state as u8), 4, 8);
    if state == 4 || masked_group(context, player, 0x2040_0000) {
        return true;
    }
    for expected in [76, 3, 7] {
        let state = player.sprite_state().read(context);
        compare(context, u32::from(state as u8), expected, 8);
        if u32::from(state as u8) == expected {
            return true;
        }
    }
    masked_group(context, player, 0x0780_0000) || masked_group(context, player, 0x0830_c000)
}

fn direction_sprite(context: &mut dyn Context, player: Address<ObjectState>, base: u8) {
    let state = player.sprite_state().read(context) as u8;
    move_byte(context, 3, state);
    move_byte(context, 3, state & 3);
    // 0..3 + 20/64 cannot carry or overflow; ADDI.B clears X as well as V/C.
    let sprite = (context.data_register(3) as u8) + base;
    context.set_status_register(context.status_register() & !0x1f);
    move_byte(context, 3, sprite);
}

fn masked_group(context: &mut dyn Context, player: Address<ObjectState>, mask: u32) -> bool {
    let group = player.animation_group_mask().read(context);
    move_long(context, 0, group);
    let masked = group & mask;
    move_long(context, 0, masked);
    masked != 0
}

fn move_index(context: &mut dyn Context, index: i16) {
    move_word(context, 0, index as u16);
    move_long(context, 0, i32::from(index) as u32);
}

fn source_call(
    context: &mut dyn Context,
    id: FunctionId,
    arguments: &[u32],
    return_offset: u32,
) -> Result<u32> {
    let site = CallSite::relative(
        ids::PRESENT_TRY_END_ACTIVE_POWERUP,
        context.revision(),
        return_offset,
    );
    call_at(context, id, arguments, site)
}

fn source_call_void(
    context: &mut dyn Context,
    id: FunctionId,
    arguments: &[u32],
    return_offset: u32,
) -> Result<()> {
    let site = CallSite::relative(
        ids::PRESENT_TRY_END_ACTIVE_POWERUP,
        context.revision(),
        return_offset,
    );
    call_at_void(context, id, arguments, site)
}

fn retained_call(
    context: &mut dyn Context,
    id: FunctionId,
    arguments: &[u32],
    return_offset: u32,
) -> Result<OutgoingArguments> {
    let outgoing = OutgoingArguments::push(context, arguments)?;
    let dispatcher = context.dispatcher();
    let site = CallSite::relative(
        ids::PRESENT_TRY_END_ACTIVE_POWERUP,
        context.revision(),
        return_offset,
    );
    if let Err(error) = outgoing.call_void(context, dispatcher, id, site) {
        if !error.preserves_call_frame() {
            outgoing.release(context)?;
        }
        return Err(error);
    }
    Ok(outgoing)
}

fn move_byte(context: &mut dyn Context, register: usize, value: u8) {
    context.set_data_register(
        register,
        (context.data_register(register) & !0xff) | u32::from(value),
    );
    logical(context, u32::from(value), 8);
}

fn move_word(context: &mut dyn Context, register: usize, value: u16) {
    context.set_data_register(
        register,
        (context.data_register(register) & !0xffff) | u32::from(value),
    );
    logical(context, u32::from(value), 16);
}

fn move_long(context: &mut dyn Context, register: usize, value: u32) {
    context.set_data_register(register, value);
    logical(context, value, 32);
}

pub(super) fn logical(context: &mut dyn Context, value: u32, bits: u32) {
    context.set_status_register(nz(context.status_register() & !0x0f, value, bits));
}

fn nz(status: u16, value: u32, bits: u32) -> u16 {
    status | (u16::from(value == 0) << 2) | (u16::from(value & (1 << (bits - 1)) != 0) << 3)
}

pub(super) fn compare(context: &mut dyn Context, left: u32, right: u32, bits: u32) {
    context.set_status_register(compare_flags(context.status_register(), left, right, bits));
}

fn compare_flags(status: u16, left: u32, right: u32, bits: u32) -> u16 {
    let mask = (1_u32 << bits) - 1;
    let result = left.wrapping_sub(right) & mask;
    let overflow = (left ^ right) & (left ^ result) & (1 << (bits - 1)) != 0;
    nz(status & !0x0f, result, bits) | (u16::from(overflow) << 1) | u16::from(left < right)
}

/// c_item_updates.s:798: CLR.B performs a discarded byte read before storing.
pub(super) fn clear_splash_motion(
    context: &mut dyn Context,
    player: Address<ObjectState>,
) -> Result<()> {
    context.discard_u8(player.animation_motion_axes().address())?;
    player.animation_motion_axes().write(context, 0);
    logical(context, 0, 8);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::compare_flags;

    #[test]
    fn byte_compare_matches_signed_and_unsigned_branches_for_every_operand() {
        for left in 0_u32..=255 {
            for right in 0_u32..=255 {
                for entry in [0xa5e0, 0xa5ff] {
                    let flags = compare_flags(entry, left, right, 8);
                    assert_eq!(flags & !0x0f, entry & !0x0f);
                    assert_eq!(flags & 4 != 0, left == right);
                    assert_eq!(flags & 1 != 0, left < right);
                    assert_eq!(
                        (flags & 8 != 0) ^ (flags & 2 != 0),
                        (left as i8) < (right as i8),
                    );
                }
            }
        }
    }

    #[test]
    fn no_sprite_return_sets_z_and_preserves_every_other_status_bit() {
        for entry in 0_u16..=u16::MAX {
            assert_eq!(compare_flags(entry, 0xff, 0xff, 8), (entry & !0x0f) | 4);
        }
    }
}
