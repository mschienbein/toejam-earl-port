use crate::{Error, MAX_ACTORS, MAX_PATCHES, MAX_PIECES, Result, overlay::SeatOverlay};
use tje_extended_view::{presentation::Piece, resources::piece_length};
use tje_four_player::{Character, SeatId};
pub use tje_simulation::DrawId;
use tje_simulation::PublicationStamp;
use tje_world::LevelId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    World {
        seat: SeatId,
        level: LevelId,
        origin: [i32; 2],
        attributes: u16,
        starfield: bool,
    },
    Neutral,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PriorityPatch {
    pub level: LevelId,
    /// Inclusive world tile rectangle; a fall patch is not keyed by a camera.
    pub minimum: [i32; 2],
    pub maximum: [i32; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Metasprite {
    pub count: u8,
    pub pieces: [Piece; MAX_PIECES],
}
impl Metasprite {
    pub fn validate(&self) -> Result<()> {
        if self.count == 0 || usize::from(self.count) > MAX_PIECES {
            return Err(Error::Invalid("invalid metasprite piece count"));
        }
        for piece in self.pieces.iter().take(usize::from(self.count)) {
            piece_length(*piece)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VramShadow {
    pub tile: u16,
    pub size: [u8; 2],
    pub offset: [i8; 2],
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shadow {
    Rom(Metasprite),
    Vram(VramShadow),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Actor {
    pub id: DrawId,
    pub level: LevelId,
    pub position: [i32; 2],
    pub elevation: i32,
    pub draw_late: bool,
    pub flip_horizontal: bool,
    pub flip_vertical: bool,
    pub high_priority: bool,
    pub palette_bank: u8,
    pub character: Option<Character>,
    pub seat: Option<SeatId>,
    pub mask_pixels: bool,
    pub submerged: bool,
    pub count: u8,
    pub pieces: [Piece; MAX_PIECES],
    /// Runtime resource descriptor, explicitly behind every world body.
    pub shadow: Option<Shadow>,
}
impl Actor {
    pub fn validate(&self) -> Result<()> {
        if !self.id.valid_for(self.level)
            || self.level.validate().is_err()
            || self.palette_bank > 3
            || self.count == 0
            || usize::from(self.count) > MAX_PIECES
        {
            return Err(Error::Invalid(
                "invalid published actor identity/piece count",
            ));
        }
        if self.seat.is_some_and(|seat| seat.validate().is_err()) {
            return Err(Error::Invalid("invalid actor seat indicator"));
        }
        if self
            .position
            .into_iter()
            .chain([self.elevation])
            .any(|value| value.unsigned_abs() > 32767)
        {
            return Err(Error::Invalid("published actor exceeds world envelope"));
        }
        for piece in self.pieces.iter().take(usize::from(self.count)) {
            piece_length(*piece)?;
        }
        match &self.shadow {
            Some(Shadow::Rom(frame)) => frame.validate()?,
            Some(Shadow::Vram(shadow))
                if shadow.size.iter().any(|axis| !(1..=4).contains(axis)) =>
            {
                return Err(Error::Invalid("invalid VRAM shadow extent"));
            }
            _ => {}
        }
        Ok(())
    }
}
/// All fields are private: a reader can never observe the staging buffer. ROM
/// descriptors are frozen, geometry is borrowed through a stamp-matched source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packet {
    stamp: Option<PublicationStamp>,
    views: [View; 4],
    actors: Vec<Actor>,
    overlays: [SeatOverlay; 4],
    patches: Vec<PriorityPatch>,
}
impl Packet {
    fn empty() -> Self {
        Self {
            stamp: None,
            views: [View::Neutral; 4],
            actors: Vec::with_capacity(MAX_ACTORS),
            overlays: std::array::from_fn(|_| SeatOverlay::default()),
            patches: Vec::with_capacity(MAX_PATCHES),
        }
    }
    pub fn stamp(&self) -> Option<PublicationStamp> {
        self.stamp
    }
    pub fn views(&self) -> &[View; 4] {
        &self.views
    }
    pub fn actors(&self) -> &[Actor] {
        &self.actors
    }
    pub fn overlays(&self) -> &[SeatOverlay; 4] {
        &self.overlays
    }
    pub fn patches(&self) -> &[PriorityPatch] {
        &self.patches
    }
}
pub struct Publisher {
    buffers: [Packet; 2],
    active: usize,
    identities: Vec<DrawId>,
}
impl Default for Publisher {
    fn default() -> Self {
        Self::new()
    }
}
impl Publisher {
    pub fn new() -> Self {
        Self {
            buffers: [Packet::empty(), Packet::empty()],
            active: 0,
            identities: Vec::with_capacity(MAX_ACTORS),
        }
    }
    pub fn current(&self) -> &Packet {
        &self.buffers[self.active]
    }
    /// Restore invalidates derivative buffers; the aggregate republishes the
    /// restored state without admitting another world phase or audio event.
    pub fn invalidate(&mut self) {
        for buffer in &mut self.buffers {
            buffer.stamp = None;
        }
    }
    pub fn publish(
        &mut self,
        stamp: PublicationStamp,
        views: [View; 4],
        actors: &[Actor],
        overlays: &[SeatOverlay; 4],
        patches: &[PriorityPatch],
    ) -> Result<()> {
        if !stamp.phase.publishes()
            || stamp.epoch == 0
            || actors.len() > MAX_ACTORS
            || patches.len() > MAX_PATCHES
        {
            return Err(Error::Invalid("invalid phase/publication capacity"));
        }
        if self.current().stamp.is_some_and(|previous| {
            stamp.epoch <= previous.epoch || stamp.hardware_frame < previous.hardware_frame
        }) {
            return Err(Error::Invalid("publication is repeated or out of order"));
        }
        let mut occupied = [false; MAX_ACTORS];
        self.identities.clear();
        for actor in actors {
            actor.validate()?;
            if let DrawId::Actor(id) = actor.id
                && std::mem::replace(&mut occupied[id.index()], true)
            {
                return Err(Error::Invalid(
                    "duplicate actor slot or stale generation in publication",
                ));
            }
            self.identities.push(actor.id);
        }
        self.identities.sort_unstable();
        if self.identities.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(Error::Invalid("duplicate persistent draw identity"));
        }
        for (index, view) in views.iter().enumerate() {
            if let View::World {
                seat,
                level,
                origin,
                ..
            } = view
                && (seat.index() != index
                    || level.validate().is_err()
                    || origin.iter().any(|value| value.unsigned_abs() > 32767))
            {
                return Err(Error::Invalid("invalid seat view/level/origin"));
            }
            overlays[index].validate()?;
            if overlays[index]
                .hud
                .is_some_and(|hud| hud.seat.index() != index)
            {
                return Err(Error::Invalid("HUD belongs to another input seat"));
            }
            if let (View::World { level, .. }, Some(map)) = (view, &overlays[index].map)
                && *level != map.level
            {
                return Err(Error::Invalid("map overlay belongs to another view level"));
            }
        }
        for patch in patches {
            if patch.level.validate().is_err()
                || patch.minimum[0] > patch.maximum[0]
                || patch.minimum[1] > patch.maximum[1]
                || i64::from(patch.maximum[0]) - i64::from(patch.minimum[0]) > 16
                || i64::from(patch.maximum[1]) - i64::from(patch.minimum[1]) > 16
            {
                return Err(Error::Invalid("invalid bounded falling priority patch"));
            }
        }
        let pending = &mut self.buffers[self.active ^ 1];
        pending.actors.clear();
        pending.actors.extend_from_slice(actors);
        pending.actors.sort_unstable_by_key(|actor| {
            (
                actor.draw_late,
                std::cmp::Reverse(actor.position[1]),
                actor.id,
            )
        });
        pending.patches.clear();
        pending.patches.extend_from_slice(patches);
        pending.views = views;
        pending.overlays.clone_from(overlays);
        pending.stamp = Some(stamp);
        self.active ^= 1;
        Ok(())
    }
}
