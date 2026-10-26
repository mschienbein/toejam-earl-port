//! Rules layered over one aggregate owner; no second simulation or render clock.
use crate::publication::resources::ResourceCatalog;
use std::fmt;
use tje_four_player::input::InputFrame;
use tje_mem::AddressSpace;
use tje_simulation::{
    PublicationStamp,
    population::director::{DecisionIntent, StepReceipt},
    resources::PresentCatalog,
    session::{Session, SessionConfig, SessionStorage},
    world_bridge::ControlRemaps,
};

#[derive(Debug)]
pub enum Error {
    Game(tje_game::Error),
    Simulation(tje_simulation::Error),
    World(tje_world::Error),
    Player(tje_simulation::players::PlayerError),
    Publication(crate::publication::Error),
    Resource(crate::publication::resources::ResourceError),
    Admission(&'static str),
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Game(error) => error.fmt(f),
            Self::Simulation(error) => error.fmt(f),
            Self::World(error) => error.fmt(f),
            Self::Player(error) => error.fmt(f),
            Self::Publication(error) => error.fmt(f),
            Self::Resource(error) => error.fmt(f),
            Self::Admission(reason) => write!(f, "mode-owned admission: {reason}"),
        }
    }
}
impl std::error::Error for Error {}
impl From<tje_game::Error> for Error {
    fn from(error: tje_game::Error) -> Self {
        Self::Game(error)
    }
}
impl From<tje_simulation::Error> for Error {
    fn from(error: tje_simulation::Error) -> Self {
        Self::Simulation(error)
    }
}
impl From<tje_world::Error> for Error {
    fn from(error: tje_world::Error) -> Self {
        Self::World(error)
    }
}
impl From<tje_simulation::players::PlayerError> for Error {
    fn from(error: tje_simulation::players::PlayerError) -> Self {
        Self::Player(error)
    }
}
impl From<crate::publication::Error> for Error {
    fn from(error: crate::publication::Error) -> Self {
        Self::Publication(error)
    }
}
impl From<crate::publication::resources::ResourceError> for Error {
    fn from(error: crate::publication::resources::ResourceError) -> Self {
        Self::Resource(error)
    }
}
pub type Result<T> = std::result::Result<T, Error>;

/// Borrowed immutable services; neither a second world nor an original RAM view
/// of the four canonical heroes is manufactured for a mode rule.
pub struct Services<'a> {
    pub graphics: &'a ResourceCatalog,
    pub presents: &'a PresentCatalog,
    pub remaps: &'a ControlRemaps,
    pub memory: &'a AddressSpace,
}
/// Mode-specific rules borrow their enclosing owner's state disjointly from the
/// shared session. Clocked hooks are not presentation callbacks.
pub trait ModeRules {
    fn validate(&self, config: &SessionConfig) -> Result<()>;
    fn paused(&self, session: &Session) -> bool;
    fn voluntary_scales(&self, session: &Session) -> [tje_simulation::players::VoluntaryScale; 4];
    fn input(
        &mut self,
        session: &mut Session,
        input: InputFrame,
        services: &Services<'_>,
    ) -> Result<()>;
    fn before_population(
        &mut self,
        session: &mut Session,
        stamp: PublicationStamp,
        paused: bool,
        services: &Services<'_>,
    ) -> Result<Option<DecisionIntent>>;
    fn after_population(
        &mut self,
        session: &mut Session,
        stamp: PublicationStamp,
        receipt: &StepReceipt,
        services: &Services<'_>,
    ) -> Result<()>;
    fn after_movement(
        &mut self,
        session: &mut Session,
        stamp: PublicationStamp,
        paused: bool,
        services: &Services<'_>,
    ) -> Result<()>;
    fn after_frame(
        &mut self,
        session: &mut Session,
        input: InputFrame,
        hardware_frame: u64,
        logical_video_tick: u64,
        paused: bool,
        services: &Services<'_>,
    ) -> Result<()>;
}
pub trait KernelStorage: SessionStorage {
    const PROFILE_ID: &'static str;
    fn queue_input(
        &mut self,
        input: InputFrame,
        assignment: Option<&tje_simulation::recording::Assignment>,
    ) -> Result<bool>;
    fn with_rules<T>(
        &mut self,
        run: impl FnOnce(&mut Session, &mut dyn ModeRules) -> Result<T>,
    ) -> Result<T>;
}
struct CoopRules;
impl ModeRules for CoopRules {
    fn validate(&self, config: &SessionConfig) -> Result<()> {
        if config.roles.director.is_some()
            || config.roles.heroes != tje_simulation::SeatMask::ALL
            || config.roles.inputs != config.roles.heroes
            || config.roles.local_owners != config.roles.heroes
            || config.generator != tje_world::GeneratorId::OriginalV1
            || !config.world.faithful()
        {
            return Err(Error::Admission(
                "stock local co-op requires four local heroes and OriginalV1",
            ));
        }
        Ok(())
    }
    fn paused(&self, session: &Session) -> bool {
        session.services.pause_agreed(&session.players)
    }
    fn voluntary_scales(&self, _: &Session) -> [tje_simulation::players::VoluntaryScale; 4] {
        [tje_simulation::players::VoluntaryScale::Normal; 4]
    }
    // All co-op roles are heroes; the common dispatcher owns their input.
    fn input(&mut self, _: &mut Session, _: InputFrame, _: &Services<'_>) -> Result<()> {
        Ok(())
    }
    fn before_population(
        &mut self,
        _: &mut Session,
        _: PublicationStamp,
        _: bool,
        _: &Services<'_>,
    ) -> Result<Option<DecisionIntent>> {
        Ok(None)
    }
    fn after_population(
        &mut self,
        _: &mut Session,
        _: PublicationStamp,
        _: &StepReceipt,
        _: &Services<'_>,
    ) -> Result<()> {
        Ok(())
    }
    fn after_movement(
        &mut self,
        _: &mut Session,
        _: PublicationStamp,
        _: bool,
        _: &Services<'_>,
    ) -> Result<()> {
        Ok(())
    }
    fn after_frame(
        &mut self,
        _: &mut Session,
        _: InputFrame,
        _: u64,
        _: u64,
        _: bool,
        _: &Services<'_>,
    ) -> Result<()> {
        Ok(())
    }
}
impl KernelStorage for Session {
    const PROFILE_ID: &'static str = "tje/coop-original-v1/1";
    fn queue_input(
        &mut self,
        input: InputFrame,
        assignment: Option<&tje_simulation::recording::Assignment>,
    ) -> Result<bool> {
        Session::queue_input(self, input, assignment).map_err(Error::from)
    }
    fn with_rules<T>(
        &mut self,
        run: impl FnOnce(&mut Session, &mut dyn ModeRules) -> Result<T>,
    ) -> Result<T> {
        run(self, &mut CoopRules)
    }
}
