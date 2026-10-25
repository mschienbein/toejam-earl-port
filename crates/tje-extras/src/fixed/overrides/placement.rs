//! B004–B007: bounded searches and checked, explicitly owned outputs.
use tje_game::{
    Context, Result, call,
    call_frame::{FrameLayout, LocalFrame},
    call_void, ids,
};
use tje_mem::{Address, LevelItem, Ptr, ram, rom};

fn random(c: &mut dyn Context) -> Result<u16> {
    Ok(call(c, ids::GET_RANDOM_NUMBER, &[])? as u16)
}

#[derive(Clone, Copy)]
enum Placement {
    Player(i32),
    Item(bool),
}

/// Every draw consumes an attempt. Then visit every cell exactly once from a
/// deterministic RNG-selected origin; failed validation never publishes X/Y.
fn find(
    c: &mut dyn Context,
    map: Address<u16>,
    level: i16,
    width: i16,
    height: i16,
    kind: Placement,
) -> Result<Option<(i16, i16)>> {
    let frame = LocalFrame::enter(c, FrameLayout::link(4, 0, 0))?;
    let x = frame.local::<i16>(-4);
    let y = frame.local::<i16>(-2);
    let result = (|| {
        for _ in 0..10 {
            let row = i32::from(random(c)? % 26);
            let col = i32::from(random(c)? % 19);
            if let Some(position) = candidate(c, map, level, width, height, kind, row, col, x, y)? {
                return Ok(Some(position));
            }
        }
        let start_row = i32::from(random(c)? % 26);
        let start_col = i32::from(random(c)? % 19);
        for row_offset in 0..26 {
            for col_offset in 0..19 {
                let row = (start_row + row_offset) % 26;
                let col = (start_col + col_offset) % 19;
                if let Some(position) =
                    candidate(c, map, level, width, height, kind, row, col, x, y)?
                {
                    return Ok(Some(position));
                }
            }
        }
        Ok(None)
    })();
    frame.leave(c)?;
    result
}
#[allow(clippy::too_many_arguments)]
fn candidate(
    c: &mut dyn Context,
    map: Address<u16>,
    level: i16,
    width: i16,
    height: i16,
    kind: Placement,
    row: i32,
    col: i32,
    x: Address<i16>,
    y: Address<i16>,
) -> Result<Option<(i16, i16)>> {
    let tile = map.offset(row * 19 + col).read(c);
    match kind {
        Placement::Player(_) if tile & 0x4000 != 0 => return Ok(None),
        Placement::Item(large)
            if tile & 0x2000 != 0
                || large
                    && call(
                        c,
                        ids::MAP_IS_LARGE_PLACEMENT_TERRAIN_EXCLUDED,
                        &[u32::from(tile)],
                    )? != 0 =>
        {
            return Ok(None);
        }
        _ => {}
    }
    if call(
        c,
        ids::TERRAIN_VALIDATION,
        &[
            u32::from(tile),
            width as u32,
            height as u32,
            x.address(),
            y.address(),
            3,
        ],
    )? == 0
    {
        return Ok(None);
    }
    let world_x = x.read(c).wrapping_add(((col + 1) * 128) as i16);
    let world_y = y.read(c).wrapping_add(((row + 1) * 64) as i16);
    match kind {
        Placement::Player(player) => {
            if player == 2 {
                for delta in [-32_i16, 32] {
                    if call(
                        c,
                        ids::ELEVATOR_CONTAINS_POSITION,
                        &[
                            world_x.wrapping_add(delta) as u32,
                            world_y as u32,
                            level as u32,
                        ],
                    )? != 0
                    {
                        return Ok(None);
                    }
                }
            } else if call(
                c,
                ids::ELEVATOR_CONTAINS_POSITION,
                &[world_x as u32, world_y as u32, level as u32],
            )? != 0
            {
                return Ok(None);
            }
        }
        Placement::Item(_) => {
            if call(
                c,
                ids::ELEVATOR_ALLOWS_WORLD_PLACEMENT,
                &[world_x as u32, world_y as u32, level as u32],
            )? == 0
            {
                return Ok(None);
            }
        }
    }
    Ok(Some((world_x, world_y)))
}
fn item_search(c: &mut dyn Context, level: i16, large: bool) -> Result<Option<(i16, i16)>> {
    let r = c.revision();
    let map = if i16::from(ram::g_player1_map_level(r).read(c)) == level {
        ram::g_player1_level_map(r)
    } else {
        ram::g_player2_level_map(r)
    };
    find(
        c,
        map.cast(),
        level,
        if large { 4 } else { 3 },
        if large { 2 } else { 1 },
        Placement::Item(large),
    )
}

/// B004/B005: exhausted maps produce an explicit no-placement error, not an
/// arbitrary origin or an old unvalidated position. No coordinate is mutated.
pub(super) fn player_position(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let r = c.revision();
    let player = if ram::g_player_lives(r).at(0).read(c) < 0 {
        1
    } else if ram::g_player_lives(r).at(1).read(c) < 0 {
        0
    } else {
        args[0] as i32
    };
    let level = ram::g_player_info(r)
        .at(if player == 2 { 0 } else { player })
        .level()
        .read(c);
    let (width, height) = if level <= 1 {
        (16, 8)
    } else {
        (if player == 2 { 12 } else { 4 }, 2)
    };
    let map = if player == 1 && ram::g_player_lives(r).at(0).read(c) >= 0 {
        ram::g_player2_level_map(r)
    } else {
        ram::g_player1_level_map(r)
    };
    let Some((x, y)) = find(
        c,
        map.cast(),
        i16::from(level),
        width,
        height,
        Placement::Player(player),
    )?
    else {
        return Err(tje_game::Error::Platform("Fixed B005: no valid player placement; coordinates unchanged (reset or restore a prior save)".into()));
    };
    for p in 0..2 {
        if player == p || player == 2 {
            let object = ram::g_player_info(r).at(p);
            object.x().write(
                c,
                x.wrapping_add(if player == 2 && p == 1 { 64 } else { 0 }),
            );
            object.y().write(c, y);
        }
    }
    Ok(Some(1))
}
/// B004/B006: D0 is an explicit success flag. Outputs remain untouched on failure.
pub(super) fn item_position(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    if let Some((x, y)) = item_search(c, args[2] as i16, args[3] as i16 != 0)? {
        Address::<i16>::new(args[0]).write(c, x);
        Address::<i16>::new(args[1]).write(c, y);
        Ok(Some(1))
    } else {
        Ok(Some(0))
    }
}

/// B006: consume only successful placements, with ten finite spacing retries.
pub(super) fn large_items(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let r = c.revision();
    let items = Address::<LevelItem>::new(args[0]);
    let level = args[1] as i16;
    let map = u32::from(i16::from(ram::g_player1_map_level(r).read(c)) != level);
    for slot in 28..32 {
        let mut found = None;
        for _ in 0..10 {
            let Some((x, y)) = item_search(c, level, true)? else {
                break;
            };
            let mut blocked = false;
            for index in 0..slot {
                let other = items.offset(index);
                if other.kind().read(c) == -1 {
                    continue;
                }
                let (dx, dy) = if index >= 28 { (340, 150) } else { (16, 10) };
                if (i32::from(x) - i32::from(other.x().read(c))).abs() < dx
                    && (i32::from(y) - i32::from(other.y().read(c))).abs() < dy
                {
                    blocked = true;
                    break;
                }
            }
            if !blocked {
                for index in 0..2 {
                    let anchor = ram::g_level_anchor_positions(r).at(index);
                    let ax = (((i32::from(anchor.x().read(c)) + 2) * 128) as i16).wrapping_sub(16);
                    let ay = (((i32::from(anchor.y().read(c)) + 2) * 64) as i16).wrapping_sub(5);
                    if (i32::from(x) - i32::from(ax)).abs() < 340
                        && (i32::from(y) - i32::from(ay)).abs() < 150
                    {
                        blocked = true;
                        break;
                    }
                }
            }
            if !blocked
                && call(
                    c,
                    ids::EARTHLING_IS_SPECIAL_RECORD_NEARBY,
                    &[x as u32, y as u32, level as u32],
                )? == 0
            {
                found = Some((x, y));
                break;
            }
        }
        let item = items.offset(slot);
        let Some((x, y)) = found else {
            item.kind().write(c, -1);
            continue;
        };
        let terrain = call(
            c,
            ids::GET_MAP_TERRAIN_AT_WORLD_POSITION,
            &[map, x as u32, y as u32],
        )? as u16;
        let kind = if terrain == 0x20 {
            if random(c)? & 1 != 0 { 82 } else { 83 }
        } else {
            81
        };
        item.kind().write(c, kind);
        item.level().write(c, level as i8);
        item.x().write(c, x);
        item.y().write(c, y);
    }
    Ok(None)
}

/// B006: failed searches leave the particular item absent; bank ownership, seeds
/// and the remaining deterministic item sequence still finish normally.
pub(super) fn load_bank(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let r = c.revision();
    let level = args[0] as i16;
    if level >= 26 {
        return Ok(None);
    }
    for bank in 0..2 {
        let users = ram::g_item_bank_users(r).at(bank);
        if users.read(c) != 0 && i16::from(ram::g_item_bank_levels(r).at(bank).read(c)) == level {
            users.write(c, users.read(c).wrapping_add(1));
            return Ok(None);
        }
    }
    let bank = i32::from(ram::g_item_bank_users(r).at(0).read(c) != 0);
    let seed = ram::g_level_seeds(r).at_word_index(level).read(c);
    let saved_seed = call(c, ids::STACK_BASED_RANDOM, &[u32::from(seed)])?;
    let mut count =
        if ram::g_player_lives(r).at(0).read(c) < 0 || ram::g_player_lives(r).at(1).read(c) < 0 {
            12
        } else {
            16
        };
    if level > 2 {
        count += level - 2;
    }
    count = count.min(28);
    for slot in 0..i32::from(count) {
        if level == 0 || level == 1 && slot == 12 {
            break;
        }
        let position = if level == 1 {
            let position = rom::level1_present_locations(r).at(slot);
            Some((position.x().read(c), position.y().read(c)))
        } else {
            item_search(c, level, false)?
        };
        let item = ram::g_object_table(r).at(bank).at(slot);
        let Some((x, y)) = position else {
            item.kind().write(c, -1);
            continue;
        };
        let present = level == 1 || random(c)? % 100 < 50;
        let kind = if present {
            let mut kind = call(c, ids::SELECT_RANDOM_PRESENT, &[])? as i8;
            if level == 1 && slot < 4 {
                while matches!(kind, 0 | 2 | 5 | 26 | 18 | 16) {
                    kind = call(c, ids::SELECT_RANDOM_PRESENT, &[])? as i8;
                }
            }
            kind
        } else if random(c)? % 100 < 75 {
            (random(c)? % 16) as i8 + 64
        } else {
            80
        };
        let collected = ram::g_collected_level_item_bits(r)
            .at_word_index(level)
            .read(c);
        if collected & tje_mem::arithmetic::lsr_u32(0x8000_0000, slot as u32) == 0 {
            item.kind().write(c, kind);
            item.level().write(c, level as i8);
            item.x().write(c, x);
            item.y().write(c, y);
        } else {
            item.kind().write(c, -1);
        }
    }
    let items = ram::g_object_table(r).at(bank);
    if level == 0 {
        call_void(
            c,
            ids::INITIALIZE_LEVEL_ZERO_FIXED_ITEMS,
            &[items.at(28).address()],
        )?;
    } else if level != 1 {
        large_items(c, &[items.address(), level as u32])?;
    }
    let users = ram::g_item_bank_users(r).at(bank);
    users.write(c, users.read(c).wrapping_add(1));
    ram::g_item_bank_levels(r).at(bank).write(c, level as i8);
    call(c, ids::STACK_BASED_RANDOM, &[saved_seed])?;
    Ok(None)
}

/// B006: telephone search has ten deterministic placement attempts; no placement
/// leaves its old record/activation state untouched, rather than publishing junk.
pub(super) fn telephone(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let r = c.revision();
    let p = args[0] as i16 as i32;
    let player = ram::g_player_info(r).at(p);
    let other = ram::g_player_info(r).at(p ^ 1);
    let level = player.level().read(c);
    let states = ram::g_telephone_states(r);
    if states.at(p).level().read(c) == level || states.at(p ^ 1).level().read(c) == level {
        return Ok(Some(0));
    }
    for _ in 0..10 {
        let Some((x, y)) = item_search(c, i16::from(level), false)? else {
            return Ok(Some(0));
        };
        if (i32::from(player.x().read(c)) - i32::from(x)).abs() < 480
            && (i32::from(player.y().read(c)) - i32::from(y)).abs() < 336
        {
            continue;
        }
        if ram::g_player_lives(r).at(p ^ 1).read(c) >= 0
            && (i32::from(other.x().read(c)) - i32::from(x)).abs() < 320
            && (i32::from(other.y().read(c)) - i32::from(y)).abs() < 224
        {
            continue;
        }
        let phone = states.at(p);
        phone.level().write(c, level);
        phone.x().write(c, x);
        phone.y().write(c, y);
        ram::g_telephone_activated(r).at(p).write(c, 0);
        return Ok(Some(1));
    }
    Ok(Some(0))
}

/// B007: a real fourth small-item argument; the checked search owns its outputs.
/// Successful allocation keeps the original entity initialization policy.
pub(super) fn create_item(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let r = c.revision();
    let item = Address::<LevelItem>::new(args[0]);
    let allocated = call(c, ids::ALLOCATE_ENTITY, &[])? as i8;
    item.object().write(c, allocated);
    if allocated == 0 {
        if item.kind().read(c) < 81 {
            let level = i16::from(item.level().read(c));
            if let Some((x, y)) = item_search(c, level, false)? {
                item.x().write(c, x);
                item.y().write(c, y);
            }
        }
        return Ok(None);
    }
    let object = ram::g_player_info(r).at(i32::from(allocated));
    let kind = item.kind().read(c);
    let frames = if kind < 64 {
        let appearance = ram::g_present_info(r)
            .at(i32::from(kind))
            .appearance()
            .read(c);
        rom::present_sprite_table(r)
            .at(i32::from(appearance))
            .address()
    } else if kind >= 81 {
        rom::large_ground_item_frames(r)
            .at(i32::from(kind) - 81)
            .address()
    } else {
        object
            .draw_flags()
            .write(c, object.draw_flags().read(c) | 0x80);
        rom::spr_ground_item(r).at(i32::from(kind) - 64).address()
    };
    object.frames().write(c, Ptr::new(frames));
    object
        .sequence()
        .write(c, Ptr::new(rom::single_frame_hold_animation(r).address()));
    object.frame_phase().write(c, 0);
    object.frame_duration().write(c, 1);
    object.x().write(c, item.x().read(c));
    object.y().write(c, item.y().read(c));
    object.level().write(c, item.level().read(c));
    if kind < 81 {
        object.width().write(c, 24);
        object.height_pixels().write(c, 16);
        object.sprite_count().write(c, 1);
        object.tile_count().write(c, 12);
    } else {
        object.width().write(c, 32);
        object.height_pixels().write(c, 64);
        object.sprite_count().write(c, 2);
        object.tile_count().write(c, 32);
        object
            .behavior_flags()
            .write(c, object.behavior_flags().read(c) | 1);
    }
    object.tile().write(c, 0x8000);
    object
        .draw_flags()
        .write(c, object.draw_flags().read(c) | 8);
    Ok(None)
}
