use super::{PROFILE_ID, Settings};
use bincode::{Decode, Encode};
use tje_engine_runtime::enhanced::{
    Context, Continuation, EnhancedMode, EnhancedProfile, OverrideStep, ResumableTranslation,
    Sidecar,
};
use tje_game::{FunctionId, Result, Translation, call, ids};
use tje_mem::{Address, ObjectState, Revision, ram};

mod placement;
mod resources;

#[derive(Clone, Encode, Decode)]
struct Configuration {
    fixes: u32,
    identity: [u8; 4],
}
impl Sidecar for Configuration {
    const KEY: &'static str = "fixed/configuration";
    const VERSION: u32 = 1;
    fn configuration_identity(&self) -> &[u8] {
        &self.identity
    }
    fn validate(&self) -> Result<()> {
        if self.identity != self.fixes.to_le_bytes()
            || !(Settings {
                enabled: true,
                fixes: self.fixes,
            })
            .valid()
        {
            return Err(tje_game::Error::Platform(
                "invalid immutable Fixed selection".into(),
            ));
        }
        Ok(())
    }
}
fn selected(c: &dyn Context, id: &str) -> Result<bool> {
    Ok(Settings {
        enabled: true,
        fixes: c.sidecars().get::<Configuration>()?.fixes,
    }
    .contains(id))
}

const ATOMIC: &[Translation] = &[
    // B004/B007's finite exhaustive search can call this hundreds of times.
    // Reuse the unchanged readable calculator, rather than spending the
    // original-call instruction budget on its bounded rectangle scans.
    Translation {
        id: ids::TERRAIN_VALIDATION,
        function: tje_game::map::terrain_validation,
    },
    Translation {
        id: ids::SET_POSITION_ON_NEW_LEVEL,
        function: placement::player_position,
    },
    Translation {
        id: ids::MAP_FIND_ITEM_PLACEMENT_POSITION,
        function: placement::item_position,
    },
    Translation {
        id: ids::MAP_PLACE_LARGE_LEVEL_ITEMS,
        function: placement::large_items,
    },
    Translation {
        id: ids::LOAD_LEVEL_ITEM_BANK,
        function: placement::load_bank,
    },
    Translation {
        id: ids::TELEPHONE_PLACE_OUTSIDE_PLAYER_VIEWS,
        function: placement::telephone,
    },
    Translation {
        id: ids::ENTITY_CREATE_LEVEL_ITEM,
        function: placement::create_item,
    },
    Translation {
        id: ids::OPEN_EARTHLING_PRESENT,
        function: resources::earthling_present,
    },
    Translation {
        id: ids::ALLOCATE_ENTITY_VIEWPORT_SPRITES,
        function: resources::viewport_sprites,
    },
];
const RESUMABLE: &[ResumableTranslation] = &[
    ResumableTranslation {
        id: ids::PROCESS_INPUT,
        function: identity,
    },
    ResumableTranslation {
        id: ids::PLAYER_RESET_RANK_AND_RESOURCES,
        function: resources::rank_reset,
    },
    ResumableTranslation {
        id: ids::HUD_INITIALIZE_HEALTH_BAR,
        function: resources::health_bar,
    },
    ResumableTranslation {
        id: ids::PLAYER_APPLY_ROCKET_SKATES_INPUT,
        function: skates,
    },
    ResumableTranslation {
        id: ids::PLAYER_UPDATE_HEALTH_SHARING_CLAP,
        function: clap,
    },
    ResumableTranslation {
        id: ids::GENERATE_SPEECH,
        function: promotion_speaker,
    },
    ResumableTranslation {
        id: ids::START_DIALOGUE,
        function: dialogue_speaker,
    },
    ResumableTranslation {
        id: ids::SETUP_SELECTED_PRESENT,
        function: dispatch,
    },
    ResumableTranslation {
        id: ids::PROCESS_LAND,
        function: land,
    },
    ResumableTranslation {
        id: ids::PROCESS_SLEEP,
        function: sleep,
    },
    ResumableTranslation {
        id: ids::DRAW_GAME_MAP,
        function: map_entry,
    },
    ResumableTranslation {
        id: ids::DRAW_INVENTORY,
        function: inventory_entry,
    },
    ResumableTranslation {
        id: ids::TITLE_SCREEN_VIDEO,
        function: title_entry,
    },
    ResumableTranslation {
        id: ids::SEGA_VIDEO,
        function: sega_entry,
    },
    ResumableTranslation {
        id: ids::PLAYER_PROCESS_TERRAIN_MOTION,
        function: terrain_motion,
    },
];
fn fix_for(id: FunctionId) -> &'static str {
    match id {
        ids::TERRAIN_VALIDATION => "B004",
        ids::SET_POSITION_ON_NEW_LEVEL => "B005",
        ids::MAP_FIND_ITEM_PLACEMENT_POSITION
        | ids::MAP_PLACE_LARGE_LEVEL_ITEMS
        | ids::LOAD_LEVEL_ITEM_BANK
        | ids::TELEPHONE_PLACE_OUTSIDE_PLAYER_VIEWS => "B006",
        ids::ENTITY_CREATE_LEVEL_ITEM => "B007",
        ids::OPEN_EARTHLING_PRESENT => "B008",
        ids::ALLOCATE_ENTITY_VIEWPORT_SPRITES => "B011",
        ids::PLAYER_RESET_RANK_AND_RESOURCES => "B017",
        ids::HUD_INITIALIZE_HEALTH_BAR => "B018",
        ids::PLAYER_APPLY_ROCKET_SKATES_INPUT => "B001",
        ids::PLAYER_UPDATE_HEALTH_SHARING_CLAP => "B002",
        ids::GENERATE_SPEECH => "B014",
        ids::START_DIALOGUE => "B015",
        ids::SETUP_SELECTED_PRESENT => "R001",
        ids::PROCESS_LAND => "R002",
        ids::PROCESS_SLEEP => "R003",
        ids::DRAW_GAME_MAP => "R004",
        ids::DRAW_INVENTORY => "R005",
        ids::TITLE_SCREEN_VIDEO => "R006",
        ids::SEGA_VIDEO => "R007",
        ids::PLAYER_PROCESS_TERRAIN_MOTION => "R008",
        _ => "",
    }
}
pub fn new_mode(revision: Revision, settings: Settings) -> Result<EnhancedMode> {
    if !settings.valid() {
        return Err(tje_game::Error::Platform(
            "invalid Fixed fix mask (B004–B006 must be selected together)".into(),
        ));
    }
    if !settings.enabled {
        return Ok(EnhancedMode::default());
    }
    let profile = EnhancedProfile::new(PROFILE_ID, ATOMIC, &[]).with_resumable(RESUMABLE);
    let mut mode = EnhancedMode::new_selected(profile, revision, |id| {
        let fix = fix_for(id);
        id == ids::PROCESS_INPUT
            || id == ids::TERRAIN_VALIDATION && settings.contains("B007")
            || settings.contains(fix) && (!fix.starts_with('R') || revision == Revision::Rev00)
    })?;
    mode.sidecars_mut().register(Configuration {
        fixes: settings.fixes,
        identity: settings.fixes.to_le_bytes(),
    })?;
    Ok(mode)
}
fn identity(_: &mut dyn Context, _: &mut Continuation) -> Result<OverrideStep> {
    Ok(OverrideStep::ContinueOriginal)
}
fn original(
    k: &mut Continuation,
    stage: u32,
    function: FunctionId,
    arguments: Vec<u32>,
) -> OverrideStep {
    k.stage = stage;
    OverrideStep::CallOriginal {
        function,
        arguments,
    }
}
fn from(c: &mut dyn Context, caller: FunctionId) -> Result<bool> {
    let pc = c.read_u32(c.stack_pointer()?);
    let metadata = caller.metadata();
    let start = metadata.address(c.revision());
    Ok((start..start + metadata.size(c.revision())).contains(&pc))
}

/// B001: keep all original steering, then independently cap both final axes.
fn skates(c: &mut dyn Context, k: &mut Continuation) -> Result<OverrideStep> {
    if k.stage == 0 {
        let args = c.entry_arguments()?;
        k.locals[0] = args[0];
        return Ok(original(k, 1, ids::PLAYER_APPLY_ROCKET_SKATES_INPUT, args));
    }
    let player = Address::<ObjectState>::new(k.locals[0]);
    player
        .velocity_x()
        .write(c, player.velocity_x().read(c).clamp(-0x600, 0x600));
    player
        .velocity_y()
        .write(c, player.velocity_y().read(c).clamp(-0x480, 0x480));
    Ok(OverrideStep::Return(None))
}

/// B002: preserve the actual sum (including odd totals), capped by both capacities.
fn clap(c: &mut dyn Context, k: &mut Continuation) -> Result<OverrideStep> {
    let r = c.revision();
    if k.stage == 0 {
        let a = ram::g_player_info(r).at(0);
        let b = ram::g_player_info(r).at(1);
        if a.level().read(c) != b.level().read(c)
            || b.sprite_state().read(c) & !1 != 90
            || b.frame_duration().read(c) != 0
        {
            return Ok(OverrideStep::ContinueOriginal);
        }
        let health = ram::g_player_health(r);
        k.locals[0] =
            (i32::from(health.at(0).read(c)) + i32::from(health.at(1).read(c))).max(0) as u32;
        k.locals[1] = call(c, ids::PLAYER_GET_MAXIMUM_HEALTH, &[0])?;
        k.locals[2] = call(c, ids::PLAYER_GET_MAXIMUM_HEALTH, &[1])?;
        return Ok(original(
            k,
            1,
            ids::PLAYER_UPDATE_HEALTH_SHARING_CLAP,
            vec![],
        ));
    }
    let (a, b) = share_health(
        k.locals[0] as i32,
        k.locals[1] as i16 as i32,
        k.locals[2] as i16 as i32,
    );
    ram::g_player_health(r).at(0).write(c, a as i8);
    ram::g_player_health(r).at(1).write(c, b as i8);
    Ok(OverrideStep::Return(None))
}
fn share_health(total: i32, maximum_a: i32, maximum_b: i32) -> (i32, i32) {
    let a = ((total + 1) / 2).min(maximum_a);
    let b = (total - a).min(maximum_b);
    ((total - b).min(maximum_a), b)
}

/// B014: an actual promotion callsite supplies a player number, never a pointer.
/// The wrapper preserves the original fallback's allocation/rank/resource policy.
fn promotion_speaker(c: &mut dyn Context, k: &mut Continuation) -> Result<OverrideStep> {
    if k.stage != 0 {
        return Ok(OverrideStep::Return(Some(c.data_register(0))));
    }
    if !from(c, ids::PERFORM_PROMOTION)? {
        return Ok(OverrideStep::ContinueOriginal);
    }
    let mut args = c.entry_arguments()?;
    let players = ram::g_player_info(c.revision());
    args[0] = u32::from(args[0] == players.at(1).address());
    Ok(original(k, 1, ids::GENERATE_SPEECH, args))
}

/// B015: explicit reunion policy: ToeJam initiates generic/ToeJam reunion,
/// Earl initiates Earl's reunion. Collection uses the collecting player.
fn dialogue_speaker(c: &mut dyn Context, k: &mut Continuation) -> Result<OverrideStep> {
    if k.stage != 0 {
        return Ok(OverrideStep::Return(Some(c.data_register(0))));
    }
    let pc = c.read_u32(c.stack_pointer()?);
    let r = c.revision();
    let mut args = c.entry_arguments()?;
    if pc == ids::UPDATE_SPEECH_AND_DIALOGUE.metadata().address(r) + 0x1de {
        args[1] = u32::from(args[0] as i16 == 4);
    } else if pc == ids::SHIP_PIECE_ABILITY.metadata().address(r) + 0x86 {
        args[1] = c.data_register(2) & 0xffff;
    } else {
        return Ok(OverrideStep::ContinueOriginal);
    }
    Ok(original(k, 1, ids::START_DIALOGUE, args))
}

/// R001: REV 02's two conditions, using REV 00's own data and original effects.
fn dispatch(c: &mut dyn Context, k: &mut Continuation) -> Result<OverrideStep> {
    let r = c.revision();
    if k.stage == 0 {
        let args = c.entry_arguments()?;
        let present = args[2] as i16;
        if present >= 8 {
            return Ok(OverrideStep::ContinueOriginal);
        }
        k.locals[0] = args[0];
        k.locals[1] = args[1];
        k.locals[2] = args[2];
        let p = args[1] as i16 as i32;
        if present == 5 && ram::g_player_fall_states(r).at(p).read(c) != 0 {
            let points = ram::g_player_points(r).at(p);
            points.write(c, points.read(c).wrapping_add(2));
            ram::g_player_powerups(r).at(p).write(c, 5);
            return Ok(original(
                k,
                2,
                ids::HUD_LOAD_ACTION_ICON_SPRITE,
                vec![args[1]],
            ));
        }
        if present == 5 {
            return Ok(OverrideStep::ContinueOriginal);
        }
        return Ok(original(k, 1, ids::SETUP_SELECTED_PRESENT, args));
    }
    if k.stage == 2 {
        ram::g_player_powerup_timers(r)
            .at(k.locals[1] as i16 as i32)
            .write(c, 250);
        return Ok(OverrideStep::Return(None));
    }
    let player = Address::<ObjectState>::new(k.locals[0]);
    if k.stage == 1 {
        let mask = player.animation_group_mask().read(c);
        if mask & 0x0800_0000 == 0 || mask & 0x2000_0000 != 0 {
            return Ok(OverrideStep::Return(None));
        }
        return Ok(original(
            k,
            3,
            ids::GET_ACTIVE_PRESENT_SPRITE,
            vec![player.address()],
        ));
    }
    if k.stage == 3 {
        let sprite = c.data_register(0) as i8;
        if sprite == -1 {
            return Ok(OverrideStep::Return(None));
        }
        return Ok(original(
            k,
            4,
            ids::SET_SPRITE,
            vec![player.address(), sprite as u32],
        ));
    }
    player.velocity_x().write(c, 0);
    player.velocity_y().write(c, 0);
    Ok(OverrideStep::Return(None))
}

/// R002: retain source ordering and the shared-view damage-player quirk; change
/// only REV 02's two whirlwind exclusions, not unrelated land behaviour.
fn land(c: &mut dyn Context, k: &mut Continuation) -> Result<OverrideStep> {
    let r = c.revision();
    loop {
        let p = k.locals[0] as i32;
        if p == 2 {
            return Ok(OverrideStep::Return(None));
        }
        let object = ram::g_player_info(r).at(p);
        match k.stage {
            0 => {
                let row = i32::from((object.y().read(c) >> 6).wrapping_sub(1));
                let col = i32::from((object.x().read(c) >> 7).wrapping_sub(1));
                let camera = i32::from(p == 1 && ram::g_split_screen_enabled(r).read(c) != 0);
                let map = if camera == 0 {
                    ram::g_player1_level_map(r)
                } else {
                    ram::g_player2_level_map(r)
                };
                if ram::g_player_lives(r).at(p).read(c) < 0
                    || object.height().read(c) > 4
                    || object.level().read(c) <= 1
                    || ram::g_player_elevator_flags(r).at(p).read(c) != 0
                    || !(0..26).contains(&row)
                    || !(0..19).contains(&col)
                    || map.at(row).at(col).read(c) & 0x4000 == 0
                {
                    k.locals[0] += 1;
                    continue;
                }
                k.locals[1] = camera as u32;
                k.locals[2] = row as u32;
                k.locals[3] = col as u32;
                return Ok(original(
                    k,
                    1,
                    ids::PLAYER_IS_NEAR_TERRAIN_EDGE,
                    vec![p as u32],
                ));
            }
            1 => {
                if c.data_register(0) == 0 {
                    k.locals[0] += 1;
                    k.stage = 0;
                    continue;
                }
                return Ok(original(
                    k,
                    2,
                    ids::MAP_ANIMATE_MARKED_LAND_PATHS,
                    vec![k.locals[1], k.locals[2], k.locals[3]],
                ));
            }
            2 => {
                let level = if k.locals[1] == 0 {
                    ram::g_player1_map_level(r)
                } else {
                    ram::g_player2_map_level(r)
                }
                .read(c);
                let damage_player = if p == 1 && k.locals[1] == 0 { 0 } else { p };
                return Ok(original(
                    k,
                    3,
                    ids::EARTHLING_DAMAGE_ON_HAZARD_TERRAIN,
                    vec![damage_player as u32, level as u32],
                ));
            }
            3 => {
                return Ok(original(
                    k,
                    4,
                    ids::UPDATE_LAND_AROUND_PLAYER,
                    vec![p as u32],
                ));
            }
            4 => {
                let terrain = if p == 0 {
                    ram::g_player1_terrain_underfoot(r)
                } else {
                    ram::g_player2_terrain_underfoot(r)
                }
                .read(c);
                if terrain == 0x90
                    && object.height().read(c) == 0
                    && !matches!(object.sprite_state().read(c), 78 | 79)
                    && object.animation_group_mask().read(c) & 0x8020 == 0
                {
                    return Ok(original(k, 5, ids::STOP_PLAYER, vec![object.address()]));
                }
                k.locals[0] += 1;
                k.stage = 0;
            }
            5 => {
                return Ok(original(
                    k,
                    6,
                    ids::PLAYER_START_FALLING,
                    vec![object.address(), p as u32],
                ));
            }
            _ => {
                k.locals[0] += 1;
                k.stage = 0;
            }
        }
    }
}

/// R003: REV 02 resets idle rather than progressing sleep at a choice prompt.
fn sleep(c: &mut dyn Context, k: &mut Continuation) -> Result<OverrideStep> {
    let r = c.revision();
    if k.stage == 0 {
        let player = Address::<ObjectState>::new(c.entry_arguments()?[0]);
        let p = i32::from(player.entity_index().read(c));
        if player.sprite_state().read(c) != 0
            || player.level().read(c) == 26
            || ram::g_player_elevator_flags(r).at(p).read(c) != 0
            || ram::g_player_interaction_choice_active(r).at(p).read(c) == 0
        {
            return Ok(OverrideStep::ContinueOriginal);
        }
        k.locals[0] = p as u32;
        let handle = ram::g_player_wake_effect_entity_handles(r).at(p).read(c);
        if handle != 0 {
            return Ok(original(k, 1, ids::FREE_ENTITY, vec![handle as u32]));
        }
    }
    let p = k.locals[0] as i32;
    ram::g_player_wake_effect_entity_handles(r)
        .at(p)
        .write(c, 0);
    ram::g_player_idle_ticks(r).at(p).write(c, 0);
    Ok(OverrideStep::Return(None))
}
fn idle_entry(c: &mut dyn Context) -> Result<OverrideStep> {
    let p = c.entry_arguments()?[0] as i16 as i32;
    ram::g_player_idle_ticks(c.revision()).at(p).write(c, 0);
    Ok(OverrideStep::ContinueOriginal)
}
/// R004: once, at map entry; never continuously suppress idle time.
fn map_entry(c: &mut dyn Context, _: &mut Continuation) -> Result<OverrideStep> {
    idle_entry(c)
}
/// R005: once, at inventory entry.
fn inventory_entry(c: &mut dyn Context, _: &mut Continuation) -> Result<OverrideStep> {
    idle_entry(c)
}
/// R006: a fresh title deadline, using this profile's explicit entry-reset policy.
fn title_entry(c: &mut dyn Context, _: &mut Continuation) -> Result<OverrideStep> {
    ram::g_frame_counter(c.revision()).write(c, 0);
    ram::g_scene_last_music_event_time(c.revision()).write(c, 0);
    Ok(OverrideStep::ContinueOriginal)
}
/// R007: reset before the bounded Sega setup so the 120-tick fade cannot wrap.
fn sega_entry(c: &mut dyn Context, _: &mut Continuation) -> Result<OverrideStep> {
    ram::g_frame_counter(c.revision()).write(c, 0);
    Ok(OverrideStep::ContinueOriginal)
}
/// R008: REV 02 adds PLAYER_GROUP_LEAN (bit 4) to the pause mask.
fn terrain_motion(c: &mut dyn Context, _: &mut Continuation) -> Result<OverrideStep> {
    let p = c.entry_arguments()?[0] as i16 as i32;
    if ram::g_player_info(c.revision())
        .at(p)
        .animation_group_mask()
        .read(c)
        & 0x10
        != 0
    {
        Ok(OverrideStep::Return(None))
    } else {
        Ok(OverrideStep::ContinueOriginal)
    }
}

#[cfg(test)]
mod tests;
