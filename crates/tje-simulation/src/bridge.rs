//! The original relative schedule expressed as bounded typed service calls.
use crate::{Admission, Error, Phase, PublicationStamp, Result, session::Session};
use tje_four_player::input::InputFrame;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    FollowCameras,
    Land,
    Publish,
    PrepareViews,
    TerrainCaches,
    Input,
    Players,
    Shadows,
    Population,
    Elevators,
    Items,
    TomatoRain,
    Movement,
    ViewMembership,
    AnimationMotion,
    Speech,
}
const PRESENTATION: &[Operation] = &[
    Operation::FollowCameras,
    Operation::Land,
    Operation::Publish,
];
const PLAYERS: &[Operation] = &[
    Operation::PrepareViews,
    Operation::TerrainCaches,
    Operation::Input,
    Operation::Players,
    Operation::Shadows,
    Operation::Movement,
    Operation::ViewMembership,
    Operation::AnimationMotion,
    Operation::Speech,
];
const POPULATION: &[Operation] = &[
    Operation::PrepareViews,
    Operation::Population,
    Operation::Elevators,
    Operation::Items,
    Operation::TomatoRain,
    Operation::Movement,
    Operation::ViewMembership,
    Operation::AnimationMotion,
    Operation::Speech,
];

pub const fn operations(phase: Phase) -> &'static [Operation] {
    match phase {
        Phase::Players => PLAYERS,
        Phase::Population => POPULATION,
        Phase::PresentationA | Phase::PresentationB => PRESENTATION,
    }
}

/// Each operation must terminate after bounded deterministic work. Waiting and
/// original audio/device RTS calls belong to the enhanced resumable scheduler,
/// never a synchronous drain inside an operation. supports is a pure preflight.
pub trait PhaseServices {
    fn supports(&self, operation: Operation) -> bool;
    fn execute(
        &mut self,
        operation: Operation,
        session: &mut Session,
        stamp: PublicationStamp,
    ) -> Result<()>;
}

/// Reentry cannot replay service effects. A service failure latches the session,
/// matching the enhanced driver's no-fallback/no-retry execution contract.
pub fn run_phase(
    session: &mut Session,
    services: &mut impl PhaseServices,
    phase: Phase,
    input: InputFrame,
) -> Result<Admission> {
    if session.failed() {
        return Err(Error::State("mode-owned execution is latched failed"));
    }
    if operations(phase)
        .iter()
        .any(|&operation| !services.supports(operation))
    {
        return Err(Error::Ownership(
            "phase has an unimplemented service operation",
        ));
    }
    session.validate_inputs(&input)?;
    let admission = session.clock.admit(phase, input)?;
    let Admission::Step(stamp) = admission else {
        return Ok(admission);
    };
    for &operation in operations(phase) {
        if let Err(error) = services.execute(operation, session, stamp) {
            session.fail();
            return Err(error);
        }
    }
    if phase.publishes()
        && let Err(error) = session.publish(stamp)
    {
        session.fail();
        return Err(error);
    }
    Ok(admission)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_relative_service_order_is_explicit() {
        assert_eq!(
            operations(Phase::PresentationA),
            operations(Phase::PresentationB)
        );
        assert_eq!(
            operations(Phase::Players),
            &[
                Operation::PrepareViews,
                Operation::TerrainCaches,
                Operation::Input,
                Operation::Players,
                Operation::Shadows,
                Operation::Movement,
                Operation::ViewMembership,
                Operation::AnimationMotion,
                Operation::Speech
            ]
        );
        assert_eq!(
            operations(Phase::Population),
            &[
                Operation::PrepareViews,
                Operation::Population,
                Operation::Elevators,
                Operation::Items,
                Operation::TomatoRain,
                Operation::Movement,
                Operation::ViewMembership,
                Operation::AnimationMotion,
                Operation::Speech
            ]
        );
        assert_eq!(
            operations(Phase::PresentationA),
            &[
                Operation::FollowCameras,
                Operation::Land,
                Operation::Publish
            ]
        );
    }
}
