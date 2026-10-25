use crate::*;
use tje_mem::{AddressSpace, rom};

pub(crate) fn kind(object: &Object) -> Option<i8> {
    Some(match object {
        Object::ShipPiece { .. } => 0,
        Object::Earthling { definition, .. } => match definition {
            Earthling::Devil => 1,
            Earthling::Hamster => 2,
            Earthling::Shopper => 3,
            Earthling::Dentist => 4,
            Earthling::HulaDancer => 5,
            Earthling::Bees => 6,
            Earthling::Cupid => 7,
            Earthling::Whirlwind => 8,
            Earthling::NerdHerd => 12,
            Earthling::Lawnmower => 13,
            Earthling::Shark => 14,
            Earthling::Chicken => 15,
            Earthling::Boogieman => 16,
            Earthling::Mole => 18,
            Earthling::IceCreamTruck => 19,
            Earthling::Santa => 20,
            Earthling::DentistVariant => 21,
            Earthling::BeesVariant => 22,
        },
        Object::Service { definition, .. } => match definition {
            Service::Mailbox => 9,
            Service::Wizard => 10,
            Service::Wiseman => 11,
            Service::OperaSinger => 17,
            Service::LemonadeStand => 23,
            Service::HotTub => 24,
        },
        _ => return None,
    })
}
#[cfg(feature = "enhanced-mode")]
pub(crate) fn item_kind(definition: Item) -> i8 {
    match definition {
        Item::Present { identity } => identity as i8,
        Item::Food { identity } => (64 + identity) as i8,
        Item::Buck => 80,
        Item::Large => 81,
        Item::SinkingLarge => 82,
        Item::HurtHazard => 83,
    }
}
pub(crate) fn terrain(memory: &AddressSpace, level: &Level, position: Point) -> Option<u8> {
    let x = usize::from(position.x);
    let y = usize::from(position.y);
    if x < 128 || y < 64 {
        return None;
    }
    let cell = level.cell(x / 128 - 1, y / 64 - 1)?;
    terrain_reference(
        memory,
        memory.revision(),
        cell,
        (y / 8 % 8) as i32,
        (x / 8 % 16) as i32,
    )
    .map(|property| property & 0xf0)
}
pub(crate) fn terrain_reference<B: tje_mem::Bus + ?Sized>(
    memory: &B,
    revision: tje_mem::Revision,
    cell: u16,
    row: i32,
    column: i32,
) -> Option<u8> {
    let overlay = (cell >> 8) & 31;
    let mut tile = 0;
    if overlay & 15 != 0 {
        tile = rom::overlay_metatile_maps(revision)
            .at(i32::from((overlay & 15) - 1))
            .at(row)
            .at(if overlay & 16 != 0 {
                15 - column
            } else {
                column
            })
            .read(memory);
    }
    if tile == 0 {
        tile = rom::level_metatile_maps(revision)
            .at(i32::from(cell & 127))
            .at(row)
            .at(if cell & 128 != 0 { 15 - column } else { column })
            .read(memory);
    }
    let index = tile & 511;
    if index >= 398 {
        return None;
    }
    Some(
        rom::player_tile_property_lookup(revision)
            .at(i32::from(index))
            .read(memory),
    )
}
fn footprint(
    memory: &AddressSpace,
    level: &Level,
    point: Point,
    half_width: u16,
    half_height: u16,
    allowed: &[u8],
) -> bool {
    let (Some(left), Some(top)) = (
        point.x.checked_sub(half_width),
        point.y.checked_sub(half_height),
    ) else {
        return false;
    };
    let (Some(right), Some(bottom)) = (
        point.x.checked_add(half_width),
        point.y.checked_add(half_height),
    ) else {
        return false;
    };
    (top..=bottom).step_by(8).all(|y| {
        (left..=right).step_by(8).all(|x| {
            terrain(memory, level, Point { x, y }).is_some_and(|class| allowed.contains(&class))
        })
    })
}

/// Complete pre-mutation capability and ROM-terrain admission for the first
/// phase. The broader bounded format is not a promise of a dynamic arena.
pub fn admit_native(document: &ValidatedCourse, memory: &AddressSpace) -> Result<(), Error> {
    let c = document.course();
    let fail = |path: &str, message: &str| {
        Error::Format(format!("native Maker admission {path}: {message}"))
    };
    if c.catalog.revision() != memory.revision() {
        return Err(fail(
            "catalog",
            "course requires a different ROM revision; explicitly author/validate the matching catalog",
        ));
    }
    let r = memory.revision();
    let required_rom = [
        rom::level_metatile_maps(r).address() as usize + 122 * 256,
        rom::overlay_metatile_maps(r).address() as usize + 15 * 256,
        rom::player_tile_property_lookup(r).address() as usize + 398,
        rom::earthling_definitions(r).address() as usize + 25 * 38,
    ]
    .into_iter()
    .max()
    .unwrap();
    if memory.rom().len() < required_rom {
        return Err(fail(
            "catalog",
            "import a complete supported ROM before resolving logical resources",
        ));
    }
    if c.rules.ruleset != Ruleset::ClassicV1 || c.rules.heroes > 2 {
        return Err(fail(
            "rules",
            "first-phase profile supports classic_v1 with one or two seats; four-seat/Overlord closure is not installed",
        ));
    }
    if c.levels.len() > 24 {
        return Err(fail(
            "levels",
            "native profile has 24 ordinary floor slots (2 through 25); special floors are reserved",
        ));
    }
    for level in &c.levels {
        let path = format!("levels[{}]", level.id);
        if level.width != 19 || level.height != 26 {
            return Err(fail(
                &path,
                "first-phase profile requires exactly 19 columns by 26 rows; no generalized renderer is installed",
            ));
        }
        // Validate all map query indices without exporting any decoded tiles.
        let mut checked = [false; 8192];
        for (run, &(_, cell)) in level.cells.iter().enumerate() {
            if checked[usize::from(cell)] {
                continue;
            }
            checked[usize::from(cell)] = true;
            if !(0..8).all(|row| {
                (0..16).all(|column| terrain_reference(memory, r, cell, row, column).is_some())
            }) {
                return Err(fail(
                    &format!("{path}.cells[{run}]"),
                    "a cell resolves outside the supported terrain catalog",
                ));
            }
        }
        if level.music != Music::Gameplay {
            return Err(fail(
                &path,
                "native first-phase profile supports gameplay music only; quiet mixing is not installed",
            ));
        }
        for spawn in &level.spawn_points {
            if !footprint(
                memory,
                level,
                spawn.position,
                24,
                16,
                &[0x10, 0x30, 0x40, 0x50],
            ) {
                return Err(fail(
                    &format!("{path}.spawn_points[{}]", spawn.id),
                    "unsafe mandatory arrival: both hero footprints require solid walkable ground",
                ));
            }
        }
        if let Some(start) = c
            .levels
            .iter()
            .find(|candidate| candidate.id == c.start.level)
            .filter(|start| start.id == level.id)
        {
            for (i, &a) in c.start.spawns.iter().enumerate() {
                for &b in &c.start.spawns[i + 1..] {
                    let (a, b) = (start.spawn(a).unwrap(), start.spawn(b).unwrap());
                    if a.x.abs_diff(b.x) < 48 && a.y.abs_diff(b.y) < 32 {
                        return Err(fail("start.spawns", "hero arrival footprints overlap"));
                    }
                }
            }
        }
        let (mut actors, mut items, mut endpoints, mut telephones, mut pieces) = (0, 0, 0, 0, 0);
        let mut local_dynamic = 0;
        for object in &level.objects {
            let at = format!("{path}.objects[{}]", object.id());
            let position = object.position();
            if let Some(kind) = kind(object) {
                actors += 1;
                let definition = rom::earthling_definitions(r).at(i32::from(kind));
                local_dynamic += 1 + usize::from(definition.behavior_flags().read(memory) & 1 != 0);
                let (width, height, allowed): (u16, u16, &[u8]) = match kind {
                    8 => (8, 8, &[0x20]),
                    14 => (12, 8, &[0x80]),
                    0 | 12 | 13 | 18 | 19 => (64, 32, &[0x10, 0x20]),
                    _ => (32, 8, &[0x10, 0x20]),
                };
                if !footprint(memory, level, position, width, height, allowed) {
                    return Err(fail(
                        &at,
                        "illegal authored actor footprint/terrain; it will not be relocated or rerolled",
                    ));
                }
                if matches!(object, Object::ShipPiece { .. }) {
                    pieces += 1;
                }
                if matches!(
                    object,
                    Object::Earthling {
                        initial: InitialState::Asleep,
                        ..
                    }
                ) && [0, 8, 14, 20, 23, 24].contains(&kind)
                {
                    return Err(fail(
                        &at,
                        "this catalog actor has no supported sleeping capability",
                    ));
                }
                if definition.damage().read(memory) > 0
                    && level.spawn_points.iter().any(|spawn| {
                        spawn.position.x.abs_diff(position.x) < 128
                            && spawn.position.y.abs_diff(position.y) < 96
                    })
                {
                    return Err(fail(
                        &at,
                        "hostile footprint intrudes into a mandatory arrival safe zone",
                    ));
                }
            } else {
                let allowed: &[u8] = if matches!(
                    object,
                    Object::Item {
                        definition: Item::SinkingLarge | Item::HurtHazard,
                        ..
                    }
                ) {
                    &[0x20]
                } else {
                    &[0x10, 0x20, 0x30, 0x40, 0x50]
                };
                if !footprint(memory, level, position, 16, 8, allowed) {
                    return Err(fail(&at, "unsafe authored item/endpoint footprint"));
                }
                match object {
                    Object::Item { .. } => {
                        items += 1;
                        local_dynamic += 1;
                    }
                    Object::Telephone { .. } => {
                        telephones += 1;
                        local_dynamic += 1;
                    }
                    Object::Elevator { .. } | Object::Exit { .. } => endpoints += 1,
                    _ => (),
                }
            }
        }
        if actors > 20 || items > 32 || endpoints > 2 || telephones > 1 || pieces > 1 {
            return Err(fail(
                &path,
                "native banks support 20 actors, 32 items, two endpoints, one telephone and one ship piece per floor",
            ));
        }
        if c.rules.heroes == 2 && c.levels.len() > 1 && endpoints > 1 {
            return Err(fail(
                &path,
                "two active native floors can each reserve only one endpoint sprite pair",
            ));
        }
        // At most two active floors; conservatively reserve four of the twenty
        // original dynamic handles for transient effects and present mechanics.
        if local_dynamic > 8 {
            return Err(fail(
                &path,
                "native profile permits eight dynamic bodies including companions per floor (two active floors plus four transient reservations)",
            ));
        }
        for group in &level.paths {
            for cell in &group.cells {
                let center = Point {
                    x: 128 * (u16::from(cell.x) + 1) + 64,
                    y: 64 * (u16::from(cell.y) + 1) + 32,
                };
                if !footprint(
                    memory,
                    level,
                    center,
                    56,
                    24,
                    &[0x10, 0x20, 0x30, 0x40, 0x50],
                ) {
                    return Err(fail(
                        &format!("{path}.paths[{}]", group.id),
                        "path source must be traversable ground",
                    ));
                }
                if !(0..8).all(|row| {
                    (0..16).all(|column| {
                        terrain_reference(memory, r, group.replacement, row, column).is_some()
                    })
                }) {
                    return Err(fail(
                        &path,
                        "path replacement has an invalid ROM terrain reference",
                    ));
                }
                if level.spawn_points.iter().any(|spawn| {
                    spawn.position.x / 128 == center.x / 128
                        && spawn.position.y / 64 == center.y / 64
                }) || level.objects.iter().any(|object| {
                    object.position().x / 128 == center.x / 128
                        && object.position().y / 64 == center.y / 64
                }) {
                    return Err(fail(
                        &format!("{path}.paths[{}]", group.id),
                        "destructive path cannot own mandatory arrival or object cells",
                    ));
                }
            }
        }
    }
    Ok(())
}
