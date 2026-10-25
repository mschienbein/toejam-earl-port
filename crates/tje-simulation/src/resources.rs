//! Shared present identities and bounded, generation-checked resource transactions.
//! Inventory contents are wrappers, never ROM callbacks or original player indices.
use crate::players::{PlayerError, PlayerResult};
use crate::{ActorId, DefinitionId, EffectId};
use bincode::{Decode, Encode};
use tje_four_player::{
    CoopState, SEATS, SeatId,
    state::{LifeState, Position, PresentWrapper},
};
use tje_world::{LevelId, random::StageRng};

pub const PRESENT_COUNT: usize = 28;
pub const MAX_REQUESTS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
#[repr(u8)]
pub enum PresentKind {
    IcarusWings,
    SpringShoes,
    Innertube,
    Tomatoes,
    Slingshot,
    RocketSkates,
    RoseBushes,
    SuperHiTops,
    Doorway,
    Food,
    RootBeer,
    Promotion,
    Togetherness,
    RainCloud,
    FudgeSundae,
    Decoy,
    TotalBummer,
    ExtraLife,
    Randomizer,
    Telephone,
    ExtraBuck,
    Jackpot,
    TomatoRain,
    Earthling,
    SchoolBook,
    Boombox,
    Mystery,
    BonusHiTops,
}
impl PresentKind {
    pub const ALL: [Self; PRESENT_COUNT] = [
        Self::IcarusWings,
        Self::SpringShoes,
        Self::Innertube,
        Self::Tomatoes,
        Self::Slingshot,
        Self::RocketSkates,
        Self::RoseBushes,
        Self::SuperHiTops,
        Self::Doorway,
        Self::Food,
        Self::RootBeer,
        Self::Promotion,
        Self::Togetherness,
        Self::RainCloud,
        Self::FudgeSundae,
        Self::Decoy,
        Self::TotalBummer,
        Self::ExtraLife,
        Self::Randomizer,
        Self::Telephone,
        Self::ExtraBuck,
        Self::Jackpot,
        Self::TomatoRain,
        Self::Earthling,
        Self::SchoolBook,
        Self::Boombox,
        Self::Mystery,
        Self::BonusHiTops,
    ];
    pub const fn index(self) -> usize {
        self as usize
    }
    pub const fn from_index(index: u8) -> Option<Self> {
        if (index as usize) < PRESENT_COUNT {
            Some(Self::ALL[index as usize])
        } else {
            None
        }
    }
    pub const fn equipped(self) -> bool {
        (self as u8) < 8 || matches!(self, Self::BonusHiTops)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct PresentMap {
    /// Wrapper -> logical effect. Mystery and bonus HiTops are fixed identities.
    kinds: [PresentKind; PRESENT_COUNT],
    identified: [bool; PRESENT_COUNT],
    rng: StageRng,
    pub epoch: u32,
}
impl PresentMap {
    fn new(seed: u64) -> Self {
        let mut identity = [0; 32];
        identity[..8].copy_from_slice(&seed.to_be_bytes());
        let mut result = Self {
            kinds: PresentKind::ALL,
            identified: [false; PRESENT_COUNT],
            rng: StageRng::derive(
                &identity,
                LevelId::new(0).expect("level zero"),
                0,
                b"present-wrappers/1",
            ),
            epoch: 0,
        };
        result.reroll();
        result
    }
    pub fn kind(&self, wrapper: PresentWrapper) -> PresentKind {
        self.kinds[usize::from(wrapper.get())]
    }
    pub fn identified(&self, wrapper: PresentWrapper) -> bool {
        self.identified[usize::from(wrapper.get())]
    }
    pub fn identify(&mut self, wrapper: PresentWrapper) {
        self.identified[usize::from(wrapper.get())] = true;
    }
    pub fn wrapper_for(&self, kind: PresentKind) -> PresentWrapper {
        PresentWrapper::new(
            self.kinds
                .iter()
                .position(|&value| value == kind)
                .expect("validated bijection") as u8,
        )
        .expect("wrapper")
    }
    /// Same collision-scanning permutation policy as SetupPresentIdentities, but
    /// isolated from geometry/AI RNG. No original inventory is ever borrowed.
    fn reroll(&mut self) {
        let mut occupied = [false; PRESENT_COUNT];
        occupied[26] = true;
        occupied[27] = true;
        self.kinds[26] = PresentKind::Mystery;
        self.kinds[27] = PresentKind::BonusHiTops;
        for wrapper in 0..26 {
            let mut identity = self.rng.bounded(27) as usize;
            while occupied[identity] {
                identity = (identity + 1) % 27;
            }
            occupied[identity] = true;
            self.kinds[wrapper] = PresentKind::ALL[identity];
        }
        self.identified = [false; PRESENT_COUNT];
        self.identified[27] = true;
        self.epoch = self.epoch.wrapping_add(1);
    }
    pub fn validate(&self) -> PlayerResult<()> {
        let mut seen = [false; PRESENT_COUNT];
        for kind in self.kinds {
            if seen[kind.index()] {
                return Err(PlayerError::Invalid("present map is not a permutation"));
            }
            seen[kind.index()] = true;
        }
        if self.kinds[26] != PresentKind::Mystery
            || self.kinds[27] != PresentKind::BonusHiTops
            || !self.identified[27]
            || !self.rng.valid()
        {
            return Err(PlayerError::Invalid(
                "fixed present identities or RNG changed",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct ResourceLimits {
    pub effects: u16,
    pub effects_per_seat: u16,
    pub drops: u16,
    pub drops_per_seat: u16,
    pub placed_items: u16,
    pub offers: u16,
    pub services: u16,
}
impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            effects: 64,
            effects_per_seat: 16,
            drops: 128,
            drops_per_seat: 32,
            placed_items: 4096,
            offers: 128,
            services: 128,
        }
    }
}
impl ResourceLimits {
    pub fn validate(self) -> PlayerResult<()> {
        if self.effects == 0
            || self.effects > 64
            || self.effects_per_seat == 0
            || self.effects_per_seat > self.effects
            || self.drops == 0
            || self.drops > 128
            || self.drops_per_seat == 0
            || self.drops_per_seat > self.drops
            || self.placed_items > 4096
            || self.offers > 128
            || self.services > 128
        {
            return Err(PlayerError::Invalid(
                "resource capacity outside admitted envelope",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum EffectOwner {
    Seat(SeatId),
    Actor(ActorId),
    World,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum EffectSource {
    Present(PresentKind),
    Actor(DefinitionId),
    World,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum EffectKind {
    Decoy,
    RainCloud,
    Doorway,
    Telephone,
    Boombox,
    Tomato,
    Slingshot,
    RoseBush,
    RainTomato,
    ControlRemap,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct Effect {
    pub owner: EffectOwner,
    pub source: EffectSource,
    pub kind: EffectKind,
    pub level: LevelId,
    pub position: Position,
    pub velocity: [i32; 3],
    pub acceleration: [i32; 3],
    pub ticks_left: u16,
    pub contact_mask: u8,
    pub hits: u8,
    pub delay: u16,
}
impl Effect {
    pub fn stationary(
        owner: SeatId,
        source: PresentKind,
        kind: EffectKind,
        level: LevelId,
        position: Position,
        ticks_left: u16,
    ) -> Self {
        Self {
            owner: EffectOwner::Seat(owner),
            source: EffectSource::Present(source),
            kind,
            level,
            position,
            velocity: [0; 3],
            acceleration: [0; 3],
            ticks_left,
            contact_mask: 0,
            hits: 0,
            delay: 0,
        }
    }
    fn validate(self) -> PlayerResult<()> {
        self.level
            .validate()
            .map_err(|_| PlayerError::Invalid("effect level"))?;
        self.position
            .validate()
            .map_err(|_| PlayerError::Invalid("effect position"))?;
        if let EffectOwner::Seat(seat) = self.owner {
            seat.validate()
                .map_err(|_| PlayerError::Invalid("effect owner"))?;
        }
        if let EffectOwner::Actor(actor) = self.owner
            && !actor.valid()
        {
            return Err(PlayerError::Invalid("effect actor owner"));
        }
        if let EffectSource::Actor(definition) = self.source
            && !definition.valid()
        {
            return Err(PlayerError::Invalid("effect source definition"));
        }
        if matches!(
            self.kind,
            EffectKind::Decoy
                | EffectKind::RainCloud
                | EffectKind::Doorway
                | EffectKind::Telephone
                | EffectKind::Boombox
                | EffectKind::RoseBush
        ) && !matches!(self.owner, EffectOwner::Seat(_))
        {
            return Err(PlayerError::Invalid("seat-only effect ownership"));
        }
        if self.contact_mask & !15 != 0
            || self.ticks_left == 0
            || self
                .velocity
                .iter()
                .chain(&self.acceleration)
                .any(|value| value.unsigned_abs() > 64 * 256)
        {
            return Err(PlayerError::Invalid(
                "invalid effect motion/lifetime/contact mask",
            ));
        }
        Ok(())
    }
    pub fn actor_id(id: EffectId) -> ActorId {
        ActorId::new(352 + id.slot(), id.generation()).expect("effect draw domain")
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
struct EffectSlot {
    generation: u32,
    value: Option<Effect>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Encode, Decode)]
pub struct DropId {
    slot: u16,
    generation: u32,
}
impl DropId {
    pub const fn slot(self) -> u16 {
        self.slot
    }
    pub const fn generation(self) -> u32 {
        self.generation
    }
    pub fn actor_id(self) -> ActorId {
        ActorId::new(416 + self.slot, self.generation).expect("drop draw domain")
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct DroppedPresent {
    pub owner: SeatId,
    pub level: LevelId,
    pub position: Position,
    pub present: PresentWrapper,
}
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
struct DropSlot {
    generation: u32,
    value: Option<DroppedPresent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum ItemKind {
    Present(PresentWrapper),
    Food(i8),
    Buck(u8),
    ShipPart(u8),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct PlacedItem {
    pub id: u32,
    pub level: LevelId,
    pub position: Position,
    pub kind: ItemKind,
    pub collected: bool,
}

/// OriginalV1's stable source slot, not a loaded-bank entity handle. Empty holes
/// retain their slot number so later claims can never renumber a persistent ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StockItem {
    pub slot: u8,
    pub kind: i8,
    pub level: LevelId,
    pub x: i32,
    pub y: i32,
}
pub trait ItemDefinitions {
    fn food_health_delta(&self, kind: u8) -> PlayerResult<i8>;
}
/// A bounded borrowed table from the session's authenticated user ROM. Nothing
/// ROM-derived is copied into source, persistent configuration or public data.
pub struct RomItemDefinitions<'a> {
    health: &'a [u8],
}
impl<'a> RomItemDefinitions<'a> {
    pub fn new(rom: &'a [u8], revision: tje_mem::Revision) -> PlayerResult<Self> {
        let start = tje_mem::rom::present_auxiliary_signed_lookup(revision)
            .at(0)
            .address() as usize;
        let end = start
            .checked_add(20)
            .ok_or(PlayerError::Invalid("food catalogue extent"))?;
        let health = rom
            .get(start..end)
            .ok_or(PlayerError::Rejected("user ROM lacks food catalogue"))?;
        Ok(Self { health })
    }
}
impl ItemDefinitions for RomItemDefinitions<'_> {
    fn food_health_delta(&self, kind: u8) -> PlayerResult<i8> {
        if !(64..=83).contains(&kind) || kind == 80 {
            return Err(PlayerError::Invalid("food catalogue kind"));
        }
        Ok(self.health[usize::from(kind - 64)] as i8)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum ServiceKind {
    MailOrder,
    Identify,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct TradeService {
    pub actor: ActorId,
    pub kind: ServiceKind,
    pub level: LevelId,
    pub position: Position,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct MailOffer {
    pub id: u32,
    pub service: ActorId,
    pub present: PresentWrapper,
    pub price: u8,
    pub stock: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemRef {
    Placed(u32),
    Dropped(DropId),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceAction {
    Claim(ItemRef),
    Purchase { offer: u32, service: ActorId },
    Drop { slot: u8 },
    Identify { slot: u8, service: ActorId },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceRequest {
    pub seat: SeatId,
    pub action: ResourceAction,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceFailure {
    Unavailable,
    NotNearby,
    InventoryFull,
    InsufficientBucks,
    InvalidSlot,
    Protected,
    AlreadyIdentified,
    DuplicateRequest,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceReceipt {
    Claimed,
    Purchased,
    Dropped(DropId),
    Identified(PresentKind),
    ShipPart(u8),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceOutcome {
    pub request: u8,
    pub result: Result<ResourceReceipt, ResourceFailure>,
}
impl Default for ResourceOutcome {
    fn default() -> Self {
        Self {
            request: 0,
            result: Err(ResourceFailure::Unavailable),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct ResourceState {
    pub presents: PresentMap,
    limits: ResourceLimits,
    effects: Vec<EffectSlot>,
    drops: Vec<DropSlot>,
    items: Vec<PlacedItem>,
    stock_imported: u32,
    services: Vec<TradeService>,
    offers: Vec<MailOffer>,
    pub ship_parts: u16,
    pub priority: SeatId,
    last_transaction_tick: Option<u64>,
    pub(crate) effect_rng: StageRng,
}
impl ResourceState {
    pub fn new(seed: u64, limits: ResourceLimits) -> PlayerResult<Self> {
        limits.validate()?;
        let mut identity = [0; 32];
        identity[..8].copy_from_slice(&seed.to_be_bytes());
        Ok(Self {
            presents: PresentMap::new(seed),
            limits,
            effects: vec![
                EffectSlot {
                    generation: 1,
                    value: None
                };
                usize::from(limits.effects)
            ],
            drops: vec![
                DropSlot {
                    generation: 1,
                    value: None
                };
                usize::from(limits.drops)
            ],
            items: Vec::with_capacity(usize::from(limits.placed_items)),
            stock_imported: 0,
            services: Vec::with_capacity(usize::from(limits.services)),
            offers: Vec::with_capacity(usize::from(limits.offers)),
            ship_parts: 0,
            priority: SeatId::ALL[0],
            last_transaction_tick: None,
            effect_rng: StageRng::derive(
                &identity,
                LevelId::new(0).expect("level zero"),
                0,
                b"present-effects/1",
            ),
        })
    }
    pub fn limits(&self) -> ResourceLimits {
        self.limits
    }
    pub fn effects(&self) -> impl Iterator<Item = (EffectId, &Effect)> {
        self.effects.iter().enumerate().filter_map(|(index, slot)| {
            slot.value.as_ref().map(|value| {
                (
                    EffectId::new(index as u16, slot.generation).expect("validated effect"),
                    value,
                )
            })
        })
    }
    pub fn effect(&self, id: EffectId) -> Option<&Effect> {
        self.effects
            .get(id.index())
            .filter(|slot| slot.generation == id.generation())
            .and_then(|slot| slot.value.as_ref())
    }
    pub fn effect_mut(&mut self, id: EffectId) -> Option<&mut Effect> {
        self.effects
            .get_mut(id.index())
            .filter(|slot| slot.generation == id.generation())
            .and_then(|slot| slot.value.as_mut())
    }
    pub fn effect_capacity_available(&self, owner: EffectOwner) -> bool {
        self.effects
            .iter()
            .any(|slot| slot.value.is_none() && slot.generation != 0)
            && (!matches!(owner, EffectOwner::Seat(_))
                || self
                    .effects()
                    .filter(|(_, effect)| effect.owner == owner)
                    .count()
                    < usize::from(self.limits.effects_per_seat))
    }
    pub fn spawn_effect(&mut self, effect: Effect) -> PlayerResult<EffectId> {
        effect.validate()?;
        if !self.effect_capacity_available(effect.owner) {
            return Err(PlayerError::Capacity("effect pool or owner quota"));
        }
        let index = self
            .effects
            .iter()
            .position(|slot| slot.value.is_none() && slot.generation != 0)
            .expect("capacity checked");
        let slot = &mut self.effects[index];
        let id = EffectId::new(index as u16, slot.generation).expect("bounded effect id");
        slot.value = Some(effect);
        Ok(id)
    }
    pub fn release_effect(&mut self, id: EffectId) -> bool {
        let Some(slot) = self
            .effects
            .get_mut(id.index())
            .filter(|slot| slot.generation == id.generation() && slot.value.is_some())
        else {
            return false;
        };
        slot.value = None;
        // Retire a slot rather than allowing a stale generation to become live.
        slot.generation = slot.generation.checked_add(1).unwrap_or(0);
        true
    }
    pub fn hit_decoy(&mut self, id: EffectId) -> bool {
        let Some(effect) = self
            .effect_mut(id)
            .filter(|effect| effect.kind == EffectKind::Decoy)
        else {
            return false;
        };
        effect.hits = effect.hits.saturating_add(1);
        if effect.hits >= 3 {
            self.release_effect(id);
        }
        true
    }
    pub fn drops(&self) -> impl Iterator<Item = (DropId, &DroppedPresent)> {
        self.drops.iter().enumerate().filter_map(|(index, slot)| {
            slot.value.as_ref().map(|value| {
                (
                    DropId {
                        slot: index as u16,
                        generation: slot.generation,
                    },
                    value,
                )
            })
        })
    }
    pub fn item(&self, id: u32) -> Option<&PlacedItem> {
        self.items.iter().find(|item| item.id == id)
    }
    pub fn items(&self) -> &[PlacedItem] {
        &self.items
    }
    pub fn services(&self) -> &[TradeService] {
        &self.services
    }
    pub fn offers(&self) -> &[MailOffer] {
        &self.offers
    }
    pub fn stock_imported(&self, level: LevelId) -> bool {
        1_u32
            .checked_shl(u32::from(level.get()))
            .is_some_and(|bit| self.stock_imported & bit != 0)
    }
    /// Prepare all definitions, source slots and capacity before publishing any
    /// item. Reentry reuses this ledger, never imports and repopulates a floor.
    pub fn import_stock_items(
        &mut self,
        level: LevelId,
        source: &[StockItem],
        definitions: &impl ItemDefinitions,
    ) -> PlayerResult<()> {
        level
            .validate()
            .map_err(|_| PlayerError::Invalid("stock item floor"))?;
        if source.len() > 32
            || self.stock_imported(level)
            || self.items.iter().any(|item| item.level == level)
        {
            return Err(PlayerError::Rejected(
                "stock floor was already imported or exceeds slots",
            ));
        }
        let mut prepared = [None; 32];
        let mut seen = 0_u32;
        let mut count = 0;
        for item in source {
            if item.slot >= 32 || seen & (1 << item.slot) != 0 {
                return Err(PlayerError::Invalid("stock source slot"));
            }
            seen |= 1 << item.slot;
            if item.kind == -1 {
                continue;
            }
            if item.level != level {
                return Err(PlayerError::Invalid("stock item level differs from floor"));
            }
            if item.x < 0 || item.y < 0 {
                return Err(PlayerError::Invalid("negative stock item coordinate"));
            }
            let kind = match item.kind {
                0..=27 => ItemKind::Present(self.presents.wrapper_for(
                    PresentKind::from_index(item.kind as u8).expect("bounded present"),
                )),
                80 => ItemKind::Buck(1),
                64..=83 => ItemKind::Food(definitions.food_health_delta(item.kind as u8)?),
                _ => return Err(PlayerError::Rejected("unsupported stock item identity")),
            };
            let position = Position {
                x: item
                    .x
                    .checked_mul(256)
                    .ok_or(PlayerError::Invalid("stock item x"))?,
                y: item
                    .y
                    .checked_mul(256)
                    .ok_or(PlayerError::Invalid("stock item y"))?,
                elevation: 0,
            };
            position
                .validate()
                .map_err(|_| PlayerError::Invalid("stock item position"))?;
            let id = u32::from(level.get()) * 128 + u32::from(item.slot) + 1;
            if self.item(id).is_some() {
                return Err(PlayerError::Invalid("stock stable item ID collision"));
            }
            prepared[usize::from(item.slot)] = Some(PlacedItem {
                id,
                level,
                position,
                kind,
                collected: false,
            });
            count += 1;
        }
        if self.items.len() + count > usize::from(self.limits.placed_items) {
            return Err(PlayerError::Capacity("stock items"));
        }
        for item in prepared.into_iter().flatten() {
            let index = self.items.partition_point(|existing| existing.id < item.id);
            self.items.insert(index, item);
        }
        self.stock_imported |= 1 << level.get();
        Ok(())
    }
    pub(crate) fn reroll_wrappers(&mut self) {
        let old = self.presents.kinds;
        self.presents.reroll();
        // World item identities stay fixed; only their wrapping appearance
        // changes. The separate sixty-step inventory operation rerolls contents.
        for item in &mut self.items {
            if let ItemKind::Present(wrapper) = &mut item.kind {
                *wrapper = self.presents.wrapper_for(old[usize::from(wrapper.get())]);
            }
        }
        for slot in &mut self.drops {
            if let Some(drop) = &mut slot.value {
                drop.present = self
                    .presents
                    .wrapper_for(old[usize::from(drop.present.get())]);
            }
        }
    }
    pub fn add_item(&mut self, item: PlacedItem) -> PlayerResult<()> {
        item.level
            .validate()
            .map_err(|_| PlayerError::Invalid("item level"))?;
        item.position
            .validate()
            .map_err(|_| PlayerError::Invalid("item position"))?;
        validate_item(item.kind)?;
        if item.id == 0 || item.collected || self.item(item.id).is_some() {
            return Err(PlayerError::Invalid(
                "duplicate/collected initial item identity",
            ));
        }
        if self.items.len() == usize::from(self.limits.placed_items)
            || self
                .items
                .iter()
                .filter(|existing| existing.level == item.level)
                .count()
                >= 128
        {
            return Err(PlayerError::Capacity("persistent items"));
        }
        let index = self.items.partition_point(|existing| existing.id < item.id);
        self.items.insert(index, item);
        Ok(())
    }
    pub fn add_service(&mut self, service: TradeService) -> PlayerResult<()> {
        service
            .level
            .validate()
            .map_err(|_| PlayerError::Invalid("service level"))?;
        service
            .position
            .validate()
            .map_err(|_| PlayerError::Invalid("service position"))?;
        if !service.actor.valid() {
            return Err(PlayerError::Invalid("trade service actor"));
        }
        if self
            .services
            .iter()
            .any(|existing| existing.actor == service.actor)
        {
            return Err(PlayerError::Invalid("duplicate trade service"));
        }
        if self.services.len() == usize::from(self.limits.services) {
            return Err(PlayerError::Capacity("trade services"));
        }
        self.services.push(service);
        Ok(())
    }
    pub fn add_offer(&mut self, offer: MailOffer) -> PlayerResult<()> {
        offer
            .present
            .validate()
            .map_err(|_| PlayerError::Invalid("offer wrapper"))?;
        if offer.id == 0
            || offer.price > 127
            || self.offers.iter().any(|existing| existing.id == offer.id)
            || !self.services.iter().any(|service| {
                service.actor == offer.service && service.kind == ServiceKind::MailOrder
            })
        {
            return Err(PlayerError::Invalid("invalid mail offer/service"));
        }
        if self.offers.len() == usize::from(self.limits.offers) {
            return Err(PlayerError::Capacity("mail offers"));
        }
        self.offers.push(offer);
        Ok(())
    }
    fn near_service(
        &self,
        coop: &CoopState,
        seat: SeatId,
        actor: ActorId,
        kind: ServiceKind,
    ) -> bool {
        self.services.iter().any(|service| {
            service.actor == actor
                && service.kind == kind
                && nearby(coop, seat, service.level, service.position, 48, 32)
        })
    }
    /// Every request is evaluated once in recorded rotating-seat order. Ties
    /// within one seat retain caller order. The output retains original indices.
    /// The batch itself is admitted once; replaying its tick never charges twice.
    pub fn transact(
        &mut self,
        coop: &mut CoopState,
        tick: u64,
        requests: &[ResourceRequest],
        output: &mut [ResourceOutcome],
    ) -> PlayerResult<usize> {
        if requests.len() > MAX_REQUESTS
            || output.len() < requests.len()
            || self
                .last_transaction_tick
                .is_some_and(|previous| tick <= previous)
        {
            return Err(PlayerError::Invalid(
                "resource batch capacity or repeated tick",
            ));
        }
        for request in requests {
            request
                .seat
                .validate()
                .map_err(|_| PlayerError::Invalid("resource requester"))?;
        }
        let mut written = 0;
        let mut used = [0_u8; SEATS];
        for offset in 0..SEATS {
            let seat = SeatId::ALL[(self.priority.index() + offset) % SEATS];
            for (index, request) in requests
                .iter()
                .enumerate()
                .filter(|(_, request)| request.seat == seat)
            {
                let tag = match request.action {
                    ResourceAction::Claim(_) => 1,
                    ResourceAction::Purchase { .. } => 2,
                    ResourceAction::Drop { .. } => 4,
                    ResourceAction::Identify { .. } => 8,
                };
                let result = if used[seat.index()] & tag != 0 {
                    Err(ResourceFailure::DuplicateRequest)
                } else {
                    used[seat.index()] |= tag;
                    self.apply(coop, *request)
                };
                output[written] = ResourceOutcome {
                    request: index as u8,
                    result,
                };
                written += 1;
            }
        }
        self.last_transaction_tick = Some(tick);
        self.priority = SeatId::ALL[(self.priority.index() + 1) % SEATS];
        coop.claim_priority = (0..SEATS)
            .map(|offset| SeatId::ALL[(self.priority.index() + offset) % SEATS])
            .find(|&seat| coop.config().participants.contains(seat))
            .ok_or(PlayerError::Invalid("no participating claim seat"))?;
        Ok(written)
    }
    fn apply(
        &mut self,
        coop: &mut CoopState,
        request: ResourceRequest,
    ) -> Result<ResourceReceipt, ResourceFailure> {
        let seat = request.seat;
        let player = &coop.players[seat.index()];
        if player.life != LifeState::Alive
            || player.counters.fall != tje_four_player::state::FallState::None
        {
            return Err(ResourceFailure::Protected);
        }
        match request.action {
            ResourceAction::Claim(reference) => self.claim(coop, seat, reference),
            ResourceAction::Drop { slot } => {
                let wrapper = player
                    .inventory
                    .get(usize::from(slot))
                    .copied()
                    .flatten()
                    .ok_or(ResourceFailure::InvalidSlot)?;
                if self.drops().filter(|(_, drop)| drop.owner == seat).count()
                    >= usize::from(self.limits.drops_per_seat)
                {
                    return Err(ResourceFailure::Unavailable);
                }
                let index = self
                    .drops
                    .iter()
                    .position(|slot| slot.value.is_none() && slot.generation != 0)
                    .ok_or(ResourceFailure::Unavailable)?;
                let id = DropId {
                    slot: index as u16,
                    generation: self.drops[index].generation,
                };
                self.drops[index].value = Some(DroppedPresent {
                    owner: seat,
                    level: player.level.expect("alive level"),
                    position: player.motion.position,
                    present: wrapper,
                });
                compact_slot(&mut coop.players[seat.index()].inventory, usize::from(slot));
                Ok(ResourceReceipt::Dropped(id))
            }
            ResourceAction::Purchase { offer, service } => {
                if !self.near_service(coop, seat, service, ServiceKind::MailOrder) {
                    return Err(ResourceFailure::NotNearby);
                }
                let offer = self
                    .offers
                    .iter_mut()
                    .find(|value| value.id == offer && value.service == service && value.stock != 0)
                    .ok_or(ResourceFailure::Unavailable)?;
                let slot = player
                    .inventory
                    .iter()
                    .position(Option::is_none)
                    .ok_or(ResourceFailure::InventoryFull)?;
                if player.bucks < offer.price {
                    return Err(ResourceFailure::InsufficientBucks);
                }
                let player = &mut coop.players[seat.index()];
                player.bucks -= offer.price;
                player.inventory[slot] = Some(offer.present);
                offer.stock -= 1;
                Ok(ResourceReceipt::Purchased)
            }
            ResourceAction::Identify { slot, service } => {
                if !self.near_service(coop, seat, service, ServiceKind::Identify) {
                    return Err(ResourceFailure::NotNearby);
                }
                let wrapper = player
                    .inventory
                    .get(usize::from(slot))
                    .copied()
                    .flatten()
                    .ok_or(ResourceFailure::InvalidSlot)?;
                if player.bucks < 2 {
                    return Err(ResourceFailure::InsufficientBucks);
                }
                if self.presents.identified(wrapper)
                    && self.presents.kind(wrapper) != PresentKind::Mystery
                {
                    return Err(ResourceFailure::AlreadyIdentified);
                }
                let actual = if self.presents.kind(wrapper) == PresentKind::Mystery {
                    let kind = PresentKind::ALL[self.effect_rng.bounded(26) as usize];
                    self.presents.wrapper_for(kind)
                } else {
                    wrapper
                };
                self.presents.identify(actual);
                coop.players[seat.index()].inventory[usize::from(slot)] = Some(actual);
                coop.players[seat.index()].bucks -= 2;
                Ok(ResourceReceipt::Identified(self.presents.kind(actual)))
            }
        }
    }
    fn claim(
        &mut self,
        coop: &mut CoopState,
        seat: SeatId,
        reference: ItemRef,
    ) -> Result<ResourceReceipt, ResourceFailure> {
        let (level, position, kind) = match reference {
            ItemRef::Placed(id) => {
                let item = self
                    .item(id)
                    .filter(|item| !item.collected)
                    .ok_or(ResourceFailure::Unavailable)?;
                (item.level, item.position, item.kind)
            }
            ItemRef::Dropped(id) => {
                let drop = self
                    .drops
                    .get(usize::from(id.slot))
                    .filter(|slot| slot.generation == id.generation)
                    .and_then(|slot| slot.value)
                    .ok_or(ResourceFailure::Unavailable)?;
                (drop.level, drop.position, ItemKind::Present(drop.present))
            }
        };
        if !nearby(coop, seat, level, position, 16, 8) {
            return Err(ResourceFailure::NotNearby);
        }
        let player = &mut coop.players[seat.index()];
        let receipt = match kind {
            ItemKind::Present(wrapper) => {
                let slot = player
                    .inventory
                    .iter()
                    .position(Option::is_none)
                    .ok_or(ResourceFailure::InventoryFull)?;
                player.inventory[slot] = Some(wrapper);
                ResourceReceipt::Claimed
            }
            ItemKind::Food(delta) => {
                let maximum = player
                    .character
                    .archetype
                    .maximum_health(player.rank)
                    .expect("validated rank");
                player.health = (i16::from(player.health) + i16::from(delta))
                    .clamp(0, i16::from(maximum)) as u8;
                ResourceReceipt::Claimed
            }
            ItemKind::Buck(amount) => {
                player.bucks = player.bucks.saturating_add(amount).min(127);
                ResourceReceipt::Claimed
            }
            ItemKind::ShipPart(part) => {
                if self.ship_parts & (1 << part) != 0 {
                    return Err(ResourceFailure::Unavailable);
                }
                self.ship_parts |= 1 << part;
                ResourceReceipt::ShipPart(part)
            }
        };
        match reference {
            ItemRef::Placed(id) => {
                self.items
                    .iter_mut()
                    .find(|item| item.id == id)
                    .expect("checked item")
                    .collected = true
            }
            ItemRef::Dropped(id) => {
                let slot = &mut self.drops[usize::from(id.slot)];
                slot.value = None;
                slot.generation = slot.generation.checked_add(1).unwrap_or(0);
            }
        }
        Ok(receipt)
    }
    pub fn validate(&self, coop: &CoopState) -> PlayerResult<()> {
        self.limits.validate()?;
        self.presents.validate()?;
        if self.effects.len() != usize::from(self.limits.effects)
            || self.drops.len() != usize::from(self.limits.drops)
            || self.items.len() > usize::from(self.limits.placed_items)
            || self.services.len() > usize::from(self.limits.services)
            || self.offers.len() > usize::from(self.limits.offers)
            || self.ship_parts & !0x03ff != 0
            || !self.effect_rng.valid()
        {
            return Err(PlayerError::Invalid("resource pool extent/objective/RNG"));
        }
        self.priority
            .validate()
            .map_err(|_| PlayerError::Invalid("resource priority"))?;
        let mut effects = [0_u16; SEATS];
        let mut drops = [0_u16; SEATS];
        for slot in &self.effects {
            if let Some(effect) = slot.value {
                if slot.generation == 0 {
                    return Err(PlayerError::Invalid("retired effect is live"));
                }
                effect.validate()?;
                if let EffectOwner::Seat(seat) = effect.owner {
                    if !coop.config().participants.contains(seat) {
                        return Err(PlayerError::Invalid("effect has disabled owner"));
                    }
                    effects[seat.index()] += 1;
                }
            }
        }
        for slot in &self.drops {
            if let Some(drop) = slot.value {
                drop.owner
                    .validate()
                    .map_err(|_| PlayerError::Invalid("drop owner"))?;
                drop.level
                    .validate()
                    .map_err(|_| PlayerError::Invalid("drop level"))?;
                drop.position
                    .validate()
                    .map_err(|_| PlayerError::Invalid("drop position"))?;
                drop.present
                    .validate()
                    .map_err(|_| PlayerError::Invalid("drop wrapper"))?;
                if slot.generation == 0 || !coop.config().participants.contains(drop.owner) {
                    return Err(PlayerError::Invalid("retired drop or disabled owner"));
                }
                drops[drop.owner.index()] += 1;
            }
        }
        if effects
            .iter()
            .any(|&count| count > self.limits.effects_per_seat)
            || drops
                .iter()
                .any(|&count| count > self.limits.drops_per_seat)
        {
            return Err(PlayerError::Invalid("owner quota exceeded"));
        }
        let mut last_id = 0;
        let mut per_floor = [0_u16; 32];
        for item in &self.items {
            item.level
                .validate()
                .map_err(|_| PlayerError::Invalid("item level"))?;
            item.position
                .validate()
                .map_err(|_| PlayerError::Invalid("item position"))?;
            validate_item(item.kind)?;
            per_floor[item.level.index()] += 1;
            if per_floor[item.level.index()] > 128 {
                return Err(PlayerError::Invalid("per-floor item capacity"));
            }
            if item.id <= last_id {
                return Err(PlayerError::Invalid("items not uniquely sorted"));
            }
            last_id = item.id;
        }
        for (index, service) in self.services.iter().enumerate() {
            service
                .level
                .validate()
                .map_err(|_| PlayerError::Invalid("service level"))?;
            service
                .position
                .validate()
                .map_err(|_| PlayerError::Invalid("service position"))?;
            if !service.actor.valid()
                || self.services[..index]
                    .iter()
                    .any(|existing| existing.actor == service.actor)
            {
                return Err(PlayerError::Invalid("duplicate/invalid service"));
            }
        }
        for (index, offer) in self.offers.iter().enumerate() {
            offer
                .present
                .validate()
                .map_err(|_| PlayerError::Invalid("mail wrapper"))?;
            if offer.id == 0
                || offer.price > 127
                || self.offers[..index]
                    .iter()
                    .any(|existing| existing.id == offer.id)
                || !self.services.iter().any(|service| {
                    service.actor == offer.service && service.kind == ServiceKind::MailOrder
                })
            {
                return Err(PlayerError::Invalid("mail offer identity/service"));
            }
        }
        Ok(())
    }
}
fn validate_item(kind: ItemKind) -> PlayerResult<()> {
    match kind {
        ItemKind::Present(wrapper) => wrapper
            .validate()
            .map_err(|_| PlayerError::Invalid("item wrapper")),
        ItemKind::ShipPart(part) if part >= 10 => Err(PlayerError::Invalid("ship part identity")),
        ItemKind::Buck(amount) if amount == 0 || amount > 127 => {
            Err(PlayerError::Invalid("buck item amount"))
        }
        _ => Ok(()),
    }
}
pub(crate) fn compact_slot(inventory: &mut [Option<PresentWrapper>; 16], slot: usize) {
    inventory.copy_within(slot + 1.., slot);
    inventory[15] = None;
}
fn nearby(
    coop: &CoopState,
    seat: SeatId,
    level: LevelId,
    position: Position,
    dx: i64,
    dy: i64,
) -> bool {
    let player = &coop.players[seat.index()];
    player.life == LifeState::Alive
        && player.level == Some(level)
        && player.motion.position.elevation < 10 * 256
        && (i64::from(player.motion.position.x) - i64::from(position.x)).abs() < dx * 256
        && (i64::from(player.motion.position.y) - i64::from(position.y)).abs() < dy * 256
}
