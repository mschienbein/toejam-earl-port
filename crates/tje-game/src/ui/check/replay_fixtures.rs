//! Labelled RAM/register variations of a settled live gameplay state. No ROM,
//! device readiness, IRQ, SR, PC or return address is manufactured here.
use super::Result;
use serde_json::{Value, json};
use tje_game::{Context, FunctionId, ids};
use tje_host::Host;
use tje_mem::{Bus, Revision, ram};

// These are processed game inputs, not the controller recording's pad bits.
const PROCESSED_B: u8 = 0x10;
const PROCESSED_C: u8 = 0x20;
const PROCESSED_A: u8 = 0x40;
const UP: u8 = 1;
const DOWN: u8 = 2;
const LEFT: u8 = 4;
const RIGHT: u8 = 8;
const CLOSED: i8 = 0;
const PRESENTS: i8 = 1;
const MAIL_ORDER: i8 = 2;
const IDENTIFY: i8 = 3;
const RANDOMIZER: i8 = 18;

struct Fixtures<'a> {
    host: &'a mut Host,
    baseline: Vec<u8>,
    entry_d2: u32,
    observations: Vec<Value>,
}

impl Fixtures<'_> {
    fn check(
        &mut self,
        label: String,
        id: FunctionId,
        arguments: &[u32],
        setup: impl FnOnce(&mut dyn Context),
    ) -> Result<()> {
        if !self
            .host
            .checks
            .by_function
            .contains_key(id.metadata().name)
        {
            return Ok(());
        }
        let revision = self.host.revision();
        {
            let mut context = self.host.context();
            for (offset, &byte) in self.baseline.iter().enumerate() {
                context.write_u8(0xff0000 + offset as u32, byte);
            }
            context.set_data_register(2, self.entry_d2);
            setup(&mut context);
        }
        let before = self
            .host
            .checks
            .by_function
            .iter()
            .find(|(name, _)| *name == id.metadata().name)
            .map_or(0, |(_, coverage)| coverage.calls_checked);
        let result = self
            .host
            .check_fixture(id.metadata().address(revision), arguments);
        let after = self
            .host
            .checks
            .by_function
            .iter()
            .find(|(name, _)| *name == id.metadata().name)
            .map_or(0, |(_, coverage)| coverage.calls_checked);
        self.observations.push(json!({
            "label":label, "function":id.metadata().name, "arguments":arguments,
            "source_state":"settled original gameplay prefix plus explicitly labelled RAM/register inputs",
            "checked_calls":after-before, "error":result.err().map(|error| error.to_string()),
        }));
        Ok(())
    }
}

fn selection(context: &mut dyn Context, revision: Revision, player: i32, last_slot: bool) {
    ram::g_player_inventory_selection_row(revision)
        .at(player)
        .write(context, if last_slot { 2 } else { 0 });
    ram::g_player_inventory_scroll_row(revision)
        .at(player)
        .write(context, if last_slot { 5 } else { 0 });
    ram::g_player_inventory_selection_column(revision)
        .at(player)
        .write(context, i8::from(last_slot));
}

pub(super) fn run(host: &mut Host) -> Result<Vec<Value>> {
    let revision = host.revision();
    let (baseline, entry_d2, prerequisites) = {
        let context = host.context();
        let enabled = ram::g_v_blank_graphics_enabled(revision).read(&context);
        let paused = ram::g_v_blank_graphics_paused(revision).read(&context);
        let upload = ram::g_sprite_graphics_upload_pending(revision).read(&context);
        let redraw = ram::g_inventory_redraw_phase(revision).read(&context);
        let lives = [0, 1].map(|player| ram::g_player_lives(revision).at(player).read(&context));
        let states = [0, 1].map(|player| {
            ram::g_player_info(revision)
                .at(player)
                .sprite_state()
                .read(&context)
        });
        if enabled == 0
            || paused != 0
            || upload != 0
            || redraw != 0
            || lives.iter().any(|life| *life < 0)
            || states.iter().any(|state| *state == 5 || *state == 6)
        {
            return Err("inventory fixtures require a settled, graphics-enabled two-living-player gameplay prefix; readiness is never forced".into());
        }
        let baseline = (0..0x10000)
            .map(|offset| context.read_u8(0xff0000 + offset))
            .collect();
        (
            baseline,
            context.data_register(2),
            json!({
                "vblank_enabled":enabled, "vblank_paused":paused, "upload_pending":upload,
                "redraw_phase":redraw, "lives":lives, "sprite_states":states,
                "entry_sr":context.status_register(),
            }),
        )
    };
    let mut f = Fixtures {
        host,
        baseline,
        entry_d2,
        observations: vec![json!({"prerequisites":prerequisites})],
    };
    for viewport in [0, 1] {
        f.check(
            format!("clear viewport {viewport} from settled live sprite lists"),
            ids::ENTITY_CLEAR_VIEWPORT_SPRITES,
            &[viewport],
            |_| {},
        )?;
    }
    for player in 0_i32..2 {
        for phase in 1..=4 {
            for menu in 1..=3 {
                f.check(
                    format!("redraw player {player}, phase {phase}, menu {menu}"),
                    ids::INVENTORY_SERVICE_REDRAW_PHASE,
                    &[],
                    |context| {
                        ram::g_inventory_redraw_player(revision).write(context, player as i8);
                        ram::g_inventory_redraw_phase(revision).write(context, phase);
                        ram::g_player_inventory_open(revision)
                            .at(player)
                            .write(context, menu);
                    },
                )?;
            }
        }
        for split in [0, 1] {
            for menu in 1..=3 {
                f.check(
                    format!("draw player {player}, split {split}, menu {menu}"),
                    ids::DRAW_INVENTORY,
                    &[player as u32, menu],
                    |context| {
                        ram::g_split_screen_enabled(revision).write(context, split);
                        ram::g_player_inventory_open(revision)
                            .at(player)
                            .write(context, 0);
                    },
                )?;
            }
        }
        f.check(
            format!("synchronous redraw player {player}"),
            ids::INVENTORY_REDRAW_SYNCHRONOUSLY,
            &[player as u32],
            |_| {},
        )?;
        for phase in [0, 2] {
            f.check(
                format!("refresh player {player}, outstanding phase {phase}"),
                ids::INVENTORY_REFRESH_CONTENTS_AND_WAIT,
                &[player as u32],
                |context| {
                    // Nonzero here requests real service, never a forced-ready exit.
                    ram::g_inventory_redraw_player(revision).write(context, player as i8);
                    ram::g_inventory_redraw_phase(revision).write(context, phase);
                },
            )?;
        }
        for (split, own, other, elevator, departing) in [
            (false, true, false, false, false),
            (false, true, true, false, false),
            (true, true, false, false, false),
            (true, true, true, false, false),
            (false, false, false, false, true),
            (false, false, false, true, false),
            (false, false, false, true, true),
        ] {
            f.check(format!("toggle p{player}: split={split} own={own} other={other} elevator={elevator}/{departing}"),
                ids::TOGGLE_INVENTORY, &[player as u32], |context| {
                    ram::g_split_screen_enabled(revision).write(context, i8::from(split));
                    ram::g_player_inventory_open(revision).at(player).write(context, i8::from(own));
                    ram::g_player_inventory_open(revision).at(player ^ 1).write(context, i8::from(other));
                    ram::g_player_elevator_flags(revision).at(player).write(context, i8::from(elevator));
                    ram::g_elevator_departure_in_progress(revision).write(context, i8::from(departing));
                })?;
        }
        for (other_lives, other_open) in [(-1, 0), (0, 0), (0, 1)] {
            f.check(format!("randomize p{player}: partner lives={other_lives}, open={other_open}; all sixty iterations"),
                ids::RANDOMIZE_PRESENTS, &[player as u32], |context| {
                    ram::g_player_lives(revision).at(player ^ 1).write(context, other_lives);
                    ram::g_player_inventory_open(revision).at(player).write(context, 1);
                    ram::g_player_inventory_open(revision).at(player ^ 1).write(context, other_open);
                })?;
        }
        for seed in [i16::MIN, -1, 0, 1, 2, i16::MAX] {
            f.check(
                format!("non-split CheckBButton p{player}, incoming D2.low={seed}, real future B"),
                ids::CHECK_B_BUTTON,
                &[],
                |context| {
                    context.set_data_register(2, 0x12340000 | u32::from(seed as u16));
                    ram::g_split_screen_enabled(revision).write(context, 0);
                    ram::g_player_inventory_open(revision)
                        .at(player)
                        .write(context, 1);
                    ram::g_player_inventory_open(revision)
                        .at(player ^ 1)
                        .write(context, 0);
                    ram::g_player_processed_input(revision)
                        .at(player)
                        .write(context, 0);
                    ram::g_player_previous_processed_input(revision)
                        .at(player)
                        .write(context, 0);
                },
            )?;
        }
        for (id, menu) in [
            (ids::INVENTORY_OPEN_MAIL_ORDER_MENU, 2),
            (ids::INVENTORY_OPEN_IDENTIFICATION_MENU, 3),
        ] {
            f.check(
                format!("special menu {menu}, player {player}, real future B"),
                id,
                &[player as u32],
                |context| {
                    ram::g_split_screen_enabled(revision).write(context, 0);
                    ram::g_player_inventory_open(revision)
                        .at(0)
                        .write(context, 0);
                    ram::g_player_inventory_open(revision)
                        .at(1)
                        .write(context, 0);
                    selection(context, revision, player, false);
                },
            )?;
        }
        for (present, last_slot, compact) in [
            (-1, false, false),
            (0, false, true),
            (26, false, true),
            (27, true, false),
        ] {
            f.check(format!("OpenPresent p{player}: present={present}, last-slot={last_slot}, compact={compact}"),
                ids::OPEN_PRESENT, &[player as u32], |context| {
                    selection(context, revision, player, last_slot);
                    for slot in 0..16 {
                        ram::g_player_inventory(revision).at(player).at(slot).write(context, -1);
                    }
                    ram::g_player_inventory(revision).at(player).at(if last_slot { 15 } else { 0 }).write(context, present);
                    if compact {
                        ram::g_player_inventory(revision).at(player).at(1).write(context, 1);
                        ram::g_player_inventory(revision).at(player).at(2).write(context, 2);
                    }
                })?;
        }
        for occupied in [0, 15, 16] {
            for bucks in [0, 3, 4] {
                for offer in 0..3 {
                    f.check(
                        format!(
                            "purchase p{player}: occupied={occupied}, bucks={bucks}, offer={offer}"
                        ),
                        ids::INVENTORY_BUY_SELECTED_MAIL_ORDER_PRESENT,
                        &[player as u32],
                        |context| {
                            for slot in 0..16 {
                                ram::g_player_inventory(revision)
                                    .at(player)
                                    .at(slot)
                                    .write(context, if slot < occupied { 0 } else { -1 });
                            }
                            ram::g_player_inventory_selection_row(revision)
                                .at(player)
                                .write(context, offer);
                            ram::g_player_bucks(revision)
                                .at(player)
                                .write(context, bucks);
                            for item in 0..3 {
                                ram::g_player_mailbox_items(revision)
                                    .at(player)
                                    .at(item)
                                    .write(context, item as i8);
                                ram::g_player_mailbox_prices(revision)
                                    .at(player)
                                    .at(item)
                                    .write(context, 3);
                            }
                        },
                    )?;
                }
            }
        }
    }
    let opening = ram::g_text_font_buffers(revision).cast::<i16>();
    for values in [[0, 0], [1, 0], [0, 1], [1, 1]] {
        f.check(
            format!("coordination address-taken opening={values:?}"),
            ids::CHECK_IF_OPEN_FOR_BOTH,
            &[opening.address()],
            |context| {
                opening.write(context, values[0]);
                opening.offset(1).write(context, values[1]);
            },
        )?;
    }
    for (input, previous, menu, row, scroll, present) in [
        (PROCESSED_B, 0, CLOSED, 0, 0, 0),
        (PROCESSED_C, 0, PRESENTS, 0, 0, 0),
        (PROCESSED_A, 0, PRESENTS, 0, 0, 0),
        (PROCESSED_A, 0, PRESENTS, 0, 0, RANDOMIZER),
        (PROCESSED_A, 0, MAIL_ORDER, 0, 0, 0),
        (PROCESSED_A, 0, IDENTIFY, 0, 0, 0),
        (UP, 0, PRESENTS, 1, 0, 0),
        (UP, 0, PRESENTS, 0, 1, 0),
        (DOWN, 0, PRESENTS, 1, 0, 0),
        (DOWN, 0, PRESENTS, 2, 0, 0),
        (DOWN, 0, PRESENTS, 2, 5, 0),
        (LEFT, 0, PRESENTS, 0, 0, 0),
        (RIGHT, 0, MAIL_ORDER, 0, 0, 0),
        (PROCESSED_A, PROCESSED_A, PRESENTS, 0, 0, 0),
    ] {
        f.check(format!("ProcessInventory edge input={input:02x}/previous={previous:02x}, menu={menu}, row={row}, scroll={scroll}, present={present}"),
            ids::PROCESS_INVENTORY, &[], |context| {
                ram::g_player_processed_input(revision).at(0).write(context, input);
                ram::g_player_previous_processed_input(revision).at(0).write(context, previous);
                ram::g_player_inventory_open(revision).at(0).write(context, menu);
                ram::g_player_inventory_selection_row(revision).at(0).write(context, row);
                ram::g_player_inventory_scroll_row(revision).at(0).write(context, scroll);
                ram::g_player_inventory_selection_column(revision).at(0).write(context, 0);
                for slot in 0..16 {
                    ram::g_player_inventory(revision).at(0).at(slot).write(context, present);
                }
            })?;
    }
    // Restore the actual prefix after all labelled recipes; check_fixture already
    // restores CPU/devices/IRQs after every isolated attempted proof.
    {
        let mut context = f.host.context();
        for (offset, &byte) in f.baseline.iter().enumerate() {
            context.write_u8(0xff0000 + offset as u32, byte);
        }
        context.set_data_register(2, f.entry_d2);
    }
    Ok(f.observations)
}
