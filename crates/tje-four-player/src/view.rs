//! Presentation-only layout; this does not render sidecar actors or run a
//! camera/activation algorithm. Unused seats retain neutral panels, not copies.
use crate::{Result, SEATS, SeatId, SeatMask};

pub const VIEW_WIDTH: u16 = 400;
pub const VIEW_HEIGHT: u16 = 224;
pub const CANVAS_WIDTH: u16 = 2 * VIEW_WIDTH;
pub const CANVAS_HEIGHT: u16 = 2 * VIEW_HEIGHT;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewId(u8);
impl ViewId {
    pub const fn get(self) -> u8 {
        self.0
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rectangle {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Content {
    Seat(SeatId),
    NeutralPanel,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quadrant {
    pub view: ViewId,
    pub rectangle: Rectangle,
    pub content: Content,
}

pub fn quadrants(participants: SeatMask) -> Result<[Quadrant; SEATS]> {
    participants.validate()?;
    Ok(std::array::from_fn(|index| Quadrant {
        view: ViewId(index as u8),
        rectangle: Rectangle {
            x: (index as u16 % 2) * VIEW_WIDTH,
            y: (index as u16 / 2) * VIEW_HEIGHT,
            width: VIEW_WIDTH,
            height: VIEW_HEIGHT,
        },
        content: if participants.contains(SeatId::ALL[index]) {
            Content::Seat(SeatId::ALL[index])
        } else {
            Content::NeutralPanel
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_full_density_quadrants_cover_the_canvas_exactly() {
        let views = quadrants(SeatMask::ALL).unwrap();
        let mut coverage = vec![0_u8; usize::from(CANVAS_WIDTH) * usize::from(CANVAS_HEIGHT)];
        for (index, quadrant) in views.into_iter().enumerate() {
            assert_eq!(quadrant.view.get() as usize, index);
            assert_eq!(quadrant.content, Content::Seat(SeatId::ALL[index]));
            let r = quadrant.rectangle;
            assert_eq!((r.width, r.height), (400, 224));
            for y in r.y..r.y + r.height {
                for x in r.x..r.x + r.width {
                    coverage[usize::from(y) * usize::from(CANVAS_WIDTH) + usize::from(x)] += 1;
                }
            }
        }
        assert!(coverage.iter().all(|&count| count == 1));
    }
    #[test]
    fn absent_seats_are_neutral_without_reordering_live_views() {
        for bits in 0..16 {
            let mask = SeatMask::new(bits).unwrap();
            let views = quadrants(mask).unwrap();
            for seat in SeatId::ALL {
                assert_eq!(
                    views[seat.index()].content == Content::Seat(seat),
                    mask.contains(seat)
                );
            }
            assert_eq!(
                views
                    .iter()
                    .filter(|view| view.content == Content::NeutralPanel)
                    .count(),
                SEATS - mask.iter().count()
            );
        }
    }
}
