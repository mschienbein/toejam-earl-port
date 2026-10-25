use super::OriginalRevision;
use crate::{Resources, Result, loading::SpecialFloor};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapeBank {
    Path,
    EdgePlain,
    EdgeAlternate,
    FilledPlain,
    FilledAlternate,
    BoundaryPlain,
    BoundaryAlternate,
}

/// Read-only bounded access to the importing user's revision. No table payload
/// or executable callback is serialized into OriginalV1 state.
pub trait OriginalResources: Resources {
    fn original_revision(&self) -> OriginalRevision;
    fn original_shape(&self, bank: ShapeBank, index: u8) -> Result<u8>;
    fn original_turn(&self, direction: u8, choice: u8) -> Result<u8>;
    fn original_transition_contains(&self, shape: u8, descriptor: u8) -> Result<bool>;
    fn original_special_word(&self, floor: SpecialFloor, index: usize) -> Result<u16>;
    fn original_level1_item(&self, index: u8) -> Result<(i32, i32)>;
    fn original_present_weight(&self, index: u8) -> Result<u8>;
    fn original_large_excluded(&self, word: u16) -> Result<bool>;
}
