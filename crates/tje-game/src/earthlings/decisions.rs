use crate::{Context, Result, call, ids};
use tje_mem::{Address, EarthlingState, ObjectState, ram};

/// G/src/earthlings/spawning_and_banks.c:78 (`EarthlingIsSpecialRecordNearby`).
/// Atomic: two bounded RAM records and the no-wait mailbox predicate.
pub fn earthling_is_special_record_nearby(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let x = arguments[0] as i16;
    let y = arguments[1] as i16;
    let level = arguments[2] as i16;
    let bank = if ram::g_earthling_bank_users(revision).at(0).read(context) != 0
        && i16::from(ram::g_earthling_bank_levels(revision).at(0).read(context)) == level
    {
        0
    } else {
        1
    };
    for slot in 0..2 {
        let earthling = ram::g_earthling_banks(revision).at(bank).at(slot);
        if earthling.kind().read(context) == 0
            || call(
                context,
                ids::EARTHLING_IS_MAIL_ORDER_MAILBOX,
                &[earthling.address()],
            )? != 0
        {
            let dx = (i32::from(x) - i32::from(earthling.x().read(context))).wrapping_abs() as i16;
            let dy = (i32::from(y) - i32::from(earthling.y().read(context))).wrapping_abs() as i16;
            if dx < 340 && dy < 200 {
                return Ok(Some(1));
            }
        }
    }
    Ok(Some(0))
}

/// G/src/earthlings/movement_decision.c:12 (`EarthlingSelectTurnState`).
/// Atomic: collision/decoy handling and a single cached RNG access, with no waits.
pub fn earthling_select_turn_state(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let earthling = Address::<EarthlingState>::new(arguments[0]);
    if earthling.kind().read(context) == 18 && earthling.object().read(context) != 0 {
        let object = ram::g_player_info(revision).at(i32::from(earthling.object().read(context)));
        let x = object.x().read(context) as u32;
        let y = object.y().read(context) as u32;
        let collision = call(
            context,
            ids::ENTITY_FIND_HORIZONTAL_COLLISION_AT_X,
            &[object.address(), x, y, 0],
        )? as i16;
        if collision != 0 {
            call(
                context,
                ids::PRESENT_REGISTER_DECOY_HIT,
                &[(i32::from(collision) - 1) as u32],
            )?;
            return Ok(Some(u32::MAX));
        }
    }
    match earthling.state().read(context) {
        6 => Ok(Some(0)),
        7 => Ok(Some(1)),
        _ => Ok(Some(u32::from(next_cached_random(context) & 1 == 0))),
    }
}

pub(super) fn next_cached_random(context: &mut dyn Context) -> u16 {
    let revision = context.revision();
    let index = ram::g_random_cache_index(revision);
    let old = index.read(context);
    let next = if old == 58 { 0 } else { old.wrapping_add(1) };
    index.write(context, next);
    ram::g_cached_random_values(revision)
        .at(i32::from(next))
        .read(context)
}

/// G/src/earthlings/steering_helpers.c:29 (`EntityTriggerTargetStatusEventOnce`).
/// Atomic: a RAM sound request, followed by the latch store; no hardware polling.
pub fn entity_trigger_target_status_event_once(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let state = Address::<ObjectState>::new(arguments[0]);
    let target = Address::<ObjectState>::new(arguments[1]);
    if state.attachment_x_or_latch().read(context) == 0
        && target.animation_group_mask().read(context) & 0x0004_0800 != 0
    {
        call(context, ids::AUDIO_REQUEST_CHANNEL_TEMPLATE_EFFECT, &[9])?;
        state.attachment_x_or_latch().write(context, 1);
    }
    Ok(None)
}

/// G/src/earthlings/steering.c:43 (`EntitySteerWithHorizontalDrift`).
/// Atomic: velocity RAM and at most one direct RNG draw.
pub fn entity_steer_with_horizontal_drift(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let state = Address::<ObjectState>::new(arguments[0]);
    let target = Address::<ObjectState>::new(arguments[1]);
    let previous_x = state.velocity_x().read(context);
    call(
        context,
        ids::ENTITY_SET_VELOCITY_TOWARD_DELTA,
        &[
            state.address(),
            arguments[2] as i16 as u32,
            arguments[3] as i16 as u32,
        ],
    )?;
    if state.velocity_x().read(context) == 0 && state.velocity_y().read(context) != 0 {
        let velocity = if previous_x != 0 {
            previous_x
        } else if call(context, ids::GET_RANDOM_NUMBER, &[])? & 1 != 0 {
            256
        } else {
            -256
        };
        state.velocity_x().write(context, velocity);
    }
    Ok(Some(
        if state.x().read(context) > target.x().read(context) {
            6
        } else {
            7
        },
    ))
}
