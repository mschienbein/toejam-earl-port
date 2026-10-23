use super::frames::{call_from, with_frame};
use crate::call_frame::{FrameLayout, LocalFrame};
use crate::{Context, Result, ids};
use tje_mem::{Address, ObjectState, ram, rom};

/// G/src/player/sprite_checks_and_movement.c:205 (`PlayerApplyDirectionalMovementAnimation`).
/// Matching c_sprite_checks_and_movement.s:465: MOVEM D2-D3/A2, no LINK.
/// The sprite parameter stays in incoming argument slot 3 and is mutated with
/// word stores; velocity/mode parameters are reread from their original slots.
/// True return offsets 0x8a, 0x154 and 0x2c2 match both revision ELFs.
pub(super) fn player_apply_directional_movement_animation(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    with_frame(
        context,
        FrameLayout::movem(0, 0x0c, 0x04),
        |context, stack| apply(context, arguments, stack),
    )
}

fn apply(context: &mut dyn Context, arguments: &[u32], stack: &LocalFrame) -> Result<Option<u32>> {
    let player = Address::<ObjectState>::new(arguments[0]);
    let number = i32::from(arguments[1] as i16);
    let input = arguments[2] as i16;
    let sprite = stack.argument_word(3);
    let velocity_x = stack.argument_word(4);
    let velocity_y = stack.argument_word(5);
    let revision = context.revision();
    let caller = ids::PLAYER_APPLY_DIRECTIONAL_MOVEMENT_ANIMATION;
    let mut frame = player.frame().read(context);
    if player.animation_group_mask().read(context) & 0x8000 != 0 {
        player
            .acceleration_x()
            .write(context, velocity_x.read(context) >> 4);
        player
            .acceleration_y()
            .write(context, velocity_y.read(context) >> 4);
        frame = frame.wrapping_sub(1);
    } else if player.animation_group_mask().read(context) & 0x800_0000 != 0 {
        player.velocity_x().write(context, velocity_x.read(context));
        player
            .velocity_y()
            .write(context, velocity_y.read(context) >> 1);
        player.animation_motion_axes().write(context, 0);
    } else {
        player.velocity_x().write(context, velocity_x.read(context));
        player.velocity_y().write(context, velocity_y.read(context));
        player
            .animation_motion_axes()
            .write(context, stack.argument_byte(6).read(context));
    }
    if sprite.read(context) != -1 {
        if sprite.read(context) == 0 {
            let active = call_from(
                context,
                caller,
                ids::GET_ACTIVE_PRESENT_SPRITE,
                &[player.address()],
                0x8a,
            )?;
            sprite.write(context, active as i16);
            frame = -1;
        } else {
            sprite.write(context, sprite.read(context) & 3);
            if player.animation_group_mask().read(context) & 0x40000 != 0
                || player.sprite_state().read(context) == 77
            {
                sprite.write(context, sprite.read(context).wrapping_add(72));
            } else if player.animation_group_mask().read(context) & 0x800 != 0
                || matches!(player.sprite_state().read(context), 3 | 7)
            {
                sprite.write(context, sprite.read(context).wrapping_add(44));
            } else if player.animation_group_mask().read(context) & 0x4000 != 0 {
                sprite.write(context, sprite.read(context).wrapping_add(56));
            } else if player.animation_group_mask().read(context) & 0x8000 != 0 {
                sprite.write(context, sprite.read(context).wrapping_add(60));
            } else if input & 0x40 != 0
                && ram::g_player_elevator_flags(revision)
                    .at(number)
                    .read(context)
                    == 0
                && ram::g_player_powerups(revision).at(number).read(context) == 7
            {
                sprite.write(context, sprite.read(context).wrapping_add(108));
                if player.animation_group_mask().read(context) & 0x800_0000 == 0 {
                    call_from(
                        context,
                        caller,
                        ids::PLAY_SOUND_EFFECT,
                        &[rom::sfx_running(revision).address(), 50, 0],
                        0x154,
                    )?;
                }
            } else if input & 0x40 != 0
                && ram::g_player_powerups(revision).at(number).read(context) == -1
            {
                sprite.write(context, sprite.read(context).wrapping_add(12));
            } else {
                sprite.write(context, sprite.read(context).wrapping_add(8));
                if player.animation_group_mask().read(context) & 0x800_0000 != 0 {
                    player.animation_motion_axes().write(context, 3);
                }
            }
        }
    } else if sprite.read(context) == -1 {
        if input & 0x40 != 0 {
            if player.animation_group_mask().read(context) & 4 != 0 {
                if ram::g_player_powerups(revision).at(number).read(context) == -1 {
                    sprite.write(
                        context,
                        i16::from(player.sprite_state().read(context)).wrapping_add(4),
                    );
                } else if ram::g_player_powerups(revision).at(number).read(context) == 7
                    && ram::g_player_elevator_flags(revision)
                        .at(number)
                        .read(context)
                        == 0
                {
                    sprite.write(
                        context,
                        i16::from(player.sprite_state().read(context) & 3) + 108,
                    );
                    call_from(
                        context,
                        caller,
                        ids::PLAY_SOUND_EFFECT,
                        &[rom::sfx_running(revision).address(), 50, 0],
                        0x154,
                    )?;
                }
            }
        } else if player.animation_group_mask().read(context) & 0x0800_0008 != 0
            && player.frame_duration().read(context) <= 1
        {
            sprite.write(
                context,
                i16::from(player.sprite_state().read(context) & 3) + 8,
            );
            player.animation_motion_axes().write(context, 3);
            if player.animation_group_mask().read(context) & 0x800_0000 != 0 {
                player
                    .velocity_x()
                    .write(context, velocity_x.read(context) / 3);
                player
                    .velocity_y()
                    .write(context, velocity_y.read(context) / 3);
            }
        }
    }
    if sprite.read(context) != -1 {
        let duration = if i32::from(sprite.read(context)) & 0xfc
            == i32::from(player.sprite_state().read(context)) & 0xfc
        {
            let current = player.frame_duration().read(context);
            if current == 0 { 1 } else { current }
        } else {
            if matches!(player.sprite_state().read(context), 0 | 4 | 76)
                || player.animation_group_mask().read(context) & 0x2040_0000 != 0
            {
                frame = -1;
            }
            1
        };
        let selected = sprite.read(context) as u32;
        call_from(
            context,
            caller,
            ids::SET_SPRITE,
            &[player.address(), selected],
            0x2c2,
        )?;
        player.frame_duration().write(context, duration);
        player.frame().write(context, frame);
        let sequence = player.sequence().read(context).target();
        let mut index = 1_i16;
        while i32::from(index) <= i32::from(frame) + 1 {
            if i32::from(sequence.offset(i32::from(index)).read(context)) & 0xf0 == 0xf0 {
                player.frame().write(context, -1);
                break;
            }
            index = index.wrapping_add(1);
        }
    }
    Ok(None)
}
