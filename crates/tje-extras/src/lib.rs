//! Shared selection keeps native and browser extras on the same deterministic
//! profile, publication sidecar and modal ownership rules. Nothing is automatic.
#![forbid(unsafe_code)]

pub mod fixed;
use tje_extended_view::{ExperimentalWidth, RenderError, ViewMode};

pub fn view_mode(width: u16) -> Result<ViewMode, RenderError> {
    match width {
        0 | 320 => Ok(ViewMode::Faithful),
        400 => Ok(ViewMode::Widescreen),
        width => Ok(ViewMode::Experimental(ExperimentalWidth::new(width)?)),
    }
}

#[cfg(feature = "enhanced-mode")]
mod enhanced {
    use tje_engine_runtime::enhanced::{
        Context, Continuation, EnhancedMode, EnhancedProfile, OverrideStep, ResumableTranslation,
    };
    use tje_extended_view::enhanced as view;
    use tje_game::{Result, ids};
    use tje_jam_out::Settings;
    use tje_mem::Revision;

    static WORLD_OVERRIDES: &[ResumableTranslation] = &[ResumableTranslation {
        id: ids::PROCESS_INPUT,
        function: world,
    }];
    fn world(context: &mut dyn Context, _: &mut Continuation) -> Result<OverrideStep> {
        view::mark_world(context)?;
        tje_jam_out::resume_world(context)?;
        Ok(OverrideStep::ContinueOriginal)
    }
    fn authored(context: &mut dyn Context, _: &mut Continuation) -> Result<OverrideStep> {
        view::mark_authored(context)?;
        tje_jam_out::suspend(context)?;
        Ok(OverrideStep::ContinueOriginal)
    }
    // Before-only hooks tolerate original reset/nonlocal tails. The next normal
    // ProcessInput explicitly reclaims world/Jam ownership; custom VBlank scenes
    // also remain protected by their hardware publication state.
    static SCENE_OVERRIDES: &[ResumableTranslation] = &[
        ResumableTranslation {
            id: ids::DRAW_GAME_MAP,
            function: authored,
        },
        ResumableTranslation {
            id: ids::GAME_PAUSED,
            function: authored,
        },
        ResumableTranslation {
            id: ids::NEW_GAME_MAIN_MENU,
            function: authored,
        },
        ResumableTranslation {
            id: ids::TITLE_SCREEN_VIDEO,
            function: authored,
        },
        ResumableTranslation {
            id: ids::SEGA_AND_INTRO_VIDEO_ROUTINE,
            function: authored,
        },
        ResumableTranslation {
            id: ids::PLAY_ENDING_SEQUENCE,
            function: authored,
        },
        ResumableTranslation {
            id: ids::PLAY_SHIP_COMPLETION_SEQUENCE,
            function: authored,
        },
        ResumableTranslation {
            id: ids::SHOW_SHIP_PIECES,
            function: authored,
        },
        ResumableTranslation {
            id: ids::ELEVATOR_BEGIN_RIDE_SCENE,
            function: authored,
        },
        ResumableTranslation {
            id: ids::ENTER_SPLIT_SCREEN,
            function: authored,
        },
        ResumableTranslation {
            id: ids::ENTER_SHARED_SCREEN,
            function: authored,
        },
        ResumableTranslation {
            id: ids::SHOW_GAME_OVER_AND_RESET,
            function: authored,
        },
    ];
    static COMBINED: &[&[ResumableTranslation]] = &[
        view::CORE_OVERRIDES,
        WORLD_OVERRIDES,
        SCENE_OVERRIDES,
        tje_jam_out::CORE_OVERRIDES,
    ];
    fn combined_id(width: u16) -> &'static str {
        match width {
            320 => "jam-out-native-presentation/2",
            400 => "extended-view-400+jam/2",
            408 => "experimental-extended-view-408+jam/2",
            416 => "experimental-extended-view-416+jam/2",
            424 => "experimental-extended-view-424+jam/2",
            432 => "experimental-extended-view-432+jam/2",
            440 => "experimental-extended-view-440+jam/2",
            448 => "experimental-extended-view-448+jam/2",
            456 => "experimental-extended-view-456+jam/2",
            464 => "experimental-extended-view-464+jam/2",
            472 => "experimental-extended-view-472+jam/2",
            480 => "experimental-extended-view-480+jam/2",
            488 => "experimental-extended-view-488+jam/2",
            496 => "experimental-extended-view-496+jam/2",
            504 => "experimental-extended-view-504+jam/2",
            512 => "experimental-extended-view-512+jam/2",
            _ => unreachable!("validated profile width"),
        }
    }
    pub fn new_mode(revision: Revision, width: u16, jam: Settings) -> Result<EnhancedMode> {
        if width == 0 && !jam.enabled {
            return Ok(EnhancedMode::default());
        }
        super::view_mode(width).map_err(|cause| tje_game::Error::Platform(cause.to_string()))?;
        let width = if width == 0 { 320 } else { width };
        let mut profile: EnhancedProfile = view::profile(width)
            .ok_or_else(|| tje_game::Error::Platform("unsupported enhanced view width".into()))?;
        if jam.enabled {
            profile.id = combined_id(width);
            profile = profile.with_resumable_sets(COMBINED);
        }
        let mut mode = EnhancedMode::new(profile, revision)?;
        view::configure(&mut mode, width)?;
        if jam.enabled {
            tje_jam_out::configure(&mut mode, jam)?;
        }
        Ok(mode)
    }
}
#[cfg(feature = "enhanced-mode")]
pub use enhanced::new_mode;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_explicit_widths_select_extra_world_coverage() {
        assert_eq!(view_mode(0).unwrap(), ViewMode::Faithful);
        assert_eq!(view_mode(400).unwrap(), ViewMode::Widescreen);
        for width in (408..=512).step_by(8) {
            assert_eq!(view_mode(width).unwrap().width(), usize::from(width));
        }
        for width in [1, 399, 401, 407, 409, 520] {
            assert!(view_mode(width).is_err());
        }
    }
    #[cfg(feature = "enhanced-mode")]
    #[test]
    fn combined_profiles_have_unique_width_identity_and_registered_state() {
        use tje_engine_runtime::enhanced::EnhancedMode;
        for revision in tje_mem::Revision::ALL {
            assert_eq!(
                new_mode(revision, 0, Default::default())
                    .unwrap()
                    .profile_id(),
                "faithful"
            );
            for width in [0, 400, 408, 512] {
                let jam = tje_jam_out::Settings {
                    enabled: true,
                    ..Default::default()
                };
                let mode: EnhancedMode = new_mode(revision, width, jam).unwrap();
                assert_ne!(mode.profile_id(), "faithful");
                assert_eq!(
                    mode.sidecars()
                        .get::<tje_extended_view::presentation::PublishedView>()
                        .unwrap()
                        .width,
                    if width == 0 { 320 } else { width }
                );
                assert!(mode.sidecars().get::<tje_jam_out::State>().is_ok());
            }
        }
    }
}
