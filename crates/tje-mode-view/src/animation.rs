//! G/src/sprites/entity_sprite_rendering.c:443-487,503-573 generalized without
//! viewport bits, tile residency, upload budgets or graphics waits.
use crate::{Error, MAX_PIECES, Result};
use bincode::{Decode, Encode};
use tje_extended_view::{
    presentation::Piece,
    resources::{MAX_PIECE_BYTES, decode_piece_into, frame_pieces, piece_length},
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Encode, Decode)]
pub struct Cursor {
    pub sequence_index: u16,
    pub ticks_left: u16,
    pub held: bool,
    pub flip_horizontal: bool,
    pub started: bool,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrameMotion {
    pub x: i8,
    pub y: i8,
    pub reuse_tiles: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub address: u32,
    pub motion: FrameMotion,
    pub count: u8,
    pub pieces: [Piece; MAX_PIECES],
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Entry {
    Frame { index: u8, flip: bool },
    Loop,
    Hold,
}
/// Immutable, runtime-only imported resources; pointers never enter a snapshot.
pub struct Program {
    sequence: Vec<Entry>,
    frames: Vec<Option<Frame>>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    pub frame: u16,
    pub motion: Option<FrameMotion>,
    pub held: bool,
    pub looped: bool,
}
impl Program {
    pub fn import(rom: &[u8], sequence_address: u32, frames_address: u32) -> Result<Self> {
        let mut sequence = Vec::with_capacity(256);
        let mut frames = vec![None; 128];
        let mut scratch = [0; MAX_PIECE_BYTES];
        let sequence_bytes = rom
            .get(sequence_address as usize..)
            .ok_or(Error::Invalid("truncated animation sequence"))?;
        let frame_table = rom
            .get(frames_address as usize..)
            .ok_or(Error::Invalid("truncated frame pointer table"))?;
        for offset in 0..256 {
            let byte = *sequence_bytes
                .get(offset)
                .ok_or(Error::Invalid("truncated animation sequence"))?;
            let entry = match byte {
                254 => Entry::Hold,
                255 => Entry::Loop,
                _ => {
                    let index = usize::from(byte & 127);
                    if frames[index].is_none() {
                        let start = index * 4;
                        let address = u32::from_be_bytes(
                            frame_table
                                .get(start..start + 4)
                                .ok_or(Error::Invalid("truncated frame pointer table"))?
                                .try_into()
                                .unwrap(),
                        );
                        let header = rom
                            .get(address as usize..)
                            .and_then(|data| data.get(..4))
                            .ok_or(Error::Invalid("truncated animation frame"))?;
                        let mut pieces = [Piece::default(); MAX_PIECES];
                        let count = frame_pieces(rom, address, &mut pieces)?;
                        for piece in pieces.iter().take(count) {
                            decode_piece_into(rom, *piece, &mut scratch[..piece_length(*piece)?])?;
                        }
                        frames[index] = Some(Frame {
                            address,
                            motion: FrameMotion {
                                x: header[1] as i8,
                                y: header[2] as i8,
                                reuse_tiles: header[3] != 0,
                            },
                            count: count as u8,
                            pieces,
                        });
                    }
                    Entry::Frame {
                        index: index as u8,
                        flip: byte & 128 != 0,
                    }
                }
            };
            sequence.push(entry);
            if matches!(entry, Entry::Loop | Entry::Hold) {
                if sequence.len() == 1 {
                    return Err(Error::Invalid("animation must begin with a frame"));
                }
                return Ok(Self { sequence, frames });
            }
        }
        Err(Error::Invalid(
            "animation sequence lacks a bounded terminator",
        ))
    }
    pub fn frame(&self, index: u16) -> Result<&Frame> {
        self.frames
            .get(usize::from(index))
            .and_then(Option::as_ref)
            .ok_or(Error::Invalid("animation frame not imported"))
    }
    fn selection(&self, cursor: &Cursor) -> Result<(u8, bool)> {
        match self.sequence.get(usize::from(cursor.sequence_index)) {
            Some(Entry::Frame { index, flip }) => Ok((*index, *flip)),
            _ => Err(Error::Invalid("animation cursor does not name a frame")),
        }
    }
    /// Admit the first frame at a simulation-animation boundary, including its
    /// motion sample exactly once. A zero authored period holds that frame.
    /// Rendering an uninitialized actor cannot initialize its animation.
    pub fn start(&self, period: u16) -> Result<(Cursor, Step)> {
        let mut cursor = Cursor {
            ticks_left: period,
            held: period == 0,
            started: true,
            ..Cursor::default()
        };
        let (index, flip) = self.selection(&cursor)?;
        cursor.flip_horizontal = flip;
        let motion = self.frame(u16::from(index))?.motion;
        Ok((
            cursor,
            Step {
                frame: u16::from(index),
                motion: Some(motion),
                held: period == 0,
                looped: false,
            },
        ))
    }
    /// One admitted simulation-animation tick. The motion sample is emitted once
    /// at the same next-frame boundary the original motion pass uses; S3/S4 apply
    /// typed terrain/collision behavior before committing their actor position.
    pub fn step(&self, cursor: &mut Cursor, period: u16) -> Result<Step> {
        if !cursor.started || (period == 0 && !cursor.held) || cursor.ticks_left > period {
            return Err(Error::Invalid("invalid animation timer/admission"));
        }
        let (old, flip) = self.selection(cursor)?;
        if cursor.flip_horizontal != flip || (cursor.held != (cursor.ticks_left == 0)) {
            return Err(Error::Invalid("inconsistent animation cursor"));
        }
        if cursor.held {
            return Ok(Step {
                frame: u16::from(old),
                motion: None,
                held: true,
                looped: false,
            });
        }
        if cursor.ticks_left > 1 {
            cursor.ticks_left -= 1;
            return Ok(Step {
                frame: u16::from(old),
                motion: None,
                held: false,
                looped: false,
            });
        }
        let next = usize::from(cursor.sequence_index) + 1;
        let mut candidate = *cursor;
        let looped = match self.sequence.get(next) {
            Some(Entry::Hold) => {
                candidate.ticks_left = 0;
                candidate.held = true;
                *cursor = candidate;
                return Ok(Step {
                    frame: u16::from(old),
                    motion: None,
                    held: true,
                    looped: false,
                });
            }
            Some(Entry::Loop) => {
                candidate.sequence_index = 0;
                true
            }
            Some(Entry::Frame { .. }) => {
                candidate.sequence_index += 1;
                false
            }
            None => return Err(Error::Invalid("animation cursor exceeds sequence")),
        };
        let (index, flip) = self.selection(&candidate)?;
        let motion = self.frame(u16::from(index))?.motion;
        candidate.ticks_left = period;
        candidate.flip_horizontal = flip;
        *cursor = candidate;
        Ok(Step {
            frame: u16::from(index),
            motion: Some(motion),
            held: false,
            looped,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rom(terminator: u8) -> Vec<u8> {
        let mut bytes = vec![0; 200];
        bytes[1..4].copy_from_slice(&[0, 129, terminator]);
        bytes[8..12].copy_from_slice(&32_u32.to_be_bytes());
        bytes[12..16].copy_from_slice(&48_u32.to_be_bytes());
        for offset in [32, 48] {
            bytes[offset..offset + 14]
                .copy_from_slice(&[1, 3, 254, 1, 1, 1, 0, 0, 0, 64, 0, 0, 0, 128]);
        }
        bytes[128..160].fill(0x12);
        bytes
    }
    #[test]
    fn loop_motion_and_hold_advance_without_any_presenter() {
        for command in [254, 255] {
            let program = Program::import(&rom(command), 1, 8).unwrap();
            let (mut cursor, _) = program.start(2).unwrap();
            assert!(program.step(&mut cursor, 2).unwrap().motion.is_none());
            let step = program.step(&mut cursor, 2).unwrap();
            assert_eq!(
                step.motion,
                Some(FrameMotion {
                    x: 3,
                    y: -2,
                    reuse_tiles: true
                })
            );
            assert!(cursor.flip_horizontal);
            program.step(&mut cursor, 2).unwrap();
            let step = program.step(&mut cursor, 2).unwrap();
            assert_eq!(step.held, command == 254);
            assert_eq!(step.looped, command == 255);
            assert_eq!(step.motion.is_some(), command == 255);
            let before = cursor;
            if cursor.held {
                assert!(program.step(&mut cursor, 0).unwrap().held);
            } else {
                assert!(program.step(&mut cursor, 0).is_err());
            }
            assert_eq!(cursor, before);
        }
    }
    #[test]
    fn zero_period_first_frame_holds_and_unstarted_cursor_never_advances() {
        let program = Program::import(&rom(255), 1, 8).unwrap();
        let mut unstarted = Cursor::default();
        assert!(program.step(&mut unstarted, 1).is_err());
        assert_eq!(unstarted, Cursor::default());
        let (mut cursor, first) = program.start(0).unwrap();
        assert!(first.motion.is_some());
        assert!(first.held && cursor.started);
        for _ in 0..20 {
            let step = program.step(&mut cursor, 0).unwrap();
            assert!(step.held && step.motion.is_none());
        }
    }
    #[test]
    fn malformed_resources_reject_before_an_actor_is_created() {
        let mut bytes = rom(255);
        bytes[1] = 255;
        assert!(Program::import(&bytes, 1, 8).is_err());
        bytes[1] = 0;
        bytes[32] = 13;
        assert!(Program::import(&bytes, 1, 8).is_err());
        assert!(Program::import(&[], 1, 8).is_err());
    }
}
