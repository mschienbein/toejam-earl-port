use super::*;

impl ResourceState {
    pub fn drop(&self, id: DropId) -> Option<&DroppedPresent> {
        self.drops
            .get(usize::from(id.slot))
            .filter(|slot| slot.generation == id.generation)
            .and_then(|slot| slot.value.as_ref())
    }
    pub fn known_drop(&self, id: DropId) -> bool {
        if DropId::new(id.slot, id.generation).is_none() {
            return false;
        }
        self.drops.get(usize::from(id.slot)).is_some_and(|slot| {
            slot.generation == 0
                || id.generation < slot.generation
                || id.generation == slot.generation && slot.value.is_some()
        })
    }
    pub fn trade_service(&self, actor: ActorId) -> Option<&TradeService> {
        self.services.iter().find(|service| service.actor == actor)
    }
    /// Rebind one persistent service and its offers after body reactivation.
    /// Capacity, source identity and every offer are checked before any write.
    pub(crate) fn bind_trade_service(
        &mut self,
        service: TradeService,
        offers: &[MailOffer],
    ) -> PlayerResult<()> {
        service
            .level
            .validate()
            .map_err(|_| PlayerError::Invalid("service level"))?;
        service
            .position
            .validate()
            .map_err(|_| PlayerError::Invalid("service position"))?;
        if !service.actor.valid()
            || service
                .source
                .is_some_and(|source| source.level != service.level || source.local >= 64)
        {
            return Err(PlayerError::Invalid("service persistent identity"));
        }
        let existing = self
            .services
            .iter()
            .position(|old| match (old.source, service.source) {
                (Some(a), Some(b)) => a == b,
                _ => old.actor == service.actor,
            });
        if existing.is_none() && self.services.len() == usize::from(self.limits.services) {
            return Err(PlayerError::Capacity("trade services"));
        }
        let old_actor = existing.map(|index| self.services[index].actor);
        let mut additional = 0;
        for (index, offer) in offers.iter().enumerate() {
            offer
                .present
                .validate()
                .map_err(|_| PlayerError::Invalid("mail wrapper"))?;
            if service.kind != ServiceKind::MailOrder
                || offer.id == 0
                || offer.service != service.actor
                || offer.price > 127
                || offers[..index].iter().any(|old| old.id == offer.id)
            {
                return Err(PlayerError::Invalid("bound mail offer"));
            }
            if let Some(old) = self.offers.iter().find(|old| old.id == offer.id) {
                if old.service != service.actor && Some(old.service) != old_actor {
                    return Err(PlayerError::Invalid(
                        "mail identity belongs to another source",
                    ));
                }
            } else {
                additional += 1;
            }
        }
        if self.offers.len() + additional > usize::from(self.limits.offers) {
            return Err(PlayerError::Capacity("mail offers"));
        }
        if let Some(index) = existing {
            self.services[index] = service;
        } else {
            self.services.push(service);
        }
        for &offer in offers {
            if let Some(old) = self.offers.iter_mut().find(|old| old.id == offer.id) {
                let stock = old.stock;
                *old = MailOffer { stock, ..offer };
            } else {
                self.offers.push(offer);
            }
        }
        Ok(())
    }
    /// G DropPresent's two randomized 5x5 perimeter passes, over the caller's
    /// independently sampled level cache. The first pass avoids existing and
    /// same-batch drops; the second permits overlap but never water/void.
    pub(crate) fn drop_position(
        &self,
        level: LevelId,
        center: Position,
        terrain: &[[u8; 5]; 5],
        random: u16,
        same_batch: &[DroppedPresent],
    ) -> Option<Position> {
        for pass in 0..2 {
            for row in 0..5 {
                for column in 0..5 {
                    if row != 0 && row != 4 && column != 0 && column != 4 {
                        continue;
                    }
                    let mut y = if random & 1 != 0 { 4 - row } else { row };
                    let mut x = if random & 2 != 0 { 4 - column } else { column };
                    if random & 4 != 0 {
                        std::mem::swap(&mut x, &mut y);
                    }
                    if matches!(terrain[y][x], 0x80 | 0x90) {
                        continue;
                    }
                    let position = Position {
                        x: ((center.x >> 8) + x as i32 * 8 - 16) * 256,
                        y: ((center.y >> 8) + y as i32 * 8 - 16) * 256,
                        elevation: 0,
                    };
                    if position.validate().is_err() {
                        continue;
                    }
                    let overlaps = |drop: &DroppedPresent| {
                        drop.level == level
                            && drop.position.x == position.x
                            && drop.position.y == position.y
                    };
                    if pass == 0
                        && (self.drops().any(|(_, drop)| overlaps(drop))
                            || same_batch.iter().any(overlaps))
                    {
                        continue;
                    }
                    return Some(position);
                }
            }
        }
        None
    }
}
