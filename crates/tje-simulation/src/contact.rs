//! Hero-side population contacts. The population owns its actor iteration;
//! contacts borrow only hero/resources and frozen geometry. World mutations are
//! detached and committed through the real bridge after that iteration ends.
use crate::players::{
    ActorEffect, AudioCue, HeroServiceKind, Landing, PcmSound, PlayerError, PlayerGeometry,
    PlayerMode, PlayerResult, PlayerServices, PlayerWorld,
};
use crate::population::{
    ContactEffect, ContactOutcome, ContactRequest, ContactSink, HeroView, Kind, PopulationId,
    Service, ServiceStage,
};
use crate::resources::{
    DroppedPresent, Effect, EffectKind, EffectOwner, EffectSource, ItemRef, MailOffer, MailStock,
    PresentCatalog, PresentKind, ServiceKind, TradeService,
};
use crate::{ActorId, EffectId, SeatId, SeatMask};
use bincode::{Decode, Encode};
use tje_four_player::{
    CoopState,
    state::{LifeState, Overlay, Position},
};
use tje_world::{Error, Result};

mod input;
pub use input::{HeroActions, InputDispatch, dispatch_inputs};
#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct OfferedService {
    pub source: PopulationId,
    pub actor: ActorId,
    pub service: Service,
    pub cost: u8,
    pub active: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct MoleHold {
    pub source: PopulationId,
    pub actor: ActorId,
    pub stolen: u8,
    pub visual: Option<EffectId>,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Encode, Decode)]
pub struct SeatContacts {
    pub offer: Option<OfferedService>,
    pub response: Option<(PopulationId, bool)>,
    pub mole: Option<MoleHold>,
    pub donation_request: Option<SeatId>,
    pub donor_cursor: Option<SeatId>,
    pub shark_alert_ticks: u16,
    pub game_input: u8,
    pub actions: u16,
    pub last_hula_tick: Option<u64>,
    pub last_theft_tick: Option<u64>,
}
impl SeatContacts {
    pub(crate) fn validate(&self, seat: SeatId, services: &PlayerServices) -> PlayerResult<()> {
        if self.actions & !HeroActions::MASK != 0 {
            return Err(PlayerError::Invalid("saved hero actions"));
        }
        if let Some(offer) = self.offer {
            offer
                .source
                .level
                .validate()
                .map_err(|_| PlayerError::Invalid("service source floor"))?;
            if !population_actor(offer.actor)
                || offer.source.local >= 64
                || offer.cost != service_cost(offer.service)
                || matches!(offer.service, Service::Lemonade | Service::HotTub)
            {
                return Err(PlayerError::Invalid("saved service offer"));
            }
        }
        if let Some((source, _)) = self.response
            && self.offer.is_none_or(|offer| offer.source != source)
        {
            return Err(PlayerError::Invalid("response names another service"));
        }
        if let Some(mole) = self.mole {
            mole.source
                .level
                .validate()
                .map_err(|_| PlayerError::Invalid("mole source floor"))?;
            if !population_actor(mole.actor)
                || mole.source.local >= 64
                || mole.stolen > 3
                || mole
                    .visual
                    .is_some_and(|id| !services.resources.known_effect(id))
            {
                return Err(PlayerError::Invalid("saved mole capture"));
            }
        }
        for other in [self.donation_request, self.donor_cursor]
            .into_iter()
            .flatten()
        {
            other
                .validate()
                .map_err(|_| PlayerError::Invalid("donation contact seat"))?;
            if other == seat {
                return Err(PlayerError::Invalid("self donation contact"));
            }
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy)]
pub struct ContactFrame {
    pub tick: u64,
    pub hero_ids: [Option<ActorId>; 4],
    pub elevator: SeatMask,
    pub suppressed: SeatMask,
}
#[derive(Debug, Clone, Copy)]
pub struct ShipBinding {
    pub source: PopulationId,
    pub part: u8,
}
#[derive(Debug, Clone, Copy)]
pub struct ContactBindings<'a> {
    pub identity: &'a [u8; 32],
    pub presents: &'a PresentCatalog,
    pub ship_parts: &'a [ShipBinding],
}
impl ContactBindings<'_> {
    pub fn validate(&self) -> PlayerResult<()> {
        if self.ship_parts.len() > 32 * 64 {
            return Err(PlayerError::Invalid("ship binding extent"));
        }
        for (index, binding) in self.ship_parts.iter().enumerate() {
            binding
                .source
                .level
                .validate()
                .map_err(|_| PlayerError::Invalid("ship binding floor"))?;
            if binding.part >= 10
                || binding.source.local >= 64
                || self.ship_parts[..index]
                    .iter()
                    .any(|other| other.source == binding.source)
            {
                return Err(PlayerError::Invalid("ship binding identity"));
            }
        }
        Ok(())
    }
}

const CONTACT_OPS: usize = 128;
#[derive(Debug, Clone, Copy)]
enum ContactOp {
    Created(EffectId),
    Retire(EffectId),
    World(ActorEffect),
}
#[must_use = "contact world effects must be applied after population.step before phase commit"]
pub struct ContactEffects {
    ops: [Option<ContactOp>; CONTACT_OPS],
    length: usize,
}
impl Default for ContactEffects {
    fn default() -> Self {
        Self {
            ops: [None; CONTACT_OPS],
            length: 0,
        }
    }
}
impl ContactEffects {
    fn reserve(&self, count: usize) -> PlayerResult<()> {
        if self.length + count > CONTACT_OPS {
            Err(PlayerError::Capacity("contact world operations"))
        } else {
            Ok(())
        }
    }
    fn push(&mut self, op: ContactOp) {
        self.ops[self.length] = Some(op);
        self.length += 1;
    }
    pub fn is_empty(&self) -> bool {
        self.length == 0
    }
    pub fn apply(
        self,
        services: &mut PlayerServices,
        world: &mut impl PlayerWorld,
    ) -> PlayerResult<()> {
        for op in self.ops.into_iter().take(self.length).flatten() {
            match op {
                ContactOp::Created(id) => {
                    let effect = services
                        .resources
                        .effect(id)
                        .copied()
                        .ok_or(PlayerError::Invalid("contact birth lost its effect"))?;
                    world.effect_created(id, effect)?;
                }
                ContactOp::Retire(id) => {
                    services.retire_effect(id, world)?;
                }
                ContactOp::World(effect) => {
                    world.apply_actor_effect(effect)?;
                }
            }
        }
        Ok(())
    }
}

pub struct PlayerContacts<'a, G: PlayerGeometry> {
    coop: &'a mut CoopState,
    services: &'a mut PlayerServices,
    geometry: &'a G,
    frame: ContactFrame,
    bindings: ContactBindings<'a>,
    pending: ContactEffects,
}
impl<'a, G: PlayerGeometry> PlayerContacts<'a, G> {
    pub fn new(
        coop: &'a mut CoopState,
        services: &'a mut PlayerServices,
        geometry: &'a G,
        frame: ContactFrame,
        bindings: ContactBindings<'a>,
    ) -> PlayerResult<Self> {
        frame
            .elevator
            .validate()
            .map_err(|_| PlayerError::Invalid("contact elevator seats"))?;
        frame
            .suppressed
            .validate()
            .map_err(|_| PlayerError::Invalid("contact suppressed seats"))?;
        for seat in SeatId::ALL {
            match (
                coop.config().participants.contains(seat),
                frame.hero_ids[seat.index()],
            ) {
                (true, Some(actor)) if actor.valid() && actor.slot() == u16::from(seat.get()) => {}
                (false, None) => {}
                _ => return Err(PlayerError::Invalid("contact hero identity")),
            }
        }
        Ok(Self {
            coop,
            services,
            geometry,
            frame,
            bindings,
            pending: ContactEffects::default(),
        })
    }
    pub fn finish(self) -> ContactEffects {
        self.pending
    }
    fn cancel_automatic_service(&mut self, seat: SeatId) {
        if let Some(service) = self.services.runtime[seat.index()].service.take() {
            self.pending
                .push(ContactOp::World(ActorEffect::ServiceCollision {
                    actor: service.actor,
                    collidable: true,
                }));
        }
    }
    fn hurt(
        &mut self,
        request: ContactRequest,
        damage: u8,
        squish: bool,
        boogie: bool,
    ) -> PlayerResult<ContactOutcome> {
        let Some(hero) = self.hero(request.seat) else {
            return Ok(ContactOutcome::Rejected);
        };
        if !(hero.receives_effects || request.kind == Kind::Shark && hero.swimming) {
            return Ok(ContactOutcome::Rejected);
        }
        self.pending.reserve(usize::from(
            self.services.runtime[request.seat.index()]
                .service
                .is_some(),
        ))?;
        self.services.audio.reserve(if boogie { 2 } else { 1 })?;
        let accepted = self.services.damage(
            self.coop,
            request.seat,
            crate::players::DamageRequest {
                source: crate::players::DamageSource::Actor(request.actor),
                amount: damage,
                style: if squish {
                    crate::players::DamageStyle::Squish
                } else {
                    crate::players::DamageStyle::Hurt
                },
                origin: request.position,
            },
            self.geometry,
        )?;
        if !accepted {
            return Ok(ContactOutcome::Rejected);
        }
        self.cancel_automatic_service(request.seat);
        if boogie {
            self.services
                .audio
                .enqueue(request.seat, pcm(PcmSound::Boogie, 50, 0))?;
        }
        Ok(ContactOutcome::Applied)
    }
    fn hula(&mut self, request: ContactRequest, distance: u16) -> PlayerResult<ContactOutcome> {
        if distance > 100
            || request.source.level.get() == 0
            || !self.services.can_receive_effect(self.coop, request.seat)
        {
            return Ok(ContactOutcome::Rejected);
        }
        let runtime = &self.services.runtime[request.seat.index()];
        if runtime.contacts.last_hula_tick == Some(self.frame.tick) {
            return Ok(ContactOutcome::Rejected);
        }
        let delay = self.coop.players[request.seat.index()]
            .counters
            .hula_contact_delay;
        let mut rng = self.services.resources.effect_rng;
        let dance = delay == 0 && rng.next_u16() & 3 == 3;
        if dance && distance < 20 {
            self.services.audio.reserve(1)?;
        }
        let player = &mut self.coop.players[request.seat.index()];
        player.counters.hula_contact_delay = if delay == 0 {
            (distance >> 5) as u8 + 4
        } else {
            delay - 1
        };
        self.services.runtime[request.seat.index()]
            .contacts
            .last_hula_tick = Some(self.frame.tick);
        if delay == 0 {
            self.services.resources.effect_rng = rng;
        }
        if dance {
            self.services.status(
                self.coop,
                request.seat,
                crate::players::StatusRequest::Hula(28),
            )?;
            if distance < 20 {
                self.services
                    .audio
                    .enqueue(request.seat, pcm(PcmSound::Aloha, 50, 1))?;
            }
        }
        Ok(ContactOutcome::Applied)
    }
    fn capture_mole(&mut self, request: ContactRequest) -> PlayerResult<ContactOutcome> {
        let Some(hero) = self.hero(request.seat) else {
            return Ok(ContactOutcome::Rejected);
        };
        if !hero.receives_effects
            || !hero.has_inventory
            || hero.elevator
            || self.services.runtime[request.seat.index()]
                .contacts
                .mole
                .is_some()
        {
            return Ok(ContactOutcome::Rejected);
        }
        let (width, height) = self.geometry.dimensions(hero.level)?.world_envelope();
        let (x, y) = (request.position.x >> 8, request.position.y >> 8);
        if x < 8
            || y < 8
            || x >= width - 8
            || y >= height - 8
            || self.geometry.terrain(hero.level, x, y)? & 0xf0 > 0x50
        {
            return Ok(ContactOutcome::Rejected);
        }
        let player = &mut self.coop.players[request.seat.index()];
        player.motion.position = Position {
            elevation: 0,
            ..request.position
        };
        player.motion.velocity = [0; 3];
        player.motion.acceleration = [0; 3];
        player.animation.descriptor = 70;
        let runtime = &mut self.services.runtime[request.seat.index()];
        runtime.mode = PlayerMode::MoleCaptured;
        runtime.authored_motion.axes = 3;
        crate::players::reset_playback(player, runtime);
        runtime.contacts.mole = Some(MoleHold {
            source: request.source,
            actor: request.actor,
            stolen: 0,
            visual: None,
        });
        runtime.effective_input = Default::default();
        Ok(ContactOutcome::Applied)
    }
    fn theft(&mut self, request: ContactRequest, random: u16) -> PlayerResult<ContactOutcome> {
        let runtime = &self.services.runtime[request.seat.index()];
        let Some(hold) = runtime.contacts.mole else {
            return Ok(ContactOutcome::Rejected);
        };
        if hold.actor != request.actor
            || hold.source != request.source
            || hold.stolen >= 3
            || runtime.contacts.last_theft_tick == Some(self.frame.tick)
            || hold
                .visual
                .is_some_and(|id| self.services.resources.effect(id).is_some())
        {
            return Ok(ContactOutcome::Rejected);
        }
        let player = &self.coop.players[request.seat.index()];
        let count = player.inventory.iter().flatten().count();
        if count == 0 {
            return Ok(ContactOutcome::Rejected);
        }
        let wrapper = *player
            .inventory
            .iter()
            .flatten()
            .nth(usize::from(random) % count)
            .expect("occupied theft selection");
        if !self
            .services
            .resources
            .effect_capacity_available(EffectOwner::Actor(request.actor))
        {
            return Ok(ContactOutcome::Rejected);
        }
        self.pending.reserve(1)?;
        let mut rng = self.services.resources.effect_rng;
        let mut effect = Effect::stationary(
            request.seat,
            self.services.resources.presents.kind(wrapper),
            EffectKind::StolenPresent,
            request.source.level,
            player.motion.position,
            120,
        );
        effect.owner = EffectOwner::Actor(request.actor);
        effect.position.y = effect
            .position
            .y
            .checked_sub(3 * 256)
            .ok_or(PlayerError::Invalid("stolen present y"))?;
        effect.position.elevation = 20 * 256;
        effect.velocity[0] = if rng.next_u16() & 1 != 0 { 256 } else { -256 };
        effect.acceleration[2] = -32;
        let id = self.services.resources.spawn_effect(effect)?;
        self.services
            .resources
            .steal_present(self.coop, request.seat, random)?;
        self.services.resources.effect_rng = rng;
        let contacts = &mut self.services.runtime[request.seat.index()].contacts;
        let hold = contacts.mole.as_mut().expect("checked mole capture");
        hold.stolen += 1;
        hold.visual = Some(id);
        contacts.last_theft_tick = Some(self.frame.tick);
        self.pending.push(ContactOp::Created(id));
        Ok(ContactOutcome::Applied)
    }
    fn santa(&mut self, request: ContactRequest, count: u8) -> PlayerResult<ContactOutcome> {
        if !(2..=3).contains(&count) {
            return Err(PlayerError::Invalid("Santa gift count"));
        }
        let terrain = self.geometry.terrain_cache(Landing {
            level: request.source.level,
            position: request.position,
        })?;
        let mut rng = self.services.resources.effect_rng;
        let initial = DroppedPresent {
            owner: EffectOwner::Actor(request.actor),
            level: request.source.level,
            position: request.position,
            present: self
                .services
                .resources
                .presents
                .wrapper_for(PresentKind::Food),
        };
        let mut gifts = [initial; 3];
        for index in 0..usize::from(count) {
            let kind = self.bindings.presents.choose(&mut rng)?;
            let position = self
                .services
                .resources
                .drop_position(
                    request.source.level,
                    request.position,
                    &terrain,
                    rng.next_u16(),
                    &gifts[..index],
                )
                .ok_or(PlayerError::Rejected(
                    "Santa has no supported gift perimeter",
                ))?;
            gifts[index] = DroppedPresent {
                present: self.services.resources.presents.wrapper_for(kind),
                position,
                ..initial
            };
        }
        let mut ids = [None; 3];
        self.services
            .resources
            .spawn_drop_batch(&gifts[..usize::from(count)], &mut ids)?;
        self.services.resources.effect_rng = rng;
        Ok(ContactOutcome::Complete)
    }
    fn service(
        &mut self,
        request: ContactRequest,
        service: Service,
        stage: ServiceStage,
        cost: u8,
    ) -> PlayerResult<ContactOutcome> {
        if request.kind.service() != Some(service)
            || cost
                != if matches!(stage, ServiceStage::Offer | ServiceStage::Begin) {
                    service_cost(service)
                } else {
                    0
                }
        {
            return Err(PlayerError::Invalid("service kind/cost contract"));
        }
        if stage == ServiceStage::End {
            let runtime = &mut self.services.runtime[request.seat.index()];
            if runtime
                .contacts
                .offer
                .is_some_and(|offer| offer.actor != request.actor)
            {
                return Ok(ContactOutcome::Rejected);
            }
            runtime.contacts.offer = None;
            runtime.contacts.response = None;
            runtime.ui_service = None;
            runtime.ui_offer = None;
            if matches!(
                self.coop.players[request.seat.index()].ui.overlay,
                Overlay::MailOrder | Overlay::Identify
            ) {
                self.coop.players[request.seat.index()].ui.overlay = Overlay::Closed;
            }
            self.services.release_forced_pose(self.coop, request.seat);
            return Ok(ContactOutcome::Complete);
        }
        if matches!(service, Service::Lemonade | Service::HotTub) {
            if stage != ServiceStage::Tick {
                return Err(PlayerError::Invalid("automatic service stage"));
            }
            self.pending.reserve(1)?;
            let (accepted, effect) = self.services.advance_automatic_service(
                self.coop,
                request.seat,
                request.actor,
                if service == Service::Lemonade {
                    HeroServiceKind::Lemonade
                } else {
                    HeroServiceKind::HotTub
                },
                request.position,
                self.geometry,
            )?;
            if let Some(effect) = effect {
                self.pending.push(ContactOp::World(effect));
            }
            return Ok(if !accepted {
                ContactOutcome::Rejected
            } else if self.services.runtime[request.seat.index()]
                .service
                .is_none()
            {
                ContactOutcome::Complete
            } else {
                ContactOutcome::Applied
            });
        }
        let Some(hero) = self.hero(request.seat) else {
            return Ok(ContactOutcome::Rejected);
        };
        if !hero.service_eligible {
            return Ok(ContactOutcome::Rejected);
        }
        let current = self.services.runtime[request.seat.index()].contacts.offer;
        if current.is_some_and(|offer| {
            offer.actor != request.actor
                || offer.source != request.source
                || offer.service != service
        }) {
            return Ok(ContactOutcome::Rejected);
        }
        match stage {
            ServiceStage::Offer => {
                if current.is_some() {
                    return Ok(ContactOutcome::Rejected);
                }
                let mut offers = [MailOffer {
                    id: 1,
                    service: request.actor,
                    present: self
                        .services
                        .resources
                        .presents
                        .wrapper_for(PresentKind::Food),
                    price: 3,
                    stock: MailStock::Unlimited,
                }; 3];
                if service == Service::MailOrder {
                    let choices = self
                        .bindings
                        .presents
                        .mail_choices(self.bindings.identity, request.source.level)?;
                    for (index, choice) in choices.into_iter().enumerate() {
                        offers[index] = MailOffer {
                            id: 0x8000_0000
                                | (u32::from(request.source.level.get()) << 12)
                                | (u32::from(request.source.local) << 2)
                                | index as u32,
                            service: request.actor,
                            present: self.services.resources.presents.wrapper_for(choice.kind),
                            price: choice.price,
                            stock: MailStock::Unlimited,
                        };
                    }
                }
                if matches!(service, Service::MailOrder | Service::Identify) {
                    self.services.resources.bind_trade_service(
                        TradeService {
                            actor: request.actor,
                            kind: if service == Service::MailOrder {
                                ServiceKind::MailOrder
                            } else {
                                ServiceKind::Identify
                            },
                            level: request.source.level,
                            position: request.position,
                            source: Some(request.source),
                        },
                        if service == Service::MailOrder {
                            &offers
                        } else {
                            &[]
                        },
                    )?;
                }
                let runtime = &mut self.services.runtime[request.seat.index()];
                runtime.contacts.offer = Some(OfferedService {
                    source: request.source,
                    actor: request.actor,
                    service,
                    cost,
                    active: false,
                });
                runtime.contacts.response = None;
                runtime.ui_service = Some(request.actor);
                Ok(ContactOutcome::Applied)
            }
            ServiceStage::Begin => {
                let Some(offer) = current else {
                    return Ok(ContactOutcome::Rejected);
                };
                if offer.active
                    || self.services.runtime[request.seat.index()]
                        .contacts
                        .response
                        != Some((request.source, true))
                {
                    return Ok(ContactOutcome::Rejected);
                }
                let price = if service == Service::Identify {
                    0
                } else {
                    cost
                };
                if self.coop.players[request.seat.index()].bucks < price {
                    return Ok(ContactOutcome::Rejected);
                }
                self.services.audio.reserve(1)?;
                if service == Service::Opera {
                    self.pending.reserve(1)?;
                }
                let player = &mut self.coop.players[request.seat.index()];
                player.bucks -= price;
                match service {
                    Service::MailOrder => {
                        player.ui.overlay = Overlay::MailOrder;
                        self.services.runtime[request.seat.index()].ui_offer = self
                            .services
                            .resources
                            .offers()
                            .iter()
                            .find(|offer| offer.service == request.actor)
                            .map(|offer| offer.id);
                        self.services.audio.enqueue(
                            request.seat,
                            AudioCue::Psg {
                                command: 7,
                                priority: 50,
                            },
                        )?;
                    }
                    Service::Identify => {
                        player.ui.overlay = Overlay::Identify;
                        self.services
                            .audio
                            .enqueue(request.seat, pcm(PcmSound::Wiseman, 64, 1))?;
                    }
                    Service::Heal => {
                        player.animation.descriptor = 71;
                        self.services
                            .audio
                            .enqueue(request.seat, pcm(PcmSound::Hallelujah, 92, 1))?;
                    }
                    Service::Opera => {
                        player.animation.descriptor = 69;
                        self.services
                            .audio
                            .enqueue(request.seat, pcm(PcmSound::Opera, 90, 1))?;
                        self.pending.push(ContactOp::World(ActorEffect::Opera {
                            seats: request.interest,
                        }));
                    }
                    Service::Lemonade | Service::HotTub => {
                        unreachable!("automatic services handled above")
                    }
                }
                player.motion.velocity = [0; 3];
                player.motion.acceleration = [0; 3];
                let runtime = &mut self.services.runtime[request.seat.index()];
                runtime.contacts.offer = Some(OfferedService {
                    active: true,
                    ..offer
                });
                runtime.contacts.response = None;
                if matches!(service, Service::Heal | Service::Opera) {
                    runtime.authored_motion.axes = 0;
                    crate::players::reset_playback(player, runtime);
                }
                Ok(ContactOutcome::Applied)
            }
            ServiceStage::Tick => {
                if current.is_none_or(|offer| !offer.active) {
                    return Ok(ContactOutcome::Rejected);
                }
                match service {
                    Service::Heal => {
                        let player = &mut self.coop.players[request.seat.index()];
                        let maximum = crate::players::maximum_health(player)?;
                        if player.health < maximum {
                            player.health += 1;
                            Ok(ContactOutcome::Applied)
                        } else {
                            self.services.release_forced_pose(self.coop, request.seat);
                            Ok(ContactOutcome::Complete)
                        }
                    }
                    Service::Opera => {
                        if request.animation_finished {
                            self.services.release_forced_pose(self.coop, request.seat);
                            Ok(ContactOutcome::Complete)
                        } else {
                            self.pending.reserve(1)?;
                            self.pending.push(ContactOp::World(ActorEffect::Opera {
                                seats: request.interest,
                            }));
                            Ok(ContactOutcome::Applied)
                        }
                    }
                    Service::MailOrder | Service::Identify => Ok(
                        if self.coop.players[request.seat.index()].ui.overlay == Overlay::Closed {
                            ContactOutcome::Complete
                        } else {
                            ContactOutcome::Applied
                        },
                    ),
                    Service::Lemonade | Service::HotTub => {
                        unreachable!("automatic services handled above")
                    }
                }
            }
            ServiceStage::End => unreachable!("end handled above"),
        }
    }
    fn apply_checked(&mut self, request: ContactRequest) -> PlayerResult<ContactOutcome> {
        request
            .seat
            .validate()
            .map_err(|_| PlayerError::Invalid("contact seat"))?;
        request
            .source
            .level
            .validate()
            .map_err(|_| PlayerError::Invalid("contact source floor"))?;
        request
            .position
            .validate()
            .map_err(|_| PlayerError::Invalid("contact source position"))?;
        if !population_actor(request.actor) || request.source.local >= 64 {
            return Err(PlayerError::Invalid("contact actor identity"));
        }
        if !matches!(
            request.effect,
            ContactEffect::Projectile { .. }
                | ContactEffect::DecoyHit { .. }
                | ContactEffect::Service {
                    stage: ServiceStage::End,
                    ..
                }
        ) && self
            .hero(request.seat)
            .is_none_or(|hero| hero.level != request.source.level)
        {
            return Ok(ContactOutcome::Rejected);
        }
        match request.effect {
            ContactEffect::Hurt { damage } => self.hurt(request, damage, false, false),
            ContactEffect::Squish { damage } => self.hurt(request, damage, true, false),
            ContactEffect::Boogie { damage } => self.hurt(request, damage, false, true),
            ContactEffect::Hula { distance } => self.hula(request, distance),
            ContactEffect::Whirlwind => Ok(
                if self.services.status(
                    self.coop,
                    request.seat,
                    crate::players::StatusRequest::Whirlwind {
                        ticks: 225,
                        position: request.position,
                    },
                )? {
                    ContactOutcome::Complete
                } else {
                    ContactOutcome::Rejected
                },
            ),
            ContactEffect::MoleCapture => self.capture_mole(request),
            ContactEffect::MoleTheft { random } => self.theft(request, random),
            ContactEffect::MoleRelease => {
                let runtime = &mut self.services.runtime[request.seat.index()];
                if runtime
                    .contacts
                    .mole
                    .is_none_or(|mole| mole.actor != request.actor || mole.source != request.source)
                {
                    return Ok(ContactOutcome::Rejected);
                }
                runtime.contacts.mole = None;
                runtime.mode = PlayerMode::Grounded;
                self.services.release_forced_pose(self.coop, request.seat);
                Ok(ContactOutcome::Complete)
            }
            ContactEffect::SantaDrops { count } => self.santa(request, count),
            ContactEffect::ShipPiece => {
                let part = self
                    .bindings
                    .ship_parts
                    .iter()
                    .find(|binding| binding.source == request.source)
                    .map(|binding| binding.part)
                    .ok_or(PlayerError::Rejected("ship actor lacks objective binding"))?;
                Ok(if self.services.resources.collect_ship_part(part)? {
                    ContactOutcome::Complete
                } else {
                    ContactOutcome::Rejected
                })
            }
            ContactEffect::Service {
                service,
                stage,
                cost,
            } => self.service(request, service, stage, cost),
            ContactEffect::DecoyHit { decoy } => {
                let Some(slot) = decoy.slot().checked_sub(ActorId::EFFECT_START) else {
                    return Ok(ContactOutcome::Rejected);
                };
                let Some(id) = EffectId::new(slot, decoy.generation()) else {
                    return Ok(ContactOutcome::Rejected);
                };
                if self.services.resources.effect(id).is_none_or(|effect| {
                    effect.kind != EffectKind::Decoy
                        || effect.level != request.source.level
                        || effect.hits >= 3
                }) {
                    return Ok(ContactOutcome::Rejected);
                }
                self.pending.reserve(1)?;
                self.services.resources.hit_decoy(id);
                if self
                    .services
                    .resources
                    .effect(id)
                    .is_some_and(|effect| effect.hits >= 3)
                {
                    self.pending.push(ContactOp::Retire(id));
                }
                Ok(ContactOutcome::Applied)
            }
            ContactEffect::Projectile {
                remap,
                position,
                velocity,
                acceleration,
            } => {
                self.pending.reserve(1)?;
                self.services.audio.reserve(1)?;
                let kind = if remap {
                    EffectKind::ControlRemap
                } else {
                    EffectKind::Tomato
                };
                let mut effect = Effect::stationary(
                    request.seat,
                    PresentKind::Tomatoes,
                    kind,
                    request.source.level,
                    position,
                    180,
                );
                effect.owner = EffectOwner::Actor(request.actor);
                effect.source = EffectSource::Actor(
                    crate::DefinitionId::new(request.kind.index() as u16)
                        .expect("bounded kind definition"),
                );
                effect.velocity = velocity;
                effect.acceleration = acceleration;
                let id = self.services.resources.spawn_effect(effect)?;
                self.pending.push(ContactOp::Created(id));
                self.services.audio.enqueue(
                    request.seat,
                    if remap {
                        AudioCue::Template { effect: 4 }
                    } else {
                        pcm(PcmSound::ChickenCannon, 50, 0)
                    },
                )?;
                Ok(ContactOutcome::Applied)
            }
            ContactEffect::SharkAlert => {
                self.services.runtime[request.seat.index()]
                    .contacts
                    .shark_alert_ticks = 128;
                Ok(ContactOutcome::Applied)
            }
        }
    }
}
impl<G: PlayerGeometry> ContactSink for PlayerContacts<'_, G> {
    fn hero(&self, seat: SeatId) -> Option<HeroView> {
        hero_view(self.coop, self.services, self.frame, seat)
    }
    fn apply(&mut self, request: ContactRequest) -> Result<ContactOutcome> {
        match self.apply_checked(request) {
            Ok(outcome) => Ok(outcome),
            Err(PlayerError::Rejected(_) | PlayerError::Capacity(_)) => {
                Ok(ContactOutcome::Rejected)
            }
            Err(error) => Err(player_error(error)),
        }
    }
}
pub fn hero_view(
    coop: &CoopState,
    services: &PlayerServices,
    frame: ContactFrame,
    seat: SeatId,
) -> Option<HeroView> {
    if seat.validate().is_err() || frame.hero_ids[seat.index()].is_none() {
        return None;
    }
    let player = &coop.players[seat.index()];
    let runtime = &services.runtime[seat.index()];
    if player.life != LifeState::Alive {
        return None;
    }
    let level = player.level?;
    let elevator = frame.elevator.contains(seat);
    let suppressed = frame.suppressed.contains(seat)
        || matches!(
            runtime.mode,
            PlayerMode::Dying
                | PlayerMode::Falling
                | PlayerMode::AwaitingFloor
                | PlayerMode::Whirlwind
        );
    let receives = services.can_receive_effect(coop, seat) && !elevator && !suppressed;
    let service_eligible = receives
        && runtime.mode == PlayerMode::Grounded
        && !runtime.locomotion_locked
        && runtime.contacts.mole.is_none()
        && player.motion.position.elevation == 0
        && (player.ui.overlay == Overlay::Closed || runtime.contacts.offer.is_some());
    Some(HeroView {
        seat,
        level,
        position: player.motion.position,
        width: player.character.archetype.dimensions().0,
        velocity: player.motion.velocity,
        targetable: !elevator && !suppressed,
        active_motion: !suppressed && player.motion.velocity[..2].iter().any(|&value| value != 0),
        receives_effects: receives,
        swimming: runtime.mode == PlayerMode::Swimming
            || player.terrain[2][2] & 0xf0 == 0x80 && player.motion.position.elevation <= 0,
        elevator,
        suppressed,
        has_inventory: player.inventory.iter().any(Option::is_some),
        remapped: player.counters.controls_remap_ticks != 0,
        sneaking: runtime.previous_input.sneak,
        burping: runtime.burps_left != 0 && runtime.burp_delay <= 1,
        service_eligible,
        response: runtime.contacts.response,
    })
}
fn population_actor(actor: ActorId) -> bool {
    actor.valid() && (ActorId::POPULATION_START..ActorId::EFFECT_START).contains(&actor.slot())
}
fn pcm(sound: PcmSound, priority: u8, mode: u8) -> AudioCue {
    AudioCue::Pcm {
        sound,
        priority,
        mode,
    }
}
fn service_cost(service: Service) -> u8 {
    match service {
        Service::Heal => 1,
        Service::Identify => 2,
        Service::Opera => 3,
        Service::MailOrder | Service::Lemonade | Service::HotTub => 0,
    }
}
pub(crate) fn player_error(error: PlayerError) -> Error {
    match error {
        PlayerError::Capacity(reason) => Error::Capacity(reason),
        PlayerError::Invalid(reason)
        | PlayerError::Geometry(reason)
        | PlayerError::Rejected(reason) => Error::State(reason),
    }
}
