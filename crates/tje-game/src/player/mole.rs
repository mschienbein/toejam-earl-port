use crate::{Context, Result, call, call_void, ids};
use tje_mem::{Address, EarthlingState, ObjectState, Ptr, ram, rom};

/// G/src/player/mole.c:23 (`DoMoleAbility`). Inventory loss precedes allocation;
/// failed allocation does not restore the stolen item.
pub(super) fn do_mole_ability(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let victim = Address::<ObjectState>::new(arguments[0]);
    let number = i32::from(arguments[1] as i16);
    let revision = context.revision();
    let inventory = ram::g_player_inventory(revision).at(number);
    let mut count = 0_i16;
    while count < 16 && inventory.at(i32::from(count)).read(context) >= 0 {
        count += 1;
    }
    if count == 0 {
        return Ok(None);
    }
    count -= 1;
    let selected = if count > 0 {
        (u32::from(super::poses::next_random(context)) % (i32::from(count) + 1) as u32) as i16
    } else {
        0
    };
    let present_number = inventory.at(i32::from(selected)).read(context);
    if selected < count {
        inventory
            .at(i32::from(selected))
            .write(context, inventory.at(i32::from(count)).read(context));
    }
    inventory.at(i32::from(count)).write(context, -1);
    let slot = ram::g_player_stolen_present_entities(revision).at(number);
    let allocated = call(context, ids::ALLOCATE_ENTITY, &[])? as i8;
    slot.write(context, allocated);
    if allocated != 0 {
        let present = ram::g_player_info(revision).at(i32::from(slot.read(context)));
        present.x().write(context, victim.x().read(context));
        present
            .y()
            .write(context, victim.y().read(context).wrapping_sub(3));
        present.height().write(context, 20);
        present.acceleration_height().write(context, -32);
        let velocity = if super::poses::next_random(context) & 1 != 0 {
            256
        } else {
            -256
        };
        present.velocity_x().write(context, velocity);
        present.level().write(context, victim.level().read(context));
        present.width().write(context, 24);
        present.height_pixels().write(context, 16);
        present.sprite_count().write(context, 1);
        present.tile_count().write(context, 6);
        present.tile().write(context, 0x8000);
        let appearance = ram::g_present_info(revision)
            .at(i32::from(present_number))
            .appearance()
            .read(context);
        present.frames().write(
            context,
            Ptr::new(
                rom::present_sprite_table(revision)
                    .at(i32::from(appearance))
                    .address(),
            ),
        );
        present.sequence().write(
            context,
            Ptr::new(rom::single_frame_hold_animation(revision).address()),
        );
        present.frame().write(context, -1);
        present.frame_duration().write(context, 1);
        present
            .draw_flags()
            .write(context, present.draw_flags().read(context) | 0x40);
    }
    let counter = ram::g_player_mole_theft_counts(revision).at(number);
    counter.write(context, counter.read(context).wrapping_add(1));
    Ok(None)
}

/// G/src/player/mole.c:77 (`MoleAbility`).
pub(super) fn mole_ability(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let number = i32::from(arguments[0] as i16);
    let earthling = Address::<EarthlingState>::new(arguments[1]);
    let revision = context.revision();
    let victim = super::indexing::player_by_word_index(revision, number);
    if earthling.state().read(context) == 3 {
        if victim.sprite_state().read(context) == 70 {
            if earthling.timer().read(context) < 15 {
                let sprite = call(context, ids::GET_ACTIVE_PRESENT_SPRITE, &[victim.address()])?;
                call_void(context, ids::SET_SPRITE, &[victim.address(), sprite])?;
                victim
                    .behavior_flags()
                    .write(context, victim.behavior_flags().read(context) & !0x20);
            } else if ram::g_player_stolen_present_entities(revision)
                .at(number)
                .read(context)
                == 0
            {
                let count = ram::g_player_mole_theft_counts(revision).at(number);
                if count.read(context) == 0
                    || (count.read(context) < 3
                        && u32::from(super::poses::next_random(context)) % 100 < 8)
                {
                    call_void(
                        context,
                        ids::DO_MOLE_ABILITY,
                        &[victim.address(), number as u32],
                    )?;
                }
            }
        }
        return Ok(None);
    }
    let mole = ram::g_player_info(revision).at(i32::from(earthling.object().read(context)));
    if ram::g_player_inventory(revision)
        .at(number)
        .at(0)
        .read(context)
        == -1
        || (number == 0 && mole.attachment_x_or_latch().read(context) != 0)
        || (number == 1 && mole.attachment_y_or_latch().read(context) != 0)
        || call(
            context,
            ids::PLAYER_CAN_RECEIVE_EARTHLING_EFFECT,
            &[victim.address(), number as u32],
        )? == 0
    {
        return Ok(None);
    }
    let args = [
        mole.x().read(context) as u32,
        mole.y().read(context) as u32,
        victim.level().read(context) as u32,
    ];
    if call(context, ids::ELEVATOR_CONTAINS_POSITION, &args)? == 0 {
        victim.x().write(context, mole.x().read(context));
        victim.y().write(context, mole.y().read(context));
    } else {
        mole.x().write(context, victim.x().read(context));
        mole.y().write(context, victim.y().read(context));
    }
    call_void(context, ids::SET_EARTHLING_STATE, &[earthling.address(), 3])?;
    earthling.timer().write(context, 30);
    mole.frame().write(context, -1);
    mole.behavior_flags()
        .write(context, mole.behavior_flags().read(context) | 1);
    mole.draw_flags()
        .write(context, mole.draw_flags().read(context) | 0x80);
    if number == 0 {
        mole.attachment_x_or_latch().write(
            context,
            mole.attachment_x_or_latch().read(context).wrapping_add(1),
        );
    } else {
        mole.attachment_y_or_latch().write(
            context,
            mole.attachment_y_or_latch().read(context).wrapping_add(1),
        );
    }
    call_void(context, ids::SET_SPRITE, &[victim.address(), 70])?;
    victim.animation_motion_axes().write(context, 3);
    victim
        .behavior_flags()
        .write(context, victim.behavior_flags().read(context) | 0x20);
    ram::g_player_mole_theft_counts(revision)
        .at(number)
        .write(context, 0);
    Ok(None)
}
