use super::PresentKind;
use crate::players::{PlayerError, PlayerResult};
use tje_world::{LevelId, random::StageRng};

/// Read-only imported selection semantics, never serialized ROM table payloads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresentCatalog {
    cumulative: [u16; 27],
    total: u16,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MailChoice {
    pub kind: PresentKind,
    pub price: u8,
}
impl PresentCatalog {
    pub fn new(weights: [u8; 27]) -> PlayerResult<Self> {
        let mut cumulative = [0; 27];
        let mut total = 0_u16;
        for (index, weight) in weights.into_iter().enumerate() {
            total += u16::from(weight);
            cumulative[index] = total;
        }
        if total == 0 {
            return Err(PlayerError::Invalid("empty present weight catalogue"));
        }
        Ok(Self { cumulative, total })
    }
    pub fn from_rom(rom: &[u8], revision: tje_mem::Revision) -> PlayerResult<Self> {
        let start = tje_mem::rom::present_weight_table(revision).at(0).address() as usize;
        let bytes = rom
            .get(start..start + 27)
            .ok_or(PlayerError::Rejected("user ROM lacks present weights"))?;
        Self::new(bytes.try_into().expect("checked 27-byte extent"))
    }
    /// G/level_item_setup.c:303–321: cumulative weights and zero-draw retry.
    /// The enhanced operational ceiling rejects, never substitutes a fake gift.
    pub fn choose(&self, rng: &mut StageRng) -> PlayerResult<PresentKind> {
        for _ in 0..1024 {
            let selected = rng.bounded(u32::from(self.total) + 1) as u16;
            if selected != 0 {
                let index = self.cumulative.partition_point(|&sum| sum < selected);
                return Ok(PresentKind::ALL[index]);
            }
        }
        Err(PlayerError::Rejected("present selection work exhausted"))
    }
    /// G/inventory.c:176–194: same-floor deterministic three distinct offers,
    /// excluding extra life, promotion and jackpot; price draws follow each gift.
    pub fn mail_choices(
        &self,
        identity: &[u8; 32],
        level: LevelId,
    ) -> PlayerResult<[MailChoice; 3]> {
        let allowed = (0..27)
            .filter(|&index| {
                let weight = self.cumulative[index]
                    - if index == 0 {
                        0
                    } else {
                        self.cumulative[index - 1]
                    };
                weight != 0
                    && !matches!(
                        PresentKind::ALL[index],
                        PresentKind::ExtraLife | PresentKind::Promotion | PresentKind::Jackpot
                    )
            })
            .count();
        if allowed < 3 {
            return Err(PlayerError::Rejected(
                "mail catalogue lacks three eligible gifts",
            ));
        }
        let mut rng = StageRng::derive(identity, level, 0, b"mail-order/1");
        let mut result = [MailChoice {
            kind: PresentKind::Food,
            price: 3,
        }; 3];
        for slot in 0..3 {
            let mut selected = None;
            for _ in 0..1024 {
                let kind = self.choose(&mut rng)?;
                if !matches!(
                    kind,
                    PresentKind::ExtraLife | PresentKind::Promotion | PresentKind::Jackpot
                ) && !result[..slot].iter().any(|choice| choice.kind == kind)
                {
                    selected = Some(kind);
                    break;
                }
            }
            result[slot] = MailChoice {
                kind: selected.ok_or(PlayerError::Rejected("mail selection work exhausted"))?,
                price: rng.bounded(3) as u8 + 3,
            };
        }
        Ok(result)
    }
}
