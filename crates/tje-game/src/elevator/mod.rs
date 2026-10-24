//! Elevator record and animation leaves; see docs/PORTING.md.

use crate::{Context, Result, Translation, execution::ExecutionClass, ids};
use tje_mem::{Address, Bus, ElevatorRecord, ObjectState, Revision, ram, rom};

mod geometry;
mod management;
mod ride;
mod scenes;
mod transitions;

pub use geometry::{
    elevator_blocks_entity_movement, elevator_blocks_player_movement,
    elevator_check_player_proximity_and_input, elevator_contains_position,
    elevator_step_departure_height_cycles,
};
pub use management::{
    elevator_create_entity_pair, elevator_update_entity_visibility, setup_elevator,
    setup_initial_elevator,
};
pub use ride::{
    elevator_advance_riders_to_next_level, elevator_alternate_ride_entities,
    elevator_begin_ride_scene, elevator_end_ride_scene, elevator_load_ride_background_tiles,
    elevator_prepare_ride_dialogue, elevator_ride_v_blank, elevator_update_ride_dialogue,
};
pub use scenes::{load_final_level_map, map_clear_active_level_objects, map_enter_final_level};
pub use transitions::{process_elevator_step1, process_elevator_step2, process_elevators};

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
    Translation {
        id: ids::ELEVATOR_BLOCKS_PLAYER_MOVEMENT,
        function: elevator_blocks_player_movement,
    },
    Translation {
        id: ids::ELEVATOR_BLOCKS_ENTITY_MOVEMENT,
        function: elevator_blocks_entity_movement,
    },
    Translation {
        id: ids::ELEVATOR_CONTAINS_POSITION,
        function: elevator_contains_position,
    },
    Translation {
        id: ids::ELEVATOR_CHECK_PLAYER_PROXIMITY_AND_INPUT,
        function: elevator_check_player_proximity_and_input,
    },
    Translation {
        id: ids::ELEVATOR_STEP_DEPARTURE_HEIGHT_CYCLES,
        function: elevator_step_departure_height_cycles,
    },
    Translation {
        id: ids::ELEVATOR_CREATE_ENTITY_PAIR,
        function: elevator_create_entity_pair,
    },
    Translation {
        id: ids::ELEVATOR_UPDATE_ENTITY_VISIBILITY,
        function: elevator_update_entity_visibility,
    },
    Translation {
        id: ids::SETUP_ELEVATOR,
        function: setup_elevator,
    },
    Translation {
        id: ids::SETUP_INITIAL_ELEVATOR,
        function: setup_initial_elevator,
    },
    Translation {
        id: ids::PROCESS_ELEVATOR_STEP2,
        function: process_elevator_step2,
    },
    Translation {
        id: ids::ELEVATOR_UPDATE_RIDE_DIALOGUE,
        function: elevator_update_ride_dialogue,
    },
    Translation {
        id: ids::MAP_CLEAR_ACTIVE_LEVEL_OBJECTS,
        function: map_clear_active_level_objects,
    },
    Translation {
        id: ids::ELEVATOR_LOAD_RIDE_BACKGROUND_TILES,
        function: elevator_load_ride_background_tiles,
    },
    Translation {
        id: ids::ELEVATOR_ALTERNATE_RIDE_ENTITIES,
        function: elevator_alternate_ride_entities,
    },
    Translation {
        id: ids::LOAD_FINAL_LEVEL_MAP,
        function: load_final_level_map,
    },
    Translation {
        id: ids::ELEVATOR_RIDE_V_BLANK,
        function: elevator_ride_v_blank,
    },
];

/// Complete Rust bodies awaiting both-revision execution-class proof; never
/// registered by the production dispatcher until a passing proof activates them.
pub static PENDING_TRANSLATIONS: &[Translation] = &[
    // Frame-1300 caller-envelope proof: original 30 completed ISRs, Rust 20.
    // Needs the shared clock-charged-region contract, not a counter exemption.
    Translation {
        id: ids::MAP_ENTER_FINAL_LEVEL,
        function: map_enter_final_level,
    },
    // Real ISR entry interleaves differently with GenerateSpeech RAM stores.
    // Exact LINK/MOVEM/local/call sites below do not resolve clock ordering.
    Translation {
        id: ids::ELEVATOR_PREPARE_RIDE_DIALOGUE,
        function: elevator_prepare_ride_dialogue,
    },
    // Settled proof disagrees in ISR-visible RAM; the stricter caller envelope
    // also exposes the host's outstanding DMA pending-write queue overflow.
    Translation {
        id: ids::ELEVATOR_BEGIN_RIDE_SCENE,
        function: elevator_begin_ride_scene,
    },
    // Original deceleration needs more than the fixed timed-call bound permits.
    // Never shorten the 2048→0 ride or raise the bound inside this translation.
    Translation {
        id: ids::ELEVATOR_END_RIDE_SCENE,
        function: elevator_end_ride_scene,
    },
    // Natural and short-state fixtures pass, but the departure-completion edge
    // invokes the still-blocked long ride/level-advance path below.
    Translation {
        id: ids::PROCESS_ELEVATOR_STEP1,
        function: process_elevator_step1,
    },
    // The Step1 departure-completion path has the same unresolved prerequisite.
    Translation {
        id: ids::PROCESS_ELEVATORS,
        function: process_elevators,
    },
    // Frame-1300 no-rider residual-D3 fixture: original 37 ISRs, Rust 27.
    // The shared-ride branch also depends on Begin/EndRideScene above.
    Translation {
        id: ids::ELEVATOR_ADVANCE_RIDERS_TO_NEXT_LEVEL,
        function: elevator_advance_riders_to_next_level,
    },
];

/// Reviewed ORIGINAL Atomic entries, translated or not.
/// These paths only perform RAM/ROM logic, entity/sprite allocation/release,
/// arithmetic runtime calls, and RAM sound requests; none waits for hardware.
pub static ATOMIC_CAPABLE: &[crate::FunctionId] = &[
    ids::ELEVATOR_SET_STATE,
    ids::PLAYER_SET_POSITION_FROM_SAVED_MAP_CELL,
    ids::ELEVATOR_INVALIDATE_SECONDARY_RECORDS,
    ids::PROCESS_ELEVATOR,
    // elevator_scene.c:97,149,161,204 — record/entity management; FreeEntity,
    // ResetEntityRuntimeState and ElevatorSetState have RAM-only return paths.
    ids::SETUP_ELEVATOR,
    ids::SETUP_INITIAL_ELEVATOR,
    ids::ELEVATOR_CREATE_ENTITY_PAIR,
    ids::ELEVATOR_UPDATE_ENTITY_VISIBILITY,
    // elevator_scene.c:240,285,311 — threshold queries; abs is no-wait.
    ids::ELEVATOR_BLOCKS_PLAYER_MOVEMENT,
    ids::ELEVATOR_BLOCKS_ENTITY_MOVEMENT,
    ids::ELEVATOR_CONTAINS_POSITION,
    // elevator_scene.c:358 — bank/entity release and transient cleanup only.
    ids::MAP_CLEAR_ACTIVE_LEVEL_OBJECTS,
    // elevator_scene.c:530 — counter tests and speech-entity release only.
    ids::ELEVATOR_UPDATE_RIDE_DIALOGUE,
    // elevator_transitions.c:22,52,192 — RAM state machines and PlaySound
    // request stores; queries and arithmetic runtime return without clocks.
    ids::ELEVATOR_CHECK_PLAYER_PROXIMITY_AND_INPUT,
    ids::ELEVATOR_STEP_DEPARTURE_HEIGHT_CYCLES,
    ids::PROCESS_ELEVATOR_STEP2,
];

/// Reviewed non-Atomic ORIGINAL contracts; sources are G/src/elevator/.
pub static EXECUTION_CONTRACTS: &[crate::execution::ExecutionContract] = &[
    // elevator_scene.c:327 — stable ROM tile/palette stream and RAM HUD state.
    crate::execution::ExecutionContract {
        function: ids::LOAD_FINAL_LEVEL_MAP,
        class: ExecutionClass::Functional,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // elevator_scene.c:437 — ROM decode into RAM, then stable sprite-tile DMA.
    crate::execution::ExecutionContract {
        function: ids::ELEVATOR_LOAD_RIDE_BACKGROUND_TILES,
        class: ExecutionClass::Functional,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // elevator_scene.c:463 — RAM link rebuild followed by sprite-table DMA;
    // asm/dma.s:15-75 contains writes only, no DMA-status read or polling.
    crate::execution::ExecutionContract {
        function: ids::ELEVATOR_ALTERNATE_RIDE_ENTITIES,
        class: ExecutionClass::Functional,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // elevator_scene.c:396 — UpdateScreenSplitMode can run VBlank animations.
    crate::execution::ExecutionContract {
        function: ids::MAP_ENTER_FINAL_LEVEL,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // elevator_scene.c:484 — GenerateSpeech waits for the IRQ-cleared upload
    // pending byte (speech/speech_and_dialogue.c:135-136).
    crate::execution::ExecutionContract {
        function: ids::ELEVATOR_PREPARE_RIDE_DIALOGUE,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // elevator_scene.c:562 — MusicUpdateSequencer can execute real Z80 mailbox
    // lock retries, DBRA delays and BUSACK polls; original fallback needs clocks.
    crate::execution::ExecutionContract {
        function: ids::ELEVATOR_RIDE_V_BLANK,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // elevator_scene.c:579 — waits for IRQ sprite upload/frame readiness.
    crate::execution::ExecutionContract {
        function: ids::ELEVATOR_BEGIN_RIDE_SCENE,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // elevator_scene.c:633 — waits on VBlank-integrated scroll velocity twice.
    crate::execution::ExecutionContract {
        function: ids::ELEVATOR_END_RIDE_SCENE,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // elevator_scene.c:652 — music-selection, ride, and split-screen waits.
    crate::execution::ExecutionContract {
        function: ids::ELEVATOR_ADVANCE_RIDERS_TO_NEXT_LEVEL,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // elevator_transitions.c:73,270 — departure completion invokes the ride.
    crate::execution::ExecutionContract {
        function: ids::PROCESS_ELEVATOR_STEP1,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    crate::execution::ExecutionContract {
        function: ids::PROCESS_ELEVATORS,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
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

// elevator_transitions.c:57,242 emit SUBQ.B immediately followed by BGT.
// The branch consumes subtraction overflow, not flags from a reloaded byte:
// decrementing -128 stores +127 but still enters the <= 0 path.
fn decrement_byte_and_test_le_zero(value: i8) -> (i8, bool) {
    (value.wrapping_sub(1), i16::from(value) - 1 <= 0)
}

#[cfg(test)]
mod tests;
