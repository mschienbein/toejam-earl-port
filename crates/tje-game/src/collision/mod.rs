//! R011 collision queries. Word wrapping and N-only magnitude tests are deliberate.
//!
//! Production ABI qualification: every catalogued caller of these five queries
//! is matched MCC68K C (`G/docs/COMPILER.md:70-79`), which treats D1 and CCR as
//! call-clobbered scratch. `strict_registers = false` is therefore intentional,
//! not an approval of D1/CCR equivalence by ordinary production checks. The exact
//! inverse caller sets and their C ABI are pinned in the caller-provenance test;
//! a new assembly or unknown caller must fail that test and trigger ABI review.
//! Bodies nevertheless preserve the full assembly outputs, independently proven
//! by the stronger all-register/CCR contracts in `check_world.rs` on both ROMs.

use crate::{Context, Result, Translation, ids};
use tje_mem::{Address, ObjectState, ram};

/// Available Rust entries owned by this subsystem.
pub static TRANSLATIONS: &[Translation] = &[
    Translation {
        id: ids::ENTITY_FIND_NARROW_COLLISION_AT_POSITION,
        function: entity_find_narrow_collision_at_position,
    },
    Translation {
        id: ids::ENTITY_FIND_MENU_AWARE_COLLISION_AT_POSITION,
        function: entity_find_menu_aware_collision_at_position,
    },
    Translation {
        id: ids::ENTITY_FIND_VOLUME_COLLISION_AT_POSITION,
        function: entity_find_volume_collision_at_position,
    },
    Translation {
        id: ids::PROJECTILE_FIND_COLLISION_EXCLUDING_OWNER,
        function: projectile_find_collision_excluding_owner,
    },
    Translation {
        id: ids::ENTITY_FIND_HORIZONTAL_COLLISION_AT_X,
        function: entity_find_horizontal_collision_at_x,
    },
];

/// All five original collision queries are reviewed Atomic.
pub static EXECUTION_CONTRACTS: &[crate::execution::ExecutionContract] = &[];

/// Each query only walks RAM links and reads object fields; there are no callees,
/// device accesses, interrupt waits, or clock-dependent return paths.
pub static ATOMIC_CAPABLE: &[crate::FunctionId] = &[
    ids::ENTITY_FIND_NARROW_COLLISION_AT_POSITION,
    ids::ENTITY_FIND_MENU_AWARE_COLLISION_AT_POSITION,
    ids::ENTITY_FIND_VOLUME_COLLISION_AT_POSITION,
    ids::PROJECTILE_FIND_COLLISION_EXCLUDING_OWNER,
    ids::ENTITY_FIND_HORIZONTAL_COLLISION_AT_X,
];

// D1 and X survive the final MOVEQ on a miss. All other scratch values are either
// restored by MOVEM or overwritten before return. Keep their actual word widths;
// CMP's signed condition differs from the N-only SUB/BPL/NEG magnitude idiom.
struct Scratch {
    d1: u32,
    x: bool,
}

impl Scratch {
    fn new(context: &dyn Context) -> Self {
        Self {
            d1: context.data_register(1),
            x: context.status_register() & 0x10 != 0,
        }
    }

    fn d1_word(&mut self, word: i16) {
        self.d1 = (self.d1 & 0xffff_0000) | u32::from(word as u16);
    }

    fn sub_word(&mut self, left: i16, right: i16) -> i16 {
        let (result, borrow) = (left as u16).overflowing_sub(right as u16);
        self.x = borrow;
        result as i16
    }

    fn add_word(&mut self, left: i16, right: i16) -> i16 {
        let (result, carry) = (left as u16).overflowing_add(right as u16);
        self.x = carry;
        result as i16
    }

    fn magnitude(&mut self, left: i16, right: i16) -> i16 {
        let difference = self.sub_word(left, right);
        if difference < 0 {
            self.x = difference != 0;
            difference.wrapping_neg()
        } else {
            difference
        }
    }

    fn half_width(&mut self, width: u8) -> i16 {
        self.x = width & 1 != 0;
        i16::from(width >> 1)
    }

    fn combined_width(
        &mut self,
        context: &dyn Context,
        entity: Address<ObjectState>,
        candidate: Address<ObjectState>,
        inset: i16,
    ) -> i16 {
        let candidate_half = self.half_width(candidate.width().read(context));
        let entity_half = self.half_width(entity.width().read(context));
        let combined = self.add_word(candidate_half, entity_half);
        if inset == 0 {
            combined
        } else {
            self.sub_word(combined, inset)
        }
    }

    fn finish(self, context: &mut dyn Context, hit: Option<(i8, bool)>) -> Result<Option<u32>> {
        let (result, flags) = if let Some((index, word_increment)) = hit {
            let before = i32::from(index) as u32;
            let result = if word_increment {
                (before & 0xffff_0000) | u32::from((before as u16).wrapping_add(1))
            } else {
                before.wrapping_add(1)
            };
            let (negative, zero) = if word_increment {
                ((result as i16) < 0, result as u16 == 0)
            } else {
                ((result as i32) < 0, result == 0)
            };
            // A sign-extended byte + 1 cannot overflow the word or long sign bit.
            let carry = index == -1;
            let flags =
                (u16::from(carry) * 0x11) | (u16::from(negative) << 3) | (u16::from(zero) << 2);
            (result, flags)
        } else {
            (0, (u16::from(self.x) << 4) | 4)
        };
        context.set_data_register(1, self.d1);
        context.set_status_register((context.status_register() & !0x1f) | flags);
        Ok(Some(result))
    }
}

/// G/src/asm/collision.s:44 (`EntityFindNarrowCollisionAtPosition`).
/// Current-Y broad phase, strict X/Y reach, inclusive current-height tolerance.
pub fn entity_find_narrow_collision_at_position(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let entity = Address::<ObjectState>::new(arguments[0]);
    let proposed_x = arguments[1] as i16;
    let proposed_y = arguments[2] as i16;
    let first_index = arguments[3] as i16;
    let mut scratch = Scratch::new(context);
    let mut candidate = ram::g_entity_draw_list_head(context.revision()).read(context);
    while candidate.address() != 0 {
        let target = candidate.target();
        let current_y = entity.y().read(context);
        scratch.d1_word(current_y);
        let target_y = target.y().read(context);
        let lower = scratch.sub_word(target_y, 13);
        if current_y >= lower {
            let upper = scratch.add_word(target_y, 13);
            if current_y > upper {
                break;
            }
            if entity != target
                && i16::from(target.entity_index().read(context)) >= first_index
                && target.draw_flags().read(context) & 1 != 0
                && target.behavior_flags().read(context) & 1 != 0
                && target.level().read(context) == entity.level().read(context)
            {
                let reach = scratch.combined_width(context, entity, target, 10);
                let target_x = target.x().read(context);
                scratch.d1_word(target_x);
                let dx = scratch.magnitude(proposed_x, target_x);
                if reach > dx {
                    let target_y = target.y().read(context);
                    scratch.d1_word(target_y);
                    let dy = scratch.magnitude(proposed_y, target_y);
                    if dy < 8 {
                        let height = entity.height().read(context);
                        let target_height = target.height().read(context);
                        if scratch.magnitude(height, target_height) <= 10 {
                            let index = target.entity_index().read(context);
                            return scratch.finish(context, Some((index, false)));
                        }
                    }
                }
            }
        }
        candidate = target.next().read(context);
    }
    scratch.finish(context, None)
}

/// G/src/asm/collision.s:144 (`EntityFindMenuAwareCollisionAtPosition`).
/// Signed indices <= 1 consult the unchecked inventory-open byte before self tests.
pub fn entity_find_menu_aware_collision_at_position(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let entity = Address::<ObjectState>::new(arguments[0]);
    let proposed_x = arguments[1] as i16;
    let proposed_y = arguments[2] as i16;
    let mut scratch = Scratch::new(context);
    let mut candidate = ram::g_entity_draw_list_head(context.revision()).read(context);
    while candidate.address() != 0 {
        let target = candidate.target();
        let current_y = entity.y().read(context);
        scratch.d1_word(current_y);
        let target_y = target.y().read(context);
        let lower = scratch.sub_word(target_y, 13);
        if current_y >= lower {
            let upper = scratch.add_word(target_y, 13);
            if current_y > upper {
                break;
            }
            let index = target.entity_index().read(context);
            let menu_eligible = if index <= 1 {
                let menus = ram::g_player_inventory_open(context.revision());
                context.set_address_register(0, menus.address());
                menus.at(i32::from(index)).read(context) != 0
            } else {
                true
            };
            if menu_eligible
                && entity != target
                && target.draw_flags().read(context) & 1 != 0
                && target.behavior_flags().read(context) & 1 != 0
                && target.level().read(context) == entity.level().read(context)
            {
                let reach = scratch.combined_width(context, entity, target, 10);
                let dx = scratch.magnitude(proposed_x, target.x().read(context));
                if reach > dx {
                    let dy = scratch.magnitude(proposed_y, target.y().read(context));
                    if dy < 8 {
                        let height = entity.height().read(context);
                        let target_height = target.height().read(context);
                        if scratch.magnitude(height, target_height) <= 10 {
                            let index = target.entity_index().read(context);
                            return scratch.finish(context, Some((index, true)));
                        }
                    }
                }
            }
        }
        candidate = target.next().read(context);
    }
    scratch.finish(context, None)
}

/// G/src/asm/collision.s:246 (`EntityFindVolumeCollisionAtPosition`).
/// Signed-byte heights and asymmetric strict/inclusive interval endpoints remain.
pub fn entity_find_volume_collision_at_position(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let entity = Address::<ObjectState>::new(arguments[0]);
    let proposed_x = arguments[1] as i16;
    let proposed_y = arguments[2] as i16;
    let proposed_height = arguments[3] as i16;
    let first_index = arguments[4] as i16;
    let mut scratch = Scratch::new(context);
    let mut candidate = ram::g_entity_draw_list_head(context.revision()).read(context);
    while candidate.address() != 0 {
        let target = candidate.target();
        let current_y = entity.y().read(context);
        scratch.d1_word(current_y);
        let target_y = target.y().read(context);
        let lower = scratch.sub_word(target_y, 13);
        if current_y >= lower {
            let upper = scratch.add_word(target_y, 13);
            if current_y > upper {
                break;
            }
            if entity != target
                && i16::from(target.entity_index().read(context)) >= first_index
                && target.draw_flags().read(context) & 1 != 0
                && target.behavior_flags().read(context) & 1 != 0
                && target.level().read(context) == entity.level().read(context)
            {
                let reach = scratch.combined_width(context, entity, target, 10);
                let target_x = target.x().read(context);
                scratch.d1_word(target_x);
                let dx = scratch.magnitude(proposed_x, target_x);
                if reach > dx {
                    let target_y = target.y().read(context);
                    scratch.d1_word(target_y);
                    // D0's high word was cleared by MOVEQ before the width test.
                    // The CMP.L treats wrapped -32768 as positive 32768 here.
                    let dy = u32::from(scratch.magnitude(proposed_y, target_y) as u16);
                    if dy < 8 {
                        scratch.d1_word(proposed_height);
                        let target_height = target.height().read(context);
                        let overlap = if target_height <= proposed_height {
                            let pixels = i16::from(target.height_pixels().read(context) as i8);
                            let top = scratch.add_word(pixels, target.height().read(context));
                            proposed_height < top
                        } else {
                            let target_height = target.height().read(context);
                            scratch.d1_word(target_height);
                            let pixels = i16::from(entity.height_pixels().read(context) as i8);
                            let top = scratch.add_word(pixels, proposed_height);
                            target_height <= top
                        };
                        if overlap {
                            let index = target.entity_index().read(context);
                            return scratch.finish(context, Some((index, false)));
                        }
                    }
                }
            }
        }
        candidate = target.next().read(context);
    }
    scratch.finish(context, None)
}

/// G/src/asm/collision.s:360 (`ProjectileFindCollisionExcludingOwner`).
/// No self-pointer exclusion; unsigned target height and distinct vertical padding.
pub fn projectile_find_collision_excluding_owner(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let entity = Address::<ObjectState>::new(arguments[0]);
    let excluded_owner = arguments[1] as i16;
    let mut scratch = Scratch::new(context);
    let mut candidate = ram::g_entity_draw_list_head(context.revision()).read(context);
    while candidate.address() != 0 {
        let target = candidate.target();
        let current_y = entity.y().read(context);
        scratch.d1_word(current_y);
        let target_y = target.y().read(context);
        let lower = scratch.sub_word(target_y, 13);
        if current_y >= lower {
            let upper = scratch.add_word(target_y, 13);
            if current_y > upper {
                break;
            }
            if target.behavior_flags().read(context) & 1 != 0
                && target.level().read(context) == entity.level().read(context)
                && i16::from(target.entity_index().read(context)) != excluded_owner
                && target.draw_flags().read(context) & 1 != 0
            {
                let half = scratch.half_width(target.width().read(context));
                let reach = scratch.sub_word(half, 1);
                let x = entity.x().read(context);
                let target_x = target.x().read(context);
                scratch.d1_word(target_x);
                let dx = scratch.magnitude(x, target_x);
                if reach > dx {
                    let y = entity.y().read(context);
                    let target_y = target.y().read(context);
                    let dy = scratch.magnitude(y, target_y);
                    if dy <= 8 {
                        let height = entity.height().read(context);
                        scratch.d1_word(height);
                        let target_height = target.height().read(context);
                        let overlap = if height > target_height {
                            let pixels = i16::from(target.height_pixels().read(context));
                            let top = scratch.add_word(pixels, target.height().read(context));
                            let padded_top = scratch.add_word(top, 8);
                            height < padded_top
                        } else {
                            let padded_height = scratch.add_word(height, 7);
                            scratch.d1_word(padded_height);
                            padded_height > target.height().read(context)
                        };
                        if overlap {
                            let index = target.entity_index().read(context);
                            return scratch.finish(context, Some((index, true)));
                        }
                    }
                }
            }
        }
        candidate = target.next().read(context);
    }
    scratch.finish(context, None)
}

/// G/src/asm/collision.s:463 (`EntityFindHorizontalCollisionAtX`).
/// The loaded Y slot is unused; broad-phase Y and elevation are current values.
pub fn entity_find_horizontal_collision_at_x(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let entity = Address::<ObjectState>::new(arguments[0]);
    let proposed_x = arguments[1] as i16;
    let _unused_y = arguments[2] as i16;
    let first_index = arguments[3] as i16;
    let mut scratch = Scratch::new(context);
    let mut candidate = ram::g_entity_draw_list_head(context.revision()).read(context);
    while candidate.address() != 0 {
        let target = candidate.target();
        let current_y = entity.y().read(context);
        scratch.d1_word(current_y);
        let target_y = target.y().read(context);
        let lower = scratch.sub_word(target_y, 13);
        if current_y >= lower {
            let upper = scratch.add_word(target_y, 13);
            if current_y > upper {
                break;
            }
            if entity != target
                && target.behavior_flags().read(context) & 1 != 0
                && target.level().read(context) == entity.level().read(context)
                && target.draw_flags().read(context) & 1 != 0
                && i16::from(target.entity_index().read(context)) >= first_index
            {
                let reach = scratch.combined_width(context, entity, target, 0);
                let target_x = target.x().read(context);
                scratch.d1_word(target_x);
                let dx = scratch.magnitude(proposed_x, target_x);
                if reach > dx {
                    let height = entity.height().read(context);
                    let target_height = target.height().read(context);
                    if scratch.magnitude(height, target_height) <= 10 {
                        let index = target.entity_index().read(context);
                        return scratch.finish(context, Some((index, true)));
                    }
                }
            }
        }
        candidate = target.next().read(context);
    }
    scratch.finish(context, None)
}

#[cfg(test)]
mod tests;
