//! Elevator record and animation leaves; see docs/PORTING.md.

use crate::{Context, Result, Translation, ids};
use tje_mem::{Address, Bus, ElevatorRecord, ObjectState, Revision, ram, rom};

/// Available Rust entries owned by this subsystem.
pub static TRANSLATIONS: &[Translation] = &[
    Translation {
        id: ids::ELEVATOR_SET_STATE,
        function: elevator_set_state_hook,
    },
    Translation {
        id: ids::PLAYER_SET_POSITION_FROM_SAVED_MAP_CELL,
        function: player_set_position_from_saved_map_cell_hook,
    },
    Translation {
        id: ids::ELEVATOR_INVALIDATE_SECONDARY_RECORDS,
        function: elevator_invalidate_secondary_records_hook,
    },
    Translation {
        id: ids::PROCESS_ELEVATOR,
        function: process_elevator_hook,
    },
];

/// All four leaves access only RAM and ROM. Every branch returns directly;
/// none calls another function or waits for devices, time, or interrupts.
pub static ATOMIC_CAPABLE: &[crate::FunctionId] = &[
    ids::ELEVATOR_SET_STATE,
    ids::PLAYER_SET_POSITION_FROM_SAVED_MAP_CELL,
    ids::ELEVATOR_INVALIDATE_SECONDARY_RECORDS,
    ids::PROCESS_ELEVATOR,
];

/// G/src/elevator/elevator_scene.c:37 (`ElevatorSetState`).
/// State is stored even when absent or lacking display entities. Descriptor reads
/// remain interleaved with their stores, and the secondary half is two slots on.
pub fn elevator_set_state(
    bus: &mut (impl Bus + ?Sized),
    revision: Revision,
    elevator: Address<ElevatorRecord>,
    state: i8,
) {
    elevator.state().write(bus, state);
    if state == 11 {
        elevator.level().write(bus, -1);
        return;
    }
    if elevator.entity().read(bus) == 0 {
        return;
    }
    let entity_index = elevator.entity().read(bus);
    let entity = ram::g_player_info(revision).at(i32::from(entity_index));
    let animation = rom::elevator_primary_animations(revision).at(i32::from(state));
    let sequence = animation.sequence().read(bus);
    entity.sequence().write(bus, sequence);
    let frames = animation.frames().read(bus);
    entity.frames().write(bus, frames);
    let period = animation.period().read(bus);
    entity.frame_phase().write(bus, period);
    entity.frame_duration().write(bus, 1);
    entity.frame().write(bus, -1);
    entity.sprite_state().write(bus, state);
    let x = entity.x().read(bus);
    let y = entity.y().read(bus);
    let entity = entity.offset(2);
    let animation = rom::elevator_secondary_animations(revision).at(i32::from(state));
    let sequence = animation.sequence().read(bus);
    entity.sequence().write(bus, sequence);
    let frames = animation.frames().read(bus);
    entity.frames().write(bus, frames);
    let period = animation.period().read(bus);
    entity.frame_phase().write(bus, period);
    let y_offset = animation.y_offset().read(bus);
    entity
        .y()
        .write(bus, (i32::from(y_offset) + i32::from(y)) as i16);
    entity.x().write(bus, x);
    entity.frame_duration().write(bus, 1);
    entity.frame().write(bus, -1);
    entity.sprite_state().write(bus, state);
}

/// G/src/elevator/elevator_scene.c:75 (`PlayerSetPositionFromSavedMapCell`).
/// Both signed map bytes are loaded before either player coordinate is stored.
pub fn player_set_position_from_saved_map_cell(
    bus: &mut (impl Bus + ?Sized),
    revision: Revision,
    player: Address<ObjectState>,
) {
    let map_x = ram::g_arrival_elevator_map_x(revision).read(bus);
    let map_y = ram::g_arrival_elevator_map_y(revision).read(bus);
    player
        .x()
        .write(bus, ((i32::from(map_x) + 2) * 128 - 16) as i16);
    player
        .y()
        .write(bus, ((i32::from(map_y) + 2) * 64 - 5) as i16);
}

/// G/src/elevator/elevator_scene.c:87 (`ElevatorInvalidateSecondaryRecords`).
/// Preserve the two level stores followed by the two state stores.
pub fn elevator_invalidate_secondary_records(bus: &mut (impl Bus + ?Sized), revision: Revision) {
    let primary = ram::g_elevator_records(revision).at(0).at(1);
    let secondary = ram::g_elevator_records(revision).at(1).at(1);
    primary.level().write(bus, -1);
    secondary.level().write(bus, -1);
    primary.state().write(bus, 11);
    secondary.state().write(bus, 11);
}

/// G/src/elevator/elevator_query.c:7 (`ProcessElevator`).
/// Subtract in the handle byte before its arithmetic right shift; do not reject
/// negative/out-of-range handles, whose original accesses extend beyond the array.
pub fn process_elevator(bus: &(impl Bus + ?Sized), revision: Revision, elevator_number: i8) -> u32 {
    let elevator_number = elevator_number.wrapping_sub(1);
    let record_number = elevator_number >> 1;
    let record = ram::g_elevator_records(revision)
        .at(i32::from(record_number))
        .at(i32::from(elevator_number & 1));
    u32::from(record.state().read(bus) == 3)
}

/// G/src/elevator/elevator_scene.c:37 (`ElevatorSetState`) stack bridge.
pub fn elevator_set_state_hook(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    elevator_set_state(
        context,
        context.revision(),
        Address::new(arguments[0]),
        arguments[1] as i8,
    );
    Ok(None)
}

/// G/src/elevator/elevator_scene.c:75 (`PlayerSetPositionFromSavedMapCell`) stack bridge.
pub fn player_set_position_from_saved_map_cell_hook(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    player_set_position_from_saved_map_cell(
        context,
        context.revision(),
        Address::new(arguments[0]),
    );
    Ok(None)
}

/// G/src/elevator/elevator_scene.c:87 (`ElevatorInvalidateSecondaryRecords`) stack bridge.
pub fn elevator_invalidate_secondary_records_hook(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    elevator_invalidate_secondary_records(context, context.revision());
    Ok(None)
}

/// G/src/elevator/elevator_query.c:7 (`ProcessElevator`) stack bridge.
pub fn process_elevator_hook(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    Ok(Some(process_elevator(
        context,
        context.revision(),
        arguments[0] as i8,
    )))
}

#[cfg(test)]
mod tests;
