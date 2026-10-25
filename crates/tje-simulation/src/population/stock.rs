//! OriginalV1's seeded first-two-record population adapter. This is shared with
//! item generation so large-item clearance does not consult a fabricated RAM bank.
use super::*;
use tje_world::{
    LevelGeometry, Resources,
    original::OriginalRng,
    terrain::{Footprint, TerrainMask, position_in_cell},
};

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
        self.records.iter().flatten().any(|record| {
            (record.kind == Kind::ShipPiece || record.kind == Kind::Mailbox && record.mail_order)
                && (i64::from(x) - i64::from(record.position.x >> 8)).abs() < 340
                && (i64::from(y) - i64::from(record.position.y >> 8)).abs() < 200
        })
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
    let mut rng = OriginalRng::new(u32::from(request.level_seed));
    // Mailboxes precede ship pieces. Four and above except twelve draw exactly
    // once for their mode, before any placement row/column draws.
    let mail_order =
        request.level.get() < 4 || request.level.get() == 12 || rng.next_u16() % 100 >= 50;
    let (width, height, mask) = Kind::Mailbox.spawn_footprint();
    let placement =
        super::spawning::scan_spawn(Dimensions::ORIGINAL, &mut rng, |column, row, rng| {
            let cell = geometry
                .cell(i32::from(column), i32::from(row))
                .ok_or(Error::Bounds("stock Earthling cell"))?;
            let Some((x, y)) = position_in_cell(
                cell,
                Footprint::new(width, height)?,
                TerrainMask::new(mask)?,
                false,
                rng,
                resources,
            )?
            else {
                return Ok(None);
            };
            let x = (i32::from(column) + 1) * 128 + x;
            let y = (i32::from(row) + 1) * 64 + y;
            // G ElevatorAllowsWorldPlacement, not the different contains predicate.
            for anchor in [geometry.anchors().arrival, geometry.anchors().exit] {
                let elevator_x = (i32::from(anchor.x) + 2) * 128 - 16;
                let elevator_y = (i32::from(anchor.y) + 2) * 64;
                if (x - elevator_x).abs() < 30 && (y - elevator_y).abs() < 20 {
                    return Ok(None);
                }
            }
            Ok(Some((x, y)))
        })?;
    if let Some((x, y)) = placement {
        result.records[0] = Some(AuthoredActor {
            local: 0,
            kind: Kind::Mailbox,
            position: Position {
                x: x * 256,
                y: y * 256,
                elevation: 0,
            },
            mail_order,
        });
    } else {
        result.failed |= 1;
    }
    if let Some(position) = request.ship_position {
        position
            .validate()
            .map_err(|_| Error::Bounds("stock ship-piece position"))?;
        if geometry
            .dimensions()
            .world_cell(position.x >> 8, position.y >> 8)
            .is_none()
        {
            return Err(Error::Bounds("stock ship-piece anchor"));
        }
        // The stock piece is not rerolled or terrain-placed after its map anchor.
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
