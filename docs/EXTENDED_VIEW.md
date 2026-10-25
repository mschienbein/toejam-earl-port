# Extended view: research, renderer contract and opt-in prototype

Status: renderer-side experiment, not a shipping widescreen mode. Faithful is still the default.
The agreed target is **400 × 224** (approximately 16:9), with 408–512-pixel widths explicitly
experimental. This is additional world coverage, not stretching a 320-pixel picture. HD art is
out of scope. Generated pictures, decoded patterns, palette values and map contents remain private.
The enhanced 400 × 224 surface uses square pixels for that approximate 16:9 presentation;
frontends must not apply the original 320-wide Genesis pixel-aspect correction a second time.
Faithful mode's existing native-NTSC/square-pixel and integer/fit choices remain unchanged.

## Source and citation convention

`G:path:lines` cites the read-only sibling `toejam-earl-decomp`, commit
`57fc83fc54781af8e992bbc3b32afe788c1dbd01`. Names and control flow below come from that source;
proposed changes are explicitly identified as design. No ROM-authored table contents are reproduced.
The generated `tje-mem` views select the correct addresses for both REV 02 and REV 00.

## What exists before the renderer

### The entire current level is in RAM

There are three full, row-major **26 × 19 arrays of big-endian 16-bit cells**: player 1, player 2,
and a saved bank. Each is 988 bytes. These are complete loaded levels, not screen-sized caches;
there is not a RAM copy of every level in the campaign. Distinct player banks allow players to
occupy different levels. Path-direction banks are separate bytes, not another tile layer.
Sources: `G:include/ram_map.h:6-21`, `G:include/terrain.h:4-13`.

A rendered cell is a 16 × 8 array of 8 × 8 patterns: **128 × 64 world pixels**. Map cell (0,0)
begins at world (128,64), not (0,0). Interior coverage is x=[128,2560), y=[64,1728).
Low bits 0–6 select one of 122 base metatiles; bit 7 mirrors it horizontally. Bits 8–12 hold an
overlay selector, whose bit 4 is its mirror and low four bits select one of 15 overlays (1-based).
The remaining flags participate in generation/path changes and must not become tile IDs.
An overlay replaces a base **tile word only when that overlay word is nonzero**; this is not
per-pixel alpha blending of two metatiles. Mirror lookup reverses the 16 tile columns and XORs
the VDP horizontal-flip bit, but preserves an empty overlay word as zero.
Sources: `G:include/assets_map.h:10-21`, `G:include/terrain.h:24-30`,
`G:include/map.h:54-61`, `G:src/camera/camera_tiles_and_scroll.c:22-52,159-193`.

Outside the interior, ordinary levels return tile zero OR `gCameraTileAttributes`. Levels 1 and 26
use base metatile 82 as the exterior fill. Despite the source variable named `wraps`, those branches
**do not wrap coordinates into the opposite map edge**. Sources:
`G:src/camera/camera_tiles_and_scroll.c:111-193,347-392`.

### VDP planes are small streaming caches, not authoritative level data

The two foreground camera tilemaps occupy separate 64-column × 32-row ring buffers: the first
at VRAM A000, the second at C000; the normal parallax starfield is plane B at E000. HBlank changes
plane A selection and scroll for the bottom player. Sources:
`G:include/hardware.h:14-19`, `G:src/camera/camera_tiles_and_scroll.c:208-238,411-445`,
`G:src/core/startup_and_main_loop.c:195-207,257-278`.

Streaming is requested at 8-pixel camera crossings, with separate entering top/right/bottom/left
bits. Opposed bits suppress the pending transfer. The exact look-ahead matters:

- Set-origin cursors are `(camera_coordinate >> 3) - 1`.
- A horizontal row contains **45 words = 360 pixels**, beginning at column-cursor minus one;
  with a freshly established cursor its range is floor(x/8)-2 through floor(x/8)+42 inclusive.
- A right-entering vertical column uses column-cursor + 42, while a left-entering one uses the
  cursor itself. It is not a fresh 512-pixel-wide world image just because VRAM has 64 columns.
- Column uploads contain 32 rows in full-screen and 19 in split-screen; they start at row-cursor
  minus one. Bottom-entering row uses row-cursor + upload-height - 3.
- Both dimensions split transfers at the ring-buffer boundary; columns use a 128-byte VRAM
  increment. Thus the maintained gutters are only a handful of 8-pixel tiles and depend on path
  and sub-tile phase, **not the 40 pixels required on each side of a centered 400-pixel view**.

Sources: `G:src/camera/camera_tiles_and_scroll.c:68-198,270-397,411-474,477-506,522-547,588-737`.
Simply widening VDP output exposes stale or unrelated ring-buffer columns. Increasing original
transfer lengths is also unsafe: the row buffer is exactly 45 words and column buffer 32 words
(`G:include/ram_camera.h:34-50`).

Use the **committed** camera for a completed-frame sidecar. Pending camera positions become
committed at VBlank when sprite graphics are published, and actual VDP scroll uses committed
positions. Pending positions remain correct for the original simulation visibility predicates.
Source: `G:src/core/startup_and_main_loop.c:175-207,257-275`.

### Entity visibility, pieces and linked viewport chains

`IsEntityVisibleInViewport` first checks inventory state and matching level. It tests width 320,
height 224 for an unsplit primary or 112 for either split half. Its exact predicates are:

- entity.width + relative_x >= 0;
- relative_x - entity.width <= 320;
- relative_y + 10 > 0;
- relative_y - entity.height_pixels - entity.height <= viewport_height.

`UpdateEntityViewportSprites` allocates/releases viewport-specific pieces and transfers each
player's ownership between views; speech-bubble entities have a separate path. The all-entity
pass visits 26 ordinary slots or all 40 on the final level. Source:
`G:src/sprites/entity_visibility.c:11-94`.

`PositionEntitySprites` uses x = entity.x + 128 - pending_camera_x, adding per-piece normal or
mirrored offsets. Primary Y bias is 128; bottom Y bias is 240 (=128+112); elevation is subtracted.
`BuildViewportSpriteLinks` walks the sorted entity list, then shadows, then inventory pieces;
its primary and secondary arrays each have capacity 54. Sentinels are hardware slots 23/24,
terminator 79. VBlank/HBlank enable the appropriate chain, so walking one final VRAM SAT after
the frame does not reconstruct the top half's chain history. Sources:
`G:src/sprites/entity_sprite_rendering.c:42-188,190-263`,
`G:src/sprites/sprite_links_vdp.c:5-35`, `G:include/ram_sprites.h:8-26`.

There are only **54 shared viewport slots**, 80 hardware sprites, 80 groups of eight sprite tiles,
and a 112-tile per-frame animation/upload budget. Allocating more visible sprites can fail even
before the VDP's hardware per-scanline limits. Render visibility bits also influence tile upload
and animation work, so changing culling is an enhanced simulation profile, not an output-only
filter. Sources: `G:include/ram_sprites.h:6-43,74-79`,
`G:src/sprites/sprite_slots.c:10-59,110-131,161-225`,
`G:src/sprites/entity_sprite_rendering.c:443-485`.

### Earthling placement is not an off-screen spawning ring

Initial/relocated Earthling records are placed on the **whole level**: ten random metatile trials,
then a cyclic exhaustive scan, terrain validation, and overlap clearance. This algorithm does not
read either camera. Do not replace it with an invented viewport-relative spawn rule.
Source: `G:src/earthlings/spawning_and_banks.c:100-175`.

The viewport-dependent part is **activation**: `LoadInEarthlings` handles alternating record
indices, updates record positions from live objects, and sets visibility flags for matching-level
records with x in (-160,480) and y in (-112,336), relative to each pending camera. Secondary
interest requires split-screen. It allocates entities entering either interest rectangle and frees
objects outside both. This y range stays the same even in split-screen in the original.
Sources: `G:src/earthlings/spawning_and_banks.c:360-473`.

An Earthling present deliberately places its creature at player x±40 and y±25; this is an intended
near-player appearance, not a pop-in defect to move out of sight. The 340×200 special-record
clearance and sleep/contact/telephone/attack ranges are gameplay distances, not necessarily view
bounds. Keep them unchanged. Sources:
`G:src/earthlings/spawning_and_banks.c:78-96,475-503,600-680`,
`G:src/presents/telephone.c:103-105`, `G:src/presents/projectile_effects.c:657-662`.

### Other 320-pixel assumptions

| Area | Existing behavior / citation | Extension design |
| --- | --- | --- |
| HUD | Combined window background is 40 columns × 5 rows starting row 23; rank text writes fixed addresses; level/action/lives/health are fixed-position SAT sprites. `G:src/ui/hud.c:55-99,132-173,283-316,464-579`. | Preserve the original 320×40 center panel without resampling. Author/derive a neutral side fill at runtime; never duplicate rank text. Move split HUD's left anchors to the new left edge and right anchors to the new right edge in host presentation, not world camera math. |
| Split | Top/bottom are 112 lines. HBlank selects the second foreground buffer and scroll Y-112. Enter/split animation centers at x+160; shared camera uses midpoint-160. Split thresholds are x>220/y>124; merge is x<200/y<104. `G:src/core/startup_and_main_loop.c:257-275,375-455,488-555`; `G:src/camera/split_views.c:10-38`. | Retain top/bottom layout: each view gets width W, 112 lines. A symmetric sidecar preserves center x+160 automatically, so no camera recenter override is needed. Keep split/merge thresholds unchanged for the first mode; making shared-screen tolerance wider is a separate balance change. |
| Camera following | Single-player/split center x=160; shared dead zone 50..270; camera X clamps 8..2360. `G:src/camera/camera_follow.c:58-89,96-159`; `G:src/camera/camera_tiles_and_scroll.c:477-506,664-737`. | Keep these original world camera values; renderer derives origin x-margin. At world edges, extra area shows the specified exterior fill rather than revealing wrapped/stale cache data. |
| Map screen | 40-column background/window, 19×26 terrain cells each drawn as two tiles; map icons use world/8; cover blocks are 40×32. `G:src/map/map_drawing_and_controller.c:58-129,157-205,213-259,276-291,348-367,501-577`. | Display this authored map at centered 320×224 with side fill. Do not pretend it is a normal playfield or reveal additional map cells. |
| Fog / points | Visitation is keyed to player world position: x=player.x-72, y=player.y-8, 320×256 cover regions; newly visited region awards points. `G:src/core/startup_and_main_loop.c:347-361`. | Do not replace 320 with W here: that changes cover indexing and score. Wider sight does not automatically uncover map-screen fog. |
| Elevators in world | Entity-pair visibility uses the same (-160,480) × (-112,336) activation rectangles as Earthlings. `G:src/elevator/elevator_scene.c:203-235`. | Extend X interest by the same symmetric margin as Earthlings; leave collision/entry/exit checks unchanged. |
| Elevator ride | A separate scene centers its entity at camera.x+160 and y+144, repeats an 8×8 pattern six times, replaces VBlank, palettes and VRAM patterns. `G:src/elevator/elevator_scene.c:436-459,561-647`. | Center the original 320×224 ride with side fill. Do not sample a level map through ride VRAM or expose uncomposed scene wings. |
| Terrain occlusion | Falling clears priority bits in a roughly 10×13-tile VDP patch, not the RAM metatile map. `G:src/player/active_present_falling_and_respawn.c:69-116,132-160`. | Full host composition must mirror this state in a world-keyed priority sidecar, captured at publication; cannot assume raw map priority exactly reproduces the center. |
| Presents / text | Camera-sensitive present recentering uses x+20..x+300; camera-owned attachment offsets are unsigned bytes. `G:src/presents/present_effects.c:245-247`; `G:src/sprites/entity_sprite_rendering.c:18-39`. | Keep gameplay recenter behavior initially. Treat screen-owned text/UI separately; do not widen byte offsets or confuse screen attachments with world entities. |

## Renderer extension design

1. Keep the faithful engine and its 320×224 output untouched. A missing/disabled extra selects
   the existing output directly, without allocating a sidecar, calling callbacks, or reading assets.
   Native and browser must use the same explicit view configuration in enhanced session identity.
2. Widescreen width W=400 adds m=(W-320)/2=40 world pixels left/right. Experimental widths must
   be an explicit 8-pixel multiple greater than 400 and at most 512. Height remains 224; top/bottom
   split keeps the same horizontal margin for both independently selected maps/cameras.
3. At a graphics publication boundary, borrow the user's ROM, the corresponding RAM map bank,
   committed camera, live pattern VRAM and live CRAM. Resolve each world 8×8 tile directly from
   its 128×64 metatile, overlays and flips. Use VRAM **pattern bytes**, never plane-name-table
   columns. Palette animation remains live. This is bounded work over at most 64×28 tile spans,
   with caller-owned reusable output storage, no per-frame ROM/map copy, and no ROM-derived assets
   in the repository. Pattern indices, palette, horizontal/vertical flip and priority remain distinct.
4. Derive the normal starfield from the user's ROM `StarfieldByteMap` and pattern bytes: each of
   its 32 source columns repeats twice into a 64-column map, and scroll is half the original
   committed camera (`G:src/core/startup_and_main_loop.c:87-102,195-207`). Do not halve screen pixel
   coordinates: the background scroll, not the pixels themselves, is half speed.
5. Start with a diagnostic background-only image. For useful play, construct host world-sprite
   pieces from each entity's current frame, including normal/mirrored piece offsets, shadows,
   draw order, clipping and text. Preserve the center's priority patches and publication timing.
   A full host sprite atlas decoded from the user's ROM avoids the original 54-slot/VDP-scanline
   limits; it must not feed host-only handles into emulated RAM arrays. Widening the SAT scan alone
   cannot display entities that were never allocated or had no patterns uploaded.
6. Compose each split half separately; no sprite crosses the split separator. UI, map, inventory,
   cutscenes, ship display, ride and transitions require an explicit published scene classification.
   Non-world authored scenes retain a centered original image; no automatic "map looks loaded"
   heuristic is sufficient. Avoid mixing completed-frame pixels with next-frame pending RAM state.
7. Keep faithful frame/audio hashes as the oracle for unchanged output. For a renderer-only
   diagnostic, a source dependency graph with **no incoming edge from shipping crates** proves it
   cannot affect faithful execution. Activating behavioral hooks is a different enhanced identity,
   never a faithful proof or a session accepted by faithful netplay.

## Function overrides through EnhancedMode

Use the integrated, non-default `tje-engine-runtime/enhanced-mode` registry, not a second hook
table or edits to the proven faithful translations. Its callback API is
`EnhancedProfile::new(name, value_translations, framed_translations)` with existing
`tje_game::Context` callbacks. Original unselected callees run compiled original instructions.
The existing `NativeCompiledOriginalCallees` transitional profile is a proven-readable timing
composition tool, not permission to silently modify faithful behavior; it remains separate.

The symmetric-camera design needs only these initial behavioral changes:

| Function ID | Prototype modification | Retained behavior |
| --- | --- | --- |
| `IS_ENTITY_VISIBLE_IN_VIEWPORT` | X predicates become entity.width+x >= -m and x-entity.width <= 320+m. | Level/inventory gating, height/elevation and split Y bounds. |
| `LOAD_IN_EARTHLINGS` | X interest becomes (-160-m,480+m); at W=400 it is (-200,520). | Alternating parity, position sync, bank/level/split flags, allocation/free, original Y range. |
| `ELEVATOR_UPDATE_ENTITY_VISIBILITY` | Same X-interest expansion. | Pair creation/free, blocked/absent states, levels and collision semantics. |

Keep `UpdateEntityViewportSprites`, sprite positioning, camera following and random spawn
selection original initially. Registry callbacks still need allocation/free through the registry's
real call dispatch, not ad-hoc host mutations or fabricated return PCs. The mode/width is static
per profile for deterministic snapshots/netplay; do not change profiles during a live session.

These three overrides alone are **not a complete widescreen game**: they can increase pressure on
the original entity/tile/sprite pools and they intentionally alter simulation work. They belong in
a diagnostic gated profile until host sprite composition and scene publication are complete.

## Prototype boundary and integration gate

`crates/tje-extended-view` is an independent, opt-in experiment. Its background renderer is a
real RAM-map/metatile/pattern implementation with synthetic ROM-free tests; it is not linked by
any shipping crate. Its native preview example reads a user ROM and an input-only session,
renders a requested gameplay frame, and writes a **private** image only to a path supplied by the
caller. The original center may be preserved for comparison; wings are explicitly background-only,
not evidence of widened sprites, HUD adaptation, full occlusion fidelity or a shipping toggle.

The non-default `enhanced-mode` feature adds explicit versioned profiles
`extended-view-400/1` and `experimental-extended-view-512/1`, containing only the three
overrides above. Other experimental renderer widths do not implicitly gain behavioral profiles.
The native diagnostic can opt into those callbacks with `--enhanced-culling`; without that
argument it replays the original engine unchanged. EnhancedMode's finite original-callee budget
still applies: allocation-pressure relocation can reject rather than silently exceeding it.

Example (all output/cache paths are user-private; supply your own paths and a normal-world frame):

```sh
cargo run --locked -p tje-extended-view --features preview --example preview -- \
  --rom /private/rev02.bin --session sessions/rev02-coop.tje.json --frame 2400 \
  --data-dir /private/engine-cache --output /private/extended-view.ppm
```

Add `--background-only` to inspect the entire map-derived image. The default image preserves
the exact original 320-pixel center and adds background-only wings. To exercise behavior changes,
build with `--features preview,enhanced-mode` and pass `--enhanced-culling`; add
`--experimental-width 512` only for the explicitly experimental profile. The example's custom
VBlank guard rejects ride scenes, but is **not** a complete scene detector: the caller must still
choose a world frame. It is not a menu or a product setting.


Before turning this into a selectable frontend mode, require: coherent scene/camera/VRAM/CRAM
publication; entity-piece and shadow sidecar composition; speech and HUD anchors; correct
falling-priority patches; no object allocation pop-in under wider interest; both split halves on
different levels; exterior fills; inventories/map/elevator ride/intro/ending; width-specific enhanced
session identity; both revisions; and unchanged faithful frame/audio replay hashes. Main controls
integration and shipping. No user decision is needed for the bounded 400-pixel default or the
centered authored-scene policy; a wider gameplay/balance change remains explicitly experimental.
