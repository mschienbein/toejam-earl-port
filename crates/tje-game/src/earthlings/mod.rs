//! Earthlings translation ownership; see docs/PORTING.md.

use crate::{Context, Result, ids};
use tje_mem::{Address, EarthlingState, ObjectState, ram};

mod animation;
mod banks;
mod behaviors;
mod decisions;
mod dispatch;
mod effects;
mod friendly;
mod generic;
mod interactions;
mod movement;
mod santa;
mod state;

/// SUBQ.B followed by a signed branch consumes N xor V, not the wrapped sign.
/// G/src/earthlings/spawning_and_banks.c:786 and animation_behaviours.c:202.
fn predecrement_byte(value: i8) -> (i8, i16) {
    let difference = i16::from(value) - 1;
    (difference as i8, difference)
}

/// G/src/earthlings/animation_behaviours.c:39,178, generic_enemy.c:41,88,
/// terrain_animation.c:109: preserve SUBQ.W's signed overflow for the branch.
fn predecrement_word(value: i16) -> (i16, i32) {
    let difference = i32::from(value) - 1;
    (difference as i16, difference)
}

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
    crate::Translation {
        id: ids::EARTHLING_IS_SPECIAL_RECORD_NEARBY,
        function: decisions::earthling_is_special_record_nearby,
    },
    crate::Translation {
        id: ids::EARTHLING_SELECT_TURN_STATE,
        function: decisions::earthling_select_turn_state,
    },
    crate::Translation {
        id: ids::ENTITY_TRIGGER_TARGET_STATUS_EVENT_ONCE,
        function: decisions::entity_trigger_target_status_event_once,
    },
    crate::Translation {
        id: ids::CHECK_EARTHLING_OVERLAP,
        function: banks::check_earthling_overlap,
    },
    crate::Translation {
        id: ids::EARTHLING_CONFIGURE_MAILBOX_LEVEL_BANK,
        function: banks::earthling_configure_mailbox_level_bank,
    },
    crate::Translation {
        id: ids::EARTHLING_MOVE_WITHIN_MAP_BOUNDS,
        function: movement::earthling_move_within_map_bounds,
    },
    crate::Translation {
        id: ids::EARTHLING_PROBE_DIRECTIONAL_TERRAIN,
        function: movement::earthling_probe_directional_terrain,
    },
    crate::Translation {
        id: ids::EARTHLING_SELECT_TERRAIN_ANIMATION_STATE,
        function: animation::earthling_select_terrain_animation_state,
    },
    crate::Translation {
        id: ids::EARTHLING_SPAWN_MOLE_EFFECT,
        function: effects::earthling_spawn_mole_effect,
    },
    crate::Translation {
        id: ids::ENTITY_STEER_WITH_HORIZONTAL_DRIFT,
        function: decisions::entity_steer_with_horizontal_drift,
    },
    crate::Translation {
        id: ids::ENTITY_UPDATE_TERRAIN_DRAW_FLAG,
        function: movement::entity_update_terrain_draw_flag,
    },
    crate::Translation {
        id: ids::CHECK_A_BUTTON,
        function: interactions::check_a_button,
    },
    crate::Translation {
        id: ids::EARTHLING_APPLY_NEARBY_SLEEP,
        function: state::earthling_apply_nearby_sleep,
    },
    crate::Translation {
        id: ids::EARTHLING_MOVE_ON_ALLOWED_TERRAIN,
        function: movement::earthling_move_on_allowed_terrain,
    },
    crate::Translation {
        id: ids::EARTHLING_RELEASE_LEVEL_BANK,
        function: state::earthling_release_level_bank,
    },
    crate::Translation {
        id: ids::EARTHLING_REMOVE_FROM_LEVEL,
        function: state::earthling_remove_from_level,
    },
    crate::Translation {
        id: ids::EARTHLING_RESTORE_SAVED_POSITION,
        function: state::earthling_restore_saved_position,
    },
    crate::Translation {
        id: ids::PLAYER_CREATE_LIFE_REQUEST_DISPLAY,
        function: interactions::player_create_life_request_display,
    },
    crate::Translation {
        id: ids::RELEASE_FINISHED_MOLE_EFFECTS_AND_STOLEN_PRESENTS,
        function: effects::release_finished_mole_effects_and_stolen_presents,
    },
    crate::Translation {
        id: ids::SET_EARTHLING_STATE,
        function: state::set_earthling_state,
    },
    crate::Translation {
        id: ids::DAMAGE_EARTHLING,
        function: state::damage_earthling,
    },
    crate::Translation {
        id: ids::DROP_PRESENTS_ABILITY,
        function: effects::drop_presents_ability,
    },
    crate::Translation {
        id: ids::EARTHLING_ADVANCE_HORIZONTAL_TURN_ANIMATION,
        function: behaviors::earthling_advance_horizontal_turn_animation,
    },
    crate::Translation {
        id: ids::EARTHLING_RELOCATE_RECORD,
        function: state::earthling_relocate_record,
    },
    crate::Translation {
        id: ids::EARTHLING_RESET_STATE_TWO_BY_DRAW_MASK,
        function: state::earthling_reset_state_two_by_draw_mask,
    },
    crate::Translation {
        id: ids::EARTHLING_THINK_HORIZONTAL_REVERSAL,
        function: behaviors::earthling_think_horizontal_reversal,
    },
    crate::Translation {
        id: ids::EARTHLING_THINK_HORIZONTAL_WANDER,
        function: behaviors::earthling_think_horizontal_wander,
    },
    crate::Translation {
        id: ids::EARTHLING_THINK_TARGETED_RESET_CYCLE,
        function: behaviors::earthling_think_targeted_reset_cycle,
    },
    crate::Translation {
        id: ids::EARTHLING_UPDATE_TERRAIN_ANIMATION_STATE,
        function: animation::earthling_update_terrain_animation_state,
    },
    crate::Translation {
        id: ids::EARTHLING_WAKE_LEVEL,
        function: state::earthling_wake_level,
    },
    crate::Translation {
        id: ids::DO_VIKING_ABILITY,
        function: state::do_viking_ability,
    },
    crate::Translation {
        id: ids::EARTHLING_DAMAGE_ON_HAZARD_TERRAIN,
        function: state::earthling_damage_on_hazard_terrain,
    },
    crate::Translation {
        id: ids::EARTHLING_PROCESS_SPECIAL_THINK_STATES,
        function: generic::earthling_process_special_think_states,
    },
    crate::Translation {
        id: ids::PLAYER_CREATE_INTERACTION_CHOICE_DISPLAY,
        function: interactions::player_create_interaction_choice_display,
    },
    crate::Translation {
        id: ids::SANTA_LOGIC,
        function: santa::santa_logic,
    },
    crate::Translation {
        id: ids::SETUP_INITIAL_EARTHLINGS,
        function: state::setup_initial_earthlings,
    },
    crate::Translation {
        id: ids::EARTHLING_ALLOCATE_ENTITIES,
        function: state::earthling_allocate_entities,
    },
    crate::Translation {
        id: ids::EARTHLING_THINK_TIMED_MOVEMENT,
        function: animation::earthling_think_timed_movement,
    },
    crate::Translation {
        id: ids::LOAD_IN_EARTHLINGS,
        function: dispatch::load_in_earthlings,
    },
];

/// Bodies awaiting their per-revision proof; never included in production dispatch.
/// Composed TimeAware proof fails on ProcessEarthlings' ordered device-write trace.
pub static PENDING_TRANSLATIONS: &[crate::Translation] = &[
    crate::Translation {
        id: ids::AI_GENERIC_ENEMY_THINK,
        function: generic::ai_generic_enemy_think,
    },
    crate::Translation {
        id: ids::AI_OPERA_SINGER_TOUCH,
        function: friendly::ai_opera_singer_touch,
    },
    crate::Translation {
        id: ids::AI_WIZARD_TOUCH,
        function: friendly::ai_wizard_touch,
    },
    crate::Translation {
        id: ids::EARTHLING_DISPATCH_ABILITIES,
        function: dispatch::earthling_dispatch_abilities,
    },
    crate::Translation {
        id: ids::PLAYER_PROCESS_LIFE_REQUEST,
        function: interactions::player_process_life_request,
    },
    crate::Translation {
        id: ids::AI_MAILBOX_TOUCH,
        function: friendly::ai_mailbox_touch,
    },
    crate::Translation {
        id: ids::AI_WISEMAN_TOUCH,
        function: friendly::ai_wiseman_touch,
    },
    crate::Translation {
        id: ids::EARTHLING_DISPATCH_PLAYER_CONTACTS,
        function: dispatch::earthling_dispatch_player_contacts,
    },
    crate::Translation {
        id: ids::PROCESS_EARTHLINGS,
        function: dispatch::process_earthlings,
    },
];

/// Unregistered body identities: the composed proof stops at REV 02 frame 2437
/// with a ProcessEarthlings device-write trace mismatch. No both-revision proof
/// is approved for these entries; this does not attribute the mismatch to each one.
pub static PENDING: &[crate::FunctionId] = &[
    ids::AI_GENERIC_ENEMY_THINK,
    ids::AI_OPERA_SINGER_TOUCH,
    ids::AI_WIZARD_TOUCH,
    ids::EARTHLING_DISPATCH_ABILITIES,
    ids::PLAYER_PROCESS_LIFE_REQUEST,
    ids::AI_MAILBOX_TOUCH,
    ids::AI_WISEMAN_TOUCH,
    ids::EARTHLING_DISPATCH_PLAYER_CONTACTS,
    ids::PROCESS_EARTHLINGS,
];

/// Original execution review, including untranslated entries. No ROM bytes are
/// embedded: spawning and AI read their definitions through emulated ROM views.
pub static ATOMIC_CAPABLE: &[crate::FunctionId] = &[
    // G/src/earthlings/bank_expiry.c:9: bounded refcount/object RAM cleanup.
    ids::EARTHLING_RELEASE_LEVEL_BANK,
    // G/src/earthlings/spawning_and_banks.c:53–576: placement, RNG, bank/object
    // bookkeeping and animation pointers. Retry loops need no clock/IRQ progress.
    ids::CHECK_EARTHLING_OVERLAP,
    ids::EARTHLING_IS_SPECIAL_RECORD_NEARBY,
    ids::FIND_SPAWN_LOCATION_FOR_EARTHLING,
    ids::MAP_GET_SHIP_PIECE_WORLD_POSITION,
    ids::EARTHLING_RETAIN_LEVEL_BANK,
    ids::SETUP_INITIAL_EARTHLINGS,
    ids::SET_EARTHLING_STATE,
    ids::EARTHLING_RELOCATE_RECORD,
    ids::EARTHLING_RESTORE_SAVED_POSITION,
    ids::EARTHLING_ALLOCATE_ENTITIES,
    ids::LOAD_IN_EARTHLINGS,
    ids::OPEN_EARTHLING_PRESENT,
    // G/src/earthlings/spawning_and_banks.c:644–868: bounded record passes,
    // RAM effect allocation/free and animation setup; no device observations.
    ids::EARTHLING_APPLY_NEARBY_SLEEP,
    ids::EARTHLING_WAKE_LEVEL,
    ids::EARTHLING_RESET_STATE_TWO_BY_DRAW_MASK,
    ids::EARTHLING_REMOVE_FROM_LEVEL,
    ids::DAMAGE_EARTHLING,
    ids::EARTHLING_DAMAGE_ON_HAZARD_TERRAIN,
    ids::DO_VIKING_ABILITY,
    ids::EARTHLING_CLEAR_MOLE_PLAYER_CONTACT_FLAGS,
    // G/src/earthlings/terrain_movement.c:20–211: fixed-point RAM movement,
    // terrain ROM/RAM lookup, collision scans and decoy-hit bookkeeping.
    ids::EARTHLING_PROBE_DIRECTIONAL_TERRAIN,
    ids::EARTHLING_MOVE_ON_ALLOWED_TERRAIN,
    ids::EARTHLING_IS_UPDATE_BLOCKED,
    ids::ENTITY_UPDATE_TERRAIN_DRAW_FLAG,
    ids::EARTHLING_MOVE_WITHIN_MAP_BOUNDS,
    // G/src/earthlings/movement_decision.c:12: collision/decoy and cached RNG.
    ids::EARTHLING_SELECT_TURN_STATE,
    // G/src/earthlings/generic_enemy.c:20: RAM special-state/attack handlers
    // and PlaySoundEffect's RAM request; unlike the main think, no speech wait.
    ids::EARTHLING_PROCESS_SPECIAL_THINK_STATES,
    // G/src/earthlings/terrain_animation.c:16–199: animation/timer/steering
    // and RAM sound requests, no polling on hardware or interrupt-owned bytes.
    ids::EARTHLING_SELECT_TERRAIN_ANIMATION_STATE,
    ids::EARTHLING_UPDATE_TERRAIN_ANIMATION_STATE,
    ids::EARTHLING_THINK_TIMED_MOVEMENT,
    // G/src/earthlings/animation_behaviours.c:16–207: timers, cached RNG,
    // steering, animation setup and object release only.
    ids::EARTHLING_THINK_HORIZONTAL_WANDER,
    ids::EARTHLING_ADVANCE_HORIZONTAL_TURN_ANIMATION,
    ids::EARTHLING_THINK_HORIZONTAL_REVERSAL,
    ids::EARTHLING_THINK_TARGETED_RESET_CYCLE,
    // G/src/earthlings/targeting.c:18–137, steering.c:6–64 and
    // steering_helpers.c:9–36: RAM candidates, velocities and sound-request latch.
    ids::EARTHLING_STEER_TOWARD_NEAREST_TARGET,
    ids::EARTHLING_CAN_TARGET_PLAYER_STATE,
    ids::ENTITY_SET_VELOCITY_TOWARD_DELTA,
    ids::ENTITY_STEER_WITH_HORIZONTAL_DRIFT,
    ids::ENTITY_STEER_WITH_VERTICAL_DEAD_ZONE,
    ids::ENTITY_TRIGGER_TARGET_STATUS_EVENT_ONCE,
    // G/src/earthlings/mailbox_earthling.c:10–43: bank RAM and direct RNG.
    ids::EARTHLING_IS_MAIL_ORDER_MAILBOX,
    ids::EARTHLING_CONFIGURE_MAILBOX_LEVEL_BANK,
    // G/src/earthlings/nice_earthling_touch.c:36–105,374–422: choice/input,
    // object allocation/free, viewport residency and animation RAM only.
    ids::PLAYER_CREATE_INTERACTION_CHOICE_DISPLAY,
    ids::CHECK_A_BUTTON,
    ids::NICE_EARTHLING_CHECK,
    ids::PLAYER_CREATE_LIFE_REQUEST_DISPLAY,
    // G/src/earthlings/mole_santa.c:33–239: RAM objects, RNG, visibility,
    // relocation and present placement. Retry loops have no concurrent observer.
    ids::EARTHLING_SPAWN_MOLE_EFFECT,
    ids::RELEASE_FINISHED_MOLE_EFFECTS_AND_STOLEN_PRESENTS,
    ids::SANTA_LOGIC,
    ids::DROP_PRESENTS_ABILITY,
];

/// Original paths requiring real interrupt progress, never atomic exceptions.
/// No Earthlings entry has a Functional-only closed device-transaction stream.
pub static EXECUTION_CONTRACTS: &[(crate::FunctionId, crate::execution::ExecutionClass)] = &[
    // G/src/earthlings/generic_enemy.c:115–116 -> GenerateSpeech; its
    // speech_and_dialogue.c:135–138 waits for the ISR-owned sprite upload byte.
    (
        ids::AI_GENERIC_ENEMY_THINK,
        crate::execution::ExecutionClass::TimeAware,
    ),
    // G/src/earthlings/spawning_and_banks.c:579–640 invokes ROM callbacks:
    // GenericEnemyThink can speak; touch includes ship screens and inventory waits.
    (
        ids::EARTHLING_DISPATCH_ABILITIES,
        crate::execution::ExecutionClass::TimeAware,
    ),
    (
        ids::EARTHLING_DISPATCH_PLAYER_CONTACTS,
        crate::execution::ExecutionClass::TimeAware,
    ),
    // G/src/earthlings/spawning_and_banks.c:871–885 invokes both dispatchers.
    (
        ids::PROCESS_EARTHLINGS,
        crate::execution::ExecutionClass::TimeAware,
    ),
    // G/src/earthlings/nice_earthling_touch.c:108–371: GenerateSpeech on
    // approach, plus mail-order/identification DrawInventory -> VBlank waits.
    (
        ids::AI_MAILBOX_TOUCH,
        crate::execution::ExecutionClass::TimeAware,
    ),
    (
        ids::AI_WIZARD_TOUCH,
        crate::execution::ExecutionClass::TimeAware,
    ),
    (
        ids::AI_WISEMAN_TOUCH,
        crate::execution::ExecutionClass::TimeAware,
    ),
    (
        ids::AI_OPERA_SINGER_TOUCH,
        crate::execution::ExecutionClass::TimeAware,
    ),
    // G/src/earthlings/nice_earthling_touch.c:425–486: explicit upload wait
    // at 461; speech and relocation also wait for real VBlank IRQ completion.
    (
        ids::PLAYER_PROCESS_LIFE_REQUEST,
        crate::execution::ExecutionClass::TimeAware,
    ),
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
#[cfg(test)]
mod decrement_tests {
    use super::{predecrement_byte, predecrement_word};

    #[test]
    fn every_byte_subq_signed_condition_including_overflow() {
        for value in i8::MIN..=i8::MAX {
            let (stored, difference) = predecrement_byte(value);
            let negative = stored < 0;
            let overflow = value == i8::MIN;
            let zero = stored == 0;
            assert_eq!(stored, value.wrapping_sub(1));
            assert_eq!(difference < 0, negative ^ overflow);
            assert_eq!(difference <= 0, zero || (negative ^ overflow));
            assert_eq!(difference > 0, !zero && !(negative ^ overflow));
        }
    }

    #[test]
    fn every_word_subq_signed_condition_including_overflow() {
        for value in i16::MIN..=i16::MAX {
            let (stored, difference) = predecrement_word(value);
            let negative = stored < 0;
            let overflow = value == i16::MIN;
            let zero = stored == 0;
            assert_eq!(stored, value.wrapping_sub(1));
            assert_eq!(difference < 0, negative ^ overflow);
            assert_eq!(difference <= 0, zero || (negative ^ overflow));
            assert_eq!(difference > 0, !zero && !(negative ^ overflow));
            assert_eq!(difference == 0, zero);
        }
    }
}
