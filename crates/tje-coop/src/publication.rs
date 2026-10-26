//! Freeze logical state at Publish; later rendering consumes that saved scene,
//! never newer even-phase actors/UI. No animation, RNG or mutations run here.
pub mod resources;
pub use resources::{EffectAnimation, ResourceCatalog, ResourceKey};
use std::fmt;
use tje_four_player::{
    SeatId,
    state::{CoopState, LifeState, Overlay, Position},
};
use tje_mem::AddressSpace;
use tje_mode_view::{
    Color, Compositor, FrameSize, HEIGHT, MAX_ACTORS, MAX_PATCHES, Publisher, WIDTH,
    overlay::{Hud, Inventory, InventoryKind, Item, Map, Marker, SeatOverlay, Speech},
    packet::{DrawId, Metasprite, PriorityPatch, Shadow, VramShadow},
    source::WorldSource,
};
use tje_simulation::{
    ActorId, EffectId, PhaseClock, PublicationStamp,
    players::PlayerServices,
    population::PopulationState,
    publication::{
        GeometryReference, PublishedActor, PublishedHud, PublishedInventory,
        PublishedInventoryKind, PublishedItem, PublishedKind, PublishedMap, PublishedMarker,
        PublishedOverlay, PublishedPatch, PublishedScene, PublishedShadow, PublishedSpeech,
        PublishedStyle, PublishedView,
    },
    resources::{PlacedItem, PresentMap},
    session::SessionConfig,
};
use tje_world::{LevelId, world::WorldState};

#[derive(Debug)]
pub enum Error {
    Invalid(&'static str),
    Resource(resources::ResourceError),
    View(tje_mode_view::Error),
    Game(tje_game::Error),
    World(tje_world::Error),
    Simulation(tje_simulation::Error),
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(reason) => f.write_str(reason),
            Self::Resource(error) => error.fmt(f),
            Self::View(error) => error.fmt(f),
            Self::Game(error) => error.fmt(f),
            Self::World(error) => error.fmt(f),
            Self::Simulation(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for Error {}
impl From<resources::ResourceError> for Error {
    fn from(error: resources::ResourceError) -> Self {
        Self::Resource(error)
    }
}
impl From<tje_mode_view::Error> for Error {
    fn from(error: tje_mode_view::Error) -> Self {
        Self::View(error)
    }
}
impl From<tje_game::Error> for Error {
    fn from(error: tje_game::Error) -> Self {
        Self::Game(error)
    }
}
impl From<tje_world::Error> for Error {
    fn from(error: tje_world::Error) -> Self {
        Self::World(error)
    }
}
impl From<tje_simulation::Error> for Error {
    fn from(error: tje_simulation::Error) -> Self {
        Self::Simulation(error)
    }
}
pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisualStyle {
    pub draw_late: bool,
    pub high_priority: bool,
    pub palette_bank: u8,
    pub flip_vertical: bool,
    pub mask_pixels: bool,
    pub shadow: bool,
    pub submerged: bool,
    pub hidden: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Playback {
    pub animation: tje_simulation::AnimationId,
    pub sequence_index: u16,
    pub style: VisualStyle,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DynamicVisual {
    Hidden,
    Animated(Playback),
    Static {
        frame: tje_simulation::FrameId,
        style: VisualStyle,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Prop {
    pub id: u16,
    pub level: LevelId,
    pub position: Position,
    pub playback: Playback,
    pub map_marker: bool,
}
pub struct SpeechView<'a> {
    pub text: &'a [u8],
    pub choices: bool,
    pub accept_selected: bool,
}
pub trait DiscoveryView {
    fn columns(&self) -> u16;
    fn rows(&self) -> u16;
    fn revealed(&self, index: u16) -> bool;
}
/// Read-only disjoint borrows, so the integration owner can simultaneously lend
/// its PublishedScene target without cloning Session or a complete world.
pub trait PublicationSource {
    fn config(&self) -> &SessionConfig;
    fn world(&self) -> &WorldState;
    fn players(&self) -> &CoopState;
    fn clock(&self) -> &PhaseClock;
    fn hero_actor(&self, seat: SeatId) -> Option<ActorId>;
    fn services(&self) -> &PlayerServices;
    fn population(&self) -> &PopulationState;
    fn world_visible(&self) -> bool;
    fn terrain_attributes(&self, seat: SeatId) -> u16;
    fn geometry_epoch(&self, level: LevelId) -> Result<u64>;
    fn hero_style(&self, seat: SeatId) -> Result<VisualStyle>;
    fn companion_visual(&self, actor: ActorId) -> Result<DynamicVisual>;
    fn effect_visual(&self, effect: EffectId) -> Result<DynamicVisual>;
    fn props(&self) -> impl Iterator<Item = Prop> + '_;
    fn speech(&self, seat: SeatId) -> Result<Option<SpeechView<'_>>>;
    fn patches(&self) -> impl Iterator<Item = PriorityPatch> + '_;
    fn discovery(&self, level: LevelId) -> Result<&dyn DiscoveryView>;
    fn item_appearance(&self, item: &PlacedItem) -> Result<ResourceKey>;
}
/// Refresh needs only frozen logical state and versioned geometry. It has no
/// accessors for current heroes, AI, effects, input or animation continuations.
pub trait FrozenPublicationSource {
    fn scene(&self) -> &PublishedScene;
    fn world(&self) -> &WorldState;
    fn geometry_epoch(&self, level: LevelId) -> Result<u64>;
    fn resource_identity(&self) -> [u8; 32];
}

impl FrozenPublicationSource for tje_simulation::session::Session {
    fn scene(&self) -> &PublishedScene {
        &self.published
    }
    fn world(&self) -> &WorldState {
        &self.world
    }
    fn geometry_epoch(&self, level: LevelId) -> Result<u64> {
        self.world
            .floor(level)
            .map(|floor| floor.geometry_epoch())
            .ok_or(Error::Invalid(
                "frozen publication geometry is not retained",
            ))
    }
    fn resource_identity(&self) -> [u8; 32] {
        self.config().catalogue_identity
    }
}

/// Borrows pre-reserved authoritative rows. A failed phase is latched by the
/// owner; partially built rows are never exposed as a successful publication.
pub struct PublicationBuilder<'a> {
    scene: &'a mut PublishedScene,
    props: [Option<Prop>; 28],
    prop_count: usize,
}
impl<'a> PublicationBuilder<'a> {
    pub fn new(scene: &'a mut PublishedScene) -> Self {
        Self {
            scene,
            props: [None; 28],
            prop_count: 0,
        }
    }
    fn clear(&mut self) {
        self.scene.kind = PublishedKind::Authored;
        self.scene.stamp = None;
        self.scene.views = [PublishedView::Neutral; 4];
        self.scene.geometry.clear();
        self.scene.actors.clear();
        self.scene.patches.clear();
        self.props.fill(None);
        self.prop_count = 0;
        self.scene.overlays = std::array::from_fn(|_| PublishedOverlay::default());
    }
    pub fn freeze(
        &mut self,
        source: &impl PublicationSource,
        resources: &ResourceCatalog,
        stamp: PublicationStamp,
    ) -> Result<()> {
        if !stamp.phase.publishes()
            || source.clock().epoch() != stamp.epoch
            || source.clock().last_admission(stamp.phase) != Some(stamp.hardware_frame)
        {
            return Err(Error::Invalid(
                "logical scene freeze requires its admitted Publish boundary",
            ));
        }
        if source.config().catalogue_identity != resources.identity() {
            return Err(Error::Invalid("freeze resource catalogue identity differs"));
        }
        self.clear();
        self.scene.stamp = Some(stamp);
        if !source.world_visible() {
            return Ok(());
        }
        self.scene.kind = PublishedKind::World;
        for &level in source.world().active().iter().flatten() {
            if self.active(level) {
                continue;
            }
            if self.scene.geometry.len() == 4 {
                return Err(Error::Invalid("more than four active publication floors"));
            }
            if source.world().floor(level).is_none() {
                return Err(Error::Invalid("active publication geometry is absent"));
            }
            self.scene.geometry.push(GeometryReference {
                level,
                epoch: source.geometry_epoch(level)?,
            });
        }
        self.scene
            .geometry
            .sort_unstable_by_key(|reference| reference.level);
        for seat in SeatId::ALL {
            if !source.config().roles.local_owners.contains(seat) {
                continue;
            }
            let hero = &source.players().players[seat.index()];
            let focus = match hero.life {
                LifeState::Alive => Some(hero),
                LifeState::Eliminated => hero
                    .spectating
                    .map(|target| &source.players().players[target.index()])
                    .filter(|hero| hero.life == LifeState::Alive),
                LifeState::Disabled | LifeState::Unplaced => None,
            };
            if let Some(focus) = focus {
                let level = focus
                    .level
                    .ok_or(Error::Invalid("living camera focus has no level"))?;
                if !self.active(level) {
                    return Err(Error::Invalid(
                        "living camera focus is outside the active floor set",
                    ));
                }
                let floor = source
                    .world()
                    .floor(level)
                    .ok_or(Error::Invalid("camera focus geometry is absent"))?;
                let ((min_x, max_x), (min_y, max_y)) = floor
                    .generated
                    .geometry
                    .dimensions()
                    .camera_bounds(WIDTH as u16, HEIGHT as u16)?;
                let origin = [
                    (focus.motion.position.x >> 8)
                        .saturating_sub(WIDTH as i32 / 2)
                        .clamp(min_x, max_x),
                    (focus.motion.position.y >> 8)
                        .saturating_sub(HEIGHT as i32 / 2)
                        .clamp(min_y, max_y),
                ];
                self.scene.views[seat.index()] = PublishedView::World {
                    seat,
                    level,
                    origin,
                    attributes: source.terrain_attributes(seat),
                    starfield: level.get() != 26,
                };
                let maximum = hero
                    .character
                    .archetype
                    .maximum_health(hero.rank)
                    .map_err(|_| Error::Invalid("published hero rank"))?;
                self.scene.overlays[seat.index()].hud = Some(PublishedHud {
                    seat,
                    level,
                    health: u16::from(hero.health),
                    maximum_health: u16::from(maximum),
                    lives: hero.lives,
                    rank: hero.rank,
                    bucks: u16::from(hero.bucks),
                    pause_requested: hero.ui.pause_requested,
                });
            }
        }
        for hero in &source.players().players {
            if hero.life != LifeState::Alive {
                continue;
            }
            let level = hero
                .level
                .ok_or(Error::Invalid("living hero has no level"))?;
            if !self.active(level) {
                return Err(Error::Invalid(
                    "living hero is outside the active floor set",
                ));
            }
            let id = source
                .hero_actor(hero.seat)
                .ok_or(Error::Invalid("living hero has no reserved body"))?;
            let style = source.hero_style(hero.seat)?;
            if style.hidden {
                continue;
            }
            let animation = resources.hero(hero.character.archetype, hero.animation.descriptor)?;
            let (frame, flip) =
                resources.selected_frame(animation, hero.animation.sequence_index)?;
            let mut actor = frozen_actor(
                resources,
                DrawId::Actor(id),
                level,
                hero.motion.position,
                frame,
                flip,
                style,
            );
            actor.style.character = Some(hero.character);
            actor.style.seat = Some(hero.seat);
            self.push(actor)?;
        }
        for floor in source.population().floors() {
            if !self.active(floor.level) {
                continue;
            }
            for body in &floor.actors {
                if body.retired || body.pending_director {
                    continue;
                }
                if let Some(id) = body.body
                    && !body.hidden
                {
                    let animation = resources.earthling(body.kind, body.state)?;
                    let (frame, flip) =
                        resources.selected_frame(animation, body.animation.sequence_index)?;
                    if body.animation.started
                        && resources.frame_id(animation, body.animation_frame)? != frame
                    {
                        return Err(Error::Invalid(
                            "population resource frame disagrees with its saved cursor",
                        ));
                    }
                    self.push(frozen_actor(
                        resources,
                        DrawId::Actor(id),
                        floor.level,
                        body.motion.position,
                        frame,
                        flip,
                        VisualStyle {
                            draw_late: body.draw_late,
                            high_priority: true,
                            palette_bank: 0,
                            flip_vertical: false,
                            mask_pixels: body.masked,
                            shadow: body.shadow,
                            submerged: false,
                            hidden: false,
                        },
                    ))?;
                }
                if let Some(id) = body.companion_body {
                    let offset = resources.companion_offset(body.kind);
                    let position = Position {
                        x: body
                            .motion
                            .position
                            .x
                            .checked_add(i32::from(offset[0]) * 256)
                            .ok_or(Error::Invalid("companion X overflow"))?,
                        y: body
                            .motion
                            .position
                            .y
                            .checked_add(256)
                            .ok_or(Error::Invalid("companion Y overflow"))?,
                        elevation: i32::from(offset[1]) * 256,
                    };
                    self.dynamic(
                        resources,
                        DrawId::Actor(id),
                        floor.level,
                        position,
                        source.companion_visual(id)?,
                    )?;
                }
            }
        }
        let items = &source.services().resources;
        for item in items.items().iter().filter(|item| !item.collected) {
            if !self.active(item.level) {
                continue;
            }
            let appearance = source.item_appearance(item)?;
            if !matches!(
                appearance,
                ResourceKey::Present(_) | ResourceKey::GroundItem(_) | ResourceKey::LargeItem(_)
            ) {
                return Err(Error::Invalid("world item has no static item appearance"));
            }
            self.push(frozen_actor(
                resources,
                DrawId::Item {
                    level: item.level,
                    id: item.id,
                },
                item.level,
                item.position,
                resources.item_frame(appearance)?,
                false,
                item_style(matches!(appearance, ResourceKey::GroundItem(_))),
            ))?;
        }
        for (id, drop) in items.drops() {
            if self.active(drop.level) {
                self.push(frozen_actor(
                    resources,
                    DrawId::Actor(id.actor_id()),
                    drop.level,
                    drop.position,
                    resources.item_frame(ResourceKey::Present(drop.present.get()))?,
                    false,
                    item_style(false),
                ))?;
            }
        }
        for (id, effect) in items.effects() {
            if self.active(effect.level) {
                let before = self.scene.actors.len();
                self.dynamic(
                    resources,
                    DrawId::Actor(
                        id.actor()
                            .ok_or(Error::Invalid("effect generation is not drawable"))?,
                    ),
                    effect.level,
                    effect.position,
                    source.effect_visual(id)?,
                )?;
                if effect.kind == tje_simulation::resources::EffectKind::Decoy
                    && self.scene.actors.len() != before
                {
                    let owner = match effect.owner {
                        tje_simulation::resources::EffectOwner::Seat(owner) => owner,
                        _ => {
                            return Err(Error::Invalid("decoy instance has no hero palette owner"));
                        }
                    };
                    let actor = self.scene.actors.last_mut().expect("published decoy");
                    actor.style.character = Some(source.players().players[owner.index()].character);
                    actor.style.seat = Some(owner);
                }
            }
        }
        for prop in source.props() {
            if !self.active(prop.level) {
                continue;
            }
            if self.prop_count == 28 {
                return Err(Error::Invalid("transition prop reservation exceeded"));
            }
            self.dynamic(
                resources,
                DrawId::Prop {
                    level: prop.level,
                    id: prop.id,
                },
                prop.level,
                prop.position,
                DynamicVisual::Animated(prop.playback),
            )?;
            self.props[self.prop_count] = Some(prop);
            self.prop_count += 1;
        }
        for patch in source.patches() {
            if !self.active(patch.level) {
                return Err(Error::Invalid("falling patch has no active level"));
            }
            if self.scene.patches.len() == MAX_PATCHES {
                return Err(Error::Invalid("falling patch reservation exceeded"));
            }
            self.scene.patches.push(PublishedPatch {
                level: patch.level,
                minimum: patch.minimum,
                maximum: patch.maximum,
            });
        }
        for seat in SeatId::ALL {
            if !source.config().roles.local_owners.contains(seat) {
                continue;
            }
            let hero = &source.players().players[seat.index()];
            if let PublishedView::World { level, .. } = self.scene.views[seat.index()] {
                match hero.ui.overlay {
                    Overlay::Closed => {}
                    Overlay::Map => {
                        self.scene.overlays[seat.index()].map = Some(self.map(source, level)?)
                    }
                    Overlay::Inventory | Overlay::Identify | Overlay::MailOrder => {
                        let kind = match hero.ui.overlay {
                            Overlay::Inventory => PublishedInventoryKind::Inventory,
                            Overlay::Identify => PublishedInventoryKind::Identify,
                            _ => PublishedInventoryKind::MailOrder,
                        };
                        let mut inventory = PublishedInventory {
                            kind,
                            selected: hero.ui.selected_slot,
                            slots: [None; 16],
                        };
                        if hero.ui.overlay == Overlay::MailOrder {
                            let runtime = &source.services().runtime[seat.index()];
                            let service = runtime
                                .ui_service
                                .ok_or(Error::Invalid("mail overlay has no identified service"))?;
                            for (count, offer) in items
                                .offers()
                                .iter()
                                .filter(|offer| offer.service == service)
                                .enumerate()
                            {
                                if count == 16 {
                                    return Err(Error::Invalid(
                                        "mail offers exceed the bounded overlay",
                                    ));
                                }
                                inventory.slots[count] = Some(frozen_item(
                                    resources,
                                    &items.presents,
                                    offer.present,
                                    Some(offer.price),
                                    offer.stock.available(),
                                )?);
                                if runtime.ui_offer == Some(offer.id) {
                                    inventory.selected = count as u8;
                                }
                            }
                        } else {
                            for (index, wrapper) in hero.inventory.iter().enumerate() {
                                if let Some(wrapper) = wrapper {
                                    inventory.slots[index] = Some(frozen_item(
                                        resources,
                                        &items.presents,
                                        *wrapper,
                                        None,
                                        true,
                                    )?);
                                }
                            }
                        }
                        self.scene.overlays[seat.index()].inventory = Some(inventory);
                    }
                }
            }
            if let Some(speech) = source.speech(seat)? {
                let checked = Speech::new(speech.text, speech.choices, speech.accept_selected)?;
                self.scene.overlays[seat.index()].speech = PublishedSpeech {
                    text: checked.text,
                    length: checked.length,
                    choices: checked.choices,
                    accept_selected: checked.accept_selected,
                };
            }
        }
        self.scene.actors.sort_unstable_by_key(|actor| actor.id);
        self.scene.validate(resources.frame_count())?;
        Ok(())
    }
    fn active(&self, level: LevelId) -> bool {
        self.scene
            .geometry
            .iter()
            .any(|reference| reference.level == level)
    }
    fn push(&mut self, actor: PublishedActor) -> Result<()> {
        if self.scene.actors.len() == MAX_ACTORS {
            return Err(Error::Invalid(
                "complete logical scene exceeds 1024 instances",
            ));
        }
        self.scene.actors.push(actor);
        Ok(())
    }
    fn dynamic(
        &mut self,
        resources: &ResourceCatalog,
        id: DrawId,
        level: LevelId,
        position: Position,
        visual: DynamicVisual,
    ) -> Result<()> {
        let (frame, flip, style) = match visual {
            DynamicVisual::Hidden => return Ok(()),
            DynamicVisual::Static { frame, style } => (frame, false, style),
            DynamicVisual::Animated(playback) => {
                let (frame, flip) =
                    resources.selected_frame(playback.animation, playback.sequence_index)?;
                (frame, flip, playback.style)
            }
        };
        if !style.hidden {
            self.push(frozen_actor(
                resources, id, level, position, frame, flip, style,
            ))?;
        }
        Ok(())
    }
    fn map(&self, source: &impl PublicationSource, level: LevelId) -> Result<PublishedMap> {
        let floor = source
            .world()
            .floor(level)
            .ok_or(Error::Invalid("map overlay geometry is absent"))?;
        let dimensions = floor.generated.geometry.dimensions();
        let fog = source.discovery(level)?;
        let mut map = PublishedMap {
            level,
            dimensions: [dimensions.width(), dimensions.height()],
            discovery: [0; 9],
            discovery_columns: fog.columns(),
            discovery_rows: fog.rows(),
            markers: [None; 68],
        };
        let bits = usize::from(fog.columns()) * usize::from(fog.rows());
        if bits > 576 {
            return Err(Error::Invalid("discovery exceeds frozen map capacity"));
        }
        for index in 0..bits {
            if fog.revealed(index as u16) {
                map.discovery[index / 64] |= 1 << (index % 64);
            }
        }
        let mut count = 0;
        for hero in &source.players().players {
            if hero.life == LifeState::Alive && hero.level == Some(level) {
                map.markers[count] = Some(PublishedMarker {
                    position: [hero.motion.position.x >> 8, hero.motion.position.y >> 8],
                    seat: Some(hero.seat),
                });
                count += 1;
            }
        }
        for prop in self
            .props
            .iter()
            .flatten()
            .filter(|prop| prop.level == level && prop.map_marker)
        {
            if count == map.markers.len() {
                return Err(Error::Invalid("map marker reservation exceeded"));
            }
            map.markers[count] = Some(PublishedMarker {
                position: [prop.position.x >> 8, prop.position.y >> 8],
                seat: None,
            });
            count += 1;
        }
        Ok(map)
    }
}
fn frozen_actor(
    resources: &ResourceCatalog,
    id: DrawId,
    level: LevelId,
    position: Position,
    frame: tje_simulation::FrameId,
    flip_horizontal: bool,
    style: VisualStyle,
) -> PublishedActor {
    let elevation = position.elevation >> 8;
    let shadow = style.shadow.then(|| {
        let shadow = resources.shadow(elevation, false);
        PublishedShadow::Vram {
            tile: shadow.tile,
            size: shadow.size,
            offset: shadow.offset,
        }
    });
    PublishedActor {
        id,
        level,
        position: [position.x >> 8, position.y >> 8],
        elevation,
        frame,
        style: PublishedStyle {
            draw_late: style.draw_late,
            flip_horizontal,
            flip_vertical: style.flip_vertical,
            high_priority: style.high_priority,
            palette_bank: style.palette_bank,
            character: None,
            seat: None,
            mask_pixels: style.mask_pixels,
            submerged: style.submerged,
            shadow,
        },
    }
}
fn item_style(draw_late: bool) -> VisualStyle {
    VisualStyle {
        draw_late,
        high_priority: true,
        palette_bank: 0,
        flip_vertical: false,
        mask_pixels: false,
        shadow: false,
        submerged: false,
        hidden: false,
    }
}
fn frozen_item(
    resources: &ResourceCatalog,
    presents: &PresentMap,
    wrapper: tje_four_player::state::PresentWrapper,
    price: Option<u8>,
    available: bool,
) -> Result<PublishedItem> {
    Ok(PublishedItem {
        wrapper: wrapper.get(),
        identified: presents.identified(wrapper),
        icon: resources.item_frame(ResourceKey::Present(wrapper.get()))?,
        price,
        available,
    })
}

pub struct PublicationRuntime {
    publisher: Publisher,
    compositor: Compositor,
    actors: Vec<tje_mode_view::Actor>,
    overlays: [SeatOverlay; 4],
    patches: Vec<PriorityPatch>,
    materialized: Option<PublicationStamp>,
    composed: Option<PublicationStamp>,
    palette: [u16; 64],
}
impl Default for PublicationRuntime {
    fn default() -> Self {
        Self::new()
    }
}
impl PublicationRuntime {
    pub fn new() -> Self {
        Self {
            publisher: Publisher::new(),
            compositor: Compositor::new(),
            actors: Vec::with_capacity(MAX_ACTORS),
            overlays: std::array::from_fn(|_| SeatOverlay::default()),
            patches: Vec::with_capacity(MAX_PATCHES),
            materialized: None,
            composed: None,
            palette: [0; 64],
        }
    }
    pub fn frame(&self) -> (&[Color], FrameSize) {
        self.compositor.frame()
    }
    pub fn stamp(&self) -> Option<PublicationStamp> {
        self.composed
    }
    pub fn invalidate(&mut self) {
        self.publisher.invalidate();
        self.materialized = None;
        self.composed = None;
        self.compositor.evict_resources();
    }
    pub fn evict_resources(&mut self) {
        self.compositor.evict_resources();
    }
    #[cfg(feature = "enhanced-mode")]
    pub fn refresh<S: tje_simulation::session::SessionStorage>(
        &mut self,
        mode: &tje_engine_runtime::enhanced::EnhancedMode,
        memory: &AddressSpace,
        vram: &[u8],
        cram: &[u16],
        original: &[Color],
    ) -> Result<()> {
        self.refresh_source(
            mode.sidecars().get::<S>()?.session(),
            mode.sidecars()
                .resource::<resources::SessionCatalogue<S>>()?
                .catalogue(),
            memory,
            vram,
            cram,
            original,
        )
    }
    pub fn refresh_source(
        &mut self,
        source: &impl FrozenPublicationSource,
        resources: &ResourceCatalog,
        memory: &AddressSpace,
        vram: &[u8],
        cram: &[u16],
        original: &[Color],
    ) -> Result<()> {
        let scene = source.scene();
        if scene.kind == PublishedKind::Authored {
            self.compositor.compose_authored(original)?;
            self.composed = None;
            return Ok(());
        }
        scene.validate(resources.frame_count())?;
        if source.resource_identity() != resources.identity() {
            return Err(Error::Invalid("frozen scene resource identity differs"));
        }
        for reference in &scene.geometry {
            if source.geometry_epoch(reference.level)? != reference.epoch {
                return Err(Error::Invalid(
                    "frozen geometry revision is no longer available",
                ));
            }
        }
        if cram.len() != 64 {
            return Err(Error::Invalid("world palette is not 64 CRAM words"));
        }
        resources.validate_static_patterns(vram)?;
        let stamp = scene
            .stamp
            .ok_or(Error::Invalid("world scene has no saved publication stamp"))?;
        if self.composed == Some(stamp) && self.palette.as_slice() == cram {
            return Ok(());
        }
        if self.materialized != Some(stamp) {
            self.materialize(scene, resources)?;
            self.materialized = Some(stamp);
        }
        let first = source
            .world()
            .floors()
            .first()
            .ok_or(Error::Invalid("frozen world has no retained geometry"))?;
        let mut bindings = [(first.spec.level(), &first.generated.geometry); 4];
        for (index, reference) in scene.geometry.iter().enumerate() {
            let floor = source
                .world()
                .floor(reference.level)
                .ok_or(Error::Invalid("frozen geometry was released"))?;
            bindings[index] = (reference.level, &floor.generated.geometry);
        }
        let world = WorldSource::new(stamp, memory, vram, cram, &bindings[..scene.geometry.len()])?;
        self.compositor.compose(self.publisher.current(), &world)?;
        self.palette.copy_from_slice(cram);
        self.composed = Some(stamp);
        Ok(())
    }
    fn materialize(&mut self, scene: &PublishedScene, resources: &ResourceCatalog) -> Result<()> {
        self.actors.clear();
        self.patches.clear();
        self.overlays = std::array::from_fn(|_| SeatOverlay::default());
        for actor in &scene.actors {
            self.actors.push(materialize_actor(actor, resources)?);
        }
        let views = scene.views.map(|view| match view {
            PublishedView::Neutral => tje_mode_view::View::Neutral,
            PublishedView::World {
                seat,
                level,
                origin,
                attributes,
                starfield,
            } => tje_mode_view::View::World {
                seat,
                level,
                origin,
                attributes,
                starfield,
            },
        });
        for (index, source) in scene.overlays.iter().enumerate() {
            let target = &mut self.overlays[index];
            target.hud = source.hud.map(|hud| Hud {
                seat: hud.seat,
                level: hud.level,
                health: hud.health,
                maximum_health: hud.maximum_health,
                lives: hud.lives,
                rank: hud.rank,
                bucks: hud.bucks,
                pause_requested: hud.pause_requested,
            });
            if let Some(inventory) = &source.inventory {
                let kind = match inventory.kind {
                    PublishedInventoryKind::Inventory => InventoryKind::Inventory,
                    PublishedInventoryKind::MailOrder => InventoryKind::MailOrder,
                    PublishedInventoryKind::Identify => InventoryKind::Identify,
                };
                let mut slots = [None; 16];
                for (slot, item) in slots.iter_mut().zip(&inventory.slots) {
                    if let Some(item) = item {
                        let frame = resources.frame(item.icon)?;
                        *slot = Some(Item {
                            wrapper: item.wrapper,
                            identified: item.identified,
                            icon: Some(Metasprite {
                                count: frame.count,
                                pieces: frame.pieces,
                            }),
                            price: item.price,
                            available: item.available,
                        });
                    }
                }
                target.inventory = Some(Inventory {
                    kind,
                    selected: inventory.selected,
                    slots,
                });
            }
            target.map = source.map.as_ref().map(|map| Map {
                level: map.level,
                dimensions: map.dimensions,
                discovery: map.discovery,
                discovery_columns: map.discovery_columns,
                discovery_rows: map.discovery_rows,
                markers: map.markers.map(|marker| {
                    marker.map(|marker| Marker {
                        position: marker.position,
                        seat: marker.seat,
                    })
                }),
            });
            target.speech = Speech {
                text: source.speech.text,
                length: source.speech.length,
                choices: source.speech.choices,
                accept_selected: source.speech.accept_selected,
            };
        }
        for patch in &scene.patches {
            self.patches.push(PriorityPatch {
                level: patch.level,
                minimum: patch.minimum,
                maximum: patch.maximum,
            });
        }
        self.publisher.publish(
            scene
                .stamp
                .ok_or(Error::Invalid("uncommitted world materialization"))?,
            views,
            &self.actors,
            &self.overlays,
            &self.patches,
        )?;
        Ok(())
    }
}
fn materialize_actor(
    actor: &PublishedActor,
    resources: &ResourceCatalog,
) -> Result<tje_mode_view::Actor> {
    let frame = resources.frame(actor.frame)?;
    let shadow = match actor.style.shadow {
        None => None,
        Some(PublishedShadow::Frame(id)) => {
            let frame = resources.frame(id)?;
            Some(Shadow::Rom(Metasprite {
                count: frame.count,
                pieces: frame.pieces,
            }))
        }
        Some(PublishedShadow::Vram { tile, size, offset }) => {
            Some(Shadow::Vram(VramShadow { tile, size, offset }))
        }
    };
    let result = tje_mode_view::Actor {
        id: actor.id,
        level: actor.level,
        position: actor.position,
        elevation: actor.elevation,
        draw_late: actor.style.draw_late,
        flip_horizontal: actor.style.flip_horizontal,
        flip_vertical: actor.style.flip_vertical,
        high_priority: actor.style.high_priority,
        palette_bank: actor.style.palette_bank,
        character: actor.style.character,
        seat: actor.style.seat,
        mask_pixels: actor.style.mask_pixels,
        submerged: actor.style.submerged,
        count: frame.count,
        pieces: frame.pieces,
        shadow,
    };
    result.validate()?;
    Ok(result)
}

#[cfg(test)]
mod tests;
