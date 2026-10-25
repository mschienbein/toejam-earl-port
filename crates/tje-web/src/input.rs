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
    GenesisInputs {
        p1: controller(p1),
        p2: if p2_connected {
            controller(p2)
        } else {
            GenesisController::None
        },
    }
}

fn controller(mask: u8) -> GenesisController {
    let mut controller = GenesisController::new(GenesisControllerType::ThreeButton);
    for (bit, button) in BUTTONS.into_iter().enumerate() {
        controller.set_field(button, mask & (1 << bit) != 0);
    }
    controller
}

/// Strict enhanced wire words; no silent masking of a disconnected/unknown pad.
pub fn local_inputs(words: [u16; 4]) -> Result<[GenesisController; 4], &'static str> {
    if words
        .into_iter()
        .any(|word| word & !0x1ff != 0 || (word & 0x100 == 0 && word != 0))
    {
        return Err("invalid enhanced four-pad wire word");
    }
    Ok(words.map(|word| {
        if word & 0x100 != 0 {
            controller(word as u8)
        } else {
            GenesisController::None
        }
    }))
}

#[cfg(all(feature = "enhanced-mode", target_arch = "wasm32"))]
pub fn enhanced_inputs(
    words: [u16; 4],
) -> Result<tje_engine_runtime::enhanced::EnhancedInputs, &'static str> {
    Ok(tje_engine_runtime::enhanced::EnhancedInputs {
        pads: local_inputs(words)?,
    })
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
    #[test]
    fn enhanced_words_route_four_independent_pads_without_touching_faithful_shape() {
        for seat in 0..4 {
            for (bit, button) in BUTTONS.into_iter().enumerate() {
                let mut words = [0x100; 4];
                words[seat] |= 1 << bit;
                let pads = local_inputs(words).unwrap();
                let mut expected = controller(0);
                expected.set_field(button, true);
                assert_eq!(pads[seat], expected);
                for (other, pad) in pads.iter().enumerate() {
                    if other != seat {
                        assert_eq!(*pad, controller(0));
                    }
                }
            }
        }
        assert_eq!(local_inputs([0; 4]).unwrap(), [GenesisController::None; 4]);
        assert!(local_inputs([1, 0, 0, 0]).is_err());
        assert!(local_inputs([0x200, 0, 0, 0]).is_err());
    }
}
