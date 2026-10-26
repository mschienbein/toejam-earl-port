//! Snapshot-rebuildable logical publication. No pixels, decoded pieces or full
//! level copies are stored. Geometry is borrowed only at its frozen revision.
use crate::{Error, FrameId, LevelId, PublicationStamp, Result, SeatId, ids::DrawId};
use bincode::{Decode, Encode};
use tje_four_player::Character;

/// Disjoint read borrows for an atomic publication build. No registered state
/// may be mutated through this view while the reusable target rows are filled.
pub struct PublicationRefs<'a> {
    pub config: &'a crate::session::SessionConfig,
    pub world: &'a tje_world::world::WorldState,
    pub players: &'a tje_four_player::CoopState,
    pub services: &'a crate::players::PlayerServices,
    pub population: &'a crate::population::PopulationState,
    pub loading: &'a crate::loading::LoadingState,
    pub visuals: &'a crate::visual::VisualState,
    pub clock: &'a crate::PhaseClock,
    pub scene: crate::session::Scene,
    pub hero_actors: &'a [Option<crate::ActorId>; 4],
    pub speech: &'a [PublishedSpeech; 4],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum PublishedView {
    Neutral,
    World {
        seat: SeatId,
        level: LevelId,
        origin: [i32; 2],
        attributes: u16,
        starfield: bool,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct GeometryReference {
    pub level: LevelId,
    pub epoch: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum PublishedShadow {
    Frame(FrameId),
    Vram {
        tile: u16,
        size: [u8; 2],
        offset: [i8; 2],
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct PublishedStyle {
    pub draw_late: bool,
    pub flip_horizontal: bool,
    pub flip_vertical: bool,
    pub high_priority: bool,
    pub palette_bank: u8,
    pub character: Option<Character>,
    pub seat: Option<SeatId>,
    pub mask_pixels: bool,
    pub submerged: bool,
    pub shadow: Option<PublishedShadow>,
}
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct PublishedActor {
    pub id: DrawId,
    pub level: LevelId,
    pub position: [i32; 2],
    pub elevation: i32,
    pub frame: FrameId,
    pub style: PublishedStyle,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct PublishedHud {
    pub seat: SeatId,
    pub level: LevelId,
    pub health: u16,
    pub maximum_health: u16,
    pub lives: u8,
    pub rank: u8,
    pub bucks: u16,
    pub pause_requested: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum PublishedInventoryKind {
    Inventory,
    MailOrder,
    Identify,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct PublishedItem {
    pub wrapper: u8,
    pub identified: bool,
    pub icon: FrameId,
    pub price: Option<u8>,
    pub available: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct PublishedInventory {
    pub kind: PublishedInventoryKind,
    pub selected: u8,
    pub slots: [Option<PublishedItem>; 16],
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct PublishedMarker {
    pub position: [i32; 2],
    pub seat: Option<SeatId>,
}
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct PublishedMap {
    pub level: LevelId,
    pub dimensions: [u16; 2],
    pub discovery: [u64; 9],
    pub discovery_columns: u16,
    pub discovery_rows: u16,
    pub markers: [Option<PublishedMarker>; 68],
}
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct PublishedSpeech {
    pub text: [u8; 96],
    pub length: u8,
    pub choices: bool,
    pub accept_selected: bool,
}
impl Default for PublishedSpeech {
    fn default() -> Self {
        Self {
            text: [0; 96],
            length: 0,
            choices: false,
            accept_selected: true,
        }
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Encode, Decode)]
pub struct PublishedOverlay {
    pub hud: Option<PublishedHud>,
    pub inventory: Option<PublishedInventory>,
    pub map: Option<PublishedMap>,
    pub speech: PublishedSpeech,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct PublishedPatch {
    pub level: LevelId,
    pub minimum: [i32; 2],
    pub maximum: [i32; 2],
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum PublishedKind {
    Authored,
    World,
}
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct PublishedScene {
    pub kind: PublishedKind,
    pub stamp: Option<PublicationStamp>,
    pub views: [PublishedView; 4],
    pub geometry: Vec<GeometryReference>,
    pub actors: Vec<PublishedActor>,
    pub overlays: [PublishedOverlay; 4],
    pub patches: Vec<PublishedPatch>,
}
impl Default for PublishedScene {
    fn default() -> Self {
        Self {
            kind: PublishedKind::Authored,
            stamp: None,
            views: [PublishedView::Neutral; 4],
            geometry: Vec::with_capacity(4),
            actors: Vec::with_capacity(1024),
            overlays: std::array::from_fn(|_| PublishedOverlay::default()),
            patches: Vec::with_capacity(4),
        }
    }
}
impl PublishedScene {
    pub fn validate(&self, frame_count: u32) -> Result<()> {
        if self.actors.len() > 1024
            || self.geometry.len() > 4
            || self.patches.len() > 4
            || self
                .geometry
                .windows(2)
                .any(|pair| pair[0].level >= pair[1].level)
            || self.actors.windows(2).any(|pair| pair[0].id >= pair[1].id)
        {
            return Err(Error::State("published scene bounds/order"));
        }
        if self.actors.windows(2).any(|pair| {
            matches!(
                (pair[0].id,pair[1].id),
                (DrawId::Actor(first),DrawId::Actor(second)) if first.slot()==second.slot()
            )
        }) {
            return Err(Error::State(
                "published actor slot has multiple generations",
            ));
        }
        let valid_frame = |id: FrameId| id.valid() && id.get() < frame_count;
        if self.stamp.is_none() {
            if self.kind != PublishedKind::Authored {
                return Err(Error::State("world publication has no admitted stamp"));
            }
            if !self.actors.is_empty()
                || !self.geometry.is_empty()
                || !self.patches.is_empty()
                || self
                    .views
                    .iter()
                    .any(|view| *view != PublishedView::Neutral)
                || self
                    .overlays
                    .iter()
                    .any(|overlay| *overlay != PublishedOverlay::default())
            {
                return Err(Error::State("uncommitted publication contains world data"));
            }
            return Ok(());
        }
        if self
            .stamp
            .is_some_and(|stamp| !stamp.phase.publishes() || stamp.epoch == 0)
        {
            return Err(Error::State("publication stamp is not a publication phase"));
        }
        for reference in &self.geometry {
            reference
                .level
                .validate()
                .map_err(|_| Error::State("published geometry level"))?;
        }
        let has_geometry = |level| {
            self.geometry
                .iter()
                .any(|reference| reference.level == level)
        };
        for (index, view) in self.views.into_iter().enumerate() {
            if let PublishedView::World {
                seat,
                level,
                origin,
                ..
            } = view
            {
                seat.validate()
                    .map_err(|_| Error::State("published view seat"))?;
                if seat.index() != index
                    || !has_geometry(level)
                    || origin.iter().any(|value| value.unsigned_abs() > 32767)
                {
                    return Err(Error::State("published view geometry/origin"));
                }
            }
        }
        for actor in &self.actors {
            if !actor.id.valid_for(actor.level)
                || !has_geometry(actor.level)
                || !valid_frame(actor.frame)
                || actor.style.palette_bank > 3
                || actor
                    .position
                    .iter()
                    .chain([&actor.elevation])
                    .any(|value| value.unsigned_abs() > 32767)
                || actor
                    .style
                    .seat
                    .is_some_and(|seat| seat.validate().is_err())
            {
                return Err(Error::State("published actor identity/resource/position"));
            }
            match actor.style.shadow {
                Some(PublishedShadow::Frame(frame)) if !valid_frame(frame) => {
                    return Err(Error::State("published shadow resource"));
                }
                Some(PublishedShadow::Vram { size, .. })
                    if size.iter().any(|axis| !(1..=4).contains(axis)) =>
                {
                    return Err(Error::State("published VRAM shadow size"));
                }
                _ => {}
            }
        }
        for (index, overlay) in self.overlays.iter().enumerate() {
            if overlay.hud.is_some_and(|hud| {
                hud.seat.validate().is_err()
                    || hud.seat.index() != index
                    || !has_geometry(hud.level)
                    || hud.maximum_health == 0
                    || hud.health > hud.maximum_health
                    || hud.rank > 8
            }) {
                return Err(Error::State("published HUD"));
            }
            if overlay.inventory.is_some() && overlay.map.is_some() {
                return Err(Error::State("inventory and map overlays cannot coexist"));
            }
            if let Some(inventory) = &overlay.inventory
                && (inventory.selected >= 16
                    || inventory
                        .slots
                        .iter()
                        .flatten()
                        .any(|item| item.wrapper >= 28 || !valid_frame(item.icon)))
            {
                return Err(Error::State("published inventory identity/resource"));
            }
            if let Some(map) = &overlay.map {
                if let PublishedView::World { level, .. } = self.views[index]
                    && map.level != level
                {
                    return Err(Error::State("published map belongs to another view level"));
                }
                if !has_geometry(map.level)
                    || map.dimensions.iter().any(|axis| !(8..=64).contains(axis))
                    || map.discovery_columns == 0
                    || map.discovery_rows == 0
                    || u32::from(map.discovery_columns) * u32::from(map.discovery_rows) > 576
                    || map.markers.iter().flatten().any(|marker| {
                        marker
                            .position
                            .iter()
                            .any(|value| value.unsigned_abs() > 32767)
                            || marker.seat.is_some_and(|seat| seat.validate().is_err())
                    })
                {
                    return Err(Error::State("published map/discovery"));
                }
            }
            let speech = &overlay.speech;
            if speech.length > 96
                || !speech.text[..usize::from(speech.length)]
                    .iter()
                    .all(|byte| byte.is_ascii_graphic() || *byte == b' ' || *byte == b'\n')
                || speech.text[usize::from(speech.length)..]
                    .iter()
                    .any(|byte| *byte != 0)
            {
                return Err(Error::State("published speech"));
            }
        }
        for patch in &self.patches {
            if !has_geometry(patch.level)
                || patch
                    .minimum
                    .iter()
                    .zip(patch.maximum)
                    .any(|(&minimum, maximum)| minimum > maximum)
                || i64::from(patch.maximum[0]) - i64::from(patch.minimum[0]) > 16
                || i64::from(patch.maximum[1]) - i64::from(patch.minimum[1]) > 16
            {
                return Err(Error::State("published terrain priority patch"));
            }
        }
        Ok(())
    }
    pub fn validate_geometry(&self, world: &tje_world::world::WorldState) -> Result<()> {
        for reference in &self.geometry {
            let floor = world
                .floor(reference.level)
                .ok_or(Error::State("published floor is absent"))?;
            if floor.geometry_epoch() != reference.epoch {
                return Err(Error::State(
                    "published geometry revision no longer matches frozen scene",
                ));
            }
        }
        for map in self
            .overlays
            .iter()
            .filter_map(|overlay| overlay.map.as_ref())
        {
            let floor = world
                .floor(map.level)
                .ok_or(Error::State("published map floor is absent"))?;
            let dimensions = floor.generated.geometry.dimensions();
            if map.dimensions != [dimensions.width(), dimensions.height()]
                || map.discovery_columns != floor.discovery.columns()
                || map.discovery_rows != floor.discovery.rows()
            {
                return Err(Error::State(
                    "published map bounds/stride differ from geometry revision",
                ));
            }
        }
        Ok(())
    }
}
