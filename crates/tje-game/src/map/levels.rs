use crate::call_frame::{CallSite, FrameLayout, LocalFrame, OutgoingArguments};
use crate::{Context, Result, call, call_at, ids};
use tje_mem::{arithmetic::expr_div_u32, ram, rom};

/// G/src/map/level_generation_and_changes.c:536 (`ChangeLevel`).
/// Rejection intentionally returns without restoring the exchanged RNG state.
pub fn change_level(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    // Both matching c_level_generation_and_changes.s:2302-2303.
    let frame = LocalFrame::enter(c, FrameLayout::link(8, 0x7c, 0x1c))?;
    let r = c.revision();
    let player = args[0] as i16;
    let requested = ram::g_requested_map_level(r);
    let seed = ram::g_level_seeds(r)
        .at(i32::from(requested.read(c)))
        .read(c);
    let saved = call(c, ids::STACK_BASED_RANDOM, &[u32::from(seed)])?;
    frame.local::<u32>(-8).write(c, saved);
    let count = if requested.read(c) > 1 {
        let count = call(c, ids::MAP_GENERATE_SEPARATED_ANCHORS, &[])? as i16;
        frame.local::<i16>(-2).write(c, count);
        Some(count)
    } else {
        None
    };
    let first = ram::g_player1_map_level(r);
    let second = ram::g_player2_map_level(r);
    let stored = ram::g_saved_map_level(r);
    let mut loaded = false;
    if player == 0 {
        if first.read(c) == requested.read(c) {
            loaded = true;
        } else {
            if second.read(c) == requested.read(c) {
                if first.read(c) != 0 {
                    call(c, ids::MAP_SWAP_PLAYER_WITH_SAVED_BANK, &[0])?;
                }
                call(c, ids::COPY_LEVEL_BETWEEN_PLAYERS, &[1])?;
                loaded = true;
            } else if stored.read(c) == requested.read(c) {
                call(c, ids::MAP_SWAP_PLAYER_WITH_SAVED_BANK, &[0])?;
                loaded = true;
            } else if first.read(c) > 1 {
                call(c, ids::MAP_SWAP_PLAYER_WITH_SAVED_BANK, &[0])?;
            }
            first.write(c, requested.read(c));
        }
    } else if second.read(c) == requested.read(c) {
        loaded = true;
    } else {
        if first.read(c) == requested.read(c) {
            if first.read(c) != 0 {
                call(c, ids::MAP_SWAP_PLAYER_WITH_SAVED_BANK, &[1])?;
            }
            call(c, ids::COPY_LEVEL_BETWEEN_PLAYERS, &[0])?;
            loaded = true;
        } else if stored.read(c) == requested.read(c) {
            call(c, ids::MAP_SWAP_PLAYER_WITH_SAVED_BANK, &[1])?;
            loaded = true;
        } else {
            if second.read(c) > 1 {
                call(c, ids::MAP_SWAP_PLAYER_WITH_SAVED_BANK, &[1])?;
            }
            call(c, ids::MAP_SWAP_PLAYER_BANKS, &[])?;
        }
        second.write(c, requested.read(c));
    }
    let map = ram::g_player1_level_map(r);
    if !loaded && requested.read(c) <= 1 {
        if requested.read(c) == 0 {
            for row in 0..26 {
                for column in 0..19 {
                    map.at(row).at(column).write(c, 34);
                }
            }
            for row in 0..5 {
                for column in 0..4 {
                    map.at(row + 12).at(column + 9).write(
                        c,
                        u16::from(rom::level0_map_data(r).at(row).at(column).read(c)),
                    );
                }
            }
        } else {
            for row in 0..26 {
                for column in 0..19 {
                    map.at(row).at(column).write(
                        c,
                        u16::from(rom::level1_map_data(r).at(row).at(column).read(c)),
                    );
                }
            }
        }
        if player == 1 {
            call(c, ids::MAP_SWAP_PLAYER_BANKS, &[])?;
        }
        loaded = true;
    }
    if loaded {
        call(c, ids::STACK_BASED_RANDOM, &[saved])?;
        frame.leave(c)?;
        return Ok(Some(0));
    }
    // Every <=1 path above loaded a fixed map and returned. The original's
    // uninitialized position_count is therefore never consumed on those paths.
    let count = count.expect("procedural generation is reached only above level one");
    super::waits::wait_pending_byte(c, ram::g_sprite_graphics_upload_pending(r).cast())?;
    let highest = ram::g_highest_level(r);
    if requested.read(c) > highest.read(c) {
        highest.write(c, requested.read(c));
    }
    for row in 0..26 {
        for column in 0..19 {
            map.at(row).at(column).write(c, 0);
            let directions = ram::g_map_path_direction_banks(r).at(row).at(column);
            directions.write(c, directions.read(c) & 0xf0);
        }
    }
    // Both linked listings retain this count slot through the arithmetic at
    // 0x4c96, releasing it at 0x4c9c rather than immediately after Connect.
    let connected = OutgoingArguments::push(c, &[i32::from(count) as u32])?;
    let dispatcher = c.dispatcher();
    connected.call(
        c,
        dispatcher,
        ids::MAP_CONNECT_ANCHOR_POSITIONS,
        CallSite::new(0x4c74),
    )?;
    let mut filled = call_at(
        c,
        ids::MAP_GET_RANDOM_GENERATION_PARAMETER,
        &[3],
        CallSite::new(0x4c7e),
    )? as u16;
    filled = filled.wrapping_mul(19).wrapping_mul(26);
    connected.release(c)?;
    filled /= 100;
    c.set_data_register(2, u32::from(filled));
    c.set_data_register(
        4,
        (c.data_register(4) & 0xffff_0000) | u32::from(player as u16),
    );
    for position in 0..i32::from(count) {
        let quotient = expr_div_u32(u32::from(filled), i32::from(count) as u32)?;
        let minimum = (position as u32 + 1).wrapping_mul(quotient);
        let anchor = ram::g_level_anchor_positions(r).at(position);
        let args = [
            0x1000,
            position as u32,
            minimum,
            i32::from(anchor.y().read(c)) as u32,
            i32::from(anchor.x().read(c)) as u32,
        ];
        c.set_data_register(6, (c.data_register(6) & 0xffff_0000) | position as u32);
        // Both linked listings: JSR 0x4d08, return 0x4d0e.
        call_at(
            c,
            ids::MAP_GROW_TERRAIN_REGION,
            &args,
            CallSite::new(0x4d0e),
        )?;
    }
    call(c, ids::MAP_STABILIZE_REGION_CONNECTIONS, &[])?;
    if call(c, ids::MAP_IS_ANCHOR_CONNECTION_MISSING, &[])? != 0 {
        frame.leave(c)?;
        return Ok(Some(1));
    }
    // External JSR.L at 0x4d2e, both revisions. Seed now owns a real frame.
    call_at(
        c,
        ids::MAP_SEED_FILLED_TERRAIN_REGIONS,
        &[],
        CallSite::new(0x4d34),
    )?;
    // The region-count slot remains on the original stack until 0x4d58,
    // including the framed MapChooseMetatileAlternatives call.
    let outgoing = OutgoingArguments::push(c, &[i32::from(count) as u32])?;
    let dispatcher = c.dispatcher();
    outgoing.call(
        c,
        dispatcher,
        ids::MAP_BUILD_REGION_EDGE_SHAPES,
        CallSite::new(0x4d42),
    )?;
    for (id, pc) in [
        (ids::MAP_BUILD_FILLED_TERRAIN_SHAPES, 0x4d46),
        (ids::MAP_BUILD_DIRECTIONAL_PATH_SHAPES, 0x4d4a),
        (ids::MAP_SCATTER_TERRAIN_FEATURES, 0x4d4e),
        (ids::MAP_ASSIGN_TRANSITION_CHAIN, 0x4d54),
        (ids::MAP_CHOOSE_METATILE_ALTERNATIVES, 0x4d58),
    ] {
        call_at(c, id, &[], CallSite::new(pc))?;
    }
    outgoing.release(c)?;
    if player == 1 {
        call(c, ids::MAP_SWAP_PLAYER_BANKS, &[])?;
    }
    call(c, ids::STACK_BASED_RANDOM, &[saved])?;
    frame.leave(c)?;
    Ok(Some(0))
}

/// G/src/map/level_generation_and_changes.c:656 (`MapLoadPlayerLevel`).
pub fn map_load_player_level(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    // Both matching listings: MOVEM D2-D4/A2, no LINK/local allocation.
    let frame = LocalFrame::enter(c, FrameLayout::movem(0, 0x1c, 0x04))?;
    let r = c.revision();
    let mut player = args[0] as i16;
    let level = args[1] as i16;
    if level >= 26 {
        if i16::from(ram::g_player1_map_level(r).read(c)) != level {
            call(c, ids::LOAD_FINAL_LEVEL, &[i32::from(level) as u32])?;
        }
        frame.leave(c)?;
        return Ok(None);
    }
    if ram::g_player_lives(r).at(0).read(c) < 0 {
        player = 0;
    }
    ram::g_requested_map_level(r).write(c, level as i8);
    while call_at(
        c,
        ids::CHANGE_LEVEL,
        &[i32::from(player) as u32],
        CallSite::new(0x4e72),
    )? != 0
    {
        let seed = super::random_generation::random(c)? as u16;
        // The original shifts the short level in a word before ADDA.W.
        ram::g_level_seeds(r)
            .cast::<u16>()
            .byte_offset(i32::from(level.wrapping_mul(2)))
            .write(c, seed);
        if player == 0 {
            ram::g_player1_map_level(r).write(c, -1);
        } else {
            ram::g_player2_map_level(r).write(c, -1);
        }
    }
    frame.leave(c)?;
    Ok(None)
}
