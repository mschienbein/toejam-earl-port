use anyhow::{Context, Result, ensure};
use genesis_config::{GenesisButton, GenesisController, GenesisControllerType, GenesisInputs};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
use tje_game::FunctionMetadata;
use tje_host::{
    CheckFailure, CheckFailurePolicy, FrameSnapshot, Host, Mode,
    execution::WaitInputPoller,
    hooks::HookRegistry,
    session_trace::{EntryObservation, FrameObservations, LogicTick},
};
use tje_mem::{Bus, Revision, ram};
use tje_session::{CONNECTED, Session, TOOL_VERSION};

pub fn functions() -> Vec<&'static FunctionMetadata> {
    tje_game::functions().collect()
}
pub fn function_name(revision: Revision, address: u32) -> &'static str {
    tje_game::functions()
        .find(|function| {
            function.address(revision) <= address
                && address < function.address(revision) + function.size(revision)
        })
        .map_or("<uncatalogued>", |function| function.name)
}
pub fn entry_name(revision: Revision, address: u32) -> &'static str {
    tje_game::function_at(revision, address).map_or("<uncatalogued>", |id| id.metadata().name)
}

pub fn inputs(pads: [u16; 2]) -> GenesisInputs {
    use GenesisButton::*;
    let pads = pads.map(|pad| {
        let mut controller = if pad & CONNECTED != 0 {
            GenesisController::new(GenesisControllerType::ThreeButton)
        } else {
            GenesisController::None
        };
        for (index, button) in [Up, Down, Left, Right, A, B, C, Start]
            .into_iter()
            .enumerate()
        {
            controller.set_field(button, pad & (1 << index) != 0);
        }
        controller
    });
    GenesisInputs {
        p1: pads[0],
        p2: pads[1],
    }
}

pub struct Ram<'a>(pub &'a [u8]);
impl Bus for Ram<'_> {
    fn read_u8(&self, address: u32) -> u8 {
        self.0[(address & 0xFFFF) as usize]
    }
    fn write_u8(&mut self, _address: u32, _value: u8) {
        panic!("read-only session observation");
    }
}

/// The sample order is a local observation contract, not serialized session data.
fn sample_addresses(revision: Revision) -> Vec<u32> {
    let mut samples = (0..32)
        .map(|index| ram::g_player_inventory(revision).address() + index)
        .collect::<Vec<_>>();
    for base in [
        ram::g_player_inventory_scroll_row(revision).address(),
        ram::g_player_inventory_selection_row(revision).address(),
        ram::g_player_inventory_selection_column(revision).address(),
    ] {
        samples.extend([base, base + 1]);
    }
    samples.extend([
        ram::g_player_bucks(revision).address(),
        ram::g_player_bucks(revision).address() + 1,
    ]);
    samples.extend([
        ram::g_map_reveal_mode(revision).address(),
        ram::g_map_reveal_mode(revision).address() + 1,
        ram::g_player_lives(revision).address(),
        ram::g_player_lives(revision).address() + 1,
        ram::g_dropped_present_table_idx(revision).address(),
        ram::g_player_interaction_responses(revision).address(),
        ram::g_player_interaction_responses(revision).address() + 1,
        ram::g_life_request_pending(revision).address(),
        ram::g_life_request_pending(revision).address() + 1,
        ram::g_player_selection(revision).address(),
    ]);
    samples
}

pub fn host(bytes: &[u8], mode: Mode, revision: Revision, capture_ticks: bool) -> Result<Host> {
    let registry = if mode == Mode::Reference {
        HookRegistry::empty(revision)
    } else {
        HookRegistry::translated(revision)?
    };
    let mut host = Host::new(bytes, mode, registry)?;
    let entries = functions()
        .iter()
        .map(|function| function.address(revision))
        .collect::<Vec<_>>();
    let watched = functions()
        .iter()
        .filter(|function| {
            matches!(
                function.name,
                "MenuWhoHandler"
                    | "MenuWhatHandler"
                    | "MenuJamOutHandler"
                    | "MenuControllerOptionsHandler"
                    | "OpenPresent"
                    | "OpenSelectedPresent"
                    | "SetupSelectedPresent"
                    | "RandomizePresents"
                    | "DrawGameMap"
                    | "ElevatorBeginRideScene"
                    | "ShipPieceAbility"
                    | "StartJamOut"
                    | "PlayEndingSequence"
                    | "EndingPlayScrollingCredits"
                    | "TitleScreenVideo"
                    | "WaitForShipScreenAcknowledgement"
                    | "MapEnterFinalLevel"
                    | "ShowGameOverAndReset"
                    | "PlayerTeleportThroughDoorway"
                    | "ProcessInput"
                    | "InventoryBuySelectedMailOrderPresent"
                    | "InventoryIdentifySelectedPresent"
                    | "InventoryDropSelectedPresent"
                    | "InventoryOpenMailOrderMenu"
                    | "InventoryOpenIdentificationMenu"
                    | "GamePaused"
                    | "IntroVideo"
                    | "PlayerFinalizeLastLifeExit"
                    | "GameProgram"
                    | "DropPresentsAbility"
                    | "PlayerProcessLifeRequest"
            )
        })
        .map(|function| function.address(revision))
        .collect::<Vec<_>>();
    host.enable_session_trace(
        &entries,
        &watched,
        &sample_addresses(revision),
        capture_ticks,
    )?;
    Ok(host)
}

fn apply_ram_recipe(memory: &mut dyn Bus, session: &Session, frame: u64) {
    for write in session
        .writes()
        .iter()
        .filter(|write| write.before_frame == frame)
    {
        match write.width {
            1 => memory.write_u8(write.address, write.value as u8),
            2 => memory.write_u16(write.address, write.value as u16),
            4 => memory.write_u32(write.address, write.value),
            _ => unreachable!("validated recipe width"),
        }
    }
}

pub fn apply_recipe(host: &mut Host, session: &Session, frame: u64) {
    apply_ram_recipe(&mut host.context(), session, frame);
}

/// One immutable, indexed timeline is shared by both independent check forks.
/// RAM recipes use the same helper as ordinary frame-by-frame replay.
struct SessionInputs {
    session: Arc<Session>,
    end_frames: Vec<u64>,
}
impl SessionInputs {
    fn new(session: Arc<Session>) -> Self {
        let mut end = 0;
        let end_frames = session
            .runs
            .iter()
            .map(|run| {
                end += run.0;
                end
            })
            .collect();
        Self {
            session,
            end_frames,
        }
    }
}
impl WaitInputPoller for SessionInputs {
    fn at(&self, frame: u64) -> tje_host::Result<GenesisInputs> {
        let index = self.end_frames.partition_point(|&end| end <= frame);
        let run = self.session.runs.get(index).ok_or_else(|| {
            tje_host::HostError::Input(format!(
                "session has no controller sample for video frame {frame}"
            ))
        })?;
        Ok(inputs([run.1, run.2]))
    }

    fn before_frame(&self, frame: u64, memory: &mut dyn Bus) -> tje_host::Result<()> {
        apply_ram_recipe(memory, &self.session, frame);
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Content {
    pub milestones: BTreeMap<String, Option<u64>>,
    /// Storage identities 0..27: mystery and bonus-hi-tops are tracked separately
    /// from the resolved/normalized effects. A successful setup is required.
    pub presents_opened: BTreeMap<u8, Option<u64>>,
    /// Requires allocated entity plus visible-in-either-viewport bits, not spawn.
    pub earthlings_encountered: BTreeMap<u8, Option<u64>>,
    pub ship_pieces_collected: BTreeMap<u8, Option<u64>>,
    pub levels_visited: BTreeSet<i8>,
    #[serde(default)]
    pub friendly_interactions: BTreeMap<String, Option<u64>>,
    #[serde(default)]
    pub ranks_reached: BTreeSet<i8>,
    #[serde(default)]
    pub jam_modes: BTreeMap<u8, Option<u64>>,
}
impl Default for Content {
    fn default() -> Self {
        Self::new()
    }
}
impl Content {
    pub fn new() -> Self {
        Self {
            milestones: [
                "title",
                "menus",
                "fixed_world",
                "one_player",
                "walking",
                "two_player_split_screen",
                "inventory",
                "elevator_ride",
                "map_screen",
                "jam_out",
                "ending_started",
                "ending_credits",
                "ending_complete",
                "final_level",
                "level_zero",
                "last_life",
                "game_over",
                "fall_to_lower_level",
                "split_rejoined",
                "elevator_departure",
                "elevator_arrival",
                "inventory_mail_order",
                "inventory_identification",
                "inventory_drop",
                "pause",
                "controller_options",
                "promotion",
                "last_life_exit",
                "present_identified",
            ]
            .into_iter()
            .map(|name| (name.into(), None))
            .collect(),
            presents_opened: (0..28).map(|kind| (kind, None)).collect(),
            earthlings_encountered: (0..25).map(|kind| (kind, None)).collect(),
            ship_pieces_collected: (0..10).map(|kind| (kind, None)).collect(),
            levels_visited: BTreeSet::new(),
            friendly_interactions: [
                "wizard_paid",
                "wiseman_identification",
                "opera_paid",
                "mail_order_purchase",
                "lemonade_reward",
                "hottub_bath",
                "santa_gifts",
                "hula_dance",
                "telephone_answered",
                "health_clap",
                "life_request",
            ]
            .into_iter()
            .map(|name| (name.into(), None))
            .collect(),
            ranks_reached: BTreeSet::new(),
            jam_modes: (0..8).map(|mode| (mode, None)).collect(),
        }
    }
    pub fn mark(&mut self, name: &str, frame: u64) {
        self.milestones
            .entry(name.into())
            .or_default()
            .get_or_insert(frame);
    }
    pub fn merge(&mut self, other: &Self) {
        fn merge_map<K: Ord + Clone>(
            target: &mut BTreeMap<K, Option<u64>>,
            source: &BTreeMap<K, Option<u64>>,
        ) {
            for (key, &frame) in source {
                let entry = target.entry(key.clone()).or_default();
                if entry.is_none() {
                    *entry = frame;
                }
            }
        }
        merge_map(&mut self.milestones, &other.milestones);
        merge_map(&mut self.presents_opened, &other.presents_opened);
        merge_map(
            &mut self.earthlings_encountered,
            &other.earthlings_encountered,
        );
        merge_map(
            &mut self.ship_pieces_collected,
            &other.ship_pieces_collected,
        );
        self.levels_visited.extend(&other.levels_visited);
        merge_map(
            &mut self.friendly_interactions,
            &other.friendly_interactions,
        );
        merge_map(&mut self.jam_modes, &other.jam_modes);
        self.ranks_reached.extend(&other.ranks_reached);
    }
}

pub struct Coverage {
    pub content: Content,
    pending_present: [Option<(i8, u64)>; 2],
    previous_ship: Option<[i8; 10]>,
    collecting_ship: bool,
    initial_positions: [Option<(i16, i16)>; 2],
    previous_levels: [i8; 2],
    previous_ranks: [i8; 2],
    ever_split: bool,
    mail_purchase: Option<(usize, i8, u64)>,
    identification: Option<(usize, i8, u64)>,
    santa_drop: Option<(u32, u8, u64)>,
    life_request: Option<(usize, i8, bool, u64)>,
}
impl Coverage {
    pub fn new() -> Self {
        Self {
            content: Content::new(),
            pending_present: [None; 2],
            previous_ship: None,
            collecting_ship: false,
            initial_positions: [None; 2],
            previous_levels: [-1; 2],
            previous_ranks: [-1; 2],
            ever_split: false,
            mail_purchase: None,
            identification: None,
            santa_drop: None,
            life_request: None,
        }
    }
    pub fn observe(
        &mut self,
        host: &Host,
        snapshot: &FrameSnapshot,
        observations: &FrameObservations,
        revision: Revision,
    ) {
        let frame = snapshot.frame;
        if host.measurements.title_frame.is_some() {
            self.content.mark("title", frame);
        }
        if host.measurements.menu_frame.is_some() {
            self.content.mark("menus", frame);
        }
        for event in &observations.entries {
            self.entry(event, revision);
        }
        if host.measurements.game_frame.is_none() {
            return;
        }
        let memory = Ram(&snapshot.ram);
        self.observe_transactions(&memory, revision, frame);
        let ship = std::array::from_fn(|index| {
            ram::g_ship_piece_level_assignments(revision)
                .at(index as i32)
                .read(&memory)
        });
        if let Some(previous) = self.previous_ship {
            let mut changed = (0..10).filter(|&index| previous[index] > 0 && ship[index] == 0);
            if let Some(index) = changed.next() {
                if self.collecting_ship && changed.next().is_none() {
                    self.content
                        .ship_pieces_collected
                        .get_mut(&(index as u8))
                        .unwrap()
                        .get_or_insert(frame);
                }
                self.collecting_ship = false;
            }
        }
        self.previous_ship = Some(ship);
        if ram::g_v_blank_graphics_enabled(revision).read(&memory) == 0
            || !(0..2).any(|player| {
                ram::g_player_lives(revision).at(player).read(&memory) >= 0
                    && ram::g_player_info(revision)
                        .at(player)
                        .draw_flags()
                        .read(&memory)
                        & 1
                        != 0
            })
        {
            return;
        }
        if ram::g_world_type(revision).read(&memory) != 0 {
            self.content.mark("fixed_world", frame);
        }
        if ram::g_player_selection(revision).read(&memory) != 2 {
            self.content.mark("one_player", frame);
        }
        if ram::g_player_selection(revision).read(&memory) == 2
            && ram::g_split_screen_enabled(revision).read(&memory) != 0
        {
            self.content.mark("two_player_split_screen", frame);
            self.ever_split = true;
        }
        if self.ever_split
            && ram::g_split_screen_enabled(revision).read(&memory) == 0
            && ram::g_player_lives(revision).at(0).read(&memory) >= 0
            && ram::g_player_lives(revision).at(1).read(&memory) >= 0
        {
            self.content.mark("split_rejoined", frame);
        }
        for player in 0..2 {
            if ram::g_player_lives(revision)
                .at(player as i32)
                .read(&memory)
                < 0
            {
                continue;
            }
            let view = ram::g_player_info(revision).at(player as i32);
            self.content
                .levels_visited
                .insert(view.level().read(&memory));
            let level = view.level().read(&memory);
            if level == 0 {
                self.content.mark("level_zero", frame);
            }
            if level == 26 {
                self.content.mark("final_level", frame);
            }
            if (0..26).contains(&self.previous_levels[player])
                && level < self.previous_levels[player]
                && (0..26).contains(&level)
                && ram::g_player_fall_states(revision)
                    .at(player as i32)
                    .read(&memory)
                    == 2
            {
                self.content.mark("fall_to_lower_level", frame);
            }
            self.previous_levels[player] = level;
            let rank = ram::g_player_rank(revision).at(player as i32).read(&memory);
            self.content.ranks_reached.insert(rank);
            if self.previous_ranks[player] >= 0 && rank > self.previous_ranks[player] && rank <= 8 {
                self.content.mark("promotion", frame);
            }
            self.previous_ranks[player] = rank;
            if ram::g_player_lives(revision)
                .at(player as i32)
                .read(&memory)
                == 0
            {
                self.content.mark("last_life", frame);
            }
            let elevator = ram::g_player_elevator_flags(revision)
                .at(player as i32)
                .read(&memory);
            if elevator != 0 {
                self.content.mark(
                    if elevator & 1 == 0 {
                        "elevator_arrival"
                    } else {
                        "elevator_departure"
                    },
                    frame,
                );
            }
            for (name, state) in [
                (
                    "wizard_paid",
                    ram::g_player_wizard_interaction_states(revision)
                        .at(player as i32)
                        .read(&memory)
                        == 3,
                ),
                (
                    "opera_paid",
                    ram::g_player_opera_singer_interaction_states(revision)
                        .at(player as i32)
                        .read(&memory)
                        == 3,
                ),
                (
                    "lemonade_reward",
                    ram::g_player_lemonade_reward_states(revision)
                        .at(player as i32)
                        .read(&memory)
                        == 2,
                ),
                ("hottub_bath", view.sprite_state().read(&memory) == 119),
                ("hula_dance", view.sprite_state().read(&memory) == 89),
            ] {
                if state {
                    self.content
                        .friendly_interactions
                        .entry(name.into())
                        .or_default()
                        .get_or_insert(frame);
                }
            }
            if let Some((customer, bucks, started)) = self.mail_purchase
                && customer == player
                && frame <= started + 2
                && ram::g_player_bucks(revision)
                    .at(player as i32)
                    .read(&memory)
                    < bucks
            {
                self.content
                    .friendly_interactions
                    .entry("mail_order_purchase".into())
                    .or_default()
                    .get_or_insert(frame);
                self.mail_purchase = None;
            }
            if self
                .mail_purchase
                .is_some_and(|(_, _, started)| frame > started + 2)
            {
                self.mail_purchase = None;
            }
            let position = (view.x().read(&memory), view.y().read(&memory));
            if let Some(initial) = self.initial_positions[player] {
                if (i32::from(initial.0) - i32::from(position.0)).abs()
                    + (i32::from(initial.1) - i32::from(position.1)).abs()
                    > 24
                {
                    self.content.mark("walking", frame);
                }
            } else {
                self.initial_positions[player] = Some(position);
            }
            if ram::g_player_inventory_open(revision)
                .at(player as i32)
                .read(&memory)
                != 0
            {
                self.content.mark("inventory", frame);
                match ram::g_player_inventory_open(revision)
                    .at(player as i32)
                    .read(&memory)
                {
                    2 => self.content.mark("inventory_mail_order", frame),
                    3 => self.content.mark("inventory_identification", frame),
                    _ => {}
                }
            }
        }
        for bank in 0..2 {
            for index in 0..20 {
                let earthling = ram::g_earthling_banks(revision).at(bank).at(index);
                let kind = earthling.kind().read(&memory);
                if (0..25).contains(&kind)
                    && earthling.object().read(&memory) != 0
                    && earthling.health_visibility().at(1).read(&memory) & 6 != 0
                {
                    self.content
                        .earthlings_encountered
                        .get_mut(&(kind as u8))
                        .unwrap()
                        .get_or_insert(frame);
                }
            }
        }
    }
    fn observe_transactions(&mut self, memory: &Ram<'_>, revision: Revision, frame: u64) {
        if let Some((player, before, started)) = self.identification {
            if frame <= started + 2
                && ram::g_player_bucks(revision).at(player as i32).read(memory) == before - 2
            {
                self.content
                    .friendly_interactions
                    .entry("wiseman_identification".into())
                    .or_default()
                    .get_or_insert(frame);
                self.content.mark("present_identified", frame);
                self.identification = None;
            } else if frame > started + 2 {
                self.identification = None;
            }
        }
        if let Some((address, before, started)) = self.santa_drop {
            let base = ram::g_earthling_banks(revision).address();
            // Two banks of twenty 18-byte records, as declared in ram_earthlings.h.
            let valid =
                address >= base && address < base + 720 && (address - base).is_multiple_of(18);
            if valid && frame <= started + 8 {
                let santa = tje_mem::Address::<tje_mem::EarthlingState>::new(address);
                if santa.kind().read(memory) == 20
                    && santa.state().read(memory) == 3
                    && ram::g_dropped_present_table_idx(revision).read(memory) as u8 != before
                {
                    self.content
                        .friendly_interactions
                        .entry("santa_gifts".into())
                        .or_default()
                        .get_or_insert(frame);
                    self.santa_drop = None;
                }
            } else {
                self.santa_drop = None;
            }
        }
        if let Some((receiver, donor_before, was_two, started)) = self.life_request {
            if frame <= started + 180
                && ram::g_player_lives(revision)
                    .at(receiver as i32)
                    .read(memory)
                    >= 0
                && ram::g_player_lives(revision)
                    .at((receiver ^ 1) as i32)
                    .read(memory)
                    == donor_before - i8::from(was_two)
            {
                self.content
                    .friendly_interactions
                    .entry("life_request".into())
                    .or_default()
                    .get_or_insert(frame);
                self.life_request = None;
            } else if frame > started + 180 {
                self.life_request = None;
            }
        }
        if ram::g_player_selection(revision).read(memory) == 2
            && (0..2).all(|player| {
                ram::g_player_lives(revision).at(player).read(memory) >= 0
                    && matches!(
                        ram::g_player_info(revision)
                            .at(player)
                            .sprite_state()
                            .read(memory),
                        90 | 91
                    )
            })
        {
            self.content
                .friendly_interactions
                .entry("health_clap".into())
                .or_default()
                .get_or_insert(frame);
        }
    }

    fn entry(&mut self, event: &EntryObservation, revision: Revision) {
        let frame = event.frame;
        match entry_name(revision, event.address) {
            "GameProgram" => {
                self.previous_levels = [-1; 2];
                self.previous_ranks = [-1; 2];
                self.initial_positions = [None; 2];
                self.previous_ship = None;
                self.collecting_ship = false;
                self.mail_purchase = None;
                self.ever_split = false;
                self.identification = None;
                self.santa_drop = None;
                self.life_request = None;
            }
            "OpenPresent" => {
                let player = event.stack_slots[0] as u16 as usize;
                if player < 2 && event.ram_samples.len() >= 38 {
                    let samples = &event.ram_samples;
                    let slot =
                        (usize::from(samples[32 + player]) + usize::from(samples[34 + player])) * 2
                            + usize::from(samples[36 + player]);
                    self.pending_present[player] = samples
                        .get(player * 16 + slot)
                        .copied()
                        .map(|value| (value as i8, frame));
                }
            }
            "SetupSelectedPresent" => {
                let player = event.stack_slots[1] as u16 as usize;
                let kind = event.stack_slots[2] as u16;
                if kind < 28 {
                    self.content
                        .presents_opened
                        .get_mut(&(kind as u8))
                        .unwrap()
                        .get_or_insert(frame);
                }
                if player < 2
                    && let Some((original, opened)) = self.pending_present[player].take()
                    && (0..28).contains(&original)
                    && frame <= opened + 8
                    && (original == 26
                        || kind == u16::from(if original == 27 { 7_u8 } else { original as u8 }))
                {
                    self.content
                        .presents_opened
                        .get_mut(&(original as u8))
                        .unwrap()
                        .get_or_insert(frame);
                }
            }
            "RandomizePresents" => {
                self.content
                    .presents_opened
                    .get_mut(&18)
                    .unwrap()
                    .get_or_insert(frame);
            }
            "DrawGameMap" => {
                self.content.mark("map_screen", frame);
                if event.ram_samples.get(40..42) == Some(&[0, 1][..]) {
                    self.content
                        .friendly_interactions
                        .entry("telephone_answered".into())
                        .or_default()
                        .get_or_insert(frame);
                }
            }
            "ElevatorBeginRideScene" => self.content.mark("elevator_ride", frame),
            "ShipPieceAbility" => self.collecting_ship = true,
            "StartJamOut" => {
                self.content.mark("jam_out", frame);
                let mode = event.stack_slots[0] as u16;
                if mode < 8 {
                    self.content
                        .jam_modes
                        .entry(mode as u8)
                        .or_default()
                        .get_or_insert(frame);
                }
            }
            "PlayEndingSequence" => self.content.mark("ending_started", frame),
            "EndingPlayScrollingCredits" => self.content.mark("ending_credits", frame),
            "MapEnterFinalLevel" => self.content.mark("ending_complete", frame),
            "ShowGameOverAndReset" => self.content.mark("game_over", frame),
            "PlayerFinalizeLastLifeExit" => self.content.mark("last_life_exit", frame),
            "InventoryOpenMailOrderMenu" => self.content.mark("inventory_mail_order", frame),
            "InventoryOpenIdentificationMenu" => {
                self.content.mark("inventory_identification", frame)
            }
            "InventoryIdentifySelectedPresent" => {
                let player = event.stack_slots[0] as u16 as usize;
                if player < 2
                    && event.ram_samples.len() >= 40
                    && (event.ram_samples[38 + player] as i8) >= 2
                {
                    self.identification =
                        Some((player, event.ram_samples[38 + player] as i8, frame));
                }
            }
            "InventoryDropSelectedPresent" => self.content.mark("inventory_drop", frame),
            "MenuControllerOptionsHandler" => self.content.mark("controller_options", frame),
            "GamePaused" => self.content.mark("pause", frame),
            "InventoryBuySelectedMailOrderPresent" => {
                let player = event.stack_slots[0] as u16 as usize;
                if player < 2 && event.ram_samples.len() >= 40 {
                    self.mail_purchase =
                        Some((player, event.ram_samples[38 + player] as i8, event.frame));
                }
            }
            "DropPresentsAbility" => {
                if event.ram_samples.len() >= 45 && event.ram_samples[44] < 32 {
                    self.santa_drop = Some((
                        event.stack_slots[1] & 0xFFFFFF,
                        event.ram_samples[44],
                        frame,
                    ));
                }
            }
            "PlayerProcessLifeRequest" => {
                let receiver = event.stack_slots[0] as u16 as usize;
                let samples = &event.ram_samples;
                if receiver < 2
                    && samples.len() >= 50
                    && (samples[42 + receiver] as i8) < 0
                    && (samples[42 + (receiver ^ 1)] as i8) > 0
                    && samples[45 + (receiver ^ 1)] == 2
                    && samples[47..49] != [0, 0]
                {
                    self.life_request = Some((
                        receiver,
                        samples[42 + (receiver ^ 1)] as i8,
                        samples[49] == 2,
                        frame,
                    ));
                }
            }
            _ => {}
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReplayReport {
    pub session: String,
    pub revision: String,
    pub mode: String,
    pub frames: u64,
    pub logic_ticks: u64,
    pub lag_frames: Vec<u64>,
    pub idle_random_draws: u64,
    pub checked_calls: u64,
    pub replaced_calls: u64,
    pub mismatches: u64,
    #[serde(default)]
    pub future_frame_provenance: tje_host::future_frame_watch::ProvenanceSummary,
    pub entries_hit: usize,
    pub inventory_entries: usize,
    pub entry_counts: BTreeMap<String, u64>,
    pub content: Content,
    /// Full production registration and per-function checks, including zero hits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checks: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disabled_functions: Vec<String>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct MissedEntry {
    pub inventory_id: String,
    pub name: String,
    pub subsystem: String,
    pub source: String,
    pub source_line: u32,
    pub disposition: String,
}

#[derive(Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReportStatus {
    #[default]
    Complete,
    Pending,
    Failed,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Report {
    pub tool: String,
    #[serde(default)]
    pub status: ReportStatus,
    pub sessions: Vec<ReplayReport>,
    pub union_entries_hit: usize,
    pub union_entry_counts: BTreeMap<String, u64>,
    pub content_by_revision: BTreeMap<String, Content>,
    #[serde(default)]
    pub entry_counts_by_revision: BTreeMap<String, BTreeMap<String, u64>>,
    #[serde(default)]
    pub entries_hit_by_revision: BTreeMap<String, usize>,
    #[serde(default)]
    pub missed_entries_by_revision: BTreeMap<String, Vec<MissedEntry>>,
}
impl Report {
    pub fn from_sessions(sessions: Vec<ReplayReport>) -> Self {
        let mut report = Self {
            tool: TOOL_VERSION.into(),
            sessions,
            ..Self::default()
        };
        for session in &report.sessions {
            let provenance = session.future_frame_provenance;
            if session.mismatches != 0
                || provenance.invalidated != 0
                || provenance.status == tje_host::future_frame_watch::ProvenanceStatus::Invalidated
            {
                report.status = ReportStatus::Failed;
            } else if !provenance.is_proof_eligible() && report.status != ReportStatus::Failed {
                report.status = ReportStatus::Pending;
            }
            for (name, count) in &session.entry_counts {
                *report.union_entry_counts.entry(name.clone()).or_default() += count;
                *report
                    .entry_counts_by_revision
                    .entry(session.revision.clone())
                    .or_default()
                    .entry(name.clone())
                    .or_default() += count;
            }
            report
                .content_by_revision
                .entry(session.revision.clone())
                .or_default()
                .merge(&session.content);
        }
        report.union_entries_hit = report
            .union_entry_counts
            .values()
            .filter(|&&count| count != 0)
            .count();
        report.entries_hit_by_revision = report
            .entry_counts_by_revision
            .iter()
            .map(|(revision, counts)| {
                (
                    revision.clone(),
                    counts.values().filter(|&&count| count != 0).count(),
                )
            })
            .collect();
        report.missed_entries_by_revision = report.entry_counts_by_revision.iter().map(|(revision, counts)| {
            let missed = functions().into_iter().filter(|function| {
                counts.get(&format!("{} {}",function.inventory_id,function.name)).copied().unwrap_or(0) == 0
            }).map(|function| MissedEntry {
                inventory_id: function.inventory_id.into(), name: function.name.into(), subsystem: function.subsystem.into(),
                source: function.source.into(), source_line: function.source_line,
                disposition: match function.name {
                    "ReloadActiveCameraViews" | "ShowRandomUncollectedShipPiece" | "NoOp"
                        | "CopyLongwordRunToVramTile" | "FillVdpDataBytes" =>
                        "No callers, vectors, typed function-pointer references or external roots in the complete paired source inventory; see docs/SESSIONS.md",
                    "ErrorTrap" => "Exception-vector terminal loop; normal play does not intentionally enter it",
                    _ => "Uncovered; no unreachability claim",
                }.into(),
            }).collect();
            (revision.clone(),missed)
        }).collect();
        report
    }
}

pub fn replay_report(
    path: &Path,
    session: &Session,
    host: &Host,
    content: Content,
) -> Result<ReplayReport> {
    let entry_counts = functions()
        .into_iter()
        .zip(host.session_entry_counts().unwrap())
        .map(|(function, &count)| {
            (
                format!("{} {}", function.inventory_id, function.name),
                count,
            )
        })
        .collect::<BTreeMap<_, _>>();
    Ok(ReplayReport {
        session: path.display().to_string(),
        revision: format!("{:?}", Revision::from(session.revision)).to_lowercase(),
        mode: format!("{:?}", host.mode()).to_lowercase(),
        frames: session.frames,
        logic_ticks: host.measurements.logic_ticks,
        lag_frames: host
            .measurements
            .lag_events
            .iter()
            .map(|event| event.frame)
            .collect(),
        idle_random_draws: host.measurements.idle_draws_per_frame.iter().sum(),
        checked_calls: host
            .checks
            .by_function
            .values()
            .map(|count| count.calls_checked)
            .sum(),
        replaced_calls: host
            .checks
            .by_function
            .values()
            .map(|count| count.calls_replaced)
            .sum(),
        mismatches: host.checks.mismatches,
        future_frame_provenance: host.future_frame_provenance_outcome(),
        disabled_functions: host
            .checks
            .failures
            .iter()
            .map(|failure| failure.function.clone())
            .collect(),
        entries_hit: entry_counts.values().filter(|&&count| count > 0).count(),
        inventory_entries: entry_counts.len(),
        entry_counts,
        content,
        checks: if host.mode() == Mode::Reference {
            None
        } else {
            Some(serde_json::to_value(&host.checks)?)
        },
    })
}

#[derive(Serialize)]
struct ReplayFailure<'a> {
    tool: &'static str,
    status: &'static str,
    session: &'a Path,
    revision: &'static str,
    mode: &'static str,
    input_frame: u64,
    host_frame: u64,
    pc: u32,
    function: &'static str,
    error: &'a str,
    completed_sessions: &'a [ReplayReport],
    checks: &'a tje_host::CheckReport,
    measurements: &'a tje_host::Measurements,
}

fn record_replay_failure(
    destination: &Path,
    failure: ReplayFailure<'_>,
    original: anyhow::Error,
) -> anyhow::Error {
    match save_json(destination, &failure) {
        Ok(()) => original.context(format!(
            "session {} input frame {}; failure diagnostics {}",
            failure.session.display(),
            failure.input_frame,
            destination.display()
        )),
        Err(write_error) => original.context(format!(
            "session {} input frame {}; additionally failed to write diagnostics: {write_error:#}",
            failure.session.display(),
            failure.input_frame
        )),
    }
}

#[derive(Serialize)]
struct ContinuedFailureEvidence<'a> {
    status: &'static str,
    session: &'a Path,
    input_frame: u64,
    failure: &'a CheckFailure,
    by_function: &'a tje_host::FunctionCoverageTable,
    time_aware: &'a BTreeMap<String, tje_host::check_projection::TimeAwareCheckSummary>,
    nested_original_calls: u64,
    original_wait_frames: u64,
    rust_wait_frames: u64,
}

fn persist_continued_failure(
    directory: &Path,
    session: &Path,
    frame: u64,
    index: usize,
    failure: &CheckFailure,
    host: &Host,
) -> Result<()> {
    if index == 0 {
        fs::create_dir_all(directory)?;
    }
    let evidence_path = directory.join(format!("failure-{index:04}-frame-{}.json", failure.frame));
    save_json(
        &evidence_path,
        &ContinuedFailureEvidence {
            status: "failed",
            session,
            input_frame: frame,
            failure,
            by_function: &host.checks.by_function,
            time_aware: &host.checks.time_aware,
            nested_original_calls: host.checks.nested_original_calls,
            original_wait_frames: host.measurements.check_original_wait_frames,
            rust_wait_frames: host.measurements.check_rust_wait_frames,
        },
    )
    .with_context(|| {
        format!(
            "preserving check failure {} at frame {}: {}",
            failure.function, failure.frame, failure.detail
        )
    })?;
    eprintln!(
        "{}",
        serde_json::json!({
            "event":"check_failure","session":session,"frame":failure.frame,
            "function":failure.function,"trace_index":failure.trace_index,
            "detail":failure.detail,"evidence_path":evidence_path,
        })
    );
    Ok(())
}

pub fn replay(
    bytes: &[u8],
    path: &Path,
    mode: Mode,
    report: &Path,
    completed: &[ReplayReport],
    continue_after_mismatch: bool,
) -> Result<ReplayReport> {
    ensure!(
        !continue_after_mismatch || mode == Mode::Check,
        "--continue-after-mismatch requires --mode check"
    );
    let session = Arc::new(Session::from_json(&fs::read_to_string(path)?)?);
    let revision = Revision::from(session.revision);
    ensure!(
        tje_rom::Rom::import(bytes)?.revision() == revision,
        "ROM revision does not match session"
    );
    let mut host = host(bytes, mode, revision, false)?;
    if continue_after_mismatch {
        host.set_check_failure_policy(CheckFailurePolicy::ContinueOriginal)?;
    }
    host.set_wait_input_poller(Arc::new(SessionInputs::new(Arc::clone(&session))));
    let mut coverage = Coverage::new();
    let mut reported_failures = 0;
    let evidence_dir = if continue_after_mismatch {
        let mut directory = report.as_os_str().to_os_string();
        directory.push(".failures");
        Some(PathBuf::from(directory).join(format!("session-{:04}", completed.len())))
    } else {
        None
    };
    for (frame, pads) in session.inputs().enumerate() {
        // A suspended replacement may already have consumed these frames and
        // applied their recipes through the wait timeline. Never replay them twice.
        if (frame as u64) < host.frame_count() {
            continue;
        }
        apply_recipe(&mut host, &session, frame as u64);
        let snapshot = match host.run_frame(inputs(pads)) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                host.finish_future_frame_provenance();
                let message = error.to_string();
                let pc = host.cpu().pc();
                let failure = ReplayFailure {
                    tool: TOOL_VERSION,
                    status: "failed",
                    session: path,
                    revision: match revision {
                        Revision::Rev02 => "rev02",
                        Revision::Rev00 => "rev00",
                    },
                    mode: match mode {
                        Mode::Reference => "reference",
                        Mode::Check => "check",
                        Mode::Replace => "replace",
                    },
                    input_frame: frame as u64,
                    host_frame: host.frame_count(),
                    pc,
                    function: function_name(revision, pc),
                    error: &message,
                    completed_sessions: completed,
                    checks: &host.checks,
                    measurements: &host.measurements,
                };
                return Err(record_replay_failure(report, failure, error.into()));
            }
        };
        let observations = host.take_session_observations();
        coverage.observe(&host, &snapshot, &observations, revision);
        if let Some(directory) = &evidence_dir {
            for (index, failure) in host
                .checks
                .failures
                .iter()
                .enumerate()
                .skip(reported_failures)
            {
                persist_continued_failure(directory, path, frame as u64, index, failure, &host)?;
            }
            reported_failures = host.checks.failures.len();
        }
    }
    host.finish_future_frame_provenance();
    replay_report(path, &session, &host, coverage.content)
}

#[derive(Debug, Serialize)]
pub struct Divergence {
    pub tick: Option<u64>,
    pub reference_frame: u64,
    pub second_frame: u64,
    pub reference_pc: u32,
    pub second_pc: u32,
    pub reference_function: String,
    pub second_function: String,
    pub ram_start: Option<u32>,
    pub ram_end_exclusive: Option<u32>,
    pub detail: String,
}
#[derive(Debug, Serialize)]
pub struct Comparison {
    pub session: String,
    pub second_mode: String,
    pub boundary: String,
    pub compared: u64,
    pub skipped_lag_ticks: Vec<(u64, u64, u64)>,
    pub divergence: Option<Divergence>,
}
fn ram_difference(a: &[u8], b: &[u8]) -> Option<(u32, u32)> {
    let start = a.iter().zip(b).position(|(a, b)| a != b)?;
    let end = a[start..]
        .iter()
        .zip(&b[start..])
        .position(|(a, b)| a == b)
        .map_or(a.len(), |count| start + count);
    Some((0xFF0000 + start as u32, 0xFF0000 + end as u32))
}
fn divergence(
    revision: Revision,
    tick: Option<u64>,
    frames: [u64; 2],
    pcs: [u32; 2],
    rams: [&[u8]; 2],
    detail: &str,
) -> Divergence {
    let range = ram_difference(rams[0], rams[1]);
    Divergence {
        tick,
        reference_frame: frames[0],
        second_frame: frames[1],
        reference_pc: pcs[0],
        second_pc: pcs[1],
        reference_function: function_name(revision, pcs[0]).into(),
        second_function: function_name(revision, pcs[1]).into(),
        ram_start: range.map(|range| range.0),
        ram_end_exclusive: range.map(|range| range.1),
        detail: detail.into(),
    }
}

/// Frame comparison checks all RAM (including dead stack), video/VDP, ordered
/// hardware writes and bit-exact PCM; no hash-only or ignored-byte shortcuts.
pub fn compare(bytes: &[u8], path: &Path, second: Mode, ticks: bool) -> Result<Comparison> {
    let session = Arc::new(Session::from_json(&fs::read_to_string(path)?)?);
    let revision = Revision::from(session.revision);
    ensure!(
        tje_rom::Rom::import(bytes)?.revision() == revision,
        "ROM revision does not match session"
    );
    let mut reference = host(bytes, Mode::Reference, revision, ticks)?;
    let mut candidate = host(bytes, second, revision, ticks)?;
    let timeline = Arc::new(SessionInputs::new(Arc::clone(&session)));
    reference.set_wait_input_poller(timeline.clone());
    candidate.set_wait_input_poller(timeline);
    let mut result = Comparison {
        session: path.display().to_string(),
        second_mode: format!("{second:?}").to_lowercase(),
        boundary: if ticks { "logic_tick" } else { "video_frame" }.into(),
        compared: 0,
        skipped_lag_ticks: Vec::new(),
        divergence: None,
    };
    let mut queues: [VecDeque<LogicTick>; 2] = std::array::from_fn(|_| VecDeque::new());
    for (frame, pads) in session.inputs().enumerate() {
        apply_recipe(&mut reference, &session, frame as u64);
        apply_recipe(&mut candidate, &session, frame as u64);
        let a = reference.run_frame(inputs(pads))?;
        let b = candidate.run_frame(inputs(pads))?;
        let observations = [
            reference.take_session_observations(),
            candidate.take_session_observations(),
        ];
        if a.frame != b.frame {
            result.divergence = Some(divergence(
                revision,
                None,
                [a.frame, b.frame],
                [a.pc, b.pc],
                [&a.ram, &b.ram],
                "time-progressing replacement crossed video boundaries without intermediate snapshots; refusing a partial timing comparison",
            ));
            return Ok(result);
        }
        if ticks {
            for (queue, observations) in queues.iter_mut().zip(observations) {
                queue.extend(observations.ticks);
            }
            while !queues[0].is_empty() && !queues[1].is_empty() {
                let a = queues[0].pop_front().unwrap();
                let b = queues[1].pop_front().unwrap();
                if a.lagged {
                    result.skipped_lag_ticks.push((a.ordinal, a.frame, b.frame));
                    continue;
                }
                result.compared += 1;
                if a.ordinal != b.ordinal || a.ram != b.ram {
                    let mut difference = divergence(
                        revision,
                        Some(a.ordinal),
                        [a.frame, b.frame],
                        [a.pc, b.pc],
                        [&a.ram, &b.ram],
                        "work RAM differs at completed logic tick",
                    );
                    // Last entered function is execution context, not a claim of
                    // causality for a later write; precise hook errors name the callee.
                    difference.reference_function = a
                        .last_entry
                        .map_or("<none>", |pc| entry_name(revision, pc))
                        .into();
                    difference.second_function = b
                        .last_entry
                        .map_or("<none>", |pc| entry_name(revision, pc))
                        .into();
                    result.divergence = Some(difference);
                    return Ok(result);
                }
            }
            ensure!(
                queues[0].len() < 120 && queues[1].len() < 120,
                "runners differ by more than 120 logic ticks; refused unbounded backlog"
            );
        } else {
            result.compared += 1;
            let detail = if a.ram != b.ram {
                Some("work RAM differs")
            } else if a.vdp != b.vdp {
                Some("VDP memories/registers differ")
            } else if reference.frame_buffer() != candidate.frame_buffer() {
                Some("rendered pixels/frame size differ")
            } else if a.writes != b.writes {
                Some("ordered VDP/YM2612/PSG writes differ")
            } else if a.audio_samples.len() != b.audio_samples.len()
                || a.audio_samples
                    .iter()
                    .flatten()
                    .zip(b.audio_samples.iter().flatten())
                    .any(|(a, b)| a.to_bits() != b.to_bits())
            {
                Some("bit-exact stereo PCM differs")
            } else if a.idle_random_draws != b.idle_random_draws || a.lag_events != b.lag_events {
                Some("idle random draws or lag events differ")
            } else {
                None
            };
            if let Some(detail) = detail {
                result.divergence = Some(divergence(
                    revision,
                    None,
                    [a.frame, b.frame],
                    [a.pc, b.pc],
                    [&a.ram, &b.ram],
                    detail,
                ));
                return Ok(result);
            }
        }
    }
    if ticks && (!queues[0].is_empty() || !queues[1].is_empty()) {
        result.divergence = Some(Divergence {
            tick: None,
            reference_frame: session.frames,
            second_frame: session.frames,
            reference_pc: reference.cpu().pc(),
            second_pc: candidate.cpu().pc(),
            reference_function: function_name(revision, reference.cpu().pc()).into(),
            second_function: function_name(revision, candidate.cpu().pc()).into(),
            ram_start: None,
            ram_end_exclusive: None,
            detail: format!(
                "unpaired completed ticks at session end: reference={}, second={}",
                queues[0].len(),
                queues[1].len()
            ),
        });
    }
    if result.divergence.is_none() {
        reference
            .finish_future_frame_provenance()
            .ensure_proof_eligible()?;
        candidate
            .finish_future_frame_provenance()
            .ensure_proof_eligible()?;
        ensure!(
            candidate.checks.mismatches == 0,
            "comparison has counted check failures"
        );
    }
    Ok(result)
}

pub fn save_json(path: &Path, value: &impl Serialize) -> Result<()> {
    fs::write(path, serde_json::to_string_pretty(value)? + "\n")
        .with_context(|| format!("write report {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reports_first_contiguous_range() {
        assert_eq!(
            ram_difference(&[0, 1, 2, 3, 4], &[0, 7, 8, 3, 9]),
            Some((0xFF0001, 0xFF0003))
        );
        assert_eq!(ram_difference(&[0, 1], &[0, 1]), None);
    }
    #[test]
    fn inventory_denominator_is_exact() {
        assert_eq!(functions().len(), 648);
        for revision in Revision::ALL {
            let entries = functions()
                .iter()
                .map(|function| function.address(revision))
                .collect::<BTreeSet<_>>();
            assert_eq!(entries.len(), 648);
        }
    }

    #[test]
    fn wait_timeline_matches_rle_boundaries_and_runs_the_same_recipes() {
        let mut session = Session::new(Revision::Rev02, "synthetic input and RAM recipe test");
        for pads in [
            [CONNECTED, 0],
            [CONNECTED, 0],
            [CONNECTED | tje_session::A, 0],
        ] {
            session.record(pads).unwrap();
        }
        session.start = tje_session::Start::Recipe {
            description: "write a synthetic word at frame two".into(),
            writes: vec![tje_session::RamWrite {
                before_frame: 2,
                address: 0xff8000,
                width: 2,
                value: 0x1234,
                reason: "exercise identical ordinary-frame and suspended-frame recipes".into(),
            }],
        };
        session.validate().unwrap();
        let timeline = SessionInputs::new(Arc::new(session.clone()));
        for (frame, pads) in session.inputs().enumerate() {
            assert_eq!(timeline.at(frame as u64).unwrap(), inputs(pads));
        }
        assert!(timeline.at(session.frames).is_err());
        let mut memory = tje_mem::AddressSpace::new(Vec::new(), Revision::Rev02);
        timeline.before_frame(1, &mut memory).unwrap();
        assert_eq!(memory.read_u16(0xff8000), 0);
        timeline.before_frame(2, &mut memory).unwrap();
        assert_eq!(memory.read_u16(0xff8000), 0x1234);
    }

    fn observed_entry(
        name: &str,
        frame: u64,
        stack_slots: [u32; 4],
        ram_samples: Vec<u8>,
    ) -> EntryObservation {
        let address = tje_game::functions()
            .find(|function| function.name == name)
            .unwrap()
            .address(Revision::Rev02);
        EntryObservation {
            address,
            frame,
            stack_slots,
            ram_samples,
        }
    }

    #[test]
    fn identification_requires_the_actual_two_buck_transaction() {
        let revision = Revision::Rev02;
        let mut coverage = Coverage::new();
        let mut memory = tje_mem::AddressSpace::new(Vec::new(), revision);
        coverage.entry(
            &observed_entry("InventoryOpenIdentificationMenu", 10, [0; 4], vec![0; 50]),
            revision,
        );
        assert_eq!(
            coverage.content.milestones["inventory_identification"],
            Some(10)
        );
        assert_eq!(
            coverage.content.friendly_interactions["wiseman_identification"],
            None
        );
        let mut samples = vec![0; 50];
        samples[38] = 3;
        ram::g_player_bucks(revision).at(0).write(&mut memory, 3);
        coverage.entry(
            &observed_entry("InventoryIdentifySelectedPresent", 11, [0; 4], samples),
            revision,
        );
        coverage.observe_transactions(&Ram(memory.ram()), revision, 11);
        assert_eq!(
            coverage.content.friendly_interactions["wiseman_identification"],
            None
        );
        ram::g_player_bucks(revision).at(0).write(&mut memory, 1);
        coverage.observe_transactions(&Ram(memory.ram()), revision, 12);
        assert_eq!(
            coverage.content.friendly_interactions["wiseman_identification"],
            Some(12)
        );
    }

    #[test]
    fn telephone_answer_is_reveal_map_entry_not_merely_a_map_or_ring() {
        let revision = Revision::Rev02;
        let mut coverage = Coverage::new();
        let mut samples = vec![0; 50];
        coverage.entry(
            &observed_entry("DrawGameMap", 10, [0; 4], samples.clone()),
            revision,
        );
        assert_eq!(
            coverage.content.friendly_interactions["telephone_answered"],
            None
        );
        samples[41] = 1;
        coverage.entry(
            &observed_entry("DrawGameMap", 11, [0; 4], samples),
            revision,
        );
        assert_eq!(
            coverage.content.friendly_interactions["telephone_answered"],
            Some(11)
        );
    }

    #[test]
    fn santa_requires_a_valid_record_and_a_real_new_dropped_present() {
        let revision = Revision::Rev02;
        let mut coverage = Coverage::new();
        let mut memory = tje_mem::AddressSpace::new(Vec::new(), revision);
        let santa = ram::g_earthling_banks(revision).at(0).at(2);
        santa.kind().write(&mut memory, 20);
        santa.state().write(&mut memory, 3);
        ram::g_dropped_present_table_idx(revision).write(&mut memory, 5);
        let mut samples = vec![0; 50];
        samples[44] = 5;
        coverage.entry(
            &observed_entry(
                "DropPresentsAbility",
                10,
                [0, santa.address(), 0, 0],
                samples,
            ),
            revision,
        );
        coverage.observe_transactions(&Ram(memory.ram()), revision, 10);
        assert_eq!(coverage.content.friendly_interactions["santa_gifts"], None);
        ram::g_dropped_present_table_idx(revision).write(&mut memory, 6);
        coverage.observe_transactions(&Ram(memory.ram()), revision, 11);
        assert_eq!(
            coverage.content.friendly_interactions["santa_gifts"],
            Some(11)
        );
    }

    #[test]
    fn life_request_requires_acceptance_and_both_counter_effects() {
        let revision = Revision::Rev02;
        let mut coverage = Coverage::new();
        let mut memory = tje_mem::AddressSpace::new(Vec::new(), revision);
        ram::g_player_lives(revision).at(0).write(&mut memory, 3);
        ram::g_player_lives(revision).at(1).write(&mut memory, -1);
        let mut samples = vec![0; 50];
        samples[42] = 3;
        samples[43] = 255;
        samples[45] = 2;
        samples[48] = 1;
        samples[49] = 2;
        coverage.entry(
            &observed_entry("PlayerProcessLifeRequest", 10, [1, 0, 0, 0], samples),
            revision,
        );
        coverage.observe_transactions(&Ram(memory.ram()), revision, 10);
        assert_eq!(coverage.content.friendly_interactions["life_request"], None);
        ram::g_player_lives(revision).at(1).write(&mut memory, 0);
        coverage.observe_transactions(&Ram(memory.ram()), revision, 11);
        assert_eq!(coverage.content.friendly_interactions["life_request"], None);
        ram::g_player_lives(revision).at(0).write(&mut memory, 2);
        coverage.observe_transactions(&Ram(memory.ram()), revision, 12);
        assert_eq!(
            coverage.content.friendly_interactions["life_request"],
            Some(12)
        );
    }

    #[test]
    fn failed_checks_persist_counters_before_the_host_is_dropped() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let destination = std::env::temp_dir().join(format!(
            "tje-session-failure-{}-{nonce}.json",
            std::process::id()
        ));
        let mut checks = tje_host::CheckReport {
            nested_original_calls: 3,
            ..Default::default()
        };
        checks.time_aware.insert(
            "AuthoredWait".into(),
            tje_host::check_projection::TimeAwareCheckSummary {
                calls: 1,
                original_interrupts: 2,
                translated_interrupts: 4,
                ..Default::default()
            },
        );
        let measurements = tje_host::Measurements {
            check_original_wait_frames: 5,
            check_rust_wait_frames: 6,
            ..Default::default()
        };
        let error = record_replay_failure(
            &destination,
            ReplayFailure {
                tool: TOOL_VERSION,
                status: "failed",
                session: Path::new("authored.tje.json"),
                revision: "rev02",
                mode: "check",
                input_frame: 7,
                host_frame: 7,
                pc: 0x100,
                function: "AuthoredWait",
                error: "VDP state differs",
                completed_sessions: &[],
                checks: &checks,
                measurements: &measurements,
            },
            anyhow::anyhow!("original check mismatch"),
        );
        let text = fs::read_to_string(&destination).unwrap();
        fs::remove_file(&destination).unwrap();
        let saved: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(saved["status"], "failed");
        assert_eq!(saved["checks"]["nested_original_calls"], 3);
        assert_eq!(
            saved["checks"]["time_aware"]["AuthoredWait"]["original_interrupts"],
            2
        );
        assert_eq!(
            saved["checks"]["time_aware"]["AuthoredWait"]["translated_interrupts"],
            4
        );
        assert_eq!(saved["measurements"]["check_original_wait_frames"], 5);
        assert_eq!(saved["measurements"]["check_rust_wait_frames"], 6);
        assert!(saved["checks"].get("last_time_aware").is_none());
        assert!(
            serde_json::from_str::<Report>(&text).is_err(),
            "a failed run cannot masquerade as a complete coverage report"
        );
        assert!(format!("{error:#}").contains("original check mismatch"));
    }

    #[test]
    fn diagnostic_write_errors_do_not_erase_the_original_check_failure() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let destination = std::env::temp_dir()
            .join(format!("tje-session-absent-{}-{nonce}", std::process::id()))
            .join("failure.json");
        let checks = tje_host::CheckReport::default();
        let measurements = tje_host::Measurements::default();
        let error = record_replay_failure(
            &destination,
            ReplayFailure {
                tool: TOOL_VERSION,
                status: "failed",
                session: Path::new("authored.tje.json"),
                revision: "rev02",
                mode: "check",
                input_frame: 7,
                host_frame: 7,
                pc: 0x100,
                function: "AuthoredWait",
                error: "VDP state differs",
                completed_sessions: &[],
                checks: &checks,
                measurements: &measurements,
            },
            anyhow::anyhow!("original check mismatch"),
        );
        let chain = format!("{error:#}");
        assert!(chain.contains("original check mismatch"));
        assert!(chain.contains("failed to write diagnostics"));
        assert!(!destination.exists());
    }

    fn authored_report_session(mismatches: u64) -> ReplayReport {
        ReplayReport {
            session: "authored.tje.json".into(),
            revision: "rev02".into(),
            mode: "check".into(),
            frames: 100,
            logic_ticks: 10,
            lag_frames: Vec::new(),
            idle_random_draws: 0,
            checked_calls: 1,
            replaced_calls: 0,
            mismatches,
            future_frame_provenance: Default::default(),
            entries_hit: 0,
            inventory_entries: 648,
            entry_counts: BTreeMap::new(),
            content: Content::new(),
            disabled_functions: if mismatches == 0 {
                Vec::new()
            } else {
                vec!["AuthoredFailed".into()]
            },
            checks: Some(serde_json::json!({"by_function":{
                "AuthoredFailed":{"calls_checked":1,"mismatches":mismatches},
                "AuthoredUntouched":{"calls_checked":0,"mismatches":0}
            }})),
        }
    }

    #[test]
    fn continued_failures_remain_failed_when_reports_are_merged() {
        let session = authored_report_session(1);
        let report = Report::from_sessions(vec![session]);
        assert_eq!(report.status, ReportStatus::Failed);
        let value = serde_json::to_value(&report).unwrap();
        assert_eq!(value["status"], "failed");
        assert_eq!(
            value["sessions"][0]["checks"]["by_function"]["AuthoredUntouched"]["calls_checked"],
            0
        );
        assert_eq!(
            value["sessions"][0]["disabled_functions"][0],
            "AuthoredFailed"
        );
        let decoded: Report = serde_json::from_value(value).unwrap();
        assert_eq!(
            Report::from_sessions(decoded.sessions).status,
            ReportStatus::Failed
        );
    }

    #[test]
    fn unresolved_or_invalidated_provenance_never_roundtrips_into_a_clean_session_report() {
        use tje_host::future_frame_watch::{ProvenanceStatus, ProvenanceSummary};
        let pending = ProvenanceSummary {
            status: ProvenanceStatus::Provisional,
            approvals: 1,
            provisional: 1,
            ..Default::default()
        };
        let invalidated = ProvenanceSummary {
            status: ProvenanceStatus::Invalidated,
            approvals: 1,
            invalidated: 1,
            ..Default::default()
        };
        for (summary, expected) in [
            (pending, ReportStatus::Pending),
            (invalidated, ReportStatus::Failed),
        ] {
            let mut session = authored_report_session(0);
            session.future_frame_provenance = summary;
            let full = serde_json::json!({
                "summary": summary, "authored_full_evidence": {"bytes":[17,34],"entry_clock":1234},
            });
            session.checks.as_mut().unwrap()["future_frame_provenance"] = full.clone();
            let report = Report::from_sessions(vec![session]);
            assert_eq!(report.status, expected);
            assert_eq!(
                report.sessions[0].mismatches, 0,
                "pending is not a fabricated game mismatch"
            );
            let encoded = serde_json::to_value(&report).unwrap();
            assert_eq!(
                encoded["sessions"][0]["checks"]["future_frame_provenance"],
                full
            );
            let decoded: Report = serde_json::from_value(encoded).unwrap();
            assert_eq!(Report::from_sessions(decoded.sessions).status, expected);
        }
        for pending_first in [false, true] {
            let mut session = authored_report_session(0);
            session.future_frame_provenance = pending;
            let failed = authored_report_session(1);
            let sessions = if pending_first {
                vec![session, failed]
            } else {
                vec![failed, session]
            };
            assert_eq!(Report::from_sessions(sessions).status, ReportStatus::Failed);
        }
        assert_eq!(
            Report::from_sessions(vec![authored_report_session(0)]).status,
            ReportStatus::Complete
        );
    }

    #[test]
    fn continuation_is_rejected_outside_check_mode_before_any_input_is_read() {
        for mode in [Mode::Reference, Mode::Replace] {
            let error = replay(
                &[],
                Path::new("must-not-be-read.tje.json"),
                mode,
                Path::new("must-not-be-written.json"),
                &[],
                true,
            )
            .unwrap_err();
            assert!(error.to_string().contains("requires --mode check"));
        }
    }
}
