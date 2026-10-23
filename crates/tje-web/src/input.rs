use genesis_config::{GenesisButton, GenesisController, GenesisControllerType, GenesisInputs};

// This wire order is shared with web/input.js. High/unknown bits are ignored.
const BUTTONS: [GenesisButton; 8] = [
    GenesisButton::Up,
    GenesisButton::Down,
    GenesisButton::Left,
    GenesisButton::Right,
    GenesisButton::A,
    GenesisButton::B,
    GenesisButton::C,
    GenesisButton::Start,
];

pub fn inputs(p1: u8, p2: u8, p2_connected: bool) -> GenesisInputs {
    let controller = |mask| {
        let mut controller = GenesisController::new(GenesisControllerType::ThreeButton);
        for (bit, button) in BUTTONS.into_iter().enumerate() {
            controller.set_field(button, mask & (1 << bit) != 0);
        }
        controller
    };
    GenesisInputs {
        p1: controller(p1),
        p2: if p2_connected {
            controller(p2)
        } else {
            GenesisController::None
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_button_has_an_independent_wire_bit() {
        for (bit, button) in BUTTONS.into_iter().enumerate() {
            let mut expected = GenesisController::new(GenesisControllerType::ThreeButton);
            expected.set_field(button, true);
            assert_eq!(inputs(1 << bit, 0, false).p1, expected);
            assert_eq!(inputs(0, 1 << bit, true).p2, expected);
        }
    }

    #[test]
    fn disconnected_player_two_does_not_receive_input() {
        assert_eq!(inputs(0, u8::MAX, false).p2, GenesisController::None);
        assert_eq!(inputs(0, 0, true).p1, inputs(0, 0, true).p2);
        let mut all = GenesisController::new(GenesisControllerType::ThreeButton);
        for button in BUTTONS {
            all.set_field(button, true);
        }
        assert_eq!(inputs(u8::MAX, u8::MAX, true).p1, all);
        assert_eq!(inputs(u8::MAX, u8::MAX, true).p2, all);
    }
}
