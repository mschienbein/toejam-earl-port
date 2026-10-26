//! A redacted geometric director panel. This consumer cannot retain floors,
//! activate populations, consume RNG or inspect inventories/health/status UI.
use crate::{
    ActorId, DIRECTOR, Error, HEROES, LevelId, MAX_SPAWNS, MAX_TRAPS, OverlordState, Result,
    SeatId, SeatMask,
    placement::Point,
    state::{MatchResult, SpawnStage, Tool},
    traps::{Stage, TrapKind},
};
use tje_world::{LevelGeometry, Resources};

pub const WIDTH: usize = 400;
pub const HEIGHT: usize = 224;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeroMarker {
    pub seat: SeatId,
    pub level: LevelId,
    pub x: i32,
    pub y: i32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Marker {
    pub id: u32,
    pub x: i32,
    pub y: i32,
    pub kind: MarkerKind,
    pub telegraph: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerKind {
    Spawn,
    Snare,
    Bump,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Camera {
    Tactical { level: LevelId, cursor: Point },
    Body(ActorId),
    Unavailable,
}
/// The publication owner explicitly supplies authorized geometry and public hero
/// markers. There is no rich hero packet to accidentally serialize for a remote
/// director. A camera request outside this list yields a neutral panel.
pub struct Authorization<'a> {
    pub owners: SeatMask,
    pub levels: &'a [LevelId],
    pub heroes: &'a [HeroMarker],
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packet {
    pub frame: u64,
    pub camera: Camera,
    pub energy: u16,
    pub result: MatchResult,
    pub tool: Tool,
    pub cooldowns: [u16; 3],
    pub possessed: Option<ActorId>,
    pub heroes: [Option<HeroMarker>; 3],
    pub markers: [Option<Marker>; MAX_TRAPS + MAX_SPAWNS],
}
pub fn publish(state: &OverlordState, auth: Authorization<'_>) -> Result<Packet> {
    if !auth.owners.contains(DIRECTOR) {
        return Err(Error::Role);
    }
    auth.owners.validate().map_err(|_| Error::Role)?;
    if auth.levels.len() > 32
        || auth.levels.windows(2).any(|pair| pair[0] >= pair[1])
        || auth.heroes.len() > 3
    {
        return Err(Error::Invalid("director authorization bounds/order"));
    }
    for level in auth.levels {
        level
            .validate()
            .map_err(|_| Error::Invalid("authorized LevelId"))?;
    }
    let allowed = |level| auth.levels.binary_search(&level).is_ok();
    let mut heroes = [None; 3];
    for marker in auth.heroes {
        if !HEROES.contains(marker.seat)
            || heroes[marker.seat.index()].is_some()
            || !allowed(marker.level)
            || marker.x < 0
            || marker.y < 0
            || marker.x > 8320
            || marker.y > 4160
        {
            return Err(Error::Invalid("authorized public hero marker"));
        }
        heroes[marker.seat.index()] = Some(*marker);
    }
    let camera = if state.body_view {
        state
            .control
            .filter(|control| allowed(control.level))
            .map_or(Camera::Unavailable, |control| Camera::Body(control.actor))
    } else if allowed(state.cursor_level) {
        Camera::Tactical {
            level: state.cursor_level,
            cursor: state.cursor,
        }
    } else {
        Camera::Unavailable
    };
    let mut markers = [None; MAX_TRAPS + MAX_SPAWNS];
    let mut index = 0;
    for trap in state
        .traps
        .iter()
        .flatten()
        .filter(|trap| trap.active() && allowed(trap.level) && trap.level == state.cursor_level)
    {
        markers[index] = Some(Marker {
            id: trap.id,
            x: trap.point.x,
            y: trap.point.y,
            kind: match trap.kind {
                TrapKind::Snare => MarkerKind::Snare,
                TrapKind::Bump => MarkerKind::Bump,
            },
            telegraph: trap.stage == Stage::Telegraph,
        });
        index += 1;
    }
    for spawn in state.spawns.iter().flatten().filter(|spawn| {
        spawn.stage != SpawnStage::Expired
            && allowed(spawn.population.level)
            && spawn.population.level == state.cursor_level
    }) {
        markers[index] = Some(Marker {
            id: spawn.source,
            x: spawn.point.x,
            y: spawn.point.y,
            kind: MarkerKind::Spawn,
            telegraph: spawn.stage == SpawnStage::Telegraph,
        });
        index += 1;
    }
    Ok(Packet {
        frame: state.last_tick().unwrap_or(0),
        camera,
        energy: state.energy,
        result: state.result,
        tool: state.tool,
        cooldowns: [
            state.spawn_cooldown,
            state.possession_cooldown,
            state.trap_cooldown,
        ],
        possessed: state.control.map(|control| control.actor),
        heroes,
        markers,
    })
}

/// Newly authored circles/arrows/bars; resource terrain is read only and never
/// retained in an authoritative snapshot. Caller reuses a 400x224 panel buffer.
/// A possessed-body camera is composed by S5, not a second actor renderer here.
pub fn draw_tactical(
    packet: &Packet,
    level: LevelId,
    geometry: &LevelGeometry,
    resources: &impl Resources,
    pixels: &mut [u32],
) -> Result<()> {
    if pixels.len() != WIDTH * HEIGHT {
        return Err(Error::Invalid("director panel extent"));
    }
    pixels.fill(0xff101820);
    let Camera::Tactical {
        level: authorized,
        cursor,
    } = packet.camera
    else {
        return Ok(());
    };
    if level != authorized {
        return Err(Error::Role);
    }
    let (world_width, world_height) = geometry.dimensions().world_envelope();
    let map_height = HEIGHT - 16;
    for y in 0..map_height {
        for x in 0..WIDTH {
            let terrain = geometry
                .terrain_at(
                    level,
                    (x as i32 * world_width) / WIDTH as i32,
                    (y as i32 * world_height) / map_height as i32,
                    resources,
                )
                .map_err(|_| Error::Shared)?;
            pixels[y * WIDTH + x] = match terrain {
                0x10 => 0xff244d36,
                0x20 => 0xff766844,
                0x80 => 0xff244a68,
                _ => 0xff101820,
            };
        }
    }
    let project = |x: i32, y: i32| {
        (
            (i64::from(x) * WIDTH as i64 / i64::from(world_width)) as i32,
            (i64::from(y) * map_height as i64 / i64::from(world_height)) as i32,
        )
    };
    for marker in packet.markers.iter().flatten() {
        let (x, y) = project(marker.x, marker.y);
        let color = if marker.telegraph {
            0xffffb840
        } else {
            0xffed6654
        };
        circle(pixels, x, y, 4, color);
        if marker.kind == MarkerKind::Bump {
            for offset in -3..=3 {
                put(pixels, x, y + offset, color);
            }
            put(pixels, x - 1, y - 2, color);
            put(pixels, x + 1, y - 2, color);
        }
    }
    for hero in packet
        .heroes
        .iter()
        .flatten()
        .filter(|hero| hero.level == level)
    {
        let (x, y) = project(hero.x, hero.y);
        circle(pixels, x, y, 2, 0xffa0e8ff);
        for offset in 0..=i32::from(hero.seat.get()) {
            put(pixels, x - 2 + offset * 2, y + 4, 0xffffffff);
        }
    }
    let (x, y) = project(cursor.x, cursor.y);
    for delta in -5..=5 {
        put(pixels, x + delta, y, 0xffffffff);
        put(pixels, x, y + delta, 0xffffffff);
    }
    for y in HEIGHT - 10..HEIGHT - 4 {
        for x in 4..4 + usize::from(packet.energy) * 3 {
            pixels[y * WIDTH + x] = 0xff67d99c;
        }
    }
    Ok(())
}
fn put(pixels: &mut [u32], x: i32, y: i32, color: u32) {
    if x >= 0 && y >= 0 && x < WIDTH as i32 && y < (HEIGHT - 16) as i32 {
        pixels[y as usize * WIDTH + x as usize] = color;
    }
}
fn circle(pixels: &mut [u32], x: i32, y: i32, radius: i32, color: u32) {
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            let distance = dx * dx + dy * dy;
            if distance <= radius * radius && distance >= (radius - 1) * (radius - 1) {
                put(pixels, x + dx, y + dy, color);
            }
        }
    }
}
