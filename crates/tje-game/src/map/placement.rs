use crate::call_frame::{CallSite, FrameLayout, LocalFrame};
use crate::{Context, Result, call_at, ids};
use tje_mem::{Address, ram};

fn placement_site(c: &dyn Context, offset: u32) -> CallSite {
    CallSite::relative(ids::SET_POSITION_ON_NEW_LEVEL, c.revision(), offset)
}

fn random_at(c: &mut dyn Context, offset: u32) -> Result<u16> {
    let site = placement_site(c, offset);
    Ok(call_at(c, ids::GET_RANDOM_NUMBER, &[], site)? as u16)
}

// G matching c_terrain_placement.s:398-477,539-618 in both revisions.
// TerrainValidation writes the actual A6-relative outputs; a failed call leaves
// their old bytes intact. Each elevator query short-circuits in source order.
fn try_position(
    c: &mut dyn Context,
    frame: &LocalFrame,
    player: i32,
    level: i8,
    cell: (i16, i16),
    exhaustive: bool,
) -> Result<bool> {
    let x = frame.local::<i16>(-16);
    let y = frame.local::<i16>(-14);
    let validation = placement_site(c, if exhaustive { 0x28a } else { 0x136 });
    // MCC68K evaluates these argument loads in reverse push order.
    let height = i32::from(frame.local::<i16>(-8).read(c)) as u32;
    let width = i32::from(frame.local::<i16>(-6).read(c)) as u32;
    let metatile = u32::from(frame.local::<u16>(-2).read(c));
    let arguments = [metatile, width, height, x.address(), y.address(), 3];
    if call_at(c, ids::TERRAIN_VALIDATION, &arguments, validation)? == 0 {
        return Ok(false);
    }
    y.write(
        c,
        y.read(c)
            .wrapping_add(((i32::from(cell.0) + 1) * 64) as i16),
    );
    x.write(
        c,
        x.read(c)
            .wrapping_add(((i32::from(cell.1) + 1) * 128) as i16),
    );
    let query_offsets = if exhaustive {
        [0x2d6, 0x2fc, 0x320]
    } else {
        [0x182, 0x1a8, 0x1ce]
    };
    if player == 2 {
        for (delta, offset) in [(-32, query_offsets[0]), (32, query_offsets[1])] {
            let world_y = i32::from(y.read(c)) as u32;
            let world_x = (i32::from(x.read(c)) + delta) as u32;
            let arguments = [world_x, world_y, i32::from(level) as u32];
            let site = placement_site(c, offset);
            if call_at(c, ids::ELEVATOR_CONTAINS_POSITION, &arguments, site)? != 0 {
                return Ok(false);
            }
        }
        Ok(true)
    } else {
        let world_y = i32::from(y.read(c)) as u32;
        let world_x = i32::from(x.read(c)) as u32;
        let arguments = [world_x, world_y, i32::from(level) as u32];
        let site = placement_site(c, query_offsets[2]);
        Ok(call_at(c, ids::ELEVATOR_CONTAINS_POSITION, &arguments, site)? == 0)
    }
}

/// G/src/map/terrain_placement.c:80 (`SetPositionOnNewLevel`).
/// Matching c_terrain_placement.s:290-291 (both revisions): LINK A6,#-16,
/// MOVEM D2-D7/A2-A5. Failed exhaustive searches deliberately consume the old
/// coordinate words at A6-16/A6-14; an all-marked map retains nontermination.
/// Return-PC offsets below are from the matching linked listings in both ROMs.
pub fn set_position_on_new_level(c: &mut dyn Context) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(c, FrameLayout::link(16, 0xfc, 0x3c))?;
    let r = c.revision();
    // Matching listing:292, MOVE.L 8(A6),D2 precedes the height-local store.
    let mut player = frame.argument_slot(0).read(c) as i32;
    frame.local::<i16>(-8).write(c, 2);
    if ram::g_player_lives(r).at(0).read(c) < 0 {
        player = 1;
    } else if ram::g_player_lives(r).at(1).read(c) < 0 {
        player = 0;
    }
    c.set_data_register(2, player as u32);
    c.set_address_register(2, ids::ELEVATOR_CONTAINS_POSITION.metadata().address(r));
    c.set_address_register(3, ids::GET_RANDOM_NUMBER.metadata().address(r));
    c.set_address_register(4, ram::g_player_info(r).address());
    let level = if player == 2 {
        frame.local::<i16>(-6).write(c, 12);
        ram::g_player_info(r).at(0).level().read(c)
    } else {
        frame.local::<i16>(-6).write(c, 4);
        ram::g_player_info(r).at(player).level().read(c)
    };
    c.set_data_register(
        4,
        (c.data_register(4) & 0xffff_ff00) | u32::from(level as u8),
    );
    if level <= 1 {
        frame.local::<i16>(-6).write(c, 16);
        frame.local::<i16>(-8).write(c, 8);
    }
    let map = if player == 1 && ram::g_player_lives(r).at(0).read(c) >= 0 {
        ram::g_player2_level_map(r)
    } else {
        ram::g_player1_level_map(r)
    };
    frame.local::<u32>(-12).write(c, map.address());
    let mut attempts = 0_i16;
    let mut found = false;
    while attempts < 10 {
        c.set_data_register(
            6,
            (c.data_register(6) & 0xffff_0000) | u32::from(attempts as u16),
        );
        let row = (random_at(c, 0x94)? % 26) as i16;
        c.set_data_register(
            5,
            (c.data_register(5) & 0xffff_0000) | u32::from(row as u16),
        );
        let column = (random_at(c, 0xb0)? % 19) as i16;
        c.set_data_register(
            3,
            (c.data_register(3) & 0xffff_0000) | u32::from(column as u16),
        );
        let current_map = Address::<u16>::new(frame.local::<u32>(-12).read(c));
        c.set_address_register(5, current_map.address());
        let metatile = current_map.offset(i32::from(row) * 19 + i32::from(column));
        frame.local::<u16>(-2).write(c, metatile.read(c));
        if frame.local::<u16>(-2).read(c) & 0x4000 != 0 {
            // The decrement and loop increment cancel, so marked cells do not
            // consume any of the ten attempts, including on an all-marked map.
            continue;
        }
        if try_position(c, &frame, player, level, (row, column), false)? {
            found = true;
            break;
        }
        attempts = attempts.wrapping_add(1);
    }
    if !found {
        let first_row = (random_at(c, 0x1e4)? % 26) as i16;
        let first_column = (random_at(c, 0x200)? % 19) as i16;
        frame.local::<i16>(-4).write(c, first_column);
        let mut row = first_row;
        'rows: loop {
            let mut column = frame.local::<i16>(-4).read(c);
            loop {
                let current_map = Address::<u16>::new(frame.local::<u32>(-12).read(c));
                let metatile = current_map.offset(i32::from(row) * 19 + i32::from(column));
                frame.local::<u16>(-2).write(c, metatile.read(c));
                if frame.local::<u16>(-2).read(c) & 0x4000 == 0
                    && try_position(c, &frame, player, level, (row, column), true)?
                {
                    break 'rows;
                }
                column = ((i32::from(column) + 1) % 19) as i16;
                if column == frame.local::<i16>(-4).read(c) {
                    break;
                }
            }
            row = ((i32::from(row) + 1) % 26) as i16;
            if row == first_row {
                break;
            }
        }
    }
    if player == 0 || player == 2 {
        let object = ram::g_player_info(r).at(0);
        object.y().write(c, frame.local::<i16>(-14).read(c));
        object.x().write(c, frame.local::<i16>(-16).read(c));
    }
    if player == 1 || player == 2 {
        let object = ram::g_player_info(r).at(1);
        object.y().write(c, frame.local::<i16>(-14).read(c));
        object.x().write(c, frame.local::<i16>(-16).read(c));
        if player == 2 {
            object.x().write(c, object.x().read(c).wrapping_add(64));
        }
    }
    frame.leave(c)?;
    Ok(None)
}
