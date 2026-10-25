use crate::model::*;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub path: String,
    pub message: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationErrors(pub Vec<Issue>);
impl fmt::Display for ValidationErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, issue) in self.0.iter().enumerate() {
            if index != 0 {
                writeln!(f)?;
            }
            write!(f, "{}: {}", issue.path, issue.message)?;
        }
        Ok(())
    }
}
impl std::error::Error for ValidationErrors {}
fn issue(errors: &mut Vec<Issue>, path: impl Into<String>, message: impl Into<String>) {
    errors.push(Issue {
        path: path.into(),
        message: message.into(),
    });
}
pub fn valid_cell(cell: u16) -> bool {
    cell & 0xe000 == 0 && cell & 0x7f < 122 && (cell & 0x0f00 != 0 || cell & 0x1000 == 0)
}
fn text(errors: &mut Vec<Issue>, path: &str, value: &str, limit: usize, multiline: bool) {
    if value.len() > limit {
        issue(errors, path, format!("UTF-8 text exceeds {limit} bytes"));
    }
    if value
        .chars()
        .any(|c| c.is_control() && !(multiline && c == '\n'))
    {
        issue(errors, path, "control characters are not permitted");
    }
}
fn point(errors: &mut Vec<Issue>, path: &str, position: Point, level: &Level) {
    if !position.x.is_multiple_of(8) || !position.y.is_multiple_of(8) {
        issue(
            errors,
            path,
            "position must be snapped to eight world pixels",
        );
    }
    if position.x < 128
        || u32::from(position.x) >= 128 * (u32::from(level.width) + 1)
        || position.y < 64
        || u32::from(position.y) >= 64 * (u32::from(level.height) + 1)
    {
        issue(
            errors,
            path,
            "position lies outside the level's (128,64) cell origin and bounds",
        );
    }
}
fn destination(
    errors: &mut Vec<Issue>,
    path: &str,
    destination: Destination,
    levels: &BTreeMap<u16, &Level>,
) {
    match levels.get(&destination.level) {
        None => issue(errors, path, format!("unknown level {}", destination.level)),
        Some(level) if level.spawn(destination.spawn).is_none() => issue(
            errors,
            path,
            format!(
                "unknown spawn {} in level {}",
                destination.spawn, destination.level
            ),
        ),
        Some(_) => (),
    }
}

/// Structural validation does not require a ROM. Actual tile-terrain/footprint
/// checks belong to admission, before the enhanced driver or machine is mutated.
pub fn validate(course: &Course) -> Result<(), ValidationErrors> {
    let mut errors = Vec::new();
    if course.schema != SCHEMA {
        issue(
            &mut errors,
            "schema",
            format!("expected {SCHEMA}, got {:?}", course.schema),
        );
    }
    text(&mut errors, "title", &course.title, 128, false);
    text(&mut errors, "author", &course.author, 64, false);
    text(&mut errors, "description", &course.description, 1024, true);
    if course.runtime_seed.len() != 16
        || !course
            .runtime_seed
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        issue(
            &mut errors,
            "runtime_seed",
            "expected exactly sixteen lowercase hexadecimal digits",
        );
    }
    if !(1..=4).contains(&course.rules.heroes) {
        issue(&mut errors, "rules.heroes", "expected 1 through 4 seats");
    }
    if course
        .rules
        .tick_limit
        .is_some_and(|ticks| ticks == 0 || ticks > 216_000)
    {
        issue(
            &mut errors,
            "rules.tick_limit",
            "expected null or 1 through 216000 ticks (one hour)",
        );
    }
    if course.rules.lives > 9 {
        issue(
            &mut errors,
            "rules.lives",
            "expected 0 through 9 spare lives",
        );
    }
    if course.rules.bucks > 99 {
        issue(&mut errors, "rules.bucks", "expected 0 through 99 bucks");
    }
    if course.rules.inventory.len() > 16 || course.rules.inventory.iter().any(|&id| id >= 28) {
        issue(
            &mut errors,
            "rules.inventory",
            "at most sixteen present identities, each 0 through 27",
        );
    }
    if course.start.spawns.len() != usize::from(course.rules.heroes) {
        issue(
            &mut errors,
            "start.spawns",
            "exactly one explicit spawn reference is required per seat",
        );
    }
    if course.levels.is_empty() || course.levels.len() > 32 {
        issue(&mut errors, "levels", "expected 1 through 32 levels");
    }
    let mut levels = BTreeMap::new();
    let mut objects = BTreeMap::new();
    let mut pieces = BTreeSet::new();
    let mut object_count = 0;
    for (index, level) in course.levels.iter().enumerate() {
        let path = format!("levels[{index}]");
        if levels.insert(level.id, level).is_some() {
            issue(
                &mut errors,
                format!("{path}.id"),
                format!("duplicate level ID {}", level.id),
            );
        }
        if !(8..=64).contains(&level.width) || !(8..=64).contains(&level.height) {
            issue(
                &mut errors,
                &path,
                "width and height must each be 8 through 64",
            );
        }
        let mut total = 0_u32;
        for (run, &(count, cell)) in level.cells.iter().enumerate() {
            if count == 0 {
                issue(
                    &mut errors,
                    format!("{path}.cells[{run}]"),
                    "run count must be positive",
                );
            }
            total += u32::from(count);
            if !valid_cell(cell) {
                issue(
                    &mut errors,
                    format!("{path}.cells[{run}]"),
                    format!(
                        "invalid cell {cell:#06x}: base 0..121, overlay 0..15, no reserved bits or orphan overlay mirror"
                    ),
                );
            }
        }
        if total != u32::from(level.width) * u32::from(level.height) {
            issue(
                &mut errors,
                format!("{path}.cells"),
                format!(
                    "run total {total} does not equal width*height ({})",
                    u32::from(level.width) * u32::from(level.height)
                ),
            );
        }
        if level.cells.len() > 4096 {
            issue(&mut errors, format!("{path}.cells"), "more than 4096 runs");
        }
        let mut spawns = BTreeSet::new();
        if level.spawn_points.is_empty() || level.spawn_points.len() > 16 {
            issue(
                &mut errors,
                format!("{path}.spawn_points"),
                "expected 1 through 16 spawn points",
            );
        }
        for spawn in &level.spawn_points {
            if !spawns.insert(spawn.id) {
                issue(
                    &mut errors,
                    format!("{path}.spawn_points"),
                    format!("duplicate spawn ID {}", spawn.id),
                );
            }
            point(
                &mut errors,
                &format!("{path}.spawn_points[{}]", spawn.id),
                spawn.position,
                level,
            );
        }
        let (mut actors, mut items, mut props) = (0, 0, 0);
        for object in &level.objects {
            let at = format!("{path}.objects[{}]", object.id());
            if objects.insert(object.id(), (level.id, object)).is_some() {
                issue(&mut errors, &at, "duplicate course-wide object ID");
            }
            point(&mut errors, &at, object.position(), level);
            match object {
                Object::Earthling { .. } | Object::Service { .. } => actors += 1,
                Object::ShipPiece { piece, .. } => {
                    actors += 1;
                    if *piece >= 10 || !pieces.insert(*piece) {
                        issue(
                            &mut errors,
                            &at,
                            "ship piece must be a unique identity 0 through 9",
                        );
                    }
                }
                Object::Item { definition, .. } => {
                    items += 1;
                    if matches!(definition, Item::Present { identity } if *identity >= 28)
                        || matches!(definition, Item::Food { identity } if *identity >= 16)
                    {
                        issue(
                            &mut errors,
                            &at,
                            "present identity must be 0..27 and food identity 0..15",
                        );
                    }
                }
                _ => props += 1,
            }
        }
        if actors > 64 || items > 128 || props > 64 {
            issue(
                &mut errors,
                format!("{path}.objects"),
                "level exceeds 64 actors, 128 items or 64 prop/endpoints",
            );
        }
        object_count += level.objects.len();
        let mut endpoints = BTreeSet::new();
        if level.links.len() > 64 {
            issue(&mut errors, format!("{path}.links"), "more than 64 links");
        }
        for link in &level.links {
            if !endpoints.insert(link.endpoint) {
                issue(
                    &mut errors,
                    format!("{path}.links"),
                    format!("duplicate link for endpoint {}", link.endpoint),
                );
            }
            if !level
                .objects
                .iter()
                .any(|object| object.id() == link.endpoint && object.endpoint())
            {
                issue(
                    &mut errors,
                    format!("{path}.links"),
                    format!("endpoint {} is not a local exit/elevator", link.endpoint),
                );
            }
        }
        for object in &level.objects {
            if object.endpoint()
                && object.id() != course.finish.exit
                && !endpoints.contains(&object.id())
            {
                issue(
                    &mut errors,
                    format!("{path}.objects[{}]", object.id()),
                    "nonterminal exit/elevator requires an explicit link",
                );
            }
            if object.id() == course.finish.exit && endpoints.contains(&object.id()) {
                issue(
                    &mut errors,
                    format!("{path}.links"),
                    "finish exit cannot also have a travel link",
                );
            }
        }
        let mut path_ids = BTreeSet::new();
        let mut owners = BTreeSet::new();
        if level.paths.len() > 64 {
            issue(
                &mut errors,
                format!("{path}.paths"),
                "more than 64 path groups",
            );
        }
        for group in &level.paths {
            let at = format!("{path}.paths[{}]", group.id);
            if !path_ids.insert(group.id) {
                issue(&mut errors, &at, "duplicate path ID");
            }
            if group.cells.is_empty()
                || group.cells.len() > 4096
                || !valid_cell(group.replacement)
                || group.arming_ticks > 3600
                || group.delay_ticks > 3600
            {
                issue(
                    &mut errors,
                    &at,
                    "invalid path cells, replacement or tick bound (0..3600)",
                );
            }
            let positions: BTreeMap<_, _> = group
                .cells
                .iter()
                .map(|cell| ((cell.x, cell.y), cell.directions))
                .collect();
            for cell in &group.cells {
                if cell.x >= level.width
                    || cell.y >= level.height
                    || cell.directions > 15
                    || !owners.insert((cell.x, cell.y))
                {
                    issue(
                        &mut errors,
                        &at,
                        "path cell outside level, invalid direction mask or duplicate ownership",
                    );
                }
                for (bit, reverse, dx, dy) in
                    [(1, 4, 0, -1), (2, 8, 1, 0), (4, 1, 0, 1), (8, 2, -1, 0)]
                {
                    if cell.directions & bit == 0 {
                        continue;
                    }
                    let x = i16::from(cell.x) + dx;
                    let y = i16::from(cell.y) + dy;
                    if !(0..=255).contains(&x)
                        || !(0..=255).contains(&y)
                        || !positions
                            .get(&(x as u8, y as u8))
                            .is_some_and(|mask| mask & reverse != 0)
                    {
                        issue(
                            &mut errors,
                            &at,
                            "path direction does not have a reciprocal cardinal neighbor",
                        );
                    }
                }
            }
        }
    }
    if object_count > 8192 {
        issue(
            &mut errors,
            "levels.objects",
            "more than 8192 course-wide objects",
        );
    }
    let mut used_spawns = BTreeSet::new();
    for &spawn in &course.start.spawns {
        destination(
            &mut errors,
            "start",
            Destination {
                level: course.start.level,
                spawn,
            },
            &levels,
        );
        if !used_spawns.insert(spawn) {
            issue(
                &mut errors,
                "start.spawns",
                "seats need distinct safe spawn points; no implicit placement offsets",
            );
        }
    }
    match objects.get(&course.finish.exit) {
        Some((_, Object::Exit { .. })) => (),
        _ => issue(
            &mut errors,
            "finish.exit",
            "must reference an explicit exit object",
        ),
    }
    if course.finish.kind == FinishKind::ReachExit && !course.finish.required.is_empty() {
        issue(
            &mut errors,
            "finish.required",
            "reach_exit cannot require collectibles",
        );
    }
    if course.finish.kind == FinishKind::CollectAndExit && course.finish.required.is_empty() {
        issue(
            &mut errors,
            "finish.required",
            "collect_and_exit requires at least one collectible",
        );
    }
    let mut required = BTreeSet::new();
    for id in &course.finish.required {
        if !required.insert(id)
            || !objects
                .get(id)
                .is_some_and(|(_, object)| object.collectible())
        {
            issue(
                &mut errors,
                "finish.required",
                format!("{id} is duplicate or not a collectible object"),
            );
        }
    }
    for level in &course.levels {
        for link in &level.links {
            let path = format!("levels[{}].links[{}]", level.id, link.endpoint);
            destination(&mut errors, &path, link.destination, &levels);
            if link.two_way
                && !levels.get(&link.destination.level).is_some_and(|other| {
                    other
                        .links
                        .iter()
                        .any(|back| back.destination.level == level.id && back.two_way)
                })
            {
                issue(
                    &mut errors,
                    &path,
                    "two_way link needs a reciprocal two_way link to this level",
                );
            }
        }
        match level.fall_target {
            FallTarget::Level {
                level: target,
                spawn,
            } => destination(
                &mut errors,
                &format!("levels[{}].fall_target", level.id),
                Destination {
                    level: target,
                    spawn,
                },
                &levels,
            ),
            FallTarget::LoseLifeAndRespawn { spawn } => destination(
                &mut errors,
                &format!("levels[{}].fall_target", level.id),
                Destination {
                    level: level.id,
                    spawn,
                },
                &levels,
            ),
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(ValidationErrors(errors))
    }
}

pub(crate) fn normalize(course: &mut Course) {
    course.levels.sort_unstable_by_key(|level| level.id);
    course.finish.required.sort_unstable();
    for level in &mut course.levels {
        let mut normalized: Vec<(u16, u16)> = Vec::with_capacity(level.cells.len());
        for &(count, cell) in &level.cells {
            if let Some(last) = normalized.last_mut().filter(|last| last.1 == cell) {
                last.0 += count;
            } else {
                normalized.push((count, cell));
            }
        }
        level.cells = normalized;
        level.spawn_points.sort_unstable_by_key(|spawn| spawn.id);
        level.objects.sort_unstable_by_key(Object::id);
        level.links.sort_unstable_by_key(|link| link.endpoint);
        level.paths.sort_unstable_by_key(|path| path.id);
        for path in &mut level.paths {
            path.cells.sort_unstable_by_key(|cell| (cell.y, cell.x));
        }
    }
}
