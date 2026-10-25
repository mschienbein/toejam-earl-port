# Maker course format 1 and native first-phase loader

The first executable phase is a strict, asset-free course model and local codec,
with an explicit `maker-native-v1` enhanced profile. Faithful remains the default.
There is no graphical editor, hosted sharing, network request, asset importer or
original-map exporter. `courses/*.tjemaker.json` are three newly authored layouts,
not fixed/generated original-level transcriptions.

## Authoring and local sharing

Edit a sample as UTF-8 JSON, then use the shared Rust implementation:

```sh
cargo run --locked -p tje-maker -- validate courses/meadow.tjemaker.json
cargo run --locked -p tje-maker -- encode courses/meadow.tjemaker.json
cargo run --locked -p tje-maker -- decode /your/local/code.txt
```

`encode` prints a complete `TJM1:` code; `decode` prints canonical JSON. Shell
redirection or an editor can save that output locally. The CLI opens only the
explicit input path, bounds reads, never interprets a course string as a path,
and never uploads anything. Validation without a ROM checks structure and
references; selecting the playable profile additionally checks the actual local
ROM terrain and supported capabilities before any machine writes.

For a deterministic headless playtest using ordinary recorded menu/pad input:

```sh
cargo run --locked -p tje-maker --features preview --example play_course -- \
  /your/rom.bin courses/two-gardens.tjemaker.json sessions/rev00-one-player.tje.json 1200
```

The recording is input, not an enhanced replay claim or a shareable machine save.
A course catalog must match the imported ROM; changing catalogs is an explicit
authoring change that produces a different digest, not an automatic fallback.
Interactive native selection is `--maker /your/course.tjemaker.json` in an
`enhanced-mode` build. It is mutually exclusive with the debug/Jam Out/extended
view selections. Exit and travel pads are represented by the original elevator
art resolved at runtime; approach the authored coordinates and press **Up**.

## Exact schema

Every object has exactly the listed fields. Unknown fields, duplicate JSON keys,
floats, invalid UTF-8, trailing JSON, unknown enum values and unsupported schemas
are errors. Integer sizes below are unsigned; negative numbers do not wrap.
Optional numeric values are explicit JSON `null` or an integer. Positions are
objects `{ "x": u16, "y": u16 }`, snapped to eight world pixels. Cell `(0,0)`
starts at world `(128,64)`; its extent is 128×64 pixels. Array order is editable
except where seat order or row-major cells carry meaning.

Top-level fields:

| Field | Meaning |
| --- | --- |
| `schema` | Exactly `tje-maker/1`. |
| `catalog` | `tje91-rev02/1` or `tje91-rev00/1`; logical references resolve only against that user's matching ROM. |
| `title`, `author`, `description` | UTF-8 text, at most 128/64/1024 bytes. No control characters; only description permits LF. Plain text, not markup. |
| `runtime_seed` | Exactly sixteen lowercase hexadecimal digits for u64. Never a JSON number. |
| `rules` | Fields below. |
| `start` | `{ "level": u16, "spawns": [u32, ...] }`; one distinct, nonoverlapping safe spawn per declared hero, in seat order. |
| `finish` | `{ "kind": "reach_exit" or "collect_and_exit", "exit": u32, "required": [u32, ...] }`. The exit must be an explicit exit object. Required IDs must be unique collectible objects; nonempty exactly for collect-and-exit. |
| `levels` | 1–32 levels with unique u16 IDs; IDs, not array positions, identify authored levels. |

`rules` contains `ruleset` (`classic_v1` or `overlord_v1`), `heroes` (1–4),
`tick_limit` (null or 1–216000 video frames), `lives` (0–9 spare lives), `bucks`
(0–99), `inventory` (0–16 present identity integers, each 0–27), and
`identify_presents` (boolean). `classic_v1` uses identity-index wrapping and the
chosen identification policy. Its original runtime AI RNG is initialized by
XOR-folding the explicit 64-bit seed to 32 bits, setting its low bit to one
(avoiding the original multiplicative RNG's zero state), and refreshing the
original cached RNG values. It never uses that stream to generate course cells or initial
population. A future generalized ruleset can introduce per-ID RNG streams;
that is not silently substituted into this version.

Native seat zero is ToeJam and seat one is Earl; the course owns the declared
seat count and applies initial resources/inventory to each declared hero. A
solo present's request to advance a story floor instead relocates safely to
the current floor's named respawn checkpoint (or first stable spawn when its
fall target is another floor). Course progression remains owned by authored
links and the finish rule, never an implicit original level+1. Native HUD floor
numbers are the dense internal slots; authored LevelIds remain the JSON/API IDs.

A level contains `id`, `width`, `height` (each size 8–64), `environment` (`earth`),
`music` (`gameplay` or `quiet`), and all these arrays/fields:

- `cells`: row-major `[count, cell]` runs, positive u16 count, total exactly
  width×height, at most 4096 runs. Cell bits 0–6 select base ID 0–121, bit 7 mirrors
  it horizontally, bits 8–11 select overlay 0–15, bit 12 mirrors a nonzero
  overlay. Bits 13–15 and an orphan overlay-mirror bit are invalid. These are
  references, never copied tiles, palettes or generation scratch flags.
- `spawn_points`: 1–16 `{ "id": u32, "position": Point }` records, IDs unique
  within their level.
- `objects`: course-wide unique u32 IDs, using the tagged shapes below. Maximum
  64 actors (including services/ship pieces), 128 items and 64 other props or
  endpoints per level; 8192 object records across the whole course.
- `links`: at most 64 `{ "endpoint": u32, "destination": { "level": u16,
  "spawn": u32 }, "two_way": bool }`. Every nonterminal exit/elevator has exactly
  one link. Targets must resolve; a two-way link needs an explicit reciprocal
  two-way link. A finish exit cannot also travel. There is no hidden level+1.
- `fall_target`: `{ "kind": "level", "level": u16, "spawn": u32 }` or
  `{ "kind": "lose_life_and_respawn", "spawn": u32 }` on the same level. No
  original negative-level/highest-floor wrapping.
- `paths`: at most 64 groups: `{ "id": u32, "cells": [{ "x": u8, "y": u8,
  "directions": u8 }], "trigger": "step_on", "arming_ticks": u16,
  "delay_ticks": u16, "replacement": cell }`. Delays each 0–3600 video frames;
  1–4096 cells. Directions use up/right/down/left bits 1/2/4/8 and require
  reciprocal cardinal neighbors. No duplicate path ID or cell ownership.
  Triggering any group cell schedules the whole group's replacement after the
  sum of its two delays. Timers advance deterministically on the global course
  clock, including when a floor is not currently visible; replacement persists.

Objects all contain `id` and `position`, plus:

| `type` | Additional fields |
| --- | --- |
| `earthling` | `definition`: devil, hamster, shopper, dentist, hula_dancer, bees, cupid, whirlwind, nerd_herd, lawnmower, shark, chicken, boogieman, mole, ice_cream_truck, santa, dentist_variant or bees_variant. `initial`: awake or asleep, subject to catalog capability. |
| `service` | `definition`: mailbox, wizard, wiseman, opera_singer, lemonade_stand or hot_tub. Mailbox is explicitly mail-order, not a random biting mode. |
| `item` | `definition`: `{ "kind": "present", "identity": 0..27 }`, `{ "kind": "food", "identity": 0..15 }`, or `{ "kind": "buck" / "large" / "sinking_large" / "hurt_hazard" }`. |
| `ship_piece` | `piece`: unique course-wide identity 0–9. |
| `telephone` | No additional fields. |
| `elevator` | No additional fields; an explicit link is mandatory. |
| `exit` | No additional fields; either a link or the unique finish reference. |

## Native capability admission and publication

The full bounded schema can describe later capabilities, but admission is not
permission to half-load them. The first phase supports one or two heroes,
`classic_v1`, 1–24 native 19×26 levels, and the native ROM resource catalog.
Four seats, Overlord, a larger grid or too many floors fail **before mutation**.
Native levels additionally require at most 20 actor records, 32 item records,
two endpoint sprite pairs, one telephone and one ship piece per floor. With two
heroes across multiple floors, each floor reserves at most one endpoint pair.
Eight dynamic bodies (companions included) per floor conservatively reserve
space for two active floors plus four native transient handles. Larger semantic
caps in the file format never overrule those executable limits.
`gameplay` music uses the original level music policy; the reserved `quiet`
capability is rejected by native admission rather than silently ignored.

Admission samples the actual eight-pixel terrain, overlays and mirrors from the
user's ROM; validates all terrain indices, both hero arrival footprints, object
support, actor sleeping capability, hostile arrival clearance and path source/
replacement references. Destructive paths cannot own spawn/object cells. It
never relocates an invalid actor or invokes a random placement repair. Validation
is safety/capability checking, not a claim that a dynamic puzzle is solvable or
that a clear stamp was earned.

Enhanced overrides own initial seeds, map loading/change/copy/swap/final guards,
initial inventory and Earthling banks, authored ship assignment, population
retain/release, explicit links/falls/goals and path changes. The immutable course
and runtime deltas live in the registered `tje-maker` version-1 typed sidecar.
Canonical level IDs map internally to dense original floor slots; those slots
are never serialized as authored links. Terrain is published as complete native
maps; actors/items are installed at exact authored positions with unused slots
cleared, so the original level-0/level-1/ship/random population branches cannot
leak through. Original bounded art/AI/movement/audio leaves still use the ROM.
Collection/removal, actor movement and authored telephone activation/ring state
persist by stable ID when a bank unloads; revisits do not reroll the layout or
initial content. Completion is the first living hero's Up activation of the
finish exit after all required IDs are collected. Completion/failure is a
mode-owned status, not a jump to an original ending PC. `is_transitioning()`
remains true until a travel transaction has published its camera and HUD, not
merely until the physical floor byte changes.
Eliminating one hero releases that hero's bank references and keeps a surviving
partner playable; exhausting all declared heroes' lives or the course timer
sets `Failed` without invoking the original game-over reset. Immutable course
configuration is validated when decoded and retains its canonical body/digest,
so normal rewind capture does not reparse, normalize or hash the whole course.

Slots 0, 1 and 26 are reserved: course floors use 2 through 25, so original
special-water borders, fixed first/final scenes and hidden-level assumptions
cannot reinterpret an authored ordinary floor.

Enhanced snapshots include the course/configuration identity and all sidecar,
continuation, core and device state. Cross-course/profile/ROM restores reject
before changing either half. Core/enhanced saves remain private ROM-derived
machine state, **not** course files or share codes. No enhanced netplay or
controller-only replay compatibility is claimed by this loader.

## Canonical body and share envelope

Canonical JSON is RFC 8785-compatible for this fixed ASCII-key, integer-only
schema: keys sorted, no insignificant whitespace, UTF-8 strings without Unicode
normalization, shortest integer notation. Level/object/spawn/path arrays sort by
ID; links sort by endpoint ID, required collectibles by ID, path cells by `(y,x)`.
Seat spawn order and initial inventory order are preserved. Adjacent equal cell
runs merge. Semantic ordering/whitespace changes preserve the SHA-256 identity.

`TJM1:` + unpadded Base64URL encodes:

| Bytes | Field |
| --- | --- |
| 0–3 | ASCII `TJMC`. |
| 4–5 | u16 little-endian schema major = 1. |
| 6–7 | u16 little-endian codec flags = 1 (RFC1950 zlib/DEFLATE only). |
| 8–11 | u32 little-endian exact uncompressed length. |
| 12–15 | u32 little-endian exact compressed length. |
| 16–47 | SHA-256 of canonical uncompressed JSON. |
| 48 onward | Exactly the declared compressed bytes. |

Text is limited to 3 MiB; body and compressed stream each to 2 MiB. Decode checks
text/decoded bounds before allocating, exact header lengths/flags, bounded zlib
output, end-of-stream and all consumed bytes, SHA-256, strict JSON and semantic
validation, and canonical body equality. Padding, nonzero unused Base64 bits,
trailing streams/data, duplicate keys and nesting deeper than sixteen fail.
Field-specific array visitors enforce caps while parsing rather than collecting
unbounded arrays. Different valid compressors can produce different codes for
the same canonical digest. The checksum detects corruption and identity; it
authenticates neither author nor safety. Above 8 KiB the CLI recommends a local
file; it never truncates a course or invents a hosted short token.
