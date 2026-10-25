use super::{
    geometry::{OriginalGeneration, OriginalPhase},
    resources::{OriginalResources, ShapeBank},
};
use crate::{Error, Result};
use bincode::{Decode, Encode};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Encode, Decode)]
pub(super) struct ConnectionFrame {
    row: i16,
    column: i16,
    entered: bool,
    boundary: bool,
    next: u8,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Encode, Decode)]
pub(super) struct TraceFrame {
    row: i16,
    column: i16,
    incoming: u8,
    outgoing: u8,
    entered: bool,
    next: u8,
    pending: u8,
}
impl ConnectionFrame {
    pub(super) fn validate(&self) -> Result<()> {
        OriginalGeneration::index(self.row, self.column)?;
        if self.next > 4 {
            return Err(Error::State("OriginalV1 connectivity frame"));
        }
        Ok(())
    }
}
impl TraceFrame {
    pub(super) fn validate(&self) -> Result<()> {
        OriginalGeneration::index(self.row, self.column)?;
        let rules = match (self.incoming, self.outgoing) {
            (4, 8) | (4, 2) => 2,
            (8, 8) | (2, 2) | (8, 4) | (2, 4) => 3,
            _ => return Err(Error::State("OriginalV1 transition frame direction")),
        };
        if self.next > rules || self.pending > 16 {
            return Err(Error::State("OriginalV1 transition frame rule"));
        }
        Ok(())
    }
}
impl OriginalGeneration {
    pub(super) fn corners(&self, row: i16, column: i16) -> Result<u8> {
        let mut mask = 0;
        for (enabled, neighbors, bit) in [
            (
                row > 0 && column > 0,
                [(row, column - 1), (row - 1, column - 1), (row - 1, column)],
                1,
            ),
            (
                row > 0 && column < 18,
                [(row - 1, column), (row - 1, column + 1), (row, column + 1)],
                2,
            ),
            (
                row < 25 && column < 18,
                [(row, column + 1), (row + 1, column + 1), (row + 1, column)],
                4,
            ),
            (
                row < 25 && column > 0,
                [(row + 1, column), (row + 1, column - 1), (row, column - 1)],
                8,
            ),
        ] {
            if enabled {
                let mut all = true;
                for (y, x) in neighbors {
                    if self.get(y, x)? & 0x800 == 0 {
                        all = false;
                        break;
                    }
                }
                if all {
                    mask |= bit;
                }
            }
        }
        Ok(mask)
    }
    pub(super) fn start_connection(&mut self) {
        let anchor = self.anchors[0];
        self.connection[0] = ConnectionFrame {
            row: anchor.y as i16,
            column: anchor.x as i16,
            ..ConnectionFrame::default()
        };
        self.connection_depth = 1;
    }
    fn pop_connection(&mut self) -> Option<bool> {
        self.connection_depth -= 1;
        if self.connection_depth == 0 {
            Some(false)
        } else {
            None
        }
    }
    pub(super) fn connection_step(&mut self) -> Result<Option<bool>> {
        if self.connection_depth == 0 {
            return Ok(Some(false));
        }
        let index = usize::from(self.connection_depth - 1);
        let mut frame = self.connection[index];
        if !frame.entered {
            let word = self.get(frame.row, frame.column)?;
            if word & 0x800 != 0 {
                return Ok(self.pop_connection());
            }
            self.set(frame.row, frame.column, word | 0x800)?;
            if self.anchors[1].x as i16 == frame.column && self.anchors[1].y as i16 == frame.row {
                self.connection_depth = 0;
                return Ok(Some(true));
            }
            frame.boundary = word & 0x3000 == 0x1000;
            if !frame.boundary && word & 0x3000 != 0x2000 {
                return Ok(self.pop_connection());
            }
            frame.entered = true;
            self.connection[index] = frame;
            return Ok(None);
        }
        let neighbors = [
            (-1, 0, 1, 4, frame.row > 0),
            (1, 0, 4, 1, frame.row < 25),
            (0, -1, 8, 2, frame.column > 0),
            (0, 1, 2, 8, frame.column < 18),
        ];
        if usize::from(frame.next) == neighbors.len() {
            return Ok(self.pop_connection());
        }
        let (dy, dx, outgoing, incoming, bounded) = neighbors[usize::from(frame.next)];
        self.connection[index].next += 1;
        if frame.boundary && !bounded {
            return Ok(None);
        }
        let current = self.get(frame.row, frame.column)?;
        let neighbor = self.get(frame.row + dy, frame.column + dx)?;
        let eligible = if frame.boundary {
            neighbor == current & !0x800 || neighbor & incoming != 0
        } else {
            current & outgoing != 0 && (neighbor & 0x3000 == 0x1000 || neighbor & incoming != 0)
        };
        if eligible {
            if usize::from(self.connection_depth) == self.connection.len() {
                return Err(Error::Capacity("OriginalV1 connectivity stack"));
            }
            self.connection[usize::from(self.connection_depth)] = ConnectionFrame {
                row: frame.row + dy,
                column: frame.column + dx,
                ..ConnectionFrame::default()
            };
            self.connection_depth += 1;
        }
        Ok(None)
    }
    pub(super) fn build_filled_shapes(&mut self, resources: &impl OriginalResources) -> Result<()> {
        for index in 0..self.words.len() {
            let shape = self.words[index];
            if shape & 0x3000 == 0x3000 {
                self.words[index] |= 0x800;
            } else {
                self.words[index] &= 0xf7ff;
            }
            if shape & 0x3000 != 0x2000 {
                self.directions[index] = 0;
            }
        }
        for row in 0..26 {
            for column in 0..19 {
                let value = self.get(row, column)?;
                if value & 0x800 != 0 {
                    let mask = self.corners(row, column)?;
                    self.set(row, column, value | u16::from(mask))?;
                }
            }
        }
        for row in 0..26 {
            for column in 0..19 {
                let value = self.get(row, column)?;
                if value & 0x800 != 0 && value & 0xff == 3 {
                    let left = self.get(row, column - 1)? & 0xff;
                    if self.get(row, column + 1)? & 0xff != 7 && matches!(left, 11 | 19) {
                        self.set(row, column, value | 0x10)?;
                    }
                }
            }
        }
        for row in 0..26 {
            for column in 0..19 {
                let value = self.get(row, column)?;
                if value & 0x800 != 0
                    && value & 0xff == 12
                    && matches!(self.get(row, column - 1)? & 0xff, 4 | 28)
                {
                    self.set(row, column, value | 0x10)?;
                }
            }
        }
        for row in 0..26 {
            for column in 0..19 {
                let value = self.get(row, column)?;
                if value & 0x800 != 0 && value & 15 == 15 {
                    let upper = self.get(row - 1, column)? & 31;
                    let lower = self.get(row + 1, column)? & 31;
                    let upper_left = matches!(upper, 21 | 13 | 12);
                    let upper_right = matches!(upper, 14 | 28 | 26);
                    let lower_left = matches!(lower, 7 | 3);
                    let lower_right = matches!(lower, 19 | 11);
                    let shape = if upper_right {
                        if lower_left {
                            23
                        } else if lower_right {
                            26
                        } else {
                            20
                        }
                    } else if upper_left {
                        if lower_right {
                            24
                        } else if lower_left {
                            21
                        } else {
                            20
                        }
                    } else {
                        20
                    };
                    self.set(row, column, (value & 0xff00) | shape)?;
                }
            }
        }
        for word in &mut self.words {
            if *word & 0x800 != 0 {
                let bank = if *word & 0x8000 != 0 {
                    ShapeBank::FilledAlternate
                } else {
                    ShapeBank::FilledPlain
                };
                let shape = resources.original_shape(bank, (*word & 31) as u8)?;
                *word = (*word & 0xff00) | u16::from(shape);
            }
        }
        Ok(())
    }
    pub(super) fn build_path_shapes(&mut self, resources: &impl OriginalResources) -> Result<()> {
        for row in 1..25 {
            for column in 1..18 {
                let value = self.get(row, column)?;
                if value & 0x3000 == 0x1000 {
                    let neighbor = if self.get(row - 1, column)? & 0x3004 == 0x2004 {
                        Some((row - 1, column, 4, 16))
                    } else if self.get(row, column + 1)? & 0x3008 == 0x2008 {
                        Some((row, column + 1, 8, 17))
                    } else if self.get(row + 1, column)? & 0x3001 == 0x2001 {
                        Some((row + 1, column, 1, 18))
                    } else if self.get(row, column - 1)? & 0x3002 == 0x2002 {
                        Some((row, column - 1, 2, 19))
                    } else {
                        None
                    };
                    if let Some((y, x, bit, shape)) = neighbor {
                        let index = Self::index(y, x)?;
                        if self.directions[index] & bit != 0 {
                            self.set(row, column, value | 0x4000)?;
                        } else {
                            let upper = value & 0xff00;
                            let bank = if upper & 0x8000 != 0 {
                                ShapeBank::BoundaryAlternate
                            } else {
                                ShapeBank::BoundaryPlain
                            };
                            self.set(
                                row,
                                column,
                                upper | u16::from(resources.original_shape(bank, shape)?),
                            )?;
                        }
                    }
                }
            }
        }
        for index in 0..self.words.len() {
            let mut shape = self.words[index];
            let corners = if self.spec.directional_path_mode {
                0
            } else {
                self.directions[index]
            };
            if shape & 0x3000 == 0x2000 {
                shape ^= u16::from(corners);
                let tile = resources.original_shape(ShapeBank::Path, (shape & 15) as u8)?;
                self.words[index] = u16::from(tile) | 0x2000;
                if corners != 0 {
                    self.words[index] |= 0x4000;
                }
            }
        }
        Ok(())
    }
    pub(super) fn scatter_feature(
        &mut self,
        row: i16,
        column: i16,
        resources: &impl OriginalResources,
    ) -> Result<()> {
        let index = Self::index(row, column)?;
        if self.directions[index] & 15 != 0 {
            return Ok(());
        }
        let shape = self.words[index] & 0xff;
        if shape == 0 {
            if row == 0 {
                return Ok(());
            }
            let upper = self.get(row - 1, column)?;
            if matches!(upper, 27 | 45) || column == 0 {
                return Ok(());
            }
            if matches!(self.get(row, column - 1)?, 27 | 45) {
                return Ok(());
            }
            let (mut plain, mut alternate, mut feature) = (false, false, false);
            for y in row - 3..row + 3 {
                for x in column - 2..column + 2 {
                    if (0..26).contains(&y) && (0..19).contains(&x) {
                        let tile = self.get(y, x)?;
                        if tile & 0x3000 == 0x1000 {
                            if tile & 0x8000 != 0 {
                                alternate = true;
                            } else {
                                plain = true;
                            }
                        }
                        if matches!(tile, 27 | 45) {
                            feature = true;
                        }
                    }
                }
            }
            if (alternate || plain)
                && ((feature && self.rng.bounded(100) < 18) || self.rng.bounded(100) < 7)
            {
                let chance = if alternate { 70 } else { 20 };
                let block = row < 25
                    && column < 18
                    && self.rng.bounded(100) < 50
                    && self.get(row, column + 1)? == 0
                    && self.get(row + 1, column)? == 0
                    && self.get(row + 1, column + 1)? == 0;
                if chance > self.rng.bounded(100) {
                    if block {
                        self.set(row, column, 0x801e)?;
                        self.set(row, column + 1, 0x801f)?;
                        self.set(row + 1, column, 0x801d)?;
                        self.set(row + 1, column + 1, 0x8020)?;
                    } else {
                        self.set(row, column, 45)?;
                    }
                } else if block {
                    self.set(row, column, 2)?;
                    self.set(row, column + 1, 3)?;
                    self.set(row + 1, column, 1)?;
                    self.set(row + 1, column + 1, 4)?;
                } else {
                    self.set(row, column, 27)?;
                }
            }
        } else if shape == 46 {
            if self.rng.bounded(100) < 10 {
                self.set(row, column, 0x902f)?;
            }
        } else {
            let probability = self.parameter(resources, 7)?;
            if self.rng.bounded(100) < u32::from(probability) {
                let replacement = match shape {
                    75 => Some(73),
                    76 => Some(74),
                    80 => Some(77),
                    _ => None,
                };
                if let Some(value) = replacement {
                    self.set(row, column, value)?;
                }
            }
        }
        Ok(())
    }
    fn start_trace(&mut self, row: i16, column: i16, direction: u8) {
        self.trace[0] = TraceFrame {
            row,
            column,
            incoming: direction,
            outgoing: direction,
            ..TraceFrame::default()
        };
        self.trace_depth = 1;
    }
    fn pop_trace_failure(&mut self, entered: bool) -> Option<bool> {
        if entered {
            self.trace_visits = self.trace_visits.wrapping_sub(1);
        }
        self.trace_depth -= 1;
        if self.trace_depth == 0 {
            Some(false)
        } else {
            self.trace[usize::from(self.trace_depth - 1)].pending = 0;
            None
        }
    }
    fn trace_frame_step(&mut self, resources: &impl OriginalResources) -> Result<Option<bool>> {
        let index = usize::from(self.trace_depth - 1);
        let frame = self.trace[index];
        if !frame.entered {
            if self.get(frame.row, frame.column)? & 0x1f00 != 0
                || self.anchors[..3]
                    .iter()
                    .any(|anchor| anchor.x as i16 == frame.column && anchor.y as i16 == frame.row)
            {
                return Ok(self.pop_trace_failure(false));
            }
            self.trace_visits = self.trace_visits.wrapping_add(1);
            self.trace[index].entered = true;
            return Ok(None);
        }
        let rules: &[(u8, i16, i16, u8, u8, bool)] = match (frame.incoming, frame.outgoing) {
            (8, 8) => &[
                (3, 0, -1, 8, 8, false),
                (4, 1, 0, 4, 8, false),
                (2, 0, 0, 0, 0, true),
            ],
            (2, 2) => &[
                (5, 0, 1, 2, 2, false),
                (6, 1, 0, 4, 2, false),
                (1, 0, 0, 0, 0, true),
            ],
            (4, 8) => &[(7, 0, -1, 8, 4, false), (8, 1, 0, 4, 2, false)],
            (4, 2) => &[(9, 0, 1, 2, 4, false), (10, 1, 0, 4, 8, false)],
            (8, 4) => &[
                (11, 0, -1, 8, 8, false),
                (12, 1, 0, 4, 8, false),
                (13, 0, 0, 0, 0, true),
            ],
            (2, 4) => &[
                (14, 0, 1, 2, 2, false),
                (15, 1, 0, 4, 2, false),
                (16, 0, 0, 0, 0, true),
            ],
            _ => &[],
        };
        if usize::from(frame.next) == rules.len() {
            return Ok(self.pop_trace_failure(true));
        }
        let (descriptor, dy, dx, incoming, outgoing, terminal) = rules[usize::from(frame.next)];
        self.trace[index].next += 1;
        if !resources
            .original_transition_contains(self.get(frame.row, frame.column)? as u8, descriptor)?
        {
            return Ok(None);
        }
        if terminal {
            let minimum = if self.pass == 1 { 25 } else { 16 };
            if self.trace_visits >= minimum {
                self.set(
                    frame.row,
                    frame.column,
                    self.get(frame.row, frame.column)? | (u16::from(descriptor) << 8),
                )?;
                self.trace_depth -= 1;
                while self.trace_depth > 0 {
                    let parent = self.trace[usize::from(self.trace_depth - 1)];
                    self.set(
                        parent.row,
                        parent.column,
                        self.get(parent.row, parent.column)? | (u16::from(parent.pending) << 8),
                    )?;
                    self.trace_depth -= 1;
                }
                return Ok(Some(true));
            }
        } else {
            if usize::from(self.trace_depth) == self.trace.len() {
                return Err(Error::Capacity("OriginalV1 transition stack"));
            }
            self.trace[index].pending = descriptor;
            self.trace[usize::from(self.trace_depth)] = TraceFrame {
                row: frame.row + dy,
                column: frame.column + dx,
                incoming,
                outgoing,
                ..TraceFrame::default()
            };
            self.trace_depth += 1;
        }
        Ok(None)
    }
    pub(super) fn transition_step(&mut self, resources: &impl OriginalResources) -> Result<()> {
        if self.pass == 0 {
            for word in &mut self.words {
                *word &= 0xe0ff;
            }
            self.pass = 1;
            self.cursor = 0;
            return Ok(());
        }
        if self.trace_depth > 0 {
            if let Some(found) = self.trace_frame_step(resources)? {
                if found {
                    self.words[usize::from(self.cursor)] |=
                        if self.trace_side == 0 { 0x100 } else { 0x200 };
                    self.phase = OriginalPhase::Alternatives;
                    self.cursor = 0;
                    self.pass = 0;
                } else if self.trace_side == 0 {
                    self.trace_side = 1;
                    let row = (self.cursor / 19) as i16;
                    let column = (self.cursor % 19) as i16;
                    self.start_trace(row, column + 1, 2);
                } else {
                    self.cursor += 1;
                    self.trace_side = 0;
                }
            }
            return Ok(());
        }
        if usize::from(self.cursor) == self.words.len() {
            if self.pass == 1 {
                self.pass = 2;
                self.cursor = 0;
            } else {
                self.phase = OriginalPhase::Alternatives;
                self.cursor = 0;
                self.pass = 0;
            }
            return Ok(());
        }
        self.trace_visits = 0;
        if resources.original_transition_contains(self.words[usize::from(self.cursor)] as u8, 1)? {
            let row = (self.cursor / 19) as i16;
            let column = (self.cursor % 19) as i16;
            self.trace_side = 0;
            self.start_trace(row, column - 1, 8);
        } else {
            self.cursor += 1;
        }
        Ok(())
    }
}
