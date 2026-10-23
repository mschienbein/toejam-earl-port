//! Earthlings translation ownership; see docs/PORTING.md.

use crate::{Context, Result, ids};
use tje_mem::{Address, EarthlingState, ObjectState, ram};

/// Available Rust entries owned by this subsystem.
pub static TRANSLATIONS: &[crate::Translation] = &[
    crate::Translation {
        id: ids::MAP_GET_SHIP_PIECE_WORLD_POSITION,
        function: map_get_ship_piece_world_position,
    },
    crate::Translation {
        id: ids::EARTHLING_CLEAR_MOLE_PLAYER_CONTACT_FLAGS,
        function: earthling_clear_mole_player_contact_flags,
    },
    crate::Translation {
        id: ids::EARTHLING_IS_UPDATE_BLOCKED,
        function: earthling_is_update_blocked,
    },
    crate::Translation {
        id: ids::EARTHLING_CAN_TARGET_PLAYER_STATE,
        function: earthling_can_target_player_state,
    },
    crate::Translation {
        id: ids::ENTITY_SET_VELOCITY_TOWARD_DELTA,
        function: entity_set_velocity_toward_delta,
    },
    crate::Translation {
        id: ids::ENTITY_STEER_WITH_VERTICAL_DEAD_ZONE,
        function: entity_steer_with_vertical_dead_zone,
    },
    crate::Translation {
        id: ids::EARTHLING_IS_MAIL_ORDER_MAILBOX,
        function: earthling_is_mail_order_mailbox,
    },
    crate::Translation {
        id: ids::NICE_EARTHLING_CHECK,
        function: nice_earthling_check,
    },
];

/// Reviewed leaf entries: only bounded RAM/ROM accesses, no callees or waits.
pub static ATOMIC_CAPABLE: &[crate::FunctionId] = &[
    ids::MAP_GET_SHIP_PIECE_WORLD_POSITION,
    ids::EARTHLING_CLEAR_MOLE_PLAYER_CONTACT_FLAGS,
    ids::EARTHLING_IS_UPDATE_BLOCKED,
    ids::EARTHLING_CAN_TARGET_PLAYER_STATE,
    ids::ENTITY_SET_VELOCITY_TOWARD_DELTA,
    ids::ENTITY_STEER_WITH_VERTICAL_DEAD_ZONE,
    ids::EARTHLING_IS_MAIL_ORDER_MAILBOX,
    ids::NICE_EARTHLING_CHECK,
];

/// G/src/earthlings/spawning_and_banks.c:178 (`MapGetShipPieceWorldPosition`).
/// Read both signed map bytes before either output store, including aliasing.
pub fn map_get_ship_piece_world_position(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let map_x = i32::from(ram::g_ship_piece_spawn_map_x(revision).read(context));
    let map_y = i32::from(ram::g_ship_piece_spawn_map_y(revision).read(context));
    Address::<i16>::new(arguments[0]).write(context, ((map_x + 2) * 128) as i16);
    Address::<i16>::new(arguments[1]).write(context, ((map_y + 2) * 64 + 8) as i16);
    Ok(None)
}

/// G/src/earthlings/spawning_and_banks.c:849 (`EarthlingClearMolePlayerContactFlags`).
/// The fallback bank is bank one even when it is not retained for this level.
pub fn earthling_clear_mole_player_contact_flags(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let player = Address::<ObjectState>::new(arguments[0]);
    let bank = if ram::g_earthling_bank_users(revision).at(0).read(context) != 0
        && ram::g_earthling_bank_levels(revision).at(0).read(context)
            == player.level().read(context)
    {
        0
    } else {
        1
    };
    let earthlings = ram::g_earthling_banks(revision).at(bank);
    for slot in 0..20 {
        let earthling = earthlings.at(slot);
        if earthling.kind().read(context) == 18 && earthling.object().read(context) != 0 {
            let primary = player.entity_index().read(context) == 0;
            let object =
                ram::g_player_info(revision).at(i32::from(earthling.object().read(context)));
            if primary {
                object.attachment_x_or_latch().write(context, 0);
            } else {
                object.attachment_y_or_latch().write(context, 0);
            }
        }
    }
    Ok(None)
}

/// G/src/earthlings/terrain_movement.c:108 (`EarthlingIsUpdateBlocked`).
pub fn earthling_is_update_blocked(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    if arguments[0] == 0 {
        return Ok(Some(0));
    }
    let revision = context.revision();
    let earthling = Address::<EarthlingState>::new(arguments[0]);
    let visible = earthling.health_visibility().at(1);
    let allowed = (visible.read(context) & 2 != 0
        && ram::g_player_inventory_open(revision).at(0).read(context) == 0)
        || (visible.read(context) & 4 != 0
            && ram::g_player2_inventory_open(revision).read(context) == 0);
    Ok(Some(u32::from(!allowed)))
}

/// G/src/earthlings/targeting.c:126 (`EarthlingCanTargetPlayerState`).
/// State two bypasses only the first eligibility group, not the final tests.
pub fn earthling_can_target_player_state(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let state = Address::<ObjectState>::new(arguments[0]);
    let target = Address::<ObjectState>::new(arguments[1]);
    let allowed = (state.sprite_state().read(context) != 2
        && (target.animation_group_mask().read(context) & 0x0004_00b8 != 0
            || target.sprite_state().read(context) == 0
            || target.sprite_state().read(context) == 4
            || target.animation_group_mask().read(context) & 0x2040_0000 != 0
            || target.sprite_state().read(context) == 76
            || target.sprite_state().read(context) == 78
            || target.sprite_state().read(context) == 77))
        || target.sprite_state().read(context) == 3
        || target.sprite_state().read(context) == 7
        || target.animation_group_mask().read(context) & 0x0fb5_7f44 != 0;
    Ok(Some(u32::from(allowed)))
}

/// G/src/earthlings/steering.c:6 (`EntitySetVelocityTowardDelta`).
/// Negation narrows back to short: -32768 remains negative for the dead-zone tests.
pub fn entity_set_velocity_toward_delta(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let state = Address::<ObjectState>::new(arguments[0]);
    let mut delta_x = arguments[1] as i16;
    let mut delta_y = arguments[2] as i16;
    let mut direction = 0;
    state.velocity_y().write(context, 0);
    state.velocity_x().write(context, 0);
    if delta_y < 0 {
        state.velocity_y().write(context, -256);
        delta_y = delta_y.wrapping_neg();
    } else if delta_y > 0 {
        state.velocity_y().write(context, 256);
    }
    if delta_x < 0 {
        state.velocity_x().write(context, -256);
        delta_x = delta_x.wrapping_neg();
    } else if delta_x > 0 {
        state.velocity_x().write(context, 256);
    }
    if state.velocity_x().read(context) != 0 && state.velocity_y().read(context) != 0 {
        if delta_y > 5 && delta_x < 20 {
            state.velocity_x().write(context, 0);
        } else if delta_x > 20 && delta_y < 5 {
            state.velocity_y().write(context, 0);
        }
    }
    if state.velocity_y().read(context) < 0 {
        direction = 4;
    } else if state.velocity_y().read(context) > 0 {
        direction = 5;
    }
    if state.velocity_x().read(context) < 0 {
        direction = 6;
    } else if state.velocity_x().read(context) > 0 {
        direction = 7;
    }
    Ok(Some(direction))
}

/// G/src/earthlings/steering_helpers.c:9 (`EntitySteerWithVerticalDeadZone`).
/// A zero horizontal delta deliberately still moves right and returns seven.
pub fn entity_steer_with_vertical_dead_zone(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let state = Address::<ObjectState>::new(arguments[0]);
    let delta_x = arguments[1] as i16;
    let delta_y = arguments[2] as i16;
    state.velocity_y().write(
        context,
        if delta_y < -2 {
            -256
        } else if delta_y > 2 {
            256
        } else {
            0
        },
    );
    if delta_x < 0 {
        state.velocity_x().write(context, -256);
        Ok(Some(6))
    } else {
        state.velocity_x().write(context, 256);
        Ok(Some(7))
    }
}

/// G/src/earthlings/mailbox_earthling.c:10 (`EarthlingIsMailOrderMailbox`).
/// Any matching bank whose mode is not one qualifies, not just mode zero.
pub fn earthling_is_mail_order_mailbox(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let earthling = Address::<EarthlingState>::new(arguments[0]);
    let banks = ram::g_mailbox_level_states(context.revision());
    let allowed = earthling.kind().read(context) == 9
        && ((banks.at(0).level().read(context) == earthling.level().read(context)
            && banks.at(0).mode().read(context) != 1)
            || (banks.at(1).level().read(context) == earthling.level().read(context)
                && banks.at(1).mode().read(context) != 1));
    Ok(Some(u32::from(allowed)))
}

/// G/src/earthlings/nice_earthling_touch.c:374 (`NiceEarthlingCheck`).
pub fn nice_earthling_check(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let earthling = Address::<EarthlingState>::new(arguments[0]);
    let allowed = match earthling.kind().read(context) {
        17 => earthling.state().read(context) == 3,
        10 => earthling.state().read(context) == 2,
        11 => {
            earthling.state().read(context) <= 1
                && (ram::g_player_wiseman_interaction_states(revision)
                    .at(0)
                    .read(context)
                    == 3
                    || ram::g_player2_wiseman_interaction_state(revision).read(context) == 3)
        }
        _ => false,
    };
    Ok(Some(u32::from(allowed)))
}
