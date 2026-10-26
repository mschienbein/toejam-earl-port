//! Automatic population-phase pickups. Candidate selection is read-only;
//! the existing resource transaction is the sole claim/arbitration authority.
use super::*;
use crate::resources::{
    ItemKind, ItemRef, NearbyItem, ResourceAction, ResourceFailure, ResourceOutcome,
    ResourceReceipt, ResourceRequest,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemClaim {
    pub seat: SeatId,
    pub item: NearbyItem,
    pub result: Result<ResourceReceipt, ResourceFailure>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemClaims {
    pub advanced: bool,
    /// Actual transaction order, including failed contenders. Successful
    /// receipts retain the pre-retirement source identity for world ledgers.
    pub outcomes: [Option<ItemClaim>; SEATS],
}

impl PlayerServices {
    /// Select at most one stable nearby item per hero and transact one bounded
    /// rotating-priority batch, without a controller-confirm edge. Audio space
    /// is reserved before the transaction, so pressure cannot silently consume
    /// an item. A capacity error leaves this phase available for retry.
    pub fn collect_nearby_items(
        &mut self,
        coop: &mut CoopState,
        tick: u64,
        world: &mut impl PlayerWorld,
    ) -> PlayerResult<ItemClaims> {
        let mut result = ItemClaims {
            advanced: false,
            outcomes: [None; SEATS],
        };
        if self.last_items_tick == Some(tick) {
            return Ok(result);
        }
        if self.last_items_tick.is_some_and(|last| tick < last) {
            return Err(PlayerError::Invalid("item collection tick moved backwards"));
        }
        let initial = ResourceRequest {
            seat: SeatId::ALL[0],
            action: ResourceAction::Claim(ItemRef::Placed(0)),
        };
        let mut requests = [initial; SEATS];
        let mut candidates = [None; SEATS];
        let mut count = 0;
        let mut audio_count = 0;
        for seat in coop.config().participants.iter() {
            let runtime = &self.runtime[seat.index()];
            if runtime.locomotion_locked
                || matches!(
                    runtime.mode,
                    PlayerMode::Dying
                        | PlayerMode::Falling
                        | PlayerMode::AwaitingFloor
                        | PlayerMode::Whirlwind
                        | PlayerMode::MoleCaptured
                )
            {
                continue;
            }
            let Some(item) = self.resources.nearby_claim(coop, seat) else {
                continue;
            };
            requests[count] = ResourceRequest {
                seat,
                action: ResourceAction::Claim(item.reference),
            };
            let cue = pickup_audio(item.kind);
            candidates[count] = Some((seat, item, cue));
            audio_count += usize::from(cue.is_some());
            count += 1;
        }
        self.audio.reserve(audio_count)?;
        if count != 0 {
            let mut outcomes = [ResourceOutcome::default(); SEATS];
            let written =
                self.resources
                    .transact(coop, tick, &requests[..count], &mut outcomes, |rng| {
                        world.choose_present(rng)
                    })?;
            for (index, outcome) in outcomes.into_iter().take(written).enumerate() {
                let (seat, item, cue) =
                    candidates[usize::from(outcome.request)].expect("selected item request");
                if outcome.result.is_ok() {
                    if let Some(cue) = cue {
                        self.audio.enqueue(seat, cue)?;
                    }
                    if matches!(item.kind, ItemKind::Present(_)) {
                        world.apply_actor_effect(ActorEffect::ClearMoleContacts {
                            seat,
                            level: item.level,
                        })?;
                    }
                }
                result.outcomes[index] = Some(ItemClaim {
                    seat,
                    item,
                    result: outcome.result,
                });
            }
        }
        self.last_items_tick = Some(tick);
        result.advanced = true;
        Ok(result)
    }
}

fn pickup_audio(kind: ItemKind) -> Option<AudioCue> {
    Some(match kind {
        ItemKind::Present(_) => AudioCue::Psg {
            command: 1,
            priority: 50,
        },
        ItemKind::Buck(_) => AudioCue::Pcm {
            sound: PcmSound::Money,
            priority: 50,
            mode: 0,
        },
        ItemKind::Food(delta) if delta > 0 => AudioCue::Pcm {
            sound: PcmSound::ChowDown,
            priority: 50,
            mode: 0,
        },
        ItemKind::Food(_) => AudioCue::Psg {
            command: 18,
            priority: 40,
        },
        ItemKind::ShipPart(_) => return None,
    })
}
