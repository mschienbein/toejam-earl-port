# Extended view: opt-in 400 × 224 play

Status: selectable native/browser enhanced mode. Faithful 320 × 224 remains the default.
The agreed target is **400 × 224** (approximately 16:9), with 408–512-pixel widths explicitly
experimental. This is additional world coverage, not stretching a 320-pixel picture. HD art is
out of scope. Generated pictures, decoded patterns, palette values and map contents remain private.
The enhanced 400 × 224 surface uses square pixels for that approximate 16:9 presentation;
frontends must not apply the original 320-wide Genesis pixel-aspect correction a second time.
Faithful mode's existing native-NTSC/square-pixel and integer/fit choices remain unchanged.

The host compositor now has a publication-state API in
`tje_extended_view::presentation`: `PublishedView` freezes entity positions and
piece descriptors at graphics publication, and `Presenter` owns reusable output,
terrain and sprite layers. It decodes RLE, interleaved RLE and raw sprite pieces
only from the user's cartridge, applies native piece/mirror offsets, mask and
submerged-player clipping, sorts native draw order, composes shadows last, and
clips each split half independently. Ordinary extended output preserves the exact
hardware-rendered 320-pixel center. Authored scenes keep that centered image with
black pillars; an inventory half independently keeps its authored center.

The wider columns are host-owned: they do not allocate any additional native SAT
slots or tile groups. The original viewport allocation stays 320 pixels wide;
entities outside it still advance their canonical frame descriptors. Widening
Earthling/elevator interest is separate from sprite allocation. This avoids
turning a wider display into additional pressure on the native 54-slot and
112-tile upload limits.

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
5. Construct host world-sprite pieces from published entity frames, including normal/mirrored
   piece offsets, palette/priority, shadows, submerged-player clipping and mask pixels. Runtime
   ROM decoding bypasses the native SAT/tile-upload limits for the wings without feeding host
   handles into emulated RAM. RAM-authored speech pieces use their published native tile
   descriptors and live VRAM patterns. Ordinary wider output keeps the exact hardware center.
6. Compose each split half separately; no sprite crosses the split separator. UI, map, inventory,
   cutscenes, ship display, ride and transitions require an explicit published scene classification.
   Non-world authored scenes retain a centered original image; no automatic "map looks loaded"
   heuristic is sufficient. Avoid mixing completed-frame pixels with next-frame pending RAM state.
7. Keep faithful frame/audio replay observations as the oracle for unchanged output. The shipping
   frontend links this renderer only in a non-default enhanced build and constructs no presenter
   for faithful play. Behavioral hooks have a distinct enhanced identity and are not accepted by
   faithful-only netplay or labeled as faithful controller-only report recordings.

## Enhanced publication and behavior

All changes go through the separate, non-default EnhancedMode registry. Proven
translations, original camera following, map fog/points, collision and random
placement stay unchanged. `tje-extras` is the native/browser shared selector and
combines extended view with the optional in-play Jam profile without duplicate
function hooks.

| Entry | Enhanced behavior |
| --- | --- |
| `LOAD_IN_EARTHLINGS` | X interest expands from (-160,480) to (-160-m,480+m), where m=(W-320)/2. At 400 pixels this is (-200,520), preserving 160 pixels of preactivation beyond each visible edge. Alternating parity, Y bounds and allocation/free behavior stay original. |
| `ELEVATOR_UPDATE_ENTITY_VISIBILITY` | The same expanded X interest; all original state/level/entry semantics remain. |
| `BUILD_VIEWPORT_SPRITE_LINKS` | After the original, capture a pending fixed-size entity/piece submission. No next-frame entity positions are sampled for an already published image. |
| `HANDLE_V_BLANK` | Run original graphics/audio/input work, then publish the queued submission only when sprite graphics really upload. Capture committed cameras, split/inventory state and HUD. This is the C RTS callee, not the RTE interrupt entry. |
| `MAP_CLEAR_TILE_PRIORITY_NEAR_PLAYER` | Record the world-keyed patch. Host terrain honors its cleared priority only while the corresponding native ring slot still contains that exact tile with priority cleared, naturally expiring cells as camera streaming replaces them. |
| HUD positioning | After original positioning, split health/lives anchors move toward the wider left edge and level/action anchors toward the right. The center panel remains unscaled in shared mode. |
| Authored scene entries / `PROCESS_INPUT` | Before-only scene markers classify maps, pause/menu, title/intro/ending/ship, elevator and split transitions. Original reset/nonlocal tails remain possible; a later normal input tick explicitly reclaims world ownership. Custom VBlank and graphics-disable state also prevent world sampling. |

`IS_ENTITY_VISIBLE_IN_VIEWPORT` deliberately remains original. Widening the
native allocator would spend its 54 SAT slots and 112-tile upload budget on the
wings and cause avoidable center dropouts. The host compositor instead clips
published logical entity pieces to each widened viewport, independently of
native sprite allocation. Full map banks are borrowed, not copied each frame.
Atlas entries are lazy runtime-only decoded pieces from the user's ROM.

The native center preserves hardware scanline limits, sprite priorities, font
animation, raster effects and falling patches. The host wings honor piece draw
order independently of terrain priority, native shadow/highlight DAC rules, and
each split separator. Wider sight does **not** uncover extra map-screen fog or
alter camera split/merge distances. Original entity allocation capacity remains
unchanged; this is a view option, not a larger/denser-level mode.

Enhanced state snapshots include both publication buffers, scene state and the
immutable width. The renderer/atlas is derived presentation state, regenerated
after restore. Width-specific profile identities prevent loading 400-pixel state
into another width or faithful mode. A combined Jam profile has a separate
identity; disabling Jam later does not relabel an enhanced trajectory faithful.

## Selecting the mode

Native, built with `--features enhanced-mode`:

```sh
toejam-earl --rom /private/rev02.bin --extended-view 400
```

The default argument is zero/off. Values 408–512 in multiples of eight are
explicitly experimental, for example `--extended-view 512`. Choose width at
cartridge load, not mid-session. Extended output uses square pixels even if the
faithful NTSC pixel-shape setting is selected. Native `--enhanced-debug` is
exclusive with extended view.

Browser: build with `TJE_ENHANCED=1`, open **Extended view — default off**, choose
400 or an explicitly experimental width, then import/reload the user's ROM.
The setting is validated before replacing an installed cartridge. Changes apply
on the next import/page reload; they never mutate a running simulation identity.
Faithful-only builds keep the extra width choices unavailable.

Maps, inventory halves, elevator rides, title/intro/ending, ship scenes and
transitions keep their original 320-pixel centers with black pillars. Ordinary
gameplay and both 112-line split halves get the extra world columns. The combined
HUD keeps its original 320 × 40 center panel with neutral black side fill.

## Private replay evidence

The preview example uses the same profile and `Presenter`, and accepts authored
scenes as well as gameplay. It never commits generated assets:

```sh
cargo run --locked -p tje-extended-view --features preview --example preview -- \
  --rom /private/rev02.bin --session sessions/rev02-coop.tje.json --frame 2400 \
  --data-dir /private/engine-cache --output /private/after-400.ppm
```

Add `--faithful` for the exact 320 × 224 before image and original execution.
Add `--experimental-width 512` only for an experimental after image. Optional
`--hash-output /private/frames.txt` records each original framebuffer and stereo
f64 audio SHA-1 for unchanged-faithful evidence; it contains observations, not
pixel/audio payloads. Existing output files are never overwritten.

All screenshot, runtime atlas, ROM/cache and replay-observation artifacts remain
private under scratch or outside the public repository. Main controls integration;
there is no hosted service, asset download or spending in this option.

