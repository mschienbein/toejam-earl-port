use super::decisions::next_cached_random;
use crate::{Context, Result, call, call_void, ids};
use tje_mem::{Address, EarthlingState, ObjectState, ram};

fn active_player_motion(context: &dyn Context, player: Address<ObjectState>) -> bool {
    player.sprite_state().read(context) != 0
        && player.sprite_state().read(context) != 4
        && player.animation_group_mask().read(context) & 0x2040_0000 == 0
        && player.sprite_state().read(context) != 76
}
fn in_range(
    context: &mut dyn Context,
    santa: Address<ObjectState>,
    player: Address<ObjectState>,
    x: u32,
    y: u32,
) -> Result<bool> {
    Ok(call(
        context,
        ids::ENTITY_IS_WITHIN_RANGE,
        &[santa.address(), player.address(), x, y],
    )? != 0)
}

/// G/src/earthlings/mole_santa.c:89 (`SantaLogic`).
/// Atomic: RNG, animation, position, sound requests and visibility only. Preserve
/// the stale object pointer after relocation frees it, and full int visibility.
pub fn santa_logic(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let earthling = Address::<EarthlingState>::new(arguments[0]);
    if earthling.object().read(context) == 0 {
        return Ok(None);
    }
    let santa = ram::g_player_info(revision).at(i32::from(earthling.object().read(context)));
    let toejam = ram::g_player_info(revision).at(0);
    let earl = ram::g_player_info(revision).at(1);
    let mut next = -1_i16;
    if earthling.state().read(context) == 3 || earthling.state().read(context) == 2 {
        if santa.frame_duration().read(context) == 0 {
            santa.acceleration_height().write(context, 16);
            next = 5;
            santa
                .draw_flags()
                .write(context, santa.draw_flags().read(context) | 0x40);
            call_void(context, ids::PLAY_SOUND, &[5, 94])?;
        }
    } else if i16::from(earthling.state().read(context)) & 252 == 4 {
        if santa.height().read(context) > 20
            && santa.velocity_x().read(context) == 0
            && santa.velocity_y().read(context) == 0
        {
            next = (next_cached_random(context) & 3) as i16 + 4;
            match next {
                4 | 5 => {
                    next = 4;
                    santa.velocity_y().write(context, -256);
                    santa.acceleration_y().write(context, -32);
                }
                6 => {
                    santa.velocity_x().write(context, -256);
                    santa.acceleration_x().write(context, -32);
                }
                7 => {
                    santa.velocity_x().write(context, 256);
                    santa.acceleration_x().write(context, 32);
                }
                _ => unreachable!(),
            }
            if santa.velocity_y().read(context) != 0 {
                if next_cached_random(context) & 1 != 0 {
                    santa.velocity_x().write(context, -256);
                    santa.acceleration_x().write(context, -16);
                } else {
                    santa.velocity_x().write(context, 256);
                    santa.acceleration_x().write(context, 16);
                }
            } else {
                let velocity =
                    ((call(context, ids::GET_RANDOM_NUMBER, &[])? as u16) % 129) as i16 - 64;
                santa.velocity_y().write(context, velocity);
            }
        }
        if santa.height().read(context) > 30 {
            santa.acceleration_height().write(context, 0);
            santa.velocity_height().write(context, 0);
        }
        if santa.draw_flags().read(context) & 6 == 0 {
            loop {
                call_void(
                    context,
                    ids::EARTHLING_RELOCATE_RECORD,
                    &[earthling.address()],
                )?;
                santa.x().write(context, earthling.x().read(context));
                santa.y().write(context, earthling.y().read(context));
                if call(
                    context,
                    ids::IS_ENTITY_VISIBLE_IN_VIEWPORT,
                    &[santa.address(), 0],
                )? == 0
                    && call(
                        context,
                        ids::IS_ENTITY_VISIBLE_IN_VIEWPORT,
                        &[santa.address(), 1],
                    )? == 0
                {
                    break;
                }
            }
            if toejam.animation_group_mask().read(context) & 0x0780_0000 == 0
                && earl.animation_group_mask().read(context) & 0x0780_0000 == 0
            {
                call_void(context, ids::PLAY_SOUND, &[6, 99])?;
            }
        }
    } else if earthling.state().read(context) == 1 {
        if santa.frame().read(context) > 2 && santa.frame().read(context) < 5 {
            if (in_range(context, santa, toejam, 125, 75)? && active_player_motion(context, toejam))
                || (in_range(context, santa, earl, 150, 75)? && active_player_motion(context, earl))
            {
                next = 2
            }
        } else if santa.frame_duration().read(context) == 0 {
            next = 0
        }
    } else if earthling.state().read(context) == 0 {
        if in_range(context, santa, toejam, 125, 75)? {
            if ram::g_player_burps_left(revision).at(0).read(context) != 0 {
                next = 2
            } else if toejam.animation_group_mask().read(context) & 8 != 0 {
                if (call(context, ids::GET_RANDOM_NUMBER, &[])? as u16) % 100 < 10 {
                    next = 1
                }
            } else if active_player_motion(context, toejam) {
                next = 2
            }
        }
        if next != 2 && in_range(context, santa, earl, 125, 75)? {
            if ram::g_player2_burps_left(revision).read(context) != 0 {
                next = 2
            } else if earl.animation_group_mask().read(context) & 8 != 0 {
                if (call(context, ids::GET_RANDOM_NUMBER, &[])? as u16) % 100 < 10 {
                    next = 1
                }
            } else if active_player_motion(context, earl) {
                next = 2
            }
        }
    }
    if next != -1 {
        call_void(
            context,
            ids::SET_EARTHLING_STATE,
            &[earthling.address(), next as u32],
        )?;
        santa.frame().write(context, -1);
    }
    Ok(None)
}
