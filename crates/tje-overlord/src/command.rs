use crate::{
    ActorId, DIRECTOR, Error, Kind, LevelId, Result, SeatId, placement::Point, traps::TrapKind,
};
use bincode::{Decode, Encode};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Encode, Decode)]
pub enum Direction {
    #[default]
    Neutral,
    Up,
    Down,
    Left,
    Right,
}
impl Direction {
    pub const fn delta(self) -> (i32, i32) {
        match self {
            Self::Neutral => (0, 0),
            Self::Up => (0, -1),
            Self::Down => (0, 1),
            Self::Left => (-1, 0),
            Self::Right => (1, 0),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum Command {
    SelectActor(ActorId),
    Possess(ActorId),
    Release,
    Move(Direction),
    UseAbility,
    PlaceSpawn {
        kind: Kind,
        level: LevelId,
        point: Point,
    },
    PlaceTrap {
        kind: TrapKind,
        level: LevelId,
        point: Point,
    },
    CancelPlacement,
    Cursor {
        level: LevelId,
        point: Point,
    },
    CycleTool,
    ToggleView,
    Pause(bool),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct Envelope {
    pub seat: SeatId,
    pub frame: u64,
    pub sequence: u32,
    pub command: Command,
}
impl Envelope {
    pub fn validate(self) -> Result<()> {
        if self.seat != DIRECTOR {
            return Err(Error::Role);
        }
        if self.sequence == u32::MAX {
            return Err(Error::Order);
        }
        match self.command {
            Command::SelectActor(id) | Command::Possess(id) if !id.valid() => {
                Err(Error::StaleActor)
            }
            Command::PlaceSpawn { level, point, .. }
            | Command::PlaceTrap { level, point, .. }
            | Command::Cursor { level, point } => {
                level
                    .validate()
                    .map_err(|_| Error::Invalid("command LevelId"))?;
                point.validate()
            }
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Encode, Decode)]
pub struct PadLatch {
    pub previous: u8,
    pub next_sequence: u32,
}
#[derive(Debug, Clone, Copy)]
pub struct Controls {
    pub frame: u64,
    pub buttons: u8,
    pub connected: bool,
    pub selected: Option<ActorId>,
    pub placement: Option<Command>,
    pub possessed: bool,
    pub body_view: bool,
    pub pause_requested: bool,
}
impl PadLatch {
    /// Gameplay-order directions/B/C/A/Start: A confirm/ability, B cancel/
    /// release, C tool/tactical view. Cycling past the last tactical tool while
    /// possessing returns to the body view; every operation uses three buttons.
    /// Every edge is finite data, including cooperative pause. A held direction
    /// supplies intent each admitted input frame; disconnected input is neutral.
    pub fn commands(
        &mut self,
        controls: Controls,
        out: &mut [Option<Envelope>; 5],
    ) -> Result<usize> {
        let Controls {
            frame,
            buttons,
            connected,
            selected,
            placement,
            possessed,
            body_view,
            pause_requested,
        } = controls;
        let buttons = if connected { buttons } else { 0 };
        let pressed = buttons & !self.previous;
        let direction = if buttons & 1 != 0 {
            Direction::Up
        } else if buttons & 2 != 0 {
            Direction::Down
        } else if buttons & 4 != 0 {
            Direction::Left
        } else if buttons & 8 != 0 {
            Direction::Right
        } else {
            Direction::Neutral
        };
        let confirm = if possessed && body_view {
            Some(Command::UseAbility)
        } else {
            placement.or_else(|| selected.map(Command::Possess))
        };
        let commands = [
            Some(Command::Move(direction)),
            (pressed & 0x10 != 0).then_some(if possessed {
                Command::Release
            } else {
                Command::CancelPlacement
            }),
            (pressed & 0x20 != 0).then_some(if possessed && body_view {
                Command::ToggleView
            } else {
                Command::CycleTool
            }),
            (pressed & 0x40 != 0).then_some(confirm).flatten(),
            (pressed & 0x80 != 0).then_some(Command::Pause(!pause_requested)),
        ];
        let count = commands.iter().flatten().count();
        let next = self
            .next_sequence
            .checked_add(count as u32)
            .filter(|next| *next < u32::MAX)
            .ok_or(Error::Order)?;
        out.fill(None);
        for (index, command) in commands.into_iter().flatten().enumerate() {
            out[index] = Some(Envelope {
                seat: DIRECTOR,
                frame,
                sequence: self.next_sequence + index as u32,
                command,
            });
        }
        self.next_sequence = next;
        self.previous = buttons;
        Ok(count)
    }
}
