use crate::{Context, Result, call, ids};
use tje_mem::{Address, ObjectState, ram, rom};

/// G/src/player/edge_hop_and_clap.c:15 (`PlayerCheckTerrainEdgeHop`).
pub(super) fn player_check_terrain_edge_hop(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let state = Address::<ObjectState>::new(arguments[0]);
    let revision = context.revision();
    let terrain = if state.entity_index().read(context) == 0 {
        ram::g_player1_nearby_terrain(revision)
    } else {
        ram::g_player2_nearby_terrain(revision)
    }
    .cast::<u8>();
    let cell_x =
        ((arguments[1] as i16 >> 3) as i32 - i32::from(state.x().read(context) >> 3) + 2) as i16;
    let cell_y =
        ((arguments[2] as i16 >> 3) as i32 - i32::from(state.y().read(context) >> 3) + 2) as i16;
    let tile = terrain
        .offset(i32::from(cell_y) * 5 + i32::from(cell_x))
        .read(context);
    if tile >= 0x80 {
        state.velocity_height().write(context, 512);
        state.acceleration_height().write(context, -37);
        state
            .draw_flags()
            .write(context, state.draw_flags().read(context) | 0x40);
        let sprite = (state.sprite_state().read(context) & 3) + 112;
        call(context, ids::SET_SPRITE, &[state.address(), sprite as u32])?;
        return Ok(Some(1));
    }
    if ram::g_pcm_effect_cooldown_ticks(revision).read(context) == 0 {
        if ram::g_terrain_edge_meep_pending(revision).read(context) != 0 {
            call(
                context,
                ids::PLAY_SOUND_EFFECT,
                &[rom::sfx_meep(revision).address(), 40, 0],
            )?;
            ram::g_terrain_edge_meep_pending(revision).write(context, 0);
        } else if call(context, ids::GET_RANDOM_NUMBER, &[])? & 0x1ff == 0 {
            call(
                context,
                ids::PLAY_SOUND_EFFECT,
                &[rom::sfx_meep(revision).address(), 40, 0],
            )?;
            ram::g_terrain_edge_meep_pending(revision).write(context, 1);
        }
    }
    Ok(Some(0))
}

/// G/build/{rev02,rev00}/asm/c_edge_hop_and_clap.s:194-198.
/// SUB.W's immediate BLT uses N xor V; the later CMPI.W observes the
/// truncated difference. Keep these distinct: positive overflow passes both.
#[inline]
fn clap_vertical_range(earl_y: i16, toejam_y: i16) -> bool {
    earl_y >= toejam_y && earl_y.wrapping_sub(toejam_y) <= 6
}

/// G/src/player/edge_hop_and_clap.c:60 (`PlayerUpdateHealthSharingClap`).
/// ORIGINAL BUG: the above-maximum branch assigns the average to the compared
/// player, rather than capping it; the partner subtraction still uses the limit.
/// The initial vertical-range SUB.W/BLT preserves overflow before a separate
/// signed-word comparison; see `clap_vertical_range` and review finding RH01.
pub(super) fn player_update_health_sharing_clap(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let toejam = ram::g_player_info(revision).at(0);
    let earl = ram::g_player_info(revision).at(1);
    if toejam.level().read(context) != earl.level().read(context) {
        return Ok(None);
    }
    if toejam.sprite_state().read(context) & 0xfe_u8 as i8 == 90 {
        if toejam.frame_duration().read(context) == 0 {
            call(context, ids::SET_SPRITE, &[toejam.address(), 0])?;
        } else if toejam.frame().read(context) == 2 {
            call(
                context,
                ids::PLAY_SOUND_EFFECT,
                &[rom::sfx_jam_clap(revision).address(), 95, 1],
            )?;
            toejam
                .frame()
                .write(context, toejam.frame().read(context).wrapping_add(1));
        }
    }
    if earl.sprite_state().read(context) & 0xfe_u8 as i8 == 90 {
        if earl.frame_duration().read(context) == 0 {
            call(context, ids::SET_SPRITE, &[earl.address(), 0])?;
            let health = ram::g_player_health(revision);
            let average = ((i32::from(health.at(0).read(context))
                + i32::from(health.at(1).read(context))
                + 1)
                >> 1) as i16;
            if i32::from(average)
                > call(context, ids::PLAYER_GET_MAXIMUM_HEALTH, &[0])? as i16 as i32
            {
                health.at(0).write(context, average as i8);
                let limit = call(context, ids::PLAYER_GET_MAXIMUM_HEALTH, &[0])? as i16;
                health.at(1).write(
                    context,
                    ((i32::from(average) << 1) - i32::from(limit)) as i8,
                );
            } else if i32::from(average)
                > call(context, ids::PLAYER_GET_MAXIMUM_HEALTH, &[1])? as i16 as i32
            {
                health.at(1).write(context, average as i8);
                let limit = call(context, ids::PLAYER_GET_MAXIMUM_HEALTH, &[1])? as i16;
                health.at(0).write(
                    context,
                    ((i32::from(average) << 1) - i32::from(limit)) as i8,
                );
            } else {
                health.at(0).write(context, average as i8);
                health.at(1).write(context, average as i8);
            }
        } else if earl.frame().read(context) == 2 {
            earl.frame()
                .write(context, earl.frame().read(context).wrapping_add(1));
        }
        return Ok(None);
    }
    let earl_y = earl.y().read(context);
    let toejam_y = toejam.y().read(context);
    if !clap_vertical_range(earl_y, toejam_y) {
        return Ok(None);
    }
    let separation_x = earl
        .x()
        .read(context)
        .wrapping_sub(toejam.x().read(context));
    if toejam.sprite_state().read(context) == 11 && earl.sprite_state().read(context) == 10 {
        if separation_x > 0 && separation_x < 25 {
            call(context, ids::SET_SPRITE, &[toejam.address(), 91])?;
            toejam
                .x()
                .write(context, earl.x().read(context).wrapping_sub(23));
            toejam
                .y()
                .write(context, earl.y().read(context).wrapping_sub(3));
            call(context, ids::SET_SPRITE, &[earl.address(), 90])?;
            toejam.height().write(context, 0);
            earl.height().write(context, 0);
        }
    } else if earl.sprite_state().read(context) == 11
        && toejam.sprite_state().read(context) == 10
        && separation_x < 0
        && separation_x > -25
    {
        call(context, ids::SET_SPRITE, &[toejam.address(), 90])?;
        toejam
            .x()
            .write(context, earl.x().read(context).wrapping_add(23));
        toejam
            .y()
            .write(context, earl.y().read(context).wrapping_sub(3));
        call(context, ids::SET_SPRITE, &[earl.address(), 91])?;
        toejam.height().write(context, 0);
        earl.height().write(context, 0);
    }
    Ok(None)
}

#[cfg(test)]
mod vertical_range_tests {
    use super::clap_vertical_range;
    use tje_mem::Revision;

    fn original_subw_blt(left: i16, right: i16) -> bool {
        let result = left.wrapping_sub(right);
        let overflow = ((left ^ right) & (left ^ result)) < 0;
        (result < 0) != overflow
    }

    fn original_vertical_range(earl: i16, toejam: i16) -> bool {
        let difference = earl.wrapping_sub(toejam);
        !original_subw_blt(earl, toejam) && (difference == 6 || original_subw_blt(difference, 6))
    }

    #[test]
    fn rh01_counterexamples_keep_the_two_distinct_signed_tests() {
        for revision in [Revision::Rev02, Revision::Rev00] {
            assert!(
                !clap_vertical_range(i16::MIN, i16::MAX),
                "{revision:?}: negative overflow rejects despite stored +1"
            );
            assert!(
                clap_vertical_range(i16::MAX, i16::MIN),
                "{revision:?}: positive overflow accepts stored -1"
            );
        }
    }

    #[test]
    fn every_y_operand_and_every_wrapped_result_matches_original_n_z_v() {
        for revision in [Revision::Rev02, Revision::Rev00] {
            // Exhaust every value of each operand, sweeping the opposite
            // operand at signed endpoints/zero and every threshold neighbour.
            for value in i16::MIN..=i16::MAX {
                for anchor in [
                    i16::MIN,
                    i16::MIN + 1,
                    -7,
                    -1,
                    0,
                    1,
                    6,
                    7,
                    i16::MAX - 1,
                    i16::MAX,
                ] {
                    for (earl, toejam) in [(value, anchor), (anchor, value)] {
                        assert_eq!(
                            clap_vertical_range(earl, toejam),
                            original_vertical_range(earl, toejam),
                            "{revision:?}: earl={earl} toejam={toejam}"
                        );
                    }
                }
                // Exhaust every representable word difference with V clear.
                assert_eq!(
                    clap_vertical_range(value, 0),
                    original_vertical_range(value, 0),
                    "{revision:?}: ordinary difference={value}"
                );
                // Also exhaust every possible overflowed word difference.
                // Difference zero cannot overflow for two signed-word inputs.
                if value < 0 {
                    let toejam = (i32::from(i16::MAX) - (i32::from(value) + 65536)) as i16;
                    assert_eq!(
                        clap_vertical_range(i16::MAX, toejam),
                        original_vertical_range(i16::MAX, toejam),
                        "{revision:?}: positive-overflow difference={value}"
                    );
                } else if value > 0 {
                    let toejam = (i32::from(i16::MIN) - (i32::from(value) - 65536)) as i16;
                    assert_eq!(
                        clap_vertical_range(i16::MIN, toejam),
                        original_vertical_range(i16::MIN, toejam),
                        "{revision:?}: negative-overflow difference={value}"
                    );
                }
            }
        }
    }
}
