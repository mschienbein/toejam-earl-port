use super::*;
use genesis_config::GenesisController;
use tje_engine_runtime::enhanced::{
    Context, Continuation, EnhancedMode, EnhancedProfile, OverrideStep, ResumableTranslation,
    Sidecar,
};
use tje_game::{FunctionId, Result, ids};

impl Sidecar for State {
    const KEY: &'static str = "jam-out-in-play";
    const VERSION: u32 = 1;

    fn validate(&self) -> Result<()> {
        if !self.settings.valid()
            || !self.requested.valid()
            || self.applied_backing > 6
            || self.pcm_cooldown > 6
            || self.suspended > 32
            || self.seats.iter().any(|seat| {
                seat.chord_ticks > 10
                    || seat.timer > 6
                    || !(-1..=127).contains(&seat.frame)
                    || seat.suppressed & !ACTIONS != 0
                    || !(-1..=127).contains(&seat.displayed_frame)
                    || seat.previous & START != 0
            })
        {
            return Err(tje_game::Error::Platform(
                "invalid Jam Out sidecar state".into(),
            ));
        }
        Ok(())
    }
}

static PROFILE_SETS: &[&[ResumableTranslation]] = &[CORE_OVERRIDES, MODAL_OVERRIDES];
pub const PROFILE: EnhancedProfile =
    EnhancedProfile::new("jam-out-in-play/1", &[], &[]).with_resumable_sets(PROFILE_SETS);

/// Kept separate from modal hooks so the combined extras profile can own one
/// shared scene wrapper rather than register two replacements for the same PC.
pub static CORE_OVERRIDES: &[ResumableTranslation] = &[
    ResumableTranslation {
        id: ids::GET_PLAYER_INPUT,
        function: get_input,
    },
    ResumableTranslation {
        id: ids::PROCESS_PLAYERS,
        function: process_players,
    },
    ResumableTranslation {
        id: ids::UPDATE_AND_QUEUE_ENTITY_SPRITES,
        function: animate,
    },
    ResumableTranslation {
        id: ids::MUSIC_UPDATE_SEQUENCER,
        function: music,
    },
    ResumableTranslation {
        id: ids::MUSIC_SELECT_SONG_TEMPLATE_BANK,
        function: select_song,
    },
    ResumableTranslation {
        id: ids::MUSIC_LOAD_SONG_CHANNEL_TEMPLATES,
        function: load_song,
    },
];

macro_rules! modal_callbacks {
    ($(($name:ident, $id:ident)),* $(,)?) => {
        pub static MODAL_OVERRIDES: &[ResumableTranslation] = &[
            $(ResumableTranslation { id: ids::$id, function: $name }),*
        ];
        $(fn $name(context: &mut dyn Context, _: &mut Continuation) -> Result<OverrideStep> {
            suspend(context)?;
            Ok(OverrideStep::ContinueOriginal)
        })*
    };
}
modal_callbacks!(
    (pause, GAME_PAUSED),
    (map, DRAW_GAME_MAP),
    (elevator, ELEVATOR_BEGIN_RIDE_SCENE),
    (menu, NEW_GAME_MAIN_MENU),
    (ending, PLAY_ENDING_SEQUENCE),
    (ship, SHOW_SHIP_PIECES),
    (ship_completion, PLAY_SHIP_COMPLETION_SEQUENCE),
    (game_over, SHOW_GAME_OVER_AND_RESET),
);

pub fn configure(mode: &mut EnhancedMode, settings: Settings) -> Result<()> {
    if !settings.valid() {
        return Err(tje_game::Error::Platform(
            "Jam backing must be 0..=6".into(),
        ));
    }
    if let Ok(state) = mode.sidecars_mut().get_mut::<State>() {
        state.requested = settings;
        Ok(())
    } else {
        mode.sidecars_mut().register(State::new(settings))
    }
}

pub fn presentation<B: Bus + ?Sized>(
    mode: &EnhancedMode,
    bus: &B,
    revision: Revision,
) -> [Option<Presentation>; 2] {
    mode.sidecars()
        .get::<State>()
        .map_or([None; 2], |state| state.presentation(bus, revision))
}

/// Shared authored-scene wrappers may call these even without a Jam sidecar.
/// Suspension cancels rather than saving/restoring a gameplay pose or velocity.
pub fn suspend(context: &mut dyn Context) -> Result<()> {
    if let Ok(state) = context.sidecars_mut().get_mut::<State>() {
        state.suspended = state.suspended.saturating_add(1).min(32);
        state.in_play = false;
        state.cancel();
        state.applied_backing = 0;
    }
    Ok(())
}
pub fn resume(context: &mut dyn Context) -> Result<()> {
    if let Ok(state) = context.sidecars_mut().get_mut::<State>() {
        state.suspended = state.suspended.saturating_sub(1);
        state.cancel();
        state.applied_backing = 0;
    }
    Ok(())
}

/// The ordinary input phase recovers from both returning authored scenes and
/// reset-bearing original tail paths; it does not restore a previous pose.
pub fn resume_world(context: &mut dyn Context) -> Result<()> {
    if let Ok(state) = context.sidecars_mut().get_mut::<State>() {
        if state.suspended != 0 {
            state.cancel();
            state.suspended = 0;
            state.applied_backing = 0;
        }
        state.in_play = true;
    }
    Ok(())
}

fn original(
    context: &mut dyn Context,
    continuation: &mut Continuation,
    function: FunctionId,
) -> Result<OverrideStep> {
    continuation.stage = 1;
    Ok(OverrideStep::CallOriginal {
        function,
        arguments: context.entry_arguments()?,
    })
}

fn get_input(context: &mut dyn Context, continuation: &mut Continuation) -> Result<OverrideStep> {
    if continuation.stage == 0 {
        resume_world(context)?;
        return original(context, continuation, ids::GET_PLAYER_INPUT);
    }
    let revision = context.revision();
    let pads = context.inputs().pads;
    let swapped = ram::g_controller_ports_swapped(revision).read(context) != 0;
    let settings = {
        let state = context.sidecars_mut().get_mut::<State>()?;
        state.apply_requested();
        state.in_play = true;
        state.settings
    };
    for seat in 0..2 {
        let safe = eligible(context, revision, seat as i32);
        let input = ram::g_player_processed_input(revision).at(seat as i32);
        let raw = input.read(context);
        let modifier = match pads[seat ^ usize::from(swapped)] {
            GenesisController::ThreeButton(pad) | GenesisController::SixButton(pad) => pad.mode,
            _ => false,
        };
        let consumed = context.sidecars_mut().get_mut::<State>()?.seats[seat]
            .input(raw, modifier, safe, settings);
        if raw != consumed {
            input.write(context, consumed);
        }
    }
    Ok(OverrideStep::Return(None))
}

fn process_players(
    context: &mut dyn Context,
    continuation: &mut Continuation,
) -> Result<OverrideStep> {
    if continuation.stage == 0 {
        return original(context, continuation, ids::PROCESS_PLAYERS);
    }
    let revision = context.revision();
    for seat in 0..2 {
        if !eligible(context, revision, seat as i32) {
            context.sidecars_mut().get_mut::<State>()?.seats[seat].cancel();
        }
    }
    Ok(OverrideStep::Return(None))
}

fn animate(context: &mut dyn Context, continuation: &mut Continuation) -> Result<OverrideStep> {
    if continuation.stage == 0 {
        return original(context, continuation, ids::UPDATE_AND_QUEUE_ENTITY_SPRITES);
    }
    let revision = context.revision();
    // Take one tiny fixed record at a time; no mutable sidecar borrow overlaps
    // ROM reads and no gameplay record is repurposed as animation scratch RAM.
    for seat in 0..2 {
        let mut pose = context.sidecars().get::<State>()?.seats[seat];
        if !eligible(context, revision, seat as i32) {
            pose.cancel();
        } else {
            pose.animate(context, revision);
        }
        context.sidecars_mut().get_mut::<State>()?.seats[seat] = pose;
    }
    Ok(OverrideStep::Return(None))
}

fn owns_music(context: &dyn Context) -> Result<bool> {
    let state = context.sidecars().get::<State>()?;
    let revision = context.revision();
    let requested = ram::g_requested_music_selection(revision).read(context);
    Ok(state.in_play
        && state.suspended == 0
        && ram::g_v_blank_func(revision).read(context).address() == 0
        && ram::g_v_blank_graphics_enabled(revision).read(context) != 0
        && ram::g_v_blank_graphics_paused(revision).read(context) == 0
        && ram::g_boom_box_music_template_hold(revision).read(context) == 0
        && (0..6).contains(&requested))
}

/// One arbitration at the original service slot. Original effects and active
/// PCM/PSG/FM ownership always win; losers are dropped, never queued as latency.
fn audio_slot(context: &mut dyn Context, selection: bool) -> Result<()> {
    let frame = context.frame_count();
    let owner = owns_music(context)?;
    let revision = context.revision();
    let pcm_busy = ram::g_pending_pcm_sample(revision).read(context) != 0
        || ram::g_pcm_effect_cooldown_ticks(revision).read(context) != 0;
    let psg_busy = ram::g_pending_psg_effect(revision).read(context) >= 0
        || ram::g_psg_effect_status(revision).read(context) != 0;
    let original_idle = !pcm_busy && !psg_busy;
    // MIN is the Jam-only request priority; every original request outranks it.
    // A still-playing Jam sample must not masquerade as an essential game cue
    // and starve Earl's independent FM route after alternating pad polling.
    let normal_pcm_busy =
        pcm_busy && ram::g_sound_effect_priority(revision).read(context) != i16::MIN;
    let fm_idle = !normal_pcm_busy
        && !psg_busy
        && ram::g_pending_fm_effect(revision).read(context) < 0
        && ram::g_fm_effect_template_status(revision).read(context) == 0;
    let requests = context.sidecars_mut().get_mut::<State>()?.arbitrate(
        frame,
        owner && !selection,
        original_idle,
        fm_idle,
    );
    if let Some(sample) = requests[0].sample(revision) {
        tje_game::audio::play_sound_effect(context, &[sample, i16::MIN as i32 as u32, 0])?;
    }
    if let Some(effect) = requests[1].fm_effect() {
        tje_game::audio::audio_request_channel_template_effect(context, &[effect])?;
    }
    Ok(())
}

fn music(context: &mut dyn Context, continuation: &mut Continuation) -> Result<OverrideStep> {
    if continuation.stage != 0 {
        return Ok(OverrideStep::Return(None));
    }
    let owner = owns_music(context)?;
    let state = context.sidecars().get::<State>()?;
    let desired = if owner && state.requested.enabled {
        state.requested.backing
    } else {
        0
    };
    let change = owner && state.applied_backing != desired;
    audio_slot(context, change)?;
    continuation.stage = 1;
    if change {
        // A bank load consumes this service slot, exactly like the original
        // VBlank selection branch: never call the sequencer a second time.
        let revision = context.revision();
        Ok(OverrideStep::CallOriginal {
            function: ids::MUSIC_SELECT_SONG_TEMPLATE_BANK,
            arguments: vec![ram::g_requested_music_selection(revision).read(context) as i32 as u32],
        })
    } else {
        original(context, continuation, ids::MUSIC_UPDATE_SEQUENCER)
    }
}
fn select_song(context: &mut dyn Context, continuation: &mut Continuation) -> Result<OverrideStep> {
    if continuation.stage == 0 {
        audio_slot(context, true)?;
        original(context, continuation, ids::MUSIC_SELECT_SONG_TEMPLATE_BANK)
    } else {
        Ok(OverrideStep::Return(None))
    }
}
fn load_song(context: &mut dyn Context, continuation: &mut Continuation) -> Result<OverrideStep> {
    if continuation.stage != 0 {
        return Ok(OverrideStep::Return(None));
    }
    let owner = owns_music(context)?;
    let mut arguments = context.entry_arguments()?;
    let state = context.sidecars_mut().get_mut::<State>()?;
    state.apply_requested();
    let backing = if owner && state.settings.enabled {
        state.settings.backing
    } else {
        0
    };
    if owner {
        state.applied_backing = backing;
        if backing != 0
            && arguments
                .first()
                .is_some_and(|value| matches!(*value as u16, 0..=5 | 7..=12))
        {
            arguments[0] = u32::from(backing) + 6;
        }
    } else {
        state.applied_backing = 0;
    }
    continuation.stage = 1;
    Ok(OverrideStep::CallOriginal {
        function: ids::MUSIC_LOAD_SONG_CHANNEL_TEMPLATES,
        arguments,
    })
}
