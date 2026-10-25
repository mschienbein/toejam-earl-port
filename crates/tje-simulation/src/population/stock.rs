//! OriginalV1's seeded first-two-record population adapter. This is shared with
//! item generation so large-item clearance does not consult a fabricated RAM bank.
use super::*;
use tje_world::{LevelGeometry, Resources, original::items::StockSpecialPositions};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StockSpecialRequest {
    pub level: LevelId,
    pub level_seed: u16,
    /// MapGetShipPieceWorldPosition's already resolved anchor, when assigned.
    pub ship_position: Option<Position>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StockSpecials {
    pub records: [Option<AuthoredActor>; 2],
    pub failed: u8,
}
impl StockSpecials {
    /// G/spawning_and_banks.c:78-97: only ship parts and mail-order mailboxes
    /// among the original first two stable identities exclude large items.
    pub fn is_near(&self, x: i32, y: i32) -> bool {
        StockSpecialPositions {
            positions: self.records.map(|record| {
                record
                    .filter(|record| {
                        record.kind == Kind::ShipPiece
                            || record.kind == Kind::Mailbox && record.mail_order
                    })
                    .map(|record| (record.position.x >> 8, record.position.y >> 8))
            }),
            mail_order: true,
            failed: 0,
        }
        .is_near(x, y)
    }
}

/// Match the seeded special prefix of EarthlingRetainLevelBank. Its ordinary
/// records restore the ambient pre-bank RNG; that stream is deliberately NOT
/// returned here or mistaken for the item generator's independent seeded scope.
pub fn prepare_stock_specials(
    request: StockSpecialRequest,
    geometry: &LevelGeometry,
    resources: &impl Resources,
    catalog: &DefinitionCatalog,
) -> Result<StockSpecials> {
    request.level.validate()?;
    if request.level.get() > 26 || geometry.dimensions() != Dimensions::ORIGINAL {
        return Err(Error::Bounds("OriginalV1 special-population extent"));
    }
    let mut result = StockSpecials {
        records: [None; 2],
        failed: 0,
    };
    if request.level.get() <= 1 || request.level.get() == 26 {
        return Ok(result);
    }
    let prefix = StockSpecialPositions::prepare(
        request.level,
        request.level_seed,
        geometry,
        request
            .ship_position
            .map(|position| (position.x >> 8, position.y >> 8)),
        resources,
    )?;
    result.failed = prefix.failed;
    if let Some((x, y)) = prefix.positions[0] {
        result.records[0] = Some(AuthoredActor {
            local: 0,
            kind: Kind::Mailbox,
            position: Position {
                x: x * 256,
                y: y * 256,
                elevation: 0,
            },
            mail_order: prefix.mail_order,
        });
    }
    if let Some(position) = request.ship_position {
        position
            .validate()
            .map_err(|_| Error::Bounds("stock ship-piece position"))?;
        result.records[1] = Some(AuthoredActor {
            local: 1,
            kind: Kind::ShipPiece,
            position,
            mail_order: false,
        });
    }
    // Import/catalog ownership must be available even when a particular prefix
    // has no rendered instance. No raw definition/callback pointer is published.
    catalog.get(Kind::Mailbox).validate()?;
    catalog.get(Kind::ShipPiece).validate()?;
    Ok(result)
}
