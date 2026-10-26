use super::*;
use crate::players::{PlayerInput, UiCommand, UiIntent};
use crate::resources::{ResourceAction, ResourceOutcome, ResourceRequest};
use tje_four_player::{input::InputFrame, state::Choice};

/// Version-one hero action words. Director action words are interpreted only by
/// the role dispatcher; disabled/non-hero seats never become hero input here.
pub struct HeroActions;
impl HeroActions {
    pub const INVENTORY: u16 = 1;
    pub const MAP: u16 = 2;
    pub const CONFIRM: u16 = 4;
    pub const CANCEL: u16 = 8;
    pub const DROP: u16 = 16;
    pub const PAUSE: u16 = 32;
    pub const DONATE: u16 = 64;
    pub const CYCLE_RECIPIENT: u16 = 128;
    pub const DECLINE: u16 = 256;
    pub const MASK: u16 = 511;
}
#[derive(Debug, Clone, Copy)]
pub struct InputDispatch {
    pub advanced: bool,
    pub players: [PlayerInput; 4],
    pub outcomes: [ResourceOutcome; 4],
    pub outcome_count: u8,
}

/// Decode once at phase zero, apply typed per-seat UI, and commit resource
/// commands as one rotating-seat batch. Automatic ground claims run in the
/// population/items phase, not as a second transaction at this same tick.
pub fn dispatch_inputs(
    coop: &mut CoopState,
    services: &mut PlayerServices,
    tick: u64,
    frame: InputFrame,
    world: &mut impl PlayerWorld,
) -> PlayerResult<InputDispatch> {
    frame
        .validate()
        .map_err(|_| PlayerError::Invalid("hero input frame"))?;
    if services.last_input_tick == Some(tick) {
        return Ok(InputDispatch {
            advanced: false,
            players: std::array::from_fn(|index| services.runtime[index].effective_input),
            outcomes: [ResourceOutcome::default(); 4],
            outcome_count: 0,
        });
    }
    if services.last_input_tick.is_some_and(|last| tick < last) {
        return Err(PlayerError::Invalid("hero input moved backwards"));
    }
    let participants = coop.config().participants;
    for seat in participants.iter() {
        if frame.actions[seat.index()] & !HeroActions::MASK != 0 {
            return Err(PlayerError::Invalid("unknown hero action"));
        }
    }
    let game = frame.game_bytes(coop.config().swap_ac);
    let mut players = [PlayerInput::default(); 4];
    for seat in participants.iter() {
        let index = seat.index();
        let previous = services.runtime[index].contacts;
        let current = game[index];
        let pressed = current & !previous.game_input;
        let action_edge = frame.actions[index] & !previous.actions;
        services.runtime[index].contacts.game_input = current;
        services.runtime[index].contacts.actions = frame.actions[index];
        if current & 0x40 == 0 && frame.actions[index] & HeroActions::CONFIRM == 0 {
            services.release_ui_action(coop, seat);
        }
        if pressed & 0x80 != 0 || action_edge & HeroActions::PAUSE != 0 {
            services.ui(
                coop,
                seat,
                UiIntent::Pause(!coop.players[index].ui.pause_requested),
            )?;
        }
        let confirm = pressed & 0x40 != 0 || action_edge & HeroActions::CONFIRM != 0;
        let cancel =
            pressed & 0x30 != 0 || action_edge & (HeroActions::CANCEL | HeroActions::DECLINE) != 0;
        if coop.players[index].life == LifeState::Eliminated {
            if action_edge & HeroActions::CYCLE_RECIPIENT != 0 || pressed & 0x20 != 0 {
                cycle_donor(coop, services, seat);
            }
            if confirm || action_edge & HeroActions::DONATE != 0 {
                if services.runtime[index].contacts.donor_cursor.is_none() {
                    cycle_donor(coop, services, seat);
                }
                if let Some(donor) = services.runtime[index].contacts.donor_cursor
                    && coop.players[donor.index()].life == LifeState::Alive
                    && coop.players[donor.index()].lives != 0
                    && services.runtime[donor.index()].contacts.offer.is_none()
                    && services.runtime[donor.index()]
                        .contacts
                        .donation_request
                        .is_none()
                {
                    services.runtime[donor.index()].contacts.donation_request = Some(seat);
                    coop.players[donor.index()].ui.life_recipient = Some(seat);
                }
            }
            continue;
        }
        if coop.players[index].life != LifeState::Alive
            || matches!(
                services.runtime[index].mode,
                PlayerMode::Dying
                    | PlayerMode::Falling
                    | PlayerMode::AwaitingFloor
                    | PlayerMode::MoleCaptured
            )
        {
            continue;
        }
        if let Some(recipient) = previous.donation_request {
            if confirm {
                match services.donate_life(coop, tick, seat, recipient, world) {
                    Ok(()) => services.runtime[index].contacts.donation_request = None,
                    Err(PlayerError::Rejected(_) | PlayerError::Capacity(_)) => {}
                    Err(error) => return Err(error),
                }
            } else if cancel {
                services.runtime[index].contacts.donation_request = None;
            }
            continue;
        }
        if let Some(offer) = previous.offer
            && !offer.active
        {
            if confirm || cancel {
                services.runtime[index].contacts.response =
                    Some((offer.source, confirm && !cancel));
                coop.players[index].ui.choice = Some(if confirm && !cancel {
                    Choice::Accept
                } else {
                    Choice::Decline
                });
            }
            continue;
        }
        if services.runtime[index].locomotion_locked || services.runtime[index].service.is_some() {
            continue;
        }
        let overlay = coop.players[index].ui.overlay;
        if overlay != Overlay::Closed {
            if pressed & 0x20 != 0
                || pressed & 0x10 != 0 && overlay != Overlay::Inventory
                || action_edge & (HeroActions::CANCEL | HeroActions::DECLINE) != 0
                || action_edge & HeroActions::INVENTORY != 0 && overlay == Overlay::Inventory
                || action_edge & HeroActions::MAP != 0 && overlay == Overlay::Map
            {
                services.ui(coop, seat, UiIntent::Close)?;
                continue;
            }
            let direction_edge = pressed & 15;
            if overlay == Overlay::MailOrder {
                if direction_edge & 3 != 0
                    && let Some(service) = services.runtime[index].ui_service
                {
                    let ids: [Option<u32>; 3] = std::array::from_fn(|row| {
                        services
                            .resources
                            .offers()
                            .iter()
                            .filter(|offer| offer.service == service)
                            .nth(row)
                            .map(|offer| offer.id)
                    });
                    let current_row = ids
                        .iter()
                        .position(|&id| id == services.runtime[index].ui_offer)
                        .unwrap_or(0);
                    let next = if direction_edge & 1 != 0 {
                        current_row.saturating_sub(1)
                    } else {
                        (current_row + 1).min(2)
                    };
                    if let Some(id) = ids[next] {
                        services.ui(coop, seat, UiIntent::SelectOffer(id))?;
                    }
                }
            } else if matches!(overlay, Overlay::Inventory | Overlay::Identify)
                && direction_edge != 0
            {
                let current_slot = i16::from(coop.players[index].ui.selected_slot);
                let delta = if direction_edge & 1 != 0 {
                    -2
                } else if direction_edge & 2 != 0 {
                    2
                } else if direction_edge & 4 != 0 {
                    -1
                } else {
                    1
                };
                services.ui(
                    coop,
                    seat,
                    UiIntent::Select((current_slot + delta).clamp(0, 15) as u8),
                )?;
            }
            if confirm && overlay != Overlay::Map {
                services.ui(coop, seat, UiIntent::Activate)?;
            }
            if (pressed & 0x10 != 0 || action_edge & HeroActions::DROP != 0)
                && overlay == Overlay::Inventory
            {
                services.ui(coop, seat, UiIntent::Drop)?;
            }
            continue;
        }
        if pressed & 0x10 != 0 || action_edge & HeroActions::INVENTORY != 0 {
            services.ui(coop, seat, UiIntent::OpenInventory)?;
            continue;
        }
        if pressed & 0x20 != 0 || action_edge & HeroActions::MAP != 0 {
            services.ui(coop, seat, UiIntent::OpenMap)?;
            continue;
        }
        if action_edge & HeroActions::CYCLE_RECIPIENT != 0 {
            cycle_recipient(coop, seat);
        }
        if action_edge & HeroActions::DONATE != 0 {
            if let Some(recipient) = coop.players[index].ui.life_recipient {
                services.ui(coop, seat, UiIntent::DonateLifeTo(recipient))?;
            }
            continue;
        }
        players[index] = PlayerInput {
            directions: crate::players::normalize_directions(current & 15),
            action: current & 0x40 != 0,
            jump: current & 0x40 != 0,
            sneak: current & 0x40 != 0 && services.runtime[index].equipped.is_none(),
            wake: pressed & 0x7f != 0,
        };
    }

    let initial = ResourceRequest {
        seat: SeatId::ALL[0],
        action: ResourceAction::Claim(ItemRef::Placed(0)),
    };
    let mut requests = [initial; 4];
    let mut count = 0;
    let mut queued_seats = [None; 4];
    for offset in 0..4 {
        let seat = SeatId::ALL[(services.resources.priority.index() + offset) % 4];
        if !participants.contains(seat) {
            continue;
        }
        let Some(command) = services.runtime[seat.index()].ui_intent else {
            continue;
        };
        match command {
            UiCommand::OpenPresent(slot) => match services.open_present(coop, seat, slot, world) {
                Ok(_) => {
                    services.runtime[seat.index()].ui_intent = None;
                    services.ui(coop, seat, UiIntent::Close)?;
                    players[seat.index()] = PlayerInput::default();
                }
                Err(PlayerError::Capacity(_)) => {}
                Err(PlayerError::Rejected(_)) => {
                    services.runtime[seat.index()].ui_intent = None;
                }
                Err(error) => return Err(error),
            },
            UiCommand::DonateLife(recipient) => {
                match services.donate_life(coop, tick, seat, recipient, world) {
                    Ok(()) | Err(PlayerError::Rejected(_)) => {
                        services.runtime[seat.index()].ui_intent = None
                    }
                    Err(PlayerError::Capacity(_)) => {}
                    Err(error) => return Err(error),
                }
            }
            UiCommand::Drop(slot) => {
                services.refresh_terrain(coop, seat, world)?;
                requests[count] = ResourceRequest {
                    seat,
                    action: ResourceAction::Drop { slot },
                };
                queued_seats[count] = Some(seat);
                count += 1;
            }
            UiCommand::Purchase { offer, service } => {
                if !active_service(services, seat, service, Service::MailOrder) {
                    services.runtime[seat.index()].ui_intent = None;
                    continue;
                }
                requests[count] = ResourceRequest {
                    seat,
                    action: ResourceAction::Purchase { offer, service },
                };
                queued_seats[count] = Some(seat);
                count += 1;
            }
            UiCommand::Identify { slot, service } => {
                if !active_service(services, seat, service, Service::Identify) {
                    services.runtime[seat.index()].ui_intent = None;
                    continue;
                }
                requests[count] = ResourceRequest {
                    seat,
                    action: ResourceAction::Identify { slot, service },
                };
                queued_seats[count] = Some(seat);
                count += 1;
            }
        }
    }
    let mut outcomes = [ResourceOutcome::default(); 4];
    let mut committed_count = 0;
    if count != 0 && services.audio.reserve(count).is_ok() {
        services
            .resources
            .transact(coop, tick, &requests[..count], &mut outcomes, |rng| {
                world.choose_present(rng)
            })?;
        committed_count = count;
        for outcome in outcomes.iter().take(count) {
            let seat = queued_seats[usize::from(outcome.request)].expect("queued resource seat");
            services.runtime[seat.index()].ui_intent = None;
            let cue = if outcome.result.is_ok() {
                AudioCue::Psg {
                    command: 7,
                    priority: 50,
                }
            } else {
                AudioCue::Psg {
                    command: 26,
                    priority: 50,
                }
            };
            services.audio.enqueue(seat, cue)?;
        }
    }
    services.last_input_tick = Some(tick);
    Ok(InputDispatch {
        advanced: true,
        players,
        outcomes,
        outcome_count: committed_count as u8,
    })
}
fn active_service(
    services: &PlayerServices,
    seat: SeatId,
    actor: ActorId,
    service: Service,
) -> bool {
    services.runtime[seat.index()]
        .contacts
        .offer
        .is_some_and(|offer| offer.actor == actor && offer.service == service && offer.active)
}
fn cycle_recipient(coop: &mut CoopState, donor: SeatId) {
    let previous = coop.players[donor.index()]
        .ui
        .life_recipient
        .map_or(donor.index(), SeatId::index);
    let next = (1..=4)
        .map(|offset| SeatId::ALL[(previous + offset) % 4])
        .find(|&seat| seat != donor && coop.players[seat.index()].life == LifeState::Eliminated);
    coop.players[donor.index()].ui.life_recipient = next;
}
fn cycle_donor(coop: &CoopState, services: &mut PlayerServices, recipient: SeatId) {
    let previous = services.runtime[recipient.index()]
        .contacts
        .donor_cursor
        .map_or(recipient.index(), SeatId::index);
    services.runtime[recipient.index()].contacts.donor_cursor = (1..=4)
        .map(|offset| SeatId::ALL[(previous + offset) % 4])
        .find(|&seat| {
            seat != recipient
                && coop.players[seat.index()].life == LifeState::Alive
                && coop.players[seat.index()].lives != 0
        });
}
