//! Explicit native-size course ownership. The original device, art, movement,
//! animation and AI leaves remain ROM-backed; terrain and initial population do
//! not execute any original generation branch.
use crate::{
    admission::{item_kind, kind},
    *,
};
use bincode::{Decode, Encode};
use tje_engine_runtime::enhanced::{
    Context, Continuation, EnhancedMode, EnhancedProfile, OverrideStep, ResumableTranslation,
    Sidecar,
};
use tje_game::{FunctionId, ids};
use tje_mem::{AddressSpace, ram, rom};

pub const PROFILE_ID: &str = "maker-native-v1";
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum RunStatus {
    Ready,
    Playing,
    Complete,
    Failed,
}
#[derive(Debug, Clone, Copy, Encode, Decode)]
struct TelephoneRuntime {
    record: [u8; 8],
    timers: [i8; 3],
}
#[derive(Debug, Clone, Encode, Decode)]
struct RuntimeLevel {
    cells: Vec<u16>,
    actors: Vec<(u32, [u8; 18])>,
    items: Vec<(u32, [u8; 8])>,
    telephone: Option<TelephoneRuntime>,
    path_deadlines: Vec<Option<u64>>,
    collected: u32,
}
/// All authoritative enhanced data, including immutable normalized course
/// identity, is registered as one snapshot-bound typed sidecar.
#[derive(Debug, Clone, Encode, Decode)]
pub struct MakerState {
    course: ValidatedCourse,
    digest: [u8; 32],
    levels: Vec<RuntimeLevel>,
    status: RunStatus,
    first_frame: Option<u64>,
    last_tick: u64,
    pressed_up: [bool; 2],
    pending_up: [bool; 2],
    travelling: bool,
    phone_owners: [Option<u8>; 2],
    pending: [Option<Destination>; 2],
}
impl Sidecar for MakerState {
    const KEY: &'static str = "tje-maker";
    const VERSION: u32 = 1;
    fn configuration_identity(&self) -> &[u8] {
        &self.digest
    }
    fn validate(&self) -> tje_game::Result<()> {
        if self.course.digest() != self.digest || self.levels.len() != self.course.levels.len() {
            return Err(problem(
                "snapshot course identity or canonical order is invalid",
            ));
        }
        for (index, runtime) in self.levels.iter().enumerate() {
            let level = &self.course.levels[index];
            if runtime.cells.len() != 494
                || runtime.cells.iter().any(|&cell| !valid_cell(cell))
                || runtime.actors.len()
                    != level
                        .objects
                        .iter()
                        .filter(|object| kind(object).is_some())
                        .count()
                || runtime.items.len()
                    != level
                        .objects
                        .iter()
                        .filter(|object| matches!(object, Object::Item { .. }))
                        .count()
                || runtime.path_deadlines.len() != level.paths.len()
                || runtime.telephone.is_some()
                    != level
                        .objects
                        .iter()
                        .any(|object| matches!(object, Object::Telephone { .. }))
            {
                return Err(problem("snapshot runtime level shape is invalid"));
            }
            for ((id, record), object) in runtime
                .actors
                .iter()
                .zip(level.objects.iter().filter(|object| kind(object).is_some()))
            {
                if *id != object.id()
                    || (record[0] != 255 && record[0] >= 25)
                    || record[1] != 0
                    || record[15] != 0
                    || record[2] != (index + 2) as u8
                {
                    return Err(problem("snapshot actor identity/handle is invalid"));
                }
            }
            for ((id, record), object) in runtime.items.iter().zip(
                level
                    .objects
                    .iter()
                    .filter(|object| matches!(object, Object::Item { .. })),
            ) {
                if *id != object.id() || record[2] != 0 || record[1] != (index + 2) as u8 {
                    return Err(problem("snapshot item identity/handle is invalid"));
                }
            }
            if runtime.telephone.is_some_and(|phone| {
                phone.record[2] != 0 || ![0, (index + 2) as u8].contains(&phone.record[1])
            }) {
                return Err(problem("snapshot telephone identity/handle is invalid"));
            }
        }
        for destination in self.pending.iter().flatten() {
            if !self.course.levels.iter().any(|level| {
                level.id == destination.level && level.spawn(destination.spawn).is_some()
            }) {
                return Err(problem("snapshot pending travel destination is invalid"));
            }
        }
        if self
            .phone_owners
            .iter()
            .flatten()
            .any(|&owner| usize::from(owner) >= self.levels.len())
        {
            return Err(problem("snapshot telephone bank owner is invalid"));
        }
        Ok(())
    }
}
impl MakerState {
    pub fn status(&self) -> RunStatus {
        self.status
    }
    pub fn is_transitioning(&self) -> bool {
        self.travelling
    }
    pub fn digest(&self) -> [u8; 32] {
        self.digest
    }
    pub fn course(&self) -> &Course {
        &self.course
    }
    fn physical(&self, id: u16) -> i8 {
        (self
            .course
            .levels
            .iter()
            .position(|level| level.id == id)
            .expect("validated level reference")
            + 2) as i8
    }
    fn index(&self, physical: i8) -> tje_game::Result<usize> {
        let index = usize::try_from(physical)
            .ok()
            .and_then(|floor| floor.checked_sub(2));
        index.filter(|&index| index < self.levels.len()).ok_or_else(|| problem("original code requested a floor outside the authored course; stock generation is forbidden"))
    }
}
fn problem(message: impl Into<String>) -> tje_game::Error {
    tje_game::Error::Platform(format!("Maker: {}", message.into()))
}
fn returned() -> tje_game::Result<OverrideStep> {
    Ok(OverrideStep::Return(None))
}
fn original(
    continuation: &mut Continuation,
    stage: u32,
    function: FunctionId,
    arguments: Vec<u32>,
) -> tje_game::Result<OverrideStep> {
    continuation.stage = stage;
    Ok(OverrideStep::CallOriginal {
        function,
        arguments,
    })
}
fn state(context: &dyn Context) -> tje_game::Result<&MakerState> {
    context.sidecars().get::<MakerState>()
}
fn state_mut(context: &mut dyn Context) -> tje_game::Result<&mut MakerState> {
    context.sidecars_mut().get_mut::<MakerState>()
}

/// Admission constructs a candidate without touching machine state. Call only
/// after an explicit Maker selection; merely importing a file never selects it.
pub fn new_mode(document: &ValidatedCourse, memory: &AddressSpace) -> Result<EnhancedMode, Error> {
    admit_native(document, memory)?;
    let mut levels = Vec::with_capacity(document.course().levels.len());
    for (index, level) in document.course().levels.iter().enumerate() {
        let physical = (index + 2) as i8;
        let mut runtime = RuntimeLevel {
            cells: Vec::with_capacity(494),
            actors: Vec::new(),
            items: Vec::new(),
            telephone: None,
            path_deadlines: vec![None; level.paths.len()],
            collected: 0,
        };
        for &(count, cell) in &level.cells {
            runtime
                .cells
                .extend(std::iter::repeat_n(cell, usize::from(count)));
        }
        for object in &level.objects {
            let Point { x, y } = object.position();
            if let Some(kind) = kind(object) {
                let mut record = [0; 18];
                record[0] = kind as u8;
                record[2] = physical as u8;
                record[3] = if matches!(
                    object,
                    Object::Earthling {
                        initial: InitialState::Asleep,
                        ..
                    }
                ) {
                    99
                } else {
                    0
                };
                record[6..8].copy_from_slice(&x.to_be_bytes());
                record[8..10].copy_from_slice(&y.to_be_bytes());
                let saved_x = match kind {
                    9 | 19 => 0,
                    7 => 1,
                    _ => x,
                };
                let saved_y = if matches!(kind, 9 | 19) { 0 } else { y };
                record[10..12].copy_from_slice(&saved_x.to_be_bytes());
                record[12..14].copy_from_slice(&saved_y.to_be_bytes());
                record[16] = rom::earthling_definitions(memory.revision())
                    .at(i32::from(kind))
                    .health()
                    .read(memory) as u8;
                runtime.actors.push((object.id(), record));
            } else if let Object::Item { definition, .. } = object {
                let mut record = [0; 8];
                record[0] = item_kind(*definition) as u8;
                record[1] = physical as u8;
                record[4..6].copy_from_slice(&x.to_be_bytes());
                record[6..8].copy_from_slice(&y.to_be_bytes());
                runtime.items.push((object.id(), record));
            } else if matches!(object, Object::Telephone { .. }) {
                let mut record = [0; 8];
                record[1] = physical as u8;
                record[4..6].copy_from_slice(&x.to_be_bytes());
                record[6..8].copy_from_slice(&y.to_be_bytes());
                runtime.telephone = Some(TelephoneRuntime {
                    record,
                    timers: [0; 3],
                });
            }
        }
        levels.push(runtime);
    }
    let initial = MakerState {
        course: document.clone(),
        digest: document.digest(),
        levels,
        status: RunStatus::Ready,
        first_frame: None,
        last_tick: 0,
        pressed_up: [false; 2],
        pending_up: [false; 2],
        travelling: false,
        phone_owners: [None; 2],
        pending: [None; 2],
    };
    let mut mode = EnhancedMode::new(
        EnhancedProfile::new(PROFILE_ID, &[], &[]).with_resumable(OVERRIDES),
        memory.revision(),
    )
    .map_err(|error| Error::Format(error.to_string()))?;
    mode.sidecars_mut()
        .register(initial)
        .map_err(|error| Error::Format(error.to_string()))?;
    Ok(mode)
}

fn read_record<const N: usize>(context: &dyn Context, address: u32) -> [u8; N] {
    std::array::from_fn(|i| context.read_u8(address + i as u32))
}
fn write_record(context: &mut dyn Context, address: u32, record: &[u8]) {
    for (offset, &value) in record.iter().enumerate() {
        context.write_u8(address + offset as u32, value);
    }
}

fn write_map(context: &mut dyn Context, viewport: i32, index: usize) -> tje_game::Result<()> {
    let r = context.revision();
    let map = if viewport == 0 {
        ram::g_player1_level_map(r)
    } else {
        ram::g_player2_level_map(r)
    };
    for cell in 0..494 {
        let word = state(context)?.levels[index].cells[cell];
        map.at(cell as i32 / 19)
            .at(cell as i32 % 19)
            .write(context, word);
    }
    if viewport == 0 {
        ram::g_player1_map_level(r).write(context, (index + 2) as i8);
    } else {
        ram::g_player2_map_level(r).write(context, (index + 2) as i8);
    }
    Ok(())
}
fn publish_maps(context: &mut dyn Context) -> tje_game::Result<()> {
    for viewport in 0..2 {
        let index = state(context)?.index(view_floor(context, viewport))?;
        write_map(context, viewport, index)?;
    }
    ram::g_saved_map_level(context.revision()).write(context, -1);
    Ok(())
}
fn save_banks(context: &mut dyn Context) -> tje_game::Result<()> {
    let r = context.revision();
    for bank in 0..2 {
        if ram::g_earthling_bank_users(r).at(bank).read(context) != 0 {
            let index =
                state(context)?.index(ram::g_earthling_bank_levels(r).at(bank).read(context))?;
            let count = state(context)?.levels[index].actors.len();
            for slot in 0..count {
                let mut record = read_record::<18>(
                    context,
                    ram::g_earthling_banks(r).at(bank).at(slot as i32).address(),
                );
                record[1] = 0;
                record[15] = 0;
                record[17] = 0;
                state_mut(context)?.levels[index].actors[slot].1 = record;
            }
        }
        if ram::g_item_bank_users(r).at(bank).read(context) != 0 {
            let physical = ram::g_item_bank_levels(r).at(bank).read(context);
            let index = state(context)?.index(physical)?;
            let count = state(context)?.levels[index].items.len();
            for slot in 0..count {
                let mut record = read_record::<8>(
                    context,
                    ram::g_object_table(r).at(bank).at(slot as i32).address(),
                );
                record[2] = 0;
                state_mut(context)?.levels[index].items[slot].1 = record;
            }
            let bits = ram::g_collected_level_item_bits(r)
                .at(i32::from(physical))
                .read(context);
            state_mut(context)?.levels[index].collected = bits;
        }
        if let Some(owner) = state(context)?.phone_owners[bank as usize] {
            let mut record =
                read_record::<8>(context, ram::g_telephone_states(r).at(bank).address());
            record[2] = 0;
            let timers = [
                ram::g_telephone_activated(r).at(bank).read(context),
                ram::g_telephone_rings_left(r).at(bank).read(context),
                ram::g_telephone_time_to_ring(r).at(bank).read(context),
            ];
            state_mut(context)?.levels[usize::from(owner)].telephone =
                Some(TelephoneRuntime { record, timers });
        }
    }
    Ok(())
}
fn retain_actors(context: &mut dyn Context, physical: i8) -> tje_game::Result<()> {
    let r = context.revision();
    let index = state(context)?.index(physical)?;
    for bank in 0..2 {
        let users = ram::g_earthling_bank_users(r).at(bank);
        if users.read(context) != 0
            && ram::g_earthling_bank_levels(r).at(bank).read(context) == physical
        {
            users.write(context, users.read(context) + 1);
            return Ok(());
        }
    }
    let bank = (0..2)
        .find(|&bank| ram::g_earthling_bank_users(r).at(bank).read(context) == 0)
        .ok_or_else(|| problem("no unused actor bank"))?;
    for slot in 0..20 {
        let record = state(context)?.levels[index].actors.get(slot).map_or(
            {
                let mut empty = [0; 18];
                empty[0] = 255;
                empty
            },
            |record| record.1,
        );
        write_record(
            context,
            ram::g_earthling_banks(r).at(bank).at(slot as i32).address(),
            &record,
        );
    }
    ram::g_earthling_bank_levels(r)
        .at(bank)
        .write(context, physical);
    ram::g_earthling_bank_users(r).at(bank).write(context, 1);
    let mailbox = ram::g_mailbox_level_states(r).at(bank);
    mailbox.mode().write(context, 0);
    mailbox.level().write(context, physical);
    mailbox.timer().write(context, 0);
    mailbox.contact().at(0).write(context, 0);
    mailbox.contact().at(1).write(context, 0);
    Ok(())
}
fn retain_items(context: &mut dyn Context, physical: i8) -> tje_game::Result<()> {
    let r = context.revision();
    let index = state(context)?.index(physical)?;
    for bank in 0..2 {
        let users = ram::g_item_bank_users(r).at(bank);
        if users.read(context) != 0 && ram::g_item_bank_levels(r).at(bank).read(context) == physical
        {
            users.write(context, users.read(context) + 1);
            return Ok(());
        }
    }
    let bank = (0..2)
        .find(|&bank| ram::g_item_bank_users(r).at(bank).read(context) == 0)
        .ok_or_else(|| problem("no unused item bank"))?;
    for slot in 0..32 {
        let record = state(context)?.levels[index].items.get(slot).map_or(
            {
                let mut empty = [0; 8];
                empty[0] = 255;
                empty
            },
            |record| record.1,
        );
        write_record(
            context,
            ram::g_object_table(r).at(bank).at(slot as i32).address(),
            &record,
        );
    }
    let bits = state(context)?.levels[index].collected;
    ram::g_collected_level_item_bits(r)
        .at(i32::from(physical))
        .write(context, bits);
    ram::g_item_bank_levels(r).at(bank).write(context, physical);
    ram::g_item_bank_users(r).at(bank).write(context, 1);
    Ok(())
}
fn place(context: &mut dyn Context, hero: i32, destination: Destination) -> tje_game::Result<()> {
    let physical = state(context)?.physical(destination.level);
    let index = state(context)?.index(physical)?;
    let position = state(context)?.course.levels[index]
        .spawn(destination.spawn)
        .ok_or_else(|| problem("missing admitted spawn"))?;
    let r = context.revision();
    let player = ram::g_player_info(r).at(hero);
    player.level().write(context, physical);
    player.x().write(context, position.x as i16);
    player.y().write(context, position.y as i16);
    player.height().write(context, 0);
    player.velocity_x().write(context, 0);
    player.velocity_y().write(context, 0);
    player.velocity_height().write(context, 0);
    player.acceleration_height().write(context, 0);
    ram::g_player_fall_states(r).at(hero).write(context, 0);
    ram::g_player_elevator_flags(r).at(hero).write(context, 0);
    Ok(())
}

fn setup_game(context: &mut dyn Context, c: &mut Continuation) -> tje_game::Result<OverrideStep> {
    let r = context.revision();
    if c.stage == 0 {
        let heroes = state(context)?.course.rules.heroes;
        for hero in 0..2 {
            ram::g_player_lives(r)
                .at(hero)
                .write(context, if hero < i32::from(heroes) { 3 } else { -1 });
        }
        return original(c, 1, ids::SETUP_THE_MAIN_GAME, vec![]);
    }
    let (lives, bucks, level) = {
        let s = state(context)?;
        (
            s.course.rules.lives as i8,
            s.course.rules.bucks as i8,
            s.physical(s.course.start.level),
        )
    };
    for hero in 0..2 {
        ram::g_player_info(r).at(hero).level().write(context, level);
        if hero < i32::from(state(context)?.course.rules.heroes) {
            ram::g_player_lives(r).at(hero).write(context, lives);
            ram::g_player_bucks(r).at(hero).write(context, bucks);
        }
    }
    let highest = (state(context)?.course.levels.len() + 1) as i8;
    ram::g_highest_level(r).write(context, highest);
    returned()
}
fn set_position(context: &mut dyn Context, _: &mut Continuation) -> tje_game::Result<OverrideStep> {
    let hero = context.entry_arguments()?[0] as i32;
    for seat in 0..2 {
        if hero != 2 && hero != seat {
            continue;
        }
        let s = state(context)?;
        let (level, spawn) = if s.status == RunStatus::Ready {
            (
                s.course.start.level,
                s.course
                    .start
                    .spawns
                    .get(seat as usize)
                    .copied()
                    .unwrap_or(s.course.start.spawns[0]),
            )
        } else {
            let physical = ram::g_player_info(context.revision())
                .at(seat)
                .level()
                .read(context);
            let index = s.index(physical)?;
            (
                s.course.levels[index].id,
                s.course.levels[index].spawn_points[0].id,
            )
        };
        place(context, seat, Destination { level, spawn })?;
    }
    publish_maps(context)?;
    returned()
}
fn generate(context: &mut dyn Context, c: &mut Continuation) -> tje_game::Result<OverrideStep> {
    if c.stage != 0 {
        return returned();
    }
    let r = context.revision();
    let seed =
        u64::from_str_radix(&state(context)?.course.runtime_seed, 16).expect("validated seed");
    ram::g_rng_state(r).write(context, ((seed ^ (seed >> 32)) as u32 | 1) as i32);
    for index in 0..26 {
        ram::g_level_seeds(r).at(index).write(context, 0);
    }
    publish_maps(context)?;
    original(c, 1, ids::PRE_GENERATE_RANDOMS, vec![])
}
fn map_load(context: &mut dyn Context, _: &mut Continuation) -> tje_game::Result<OverrideStep> {
    let arguments = context.entry_arguments()?;
    let hero = arguments[0] as i32;
    if !(0..2).contains(&hero) {
        return Err(problem("map loader requires seat 0 or 1"));
    }
    let physical = arguments[1] as i8;
    let index = state(context)?.index(physical)?;
    let r = context.revision();
    let viewport = if ram::g_player_lives(r).at(0).read(context) < 0 {
        0
    } else {
        hero
    };
    ram::g_requested_map_level(r).write(context, physical);
    write_map(context, viewport, index)?;
    returned()
}
fn map_change(context: &mut dyn Context, _: &mut Continuation) -> tje_game::Result<OverrideStep> {
    let hero = context.entry_arguments()?[0] as i32;
    if !(0..2).contains(&hero) {
        return Err(problem("map change requires seat 0 or 1"));
    }
    let r = context.revision();
    let index = state(context)?.index(ram::g_requested_map_level(r).read(context))?;
    write_map(context, hero, index)?;
    Ok(OverrideStep::Return(Some(0)))
}
fn map_sync(context: &mut dyn Context, _: &mut Continuation) -> tje_game::Result<OverrideStep> {
    publish_maps(context)?;
    returned()
}
fn reject_final(_: &mut dyn Context, _: &mut Continuation) -> tje_game::Result<OverrideStep> {
    Err(problem(
        "original final-level loader is not a course transition",
    ))
}
fn initial_items(
    context: &mut dyn Context,
    _: &mut Continuation,
) -> tje_game::Result<OverrideStep> {
    let r = context.revision();
    for hero in 0..2 {
        ram::g_item_bank_users(r).at(hero).write(context, 0);
        ram::g_item_bank_levels(r).at(hero).write(context, -1);
        for slot in 0..32 {
            let item = ram::g_object_table(r).at(hero).at(slot);
            write_record(context, item.address(), &[255, 0, 0, 0, 0, 0, 0, 0]);
        }
        for slot in 0..16 {
            let identity = state(context)?
                .course
                .rules
                .inventory
                .get(slot as usize)
                .map_or(-1, |&id| id as i8);
            ram::g_player_inventory(r)
                .at(hero)
                .at(slot)
                .write(context, identity);
        }
    }
    for identity in 0..28 {
        ram::g_present_info(r)
            .at(identity)
            .appearance()
            .write(context, identity as i8);
        let identified = state(context)?.course.rules.identify_presents || identity == 27;
        ram::g_present_info(r)
            .at(identity)
            .identified()
            .write(context, i8::from(identified));
    }
    for slot in 0..32 {
        write_record(
            context,
            ram::g_dropped_present_table(r).at(slot).address(),
            &[255, 0, 0, 0, 0, 0, 0, 0],
        );
    }
    for level in 0..26 {
        ram::g_collected_level_item_bits(r)
            .at(level)
            .write(context, 0);
    }
    for hero in 0..i32::from(state(context)?.course.rules.heroes) {
        let physical = ram::g_player_info(r).at(hero).level().read(context);
        retain_items(context, physical)?;
    }
    returned()
}
fn assign_pieces(
    context: &mut dyn Context,
    _: &mut Continuation,
) -> tje_game::Result<OverrideStep> {
    let r = context.revision();
    for piece in 0..10 {
        ram::g_ship_piece_level_assignments(r)
            .at(piece)
            .write(context, -1);
    }
    let count = state(context)?.course.levels.len();
    for index in 0..count {
        for object_index in 0..state(context)?.course.levels[index].objects.len() {
            if let Object::ShipPiece { piece, .. } =
                state(context)?.course.levels[index].objects[object_index]
            {
                ram::g_ship_piece_level_assignments(r)
                    .at(i32::from(piece))
                    .write(context, (index + 2) as i8);
            }
        }
    }
    returned()
}
fn initial_actors(
    context: &mut dyn Context,
    c: &mut Continuation,
) -> tje_game::Result<OverrideStep> {
    let r = context.revision();
    for bank in 0..2 {
        ram::g_earthling_bank_users(r).at(bank).write(context, 0);
        ram::g_earthling_bank_levels(r).at(bank).write(context, -1);
        for slot in 0..20 {
            let record = ram::g_earthling_banks(r).at(bank).at(slot);
            record.kind().write(context, -1);
            record.object().write(context, 0);
            record.second_object().write(context, 0);
        }
    }
    for level in 0..26 {
        ram::g_earthling_spawn_failure_masks(r)
            .at(level)
            .write(context, 0);
    }
    assign_pieces(context, c)?;
    for hero in 0..i32::from(state(context)?.course.rules.heroes) {
        let physical = ram::g_player_info(r).at(hero).level().read(context);
        retain_actors(context, physical)?;
    }
    state_mut(context)?.status = RunStatus::Playing;
    state_mut(context)?.first_frame = Some(context.frame_count());
    returned()
}
fn load_actors(context: &mut dyn Context, _: &mut Continuation) -> tje_game::Result<OverrideStep> {
    let level = context.entry_arguments()?[0] as i8;
    retain_actors(context, level)?;
    returned()
}
fn load_items(context: &mut dyn Context, _: &mut Continuation) -> tje_game::Result<OverrideStep> {
    let level = context.entry_arguments()?[0] as i8;
    retain_items(context, level)?;
    returned()
}
fn release_actors(
    context: &mut dyn Context,
    c: &mut Continuation,
) -> tje_game::Result<OverrideStep> {
    if c.stage == 0 {
        save_banks(context)?;
        return original(
            c,
            1,
            ids::EARTHLING_RELEASE_LEVEL_BANK,
            context.entry_arguments()?,
        );
    }
    returned()
}
fn release_items(
    context: &mut dyn Context,
    c: &mut Continuation,
) -> tje_game::Result<OverrideStep> {
    if c.stage == 0 {
        save_banks(context)?;
        return original(
            c,
            1,
            ids::MAP_RELEASE_LEVEL_ITEM_BANK,
            context.entry_arguments()?,
        );
    }
    returned()
}

fn configure_props(context: &mut dyn Context) -> tje_game::Result<()> {
    let r = context.revision();
    for bank in 0..2 {
        let physical = view_floor(context, bank);
        let index = state(context)?.index(physical)?;
        let distinct = bank == 0
            || (ram::g_player_lives(r).at(1).read(context) >= 0
                && physical != view_floor(context, 0));
        for endpoint in 0..2 {
            let position = if distinct {
                state(context)?.course.levels[index]
                    .objects
                    .iter()
                    .filter(|object| object.endpoint())
                    .nth(endpoint as usize)
                    .map(Object::position)
            } else {
                None
            };
            let record = ram::g_elevator_records(r).at(bank).at(endpoint);
            record.entity().write(context, 0);
            record
                .state()
                .write(context, if position.is_some() { 0 } else { 11 });
            record
                .level()
                .write(context, if position.is_some() { physical } else { -1 });
            if let Some(position) = position {
                record.x().write(context, position.x as i16);
                record.y().write(context, position.y as i16);
            }
        }
        let telephone = if distinct {
            state(context)?.levels[index].telephone
        } else {
            None
        };
        state_mut(context)?.phone_owners[bank as usize] = telephone.map(|_| index as u8);
        let telephone = telephone.unwrap_or(TelephoneRuntime {
            record: [0; 8],
            timers: [0; 3],
        });
        write_record(
            context,
            ram::g_telephone_states(r).at(bank).address(),
            &telephone.record,
        );
        ram::g_telephone_activated(r)
            .at(bank)
            .write(context, telephone.timers[0]);
        ram::g_telephone_rings_left(r)
            .at(bank)
            .write(context, telephone.timers[1]);
        ram::g_telephone_time_to_ring(r)
            .at(bank)
            .write(context, telephone.timers[2]);
    }
    Ok(())
}
fn initial_props(
    context: &mut dyn Context,
    _: &mut Continuation,
) -> tje_game::Result<OverrideStep> {
    configure_props(context)?;
    returned()
}
fn setup_elevator(
    context: &mut dyn Context,
    _: &mut Continuation,
) -> tje_game::Result<OverrideStep> {
    let args = context.entry_arguments()?;
    let r = context.revision();
    for bank in 0..2 {
        for endpoint in 0..2 {
            if ram::g_elevator_records(r)
                .at(bank)
                .at(endpoint)
                .level()
                .read(context)
                == args[0] as i8
            {
                return Ok(OverrideStep::Return(Some((bank * 2 + endpoint + 1) as u32)));
            }
        }
    }
    Ok(OverrideStep::Return(Some(0)))
}
fn no_elevator_collision(
    _: &mut dyn Context,
    _: &mut Continuation,
) -> tje_game::Result<OverrideStep> {
    Ok(OverrideStep::Return(Some(0)))
}

fn allocate_actor(
    context: &mut dyn Context,
    c: &mut Continuation,
) -> tje_game::Result<OverrideStep> {
    let r = context.revision();
    if c.stage == 0 {
        let args = context.entry_arguments()?;
        c.locals[0] = args[0];
        c.locals[1] = context.read_u8(args[0] + 3) as u32;
        return original(c, 1, ids::EARTHLING_ALLOCATE_ENTITIES, args);
    }
    if c.locals[1] != 99 {
        return returned();
    }
    let address = c.locals[0];
    let number = context.read_u8(address + 1);
    if number == 0 {
        return Err(problem(
            "admitted actor allocation failed instead of preserving its authored position",
        ));
    }
    let companion = context.read_u8(address + 15);
    if companion != 0 && c.stage == 1 {
        context.write_u8(address + 15, 0);
        return original(c, 2, ids::FREE_ENTITY, vec![u32::from(companion)]);
    }
    let kind = context.read_u8(address);
    let animation = rom::sleep_animation_descriptors(r).at(i32::from(kind));
    let object = ram::g_player_info(r).at(i32::from(number));
    object
        .sequence()
        .write(context, animation.sequence().read(context));
    object
        .frames()
        .write(context, animation.frames().read(context));
    object.frame_phase().write(context, 6);
    object.frame_duration().write(context, 1);
    object.frame().write(context, -1);
    object.sprite_state().write(context, 99);
    object.velocity_x().write(context, 0);
    object.velocity_y().write(context, 0);
    context.write_u8(address + 3, 99);
    returned()
}

fn collected(s: &MakerState, id: u32) -> bool {
    s.levels.iter().any(|level| {
        level
            .items
            .iter()
            .enumerate()
            .any(|(index, (candidate, record))| {
                *candidate == id
                    && (record[0] == 255 || level.collected & (0x8000_0000 >> index) != 0)
            })
            || level
                .actors
                .iter()
                .any(|(candidate, record)| *candidate == id && record[0] == 255)
    })
}
fn tick(context: &mut dyn Context) -> tje_game::Result<bool> {
    if state(context)?.status != RunStatus::Playing {
        return Ok(false);
    }
    save_banks(context)?;
    let frame = context.frame_count();
    let r = context.revision();
    let heroes = state(context)?.course.rules.heroes;
    if state(context)?
        .course
        .rules
        .tick_limit
        .is_some_and(|limit| {
            frame.saturating_sub(
                state(context)
                    .expect("registered state")
                    .first_frame
                    .unwrap_or(frame),
            ) >= u64::from(limit)
        })
    {
        end_run(context, RunStatus::Failed)?;
        return Ok(false);
    }
    let mut redraw = false;
    for hero in 0..i32::from(heroes) {
        if ram::g_player_lives(r).at(hero).read(context) < 0 {
            continue;
        }
        let player = ram::g_player_info(r).at(hero);
        let index = state(context)?.index(player.level().read(context))?;
        let x = player.x().read(context);
        let y = player.y().read(context);
        let height = player.height().read(context);
        let edge = std::mem::take(&mut state_mut(context)?.pending_up[hero as usize]);
        if edge && height == 0 && ram::g_player_inventory_open(r).at(hero).read(context) == 0 {
            let endpoint = state(context)?.course.levels[index]
                .objects
                .iter()
                .find(|object| {
                    object.endpoint()
                        && i32::from(x).abs_diff(i32::from(object.position().x)) <= 32
                        && i32::from(y).abs_diff(i32::from(object.position().y)) <= 24
                })
                .map(Object::id);
            if let Some(id) = endpoint {
                if id == state(context)?.course.finish.exit {
                    if state(context)?
                        .course
                        .finish
                        .required
                        .iter()
                        .all(|id| collected(state(context).expect("registered state"), *id))
                    {
                        end_run(context, RunStatus::Complete)?;
                        return Ok(false);
                    }
                } else {
                    let destination = state(context)?.course.levels[index]
                        .links
                        .iter()
                        .find(|link| link.endpoint == id)
                        .expect("validated endpoint")
                        .destination;
                    state_mut(context)?.pending[hero as usize] = Some(destination);
                }
            }
        }
        let paths = state(context)?.course.levels[index].paths.len();
        for path in 0..paths {
            let on_path = height == 0
                && state(context)?.course.levels[index].paths[path]
                    .cells
                    .iter()
                    .any(|cell| {
                        i32::from(cell.x) == i32::from(x) / 128 - 1
                            && i32::from(cell.y) == i32::from(y) / 64 - 1
                    });
            if on_path && state(context)?.levels[index].path_deadlines[path].is_none() {
                let group = &state(context)?.course.levels[index].paths[path];
                let deadline = frame
                    .saturating_add(u64::from(group.arming_ticks) + u64::from(group.delay_ticks));
                state_mut(context)?.levels[index].path_deadlines[path] = Some(deadline);
            }
        }
    }
    if state(context)?.last_tick != frame {
        let count = state(context)?.levels.len();
        for index in 0..count {
            for path in 0..state(context)?.levels[index].path_deadlines.len() {
                if state(context)?.levels[index].path_deadlines[path]
                    .is_some_and(|deadline| deadline != u64::MAX && frame >= deadline)
                {
                    let count = state(context)?.course.levels[index].paths[path].cells.len();
                    let replacement = state(context)?.course.levels[index].paths[path].replacement;
                    for cell in 0..count {
                        let position = state(context)?.course.levels[index].paths[path].cells[cell];
                        state_mut(context)?.levels[index].cells
                            [usize::from(position.y) * 19 + usize::from(position.x)] = replacement;
                    }
                    state_mut(context)?.levels[index].path_deadlines[path] = Some(u64::MAX);
                    redraw = true;
                }
            }
        }
        state_mut(context)?.last_tick = frame;
    }
    if redraw {
        publish_maps(context)?;
    }
    Ok(redraw)
}

// A travel transaction owns release, authored publication, retained banks,
// native camera/tile refresh and HUD. No old level+1/fall wrapping survives.
fn travel(context: &mut dyn Context, c: &mut Continuation) -> tje_game::Result<OverrideStep> {
    let r = context.revision();
    if c.stage == 0 {
        let Some(hero) = state(context)?.pending.iter().position(Option::is_some) else {
            return returned();
        };
        state_mut(context)?.travelling = true;
        c.locals[0] = hero as u32;
        c.locals[1] = ram::g_player_info(r).at(hero as i32).level().read(context) as u32;
        c.locals[2] = 0;
        // Release all presentation-only prop handles before rebinding them.
        c.stage = 1;
    }
    if c.stage == 1 {
        while c.locals[2] < 6 {
            let slot = c.locals[2];
            c.locals[2] += 1;
            let number = if slot < 4 {
                ram::g_elevator_records(r)
                    .at((slot / 2) as i32)
                    .at((slot % 2) as i32)
                    .entity()
                    .read(context)
            } else {
                ram::g_telephone_states(r)
                    .at((slot - 4) as i32)
                    .object()
                    .read(context)
            };
            if number != 0 {
                c.locals[3] = if slot < 4 { number as u32 + 2 } else { 0 };
                return original(c, 2, ids::FREE_ENTITY, vec![number as u32]);
            }
        }
        return original(c, 3, ids::EARTHLING_RELEASE_LEVEL_BANK, vec![c.locals[1]]);
    }
    if c.stage == 2 {
        if c.locals[3] != 0 {
            let second = c.locals[3];
            c.locals[3] = 0;
            return original(c, 2, ids::FREE_ENTITY, vec![second]);
        }
        c.stage = 1;
        return travel(context, c);
    }
    if c.stage == 3 {
        return original(c, 4, ids::MAP_RELEASE_LEVEL_ITEM_BANK, vec![c.locals[1]]);
    }
    if c.stage == 4 {
        let hero = c.locals[0] as usize;
        let destination = state_mut(context)?.pending[hero]
            .take()
            .expect("pending travel");
        place(context, hero as i32, destination)?;
        let physical = ram::g_player_info(r).at(hero as i32).level().read(context);
        retain_actors(context, physical)?;
        retain_items(context, physical)?;
        publish_maps(context)?;
        configure_props(context)?;
        return original(
            c,
            5,
            ids::SET_SPRITE,
            vec![ram::g_player_info(r).at(hero as i32).address(), 0],
        );
    }
    if c.stage == 5 {
        return original(c, 6, ids::UPDATE_SCREEN_SPLIT_MODE, vec![1]);
    }
    if c.stage == 6 {
        return original(c, 7, ids::HUD_LOAD_LEVEL_NUMBER_SPRITES, vec![c.locals[0]]);
    }
    if c.stage == 7 {
        return original(
            c,
            8,
            ids::HUD_POSITION_LEVEL_NUMBER_SPRITES,
            vec![c.locals[0]],
        );
    }
    state_mut(context)?.travelling = false;
    c.stage = 0;
    travel(context, c)
}
fn process_course(
    context: &mut dyn Context,
    c: &mut Continuation,
) -> tje_game::Result<OverrideStep> {
    if c.stage == 20 {
        return returned();
    }
    if c.stage == 0 && tick(context)? && state(context)?.pending.iter().all(Option::is_none) {
        return original(c, 20, ids::UPDATE_SCREEN_SPLIT_MODE, vec![1]);
    }
    travel(context, c)
}
fn process_players(
    context: &mut dyn Context,
    c: &mut Continuation,
) -> tje_game::Result<OverrideStep> {
    if c.stage == 0 {
        if matches!(
            state(context)?.status,
            RunStatus::Complete | RunStatus::Failed
        ) {
            return returned();
        }
        return original(c, 100, ids::PROCESS_PLAYERS, vec![]);
    }
    if c.stage == 100 {
        c.stage = 0;
    }
    process_course(context, c)
}
fn fall(context: &mut dyn Context, c: &mut Continuation) -> tje_game::Result<OverrideStep> {
    let r = context.revision();
    if c.locals[9] == 0 {
        c.locals[9] = 1;
        let address = context.entry_arguments()?[0];
        for hero in 0..i32::from(state(context)?.course.rules.heroes) {
            if context.read_u8(address + hero as u32) == 0
                || ram::g_player_lives(r).at(hero).read(context) < 0
            {
                continue;
            }
            let index =
                state(context)?.index(ram::g_player_info(r).at(hero).level().read(context))?;
            let destination = match state(context)?.course.levels[index].fall_target {
                FallTarget::Level { level, spawn } => Destination { level, spawn },
                FallTarget::LoseLifeAndRespawn { spawn } => {
                    let lives = ram::g_player_lives(r).at(hero);
                    lives.write(context, lives.read(context) - 1);
                    if lives.read(context) < 0 {
                        c.locals[8] |= 1 << hero;
                        continue;
                    }
                    Destination {
                        level: state(context)?.course.levels[index].id,
                        spawn,
                    }
                }
            };
            state_mut(context)?.pending[hero as usize] = Some(destination);
        }
    }
    if c.locals[8] != 0 {
        if c.stage < 200 {
            begin_elimination(context, c, c.locals[8].trailing_zeros() as usize)?;
        }
        match eliminate(context, c)? {
            OverrideStep::Return(_) => {
                c.locals[8] &= !(1 << c.locals[0]);
                c.stage = 0;
                return fall(context, c);
            }
            step => return Ok(step),
        }
    }
    travel(context, c)
}
fn advance(context: &mut dyn Context, _: &mut Continuation) -> tje_game::Result<OverrideStep> {
    let arguments = context.entry_arguments()?;
    let hero = arguments[1] as usize;
    if hero >= usize::from(state(context)?.course.rules.heroes) {
        return Err(problem("invalid advance seat"));
    }
    let index = state(context)?.index(
        ram::g_player_info(context.revision())
            .at(hero as i32)
            .level()
            .read(context),
    )?;
    let level = &state(context)?.course.levels[index];
    // Solo present effects cannot choose between authored exits or invent a
    // story level+1. Their advance request is a safe same-floor relocation.
    let spawn = match level.fall_target {
        FallTarget::LoseLifeAndRespawn { spawn } => spawn,
        FallTarget::Level { .. } => level.spawn_points[0].id,
    };
    let destination = Destination {
        level: level.id,
        spawn,
    };
    place(context, hero as i32, destination)?;
    returned()
}
fn process_movement(
    context: &mut dyn Context,
    c: &mut Continuation,
) -> tje_game::Result<OverrideStep> {
    if matches!(
        state(context)?.status,
        RunStatus::Complete | RunStatus::Failed
    ) {
        return returned();
    }
    if c.stage == 0 {
        return original(c, 1, ids::PROCESS_MOVEMENT, vec![]);
    }
    returned()
}
fn process_earthlings(
    context: &mut dyn Context,
    c: &mut Continuation,
) -> tje_game::Result<OverrideStep> {
    if matches!(
        state(context)?.status,
        RunStatus::Complete | RunStatus::Failed
    ) {
        return returned();
    }
    if c.stage == 0 {
        return original(c, 1, ids::PROCESS_EARTHLINGS, vec![]);
    }
    returned()
}

pub static OVERRIDES: &[ResumableTranslation] = &[
    ResumableTranslation {
        id: ids::LOAD_LEVEL1_TILES,
        function: initial_map,
    },
    ResumableTranslation {
        id: ids::PLAYER_FINALIZE_LAST_LIFE_EXIT,
        function: finalize_life,
    },
    ResumableTranslation {
        id: ids::GET_PLAYER_INPUT,
        function: capture_input,
    },
    ResumableTranslation {
        id: ids::UPDATE_LAND_AROUND_PLAYER,
        function: nearby_terrain,
    },
    ResumableTranslation {
        id: ids::GET_MAP_TERRAIN_AT_WORLD_POSITION,
        function: query_terrain,
    },
    ResumableTranslation {
        id: ids::SETUP_THE_MAIN_GAME,
        function: setup_game,
    },
    ResumableTranslation {
        id: ids::SET_POSITION_ON_NEW_LEVEL,
        function: set_position,
    },
    ResumableTranslation {
        id: ids::GENERATE_LEVEL_SEEDS,
        function: generate,
    },
    ResumableTranslation {
        id: ids::MAP_LOAD_PLAYER_LEVEL,
        function: map_load,
    },
    ResumableTranslation {
        id: ids::CHANGE_LEVEL,
        function: map_change,
    },
    ResumableTranslation {
        id: ids::LOAD_FINAL_LEVEL,
        function: reject_final,
    },
    ResumableTranslation {
        id: ids::COPY_LEVEL_BETWEEN_PLAYERS,
        function: map_sync,
    },
    ResumableTranslation {
        id: ids::MAP_SWAP_PLAYER_BANKS,
        function: map_sync,
    },
    ResumableTranslation {
        id: ids::MAP_SWAP_PLAYER_WITH_SAVED_BANK,
        function: map_sync,
    },
    ResumableTranslation {
        id: ids::SETUP_INITIAL_INVENTORY,
        function: initial_items,
    },
    ResumableTranslation {
        id: ids::LOAD_LEVEL_ITEM_BANK,
        function: load_items,
    },
    ResumableTranslation {
        id: ids::MAP_RELEASE_LEVEL_ITEM_BANK,
        function: release_items,
    },
    ResumableTranslation {
        id: ids::SETUP_INITIAL_EARTHLINGS,
        function: initial_actors,
    },
    ResumableTranslation {
        id: ids::ASSIGN_SHIP_PIECE_LEVELS,
        function: assign_pieces,
    },
    ResumableTranslation {
        id: ids::EARTHLING_RETAIN_LEVEL_BANK,
        function: load_actors,
    },
    ResumableTranslation {
        id: ids::EARTHLING_RELEASE_LEVEL_BANK,
        function: release_actors,
    },
    ResumableTranslation {
        id: ids::EARTHLING_ALLOCATE_ENTITIES,
        function: allocate_actor,
    },
    ResumableTranslation {
        id: ids::SETUP_INITIAL_ELEVATOR,
        function: initial_props,
    },
    ResumableTranslation {
        id: ids::SETUP_ELEVATOR,
        function: setup_elevator,
    },
    ResumableTranslation {
        id: ids::ELEVATOR_BLOCKS_PLAYER_MOVEMENT,
        function: no_elevator_collision,
    },
    ResumableTranslation {
        id: ids::PROCESS_ELEVATORS,
        function: process_course,
    },
    ResumableTranslation {
        id: ids::PROCESS_LAND,
        function: process_course,
    },
    ResumableTranslation {
        id: ids::PROCESS_PLAYERS,
        function: process_players,
    },
    ResumableTranslation {
        id: ids::MAKE_PLAYER_FALL,
        function: fall,
    },
    ResumableTranslation {
        id: ids::PLAYER_ADVANCE_TO_NEXT_LEVEL,
        function: advance,
    },
    ResumableTranslation {
        id: ids::PROCESS_MOVEMENT,
        function: process_movement,
    },
    ResumableTranslation {
        id: ids::PROCESS_EARTHLINGS,
        function: process_earthlings,
    },
];

fn query_terrain(
    context: &mut dyn Context,
    _: &mut Continuation,
) -> tje_game::Result<OverrideStep> {
    let arguments = context.entry_arguments()?;
    let hero = arguments[0] as i16;
    let x = i32::from(arguments[1] as i16);
    let y = i32::from(arguments[2] as i16);
    if !(0..2).contains(&hero) {
        return Err(problem("terrain query requires seat 0 or 1"));
    }
    if !(128..2560).contains(&x) || !(64..1728).contains(&y) {
        return Ok(OverrideStep::Return(Some(0x90)));
    }
    let r = context.revision();
    let map = if hero == 0 {
        ram::g_player1_level_map(r)
    } else {
        ram::g_player2_level_map(r)
    };
    let cell = map.at(y / 64 - 1).at(x / 128 - 1).read(context);
    let terrain = crate::admission::terrain_reference(context, r, cell, y / 8 % 8, x / 8 % 16)
        .ok_or_else(|| problem("invalid active terrain reference"))?;
    ram::g_terrain_query_metatile_word(r).write(context, cell);
    Ok(OverrideStep::Return(Some(u32::from(terrain & 0xf0))))
}

fn capture_input(
    context: &mut dyn Context,
    c: &mut Continuation,
) -> tje_game::Result<OverrideStep> {
    if c.stage == 0 {
        return original(c, 1, ids::GET_PLAYER_INPUT, vec![]);
    }
    for hero in 0..2 {
        let up = match context.inputs().pads[hero] {
            genesis_config::GenesisController::ThreeButton(pad)
            | genesis_config::GenesisController::SixButton(pad) => pad.up,
            _ => false,
        };
        let s = state_mut(context)?;
        if s.status == RunStatus::Playing && up && !s.pressed_up[hero] {
            s.pending_up[hero] = true;
        }
        s.pressed_up[hero] = up;
    }
    returned()
}

fn nearby_terrain(
    context: &mut dyn Context,
    _: &mut Continuation,
) -> tje_game::Result<OverrideStep> {
    let hero = context.entry_arguments()?[0] as i16;
    if !(0..2).contains(&hero) {
        return Err(problem("nearby terrain requires seat 0 or 1"));
    }
    let r = context.revision();
    if ram::g_player_lives(r).at(i32::from(hero)).read(context) < 0
        || ram::g_player_inventory_open(r)
            .at(i32::from(hero))
            .read(context)
            != 0
    {
        return returned();
    }
    let player = ram::g_player_info(r).at(i32::from(hero));
    let center_x = i32::from(player.x().read(context)) >> 3;
    let center_y = i32::from(player.y().read(context)) >> 3;
    let map = if hero == 0 {
        ram::g_player1_level_map(r)
    } else {
        ram::g_player2_level_map(r)
    };
    let cache = if hero == 0 {
        ram::g_player1_nearby_terrain(r)
    } else {
        ram::g_player2_nearby_terrain(r)
    };
    for row in 0..5 {
        for column in 0..5 {
            let x = center_x + column - 2;
            let y = center_y + row - 2;
            let doorway = tje_game::presents::present_find_doorway_owner_at_tile(
                context,
                &[player.address(), y as u32, x as u32],
            )?
            .unwrap_or(0);
            let property = if doorway != 0 {
                0x10
            } else if !(16..320).contains(&x) || !(8..216).contains(&y) {
                0x90
            } else {
                let cell = map.at(y / 8 - 1).at(x / 16 - 1).read(context);
                crate::admission::terrain_reference(context, r, cell, y % 8, x % 16)
                    .ok_or_else(|| problem("invalid nearby terrain reference"))?
            };
            cache.at(row).at(column).write(context, property);
        }
    }
    returned()
}

fn view_floor(context: &dyn Context, viewport: i32) -> i8 {
    let r = context.revision();
    let hero = if viewport == 0
        && ram::g_player_lives(r).at(0).read(context) < 0
        && ram::g_player_lives(r).at(1).read(context) >= 0
    {
        1
    } else {
        viewport
    };
    ram::g_player_info(r).at(hero).level().read(context)
}

fn end_run(context: &mut dyn Context, status: RunStatus) -> tje_game::Result<()> {
    let s = state_mut(context)?;
    s.status = status;
    s.pending = [None; 2];
    s.pending_up = [false; 2];
    Ok(())
}
fn begin_elimination(
    context: &mut dyn Context,
    c: &mut Continuation,
    hero: usize,
) -> tje_game::Result<()> {
    if hero >= usize::from(state(context)?.course.rules.heroes) {
        return Err(problem("invalid eliminated seat"));
    }
    save_banks(context)?;
    let r = context.revision();
    let player = ram::g_player_info(r).at(hero as i32);
    c.locals[0] = hero as u32;
    c.locals[1] = player.level().read(context) as u32;
    c.locals[2] = u32::from(player.draw_flags().read(context));
    c.stage = 200;
    state_mut(context)?.travelling = true;
    ram::g_player_lives(r).at(hero as i32).write(context, -1);
    state_mut(context)?.pending[hero] = None;
    if (0..i32::from(state(context)?.course.rules.heroes))
        .all(|seat| ram::g_player_lives(r).at(seat).read(context) < 0)
    {
        end_run(context, RunStatus::Failed)?;
    }
    Ok(())
}
fn eliminate(context: &mut dyn Context, c: &mut Continuation) -> tje_game::Result<OverrideStep> {
    let r = context.revision();
    let player = ram::g_player_info(r).at(c.locals[0] as i32);
    if c.stage == 200 {
        return original(c, 201, ids::EARTHLING_RELEASE_LEVEL_BANK, vec![c.locals[1]]);
    }
    if c.stage == 201 {
        return original(c, 202, ids::MAP_RELEASE_LEVEL_ITEM_BANK, vec![c.locals[1]]);
    }
    if c.stage == 202 {
        c.stage = 203;
        if c.locals[2] & 2 != 0 {
            return original(
                c,
                203,
                ids::RELEASE_ENTITY_VIEWPORT_SPRITES,
                vec![player.address(), 0],
            );
        }
    }
    if c.stage == 203 {
        c.stage = 204;
        if c.locals[2] & 4 != 0 {
            return original(
                c,
                204,
                ids::RELEASE_ENTITY_VIEWPORT_SPRITES,
                vec![player.address(), 1],
            );
        }
    }
    if c.stage == 204 {
        player.draw_flags().write(context, 0);
        player.velocity_x().write(context, 0);
        player.velocity_y().write(context, 0);
        return original(c, 205, ids::HUD_HIDE_PLAYER_SPRITES, vec![c.locals[0]]);
    }
    if c.stage == 205 {
        publish_maps(context)?;
        if state(context)?.status == RunStatus::Playing
            && state(context)?.pending.iter().all(Option::is_none)
        {
            return original(c, 206, ids::UPDATE_SCREEN_SPLIT_MODE, vec![1]);
        }
    }
    state_mut(context)?.travelling = false;
    returned()
}
fn finalize_life(
    context: &mut dyn Context,
    c: &mut Continuation,
) -> tje_game::Result<OverrideStep> {
    if c.stage == 0 {
        let hero = context.entry_arguments()?[1] as usize;
        begin_elimination(context, c, hero)?;
    }
    eliminate(context, c)
}

fn initial_map(context: &mut dyn Context, _: &mut Continuation) -> tje_game::Result<OverrideStep> {
    let physical = state(context)?.physical(state(context)?.course.start.level);
    let index = state(context)?.index(physical)?;
    write_map(context, 0, index)?;
    write_map(context, 1, index)?;
    returned()
}
