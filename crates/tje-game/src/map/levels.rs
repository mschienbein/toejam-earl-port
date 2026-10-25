use crate::call_frame::{CallSite, FrameLayout, LocalFrame, OutgoingArguments};
use crate::{Context, Result, call_at, call_at_void, ids};
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
    let saved = call_at(
        c,
        ids::STACK_BASED_RANDOM,
        &[u32::from(seed)],
        CallSite::new(0x4a54),
    )?;
    frame.local::<u32>(-8).write(c, saved);
    if requested.read(c) > 1 {
        let count = call_at(
            c,
            ids::MAP_GENERATE_SEPARATED_ANCHORS,
            &[],
            CallSite::new(0x4a64),
        )? as i16;
        frame.local::<i16>(-2).write(c, count);
    }
    let first = ram::g_player1_map_level(r);
    let second = ram::g_player2_map_level(r);
    let stored = ram::g_saved_map_level(r);
    let mut loaded = false;
    if player == 0 {
        if requested.read(c) == first.read(c) {
            loaded = true;
        } else {
            if requested.read(c) == second.read(c) {
                if first.read(c) != 0 {
                    call_at_void(
                        c,
                        ids::MAP_SWAP_PLAYER_WITH_SAVED_BANK,
                        &[0],
                        CallSite::new(0x4a8a),
                    )?;
                }
                call_at_void(
                    c,
                    ids::COPY_LEVEL_BETWEEN_PLAYERS,
                    &[1],
                    CallSite::new(0x4a94),
                )?;
                loaded = true;
            } else if requested.read(c) == stored.read(c) {
                call_at_void(
                    c,
                    ids::MAP_SWAP_PLAYER_WITH_SAVED_BANK,
                    &[0],
                    CallSite::new(0x4aa8),
                )?;
                loaded = true;
            } else if first.read(c) > 1 {
                call_at_void(
                    c,
                    ids::MAP_SWAP_PLAYER_WITH_SAVED_BANK,
                    &[0],
                    CallSite::new(0x4ab4),
                )?;
            }
            first.write(c, requested.read(c));
        }
    } else if requested.read(c) == second.read(c) {
        loaded = true;
    } else {
        if requested.read(c) == first.read(c) {
            if first.read(c) != 0 {
                call_at_void(
                    c,
                    ids::MAP_SWAP_PLAYER_WITH_SAVED_BANK,
                    &[1],
                    CallSite::new(0x4ad6),
                )?;
            }
            call_at_void(
                c,
                ids::COPY_LEVEL_BETWEEN_PLAYERS,
                &[0],
                CallSite::new(0x4ade),
            )?;
            loaded = true;
        } else if requested.read(c) == stored.read(c) {
            call_at_void(
                c,
                ids::MAP_SWAP_PLAYER_WITH_SAVED_BANK,
                &[1],
                CallSite::new(0x4af4),
            )?;
            loaded = true;
        } else {
            if second.read(c) > 1 {
                call_at_void(
                    c,
                    ids::MAP_SWAP_PLAYER_WITH_SAVED_BANK,
                    &[1],
                    CallSite::new(0x4b06),
                )?;
            }
            call_at_void(c, ids::MAP_SWAP_PLAYER_BANKS, &[], CallSite::new(0x4b0c))?;
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
            call_at_void(c, ids::MAP_SWAP_PLAYER_BANKS, &[], CallSite::new(0x4bfe))?;
        }
        loaded = true;
    }
    if loaded {
        let saved = frame.local::<u32>(-8).read(c);
        call_at(c, ids::STACK_BASED_RANDOM, &[saved], CallSite::new(0x4d6e))?;
        frame.leave(c)?;
        return Ok(Some(0));
    }
    // G c_level_generation_and_changes.s:2505-2506: one fixed-byte TST/BNE
    // loop. The RAM predicate owns every poll, including the final zero.
    super::waits::wait_pending_byte(c, ram::g_sprite_graphics_upload_pending(r).cast())?;
    let highest = ram::g_highest_level(r);
    if requested.read(c) > highest.read(c) {
        highest.write(c, requested.read(c));
    }
    for row in 0..26 {
        for column in 0..19 {
            // G c_level_generation_and_changes.s:2522, CLR.W pre-read.
            c.discard_u16(map.at(row).at(column).address())?;
            map.at(row).at(column).write(c, 0);
            let directions = ram::g_map_path_direction_banks(r).at(row).at(column);
            directions.write(c, directions.read(c) & 0xf0);
        }
    }
    // Both linked listings retain this count slot through the arithmetic at
    // 0x4c96, releasing it at 0x4c9c rather than immediately after Connect.
    let count = frame.local::<i16>(-2).read(c);
    let connected = OutgoingArguments::push(c, &[i32::from(count) as u32])?;
    let dispatcher = c.dispatcher();
    connected.call_void(
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
    c.set_data_register(0, 26);
    c.set_data_register(1, u32::from(filled) * 19);
    filled = call_at(c, ids::LMULT, &[], CallSite::new(0x4c9c))? as u16;
    connected.release(c)?;
    filled /= 100;
    c.set_data_register(2, u32::from(filled));
    c.set_data_register(
        4,
        (c.data_register(4) & 0xffff_0000) | u32::from(player as u16),
    );
    let mut position = 0_i16;
    while position < frame.local::<i16>(-2).read(c) {
        // The emitted pushes read anchor X, then Y, then the divisor slot.
        let anchor = ram::g_level_anchor_positions(r).at(i32::from(position));
        let x = i32::from(anchor.x().read(c)) as u32;
        OutgoingArguments::push(c, &[x])?;
        let y = i32::from(anchor.y().read(c)) as u32;
        OutgoingArguments::push(c, &[y])?;
        let divisor = frame.local::<i16>(-2).read(c);
        let quotient = expr_div_u32(u32::from(filled), i32::from(divisor) as u32)?;
        c.set_data_register(0, position as u32 + 1);
        c.set_data_register(1, quotient);
        let minimum = call_at(c, ids::LMULT, &[], CallSite::new(0x4cfa))?;
        OutgoingArguments::push(c, &[0x1000, position as u32, minimum])?;
        let region = OutgoingArguments::existing(c, 5)?;
        c.set_data_register(6, (c.data_register(6) & 0xffff_0000) | position as u32);
        // Both linked listings: JSR 0x4d08, return 0x4d0e.
        region.call_void(
            c,
            dispatcher,
            ids::MAP_GROW_TERRAIN_REGION,
            CallSite::new(0x4d0e),
        )?;
        position = position.wrapping_add(1);
        region.release(c)?;
    }
    call_at_void(
        c,
        ids::MAP_STABILIZE_REGION_CONNECTIONS,
        &[],
        CallSite::new(0x4d20),
    )?;
    if call_at(
        c,
        ids::MAP_IS_ANCHOR_CONNECTION_MISSING,
        &[],
        CallSite::new(0x4d26),
    )? != 0
    {
        frame.leave(c)?;
        return Ok(Some(1));
    }
    // External JSR.L at 0x4d2e, both revisions. Seed now owns a real frame.
    call_at_void(
        c,
        ids::MAP_SEED_FILLED_TERRAIN_REGIONS,
        &[],
        CallSite::new(0x4d34),
    )?;
    // The region-count slot remains on the original stack until 0x4d58,
    // including the framed MapChooseMetatileAlternatives call.
    let count = frame.local::<i16>(-2).read(c);
    let outgoing = OutgoingArguments::push(c, &[i32::from(count) as u32])?;
    let dispatcher = c.dispatcher();
    outgoing.call_void(
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
        call_at_void(c, id, &[], CallSite::new(pc))?;
    }
    outgoing.release(c)?;
    if player == 1 {
        call_at_void(c, ids::MAP_SWAP_PLAYER_BANKS, &[], CallSite::new(0x4d64))?;
    }
    let saved = frame.local::<u32>(-8).read(c);
    call_at(c, ids::STACK_BASED_RANDOM, &[saved], CallSite::new(0x4d6e))?;
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
            call_at_void(
                c,
                ids::LOAD_FINAL_LEVEL,
                &[i32::from(level) as u32],
                CallSite::new(0x4e26),
            )?;
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
        let seed = call_at(c, ids::GET_RANDOM_NUMBER, &[], CallSite::new(0x4e4c))? as u16;
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
