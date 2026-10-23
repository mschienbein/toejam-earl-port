# Retail revisions: REV 00 and REV 02

The port supports the original worldwide release (**REV 00**) and the later
**REV 02**, commonly labelled **Rev A**. REV 02 is the primary naming/address
reference, not a replacement for REV 00's behavior. Supplying REV 00 must preserve
its original bugs, not silently apply REV 02's fixes.

This document describes source behavior and layout metadata only. It contains no
ROM payload, extracted assets, instruction encodings or copied compiler listings.
The behavioral authority is the matching decompilation's
[revision comparison][g-revisions], [data accounting][g-data] and the revision
branches in the source files linked below. The private `build/rev02/` and
`build/rev00/` listings at the c038-final checkpoint were also consulted; paths and
line references below let a maintainer reproduce that inspection without publishing
the listings.

## Scope of the differences

Both cartridges are 1 MiB. Their cartridge-header release dates describe July
1991 (REV 00) and October 1991 (REV 02); the product revision suffix and checksum
also differ. These fields describe the releases but are **not** how the port
trusts an imported image.

The common catalogue pairs **648 entries: 645 functions and three assembly
fragments**. The decomp's final comparison reports:

- **223** entries are byte-identical between revisions.
- **417** differ only in relocated code/data addresses, not logic.
- **8** functions have actual logic changes, represented by **13** revision
  preprocessor sites. Seven change size; the terrain-motion change only changes
  a mask. There are no extra REV 02 functions in the catalogue.
- REV 02 has **112 more code bytes** overall. Address shifts are cumulative and
  are not one constant offset valid throughout the ROM.

The first two counts describe the decomp's masked-signature/address comparison;
they are not native-port proof coverage. Current Rust readiness and both address
columns are generated separately in [FUNCTIONS.md](FUNCTIONS.md).

## All eight changed functions

Addresses below are catalogue entry offsets, not a universal revision conversion.
Source line references are to the matching decomp checkpoint.

| Function | REV 02 address | REV 00 address | REV 00 behavior and REV 02 change |
|---|---|---|---|
| `DrawGameMap` | `0x008836` | `0x008836` | REV 00 leaves the opening player's idle counter alone. REV 02 clears `gPlayerIdleTicks[player]` when opening the map, so time already counted as idle does not immediately put the player to sleep after using the map. Source: [map_drawing_and_controller.c:510–513][map-idle]. |
| `DrawInventory` | `0x008F3A` | `0x008F2A` | REV 00 preserves the opener's idle counter. REV 02 clears it before initializing the inventory display. Together with the map reset and sleep guard below, this stops menu/choice activity from being treated as uninterrupted idleness. Source: [inventory.c:220–222][inventory-idle]. |
| `PlayerProcessTerrainMotion` | `0x00DD82` | `0x00DD62` | REV 00 can apply terrain movement during a lean animation. REV 02 adds `PLAYER_GROUP_LEAN` to `TERRAIN_MOTION_PAUSED_GROUPS`, suppressing terrain motion in that animation group too. This is a same-size mask change, not a new routine or a coordinate/layout change. Source: [entity_motion_terrain.c:54–61][terrain-mask]. |
| `ProcessSleep` | `0x012342` | `0x012322` | REV 00 advances the idle-to-sleep counter even while an interaction choice is open. REV 02 also requires `gPlayerInteractionChoiceActive[player]` to be clear before advancing it; a player cannot doze off midway through that choice prompt merely because no movement was entered. The existing sleep threshold is unchanged. Source: [sleep.c:102–107][sleep-guard]. |
| `SetupSelectedPresent` | `0x017254` | `0x017224` | Two fixes: REV 02 checks `gPlayerFallStates[player_number]` before starting rocket-skates sound/motion, so opening skates while falling no longer starts them; and it includes `PLAYER_GROUP_HITOPS` (animation-group bit 27) in the groups that can be replaced by the active-present pose. REV 00 has neither guard/addition. The rocket-skates powerup timer adjustment still precedes the new guard; this is not a wholesale rejection of the present. Source: [present_dispatch.c:144–153][skates-guard] and [179–183][present-pose]. |
| `ProcessLand` | `0x01F6D0` | `0x01F68E` | REV 00 can start a fall when terrain turns to void even if the player is already in whirlwind or whirlwind-recovery animation. REV 02 excludes both animation states. The extra predicates occur in the ToeJam path and in both shared-screen and split-screen Earl paths, not just one player/view. Source: [land_processing.c:542–546][land-toejam], [563–567][land-earl-shared] and [578–582][land-earl-split]. |
| `TitleScreenVideo` | `0x027A10` | `0x027998` | REV 00 saves the entry frame counter as `start_time`, initializes `gSceneLastMusicEventTime` from the current counter, and tests elapsed time against 1,200 ticks. REV 02 clears both `gFrameCounter` and `gSceneLastMusicEventTime` and tests the counter directly against 1,200. Thus the nominal timeout remains 1,200 ticks, but the shared counters and the REV 00 start-time local/elapsed comparison differ. Source: [title_and_transition.c:282–314][title-time]. |
| `SEGAVideo` | `0x03A4E4` | `0x03A476` | REV 00 starts its 120-tick logo fade using the existing global frame count. REV 02 clears `gFrameCounter` before capturing `palette_time` and `start_time`. Palette cadence and the nominal duration still use their existing logic; it is the shared counter's starting value that changes. Source: [sega_logo.c:58–63][sega-time]. |

These are fixes in the later retail program, not permission to fix other original
bugs. In particular, the two frame-counter resets establish a different global
counter history even when a scene looks the same. Any further claim about their
effect on later gameplay or RNG requires measured evidence, not just the equal
nominal timeout.

### Listing and code-size evidence

In each revision's `build/<revision>/asm/` directory, the corresponding MCC68K
`.SRC` files confirm the added/removed conditions and stores. The following are
**compiler-reported code sizes**, not the catalogue's complete extents (which may
also account for adjacent embedded tables/text). No listing text is reproduced.

| Function | Listing file | REV 02 summary line / size | REV 00 summary line / size | REV 02 size change |
|---|---|---|---|---:|
| `DrawGameMap` | `c_map_drawing_and_controller.SRC` | 2098–2099 / 844 bytes | 2093–2094 / 828 bytes | +16 |
| `DrawInventory` | `c_inventory.SRC` | 601–602 / 264 bytes | 596–597 / 248 bytes | +16 |
| `PlayerProcessTerrainMotion` | `c_entity_motion_terrain.SRC` | 505–506 / 1,020 bytes | 505–506 / 1,020 bytes | 0 |
| `ProcessSleep` | `c_sleep.SRC` | 365–366 / 848 bytes | 360–361 / 832 bytes | +16 |
| `SetupSelectedPresent` | `c_present_dispatch.SRC` | 699–700 / 598 bytes | 694–695 / 580 bytes | +18 |
| `ProcessLand` | `c_land_processing.SRC` | 2101–2102 / 714 bytes | 2089–2090 / 660 bytes | +54 |
| `TitleScreenVideo` | `c_title_and_transition.SRC` | 1018–1019 / 318 bytes | 1025–1026 / 328 bytes | −10 |
| `SEGAVideo` | `c_sega_logo.SRC` | 255–256 / 556 bytes | 254–255 / 554 bytes | +2 |

The changes sum to +112 bytes. As focused cross-checks, the terrain-mask operand
changes at line 145 in both terrain-motion listings; the title counter resets and
direct comparison appear at REV 02 lines 958–959 and 999; and the Sega reset is
present at REV 02 line 91 but absent at the same point in REV 00.
Entry addresses are independently visible in each revision's `toejam.map` at lines
325, 339, 488, 567, 668, 851, 1033 and 1214, in the table's function order.

## Data, pointers and layout

The matching decomp names **4,163 data units in each revision**. Of these, 2,519
are identical and 1,642 differ only because embedded addresses relocate. The
remaining two differences are the cartridge header and `ROMTailFill`.

- **Graphics, text, music, sounds and level data are unchanged in content.**
  Relocated references inside an otherwise equivalent unit do not imply new art,
  music, text or levels.
- **Code pointers and data pointers move.** Pointer tables, animation/frame
  references and other address-bearing records must use the selected revision's
  layout. `FunkatronSceneDescriptors` is a particular trap: it stores **low words**
  of relocated addresses, so unchanged record sizes do not make its contents
  interchangeable. The final maps place it at REV 02 `0x0AF8C0` and REV 00
  `0x0AF850` (`toejam.map:7533` in each revision).
- **Unit sizes are unchanged except the tail fill.** `ROMTailFill` is 424 bytes
  in REV 02 versus 536 in REV 00, absorbing the net 112-byte code growth while
  preserving the 1-MiB cartridge size. No fill contents need to be copied into
  the public port.
- The decomp's non-catalogued data accounting is 867,776 bytes for REV 02 and
  867,888 for REV 00. These counts include header/fill and relocated pointers;
  they are not an additional native-port asset package.

The decomp establishes these conclusions by pairing catalogue entries with
address-masked instruction signatures and translating differing addresses in the
data comparison through the same revision map. The two revisions build from the
same readable sources, selected with `REVISION` 0 or 2; address-only differences
are not maintained as duplicate logic implementations.

## How the port selects and preserves a revision

1. **Normalize, then identify the entire ROM by hash.**
   [`tje_rom::Rom::import`](../crates/tje-rom/src/lib.rs) normalizes a raw image,
   supported byte-swapped/SMD image or single-ROM ZIP to cartridge byte order and
   compares its complete SHA-1 against the two pinned retail identities.
   Unknown/damaged images are rejected. A filename, file extension, header label
   or caller-supplied revision flag cannot override that identity. The pinned
   hashes are exposed by `tje_rom::Revision::sha1()` in the same file.
2. **Carry that revision through all address lookups.**
   [`FunctionMetadata`](../crates/tje-game/src/lib.rs) stores paired addresses and
   sizes in **[REV 02, REV 00]** order; `address(revision)`, `size(revision)` and
   `function_at(revision, address)` select the correct entry. The generated
   [metadata modules](../crates/tje-game/src/metadata/) retain each function's
   original source/ABI identity. RAM/ROM typed views in `tje-mem` also take the
   revision; resources remain in the user's matching ROM, not embedded Rust data.
3. **Keep real behavioral branches, not just relocated addresses.**
   Rust translations consult `Context::revision()` where the originals differ.
   For example, the present dispatcher, sleep logic, title scene and Sega scene
   contain explicit REV 00/REV 02 branches. An unwritten/pending function is not
   evidence that both versions have been ported: see the generated status/blocker
   row in [FUNCTIONS.md](FUNCTIONS.md). During the current hybrid phase, original
   fallback executes that cartridge's original function, preserving its revision's
   behavior instead of backporting a fix. A fully native dispatcher rejects a
   missing translation rather than silently inventing behavior.
4. **Check against each original separately.**
   `Host::new` rejects a ROM/registry revision mismatch. Check mode uses the
   selected revision's addresses, original ABI and reviewed execution class to
   compare translated execution against that ROM. Translation promotion requires
   exercised, zero-mismatch evidence in **both** revisions; an available body, a
   source match or one revision's passing run does not satisfy that requirement.
   This document describes the mechanism and source differences, not a claim
   that all 648 Rust functions have passed those proofs.

For a local smoke comparison with privately supplied cartridges:

```sh
cargo run --locked --release -p tje-host -- \
  --rom /private/rev02.bin --mode check --frames 10000
cargo run --locked --release -p tje-host -- \
  --rom /private/rev00.bin --mode check --frames 10000
```

These commands only exercise reached/registered entries; inspect unexercised
counts and failures rather than treating exit alone as complete coverage. For
repeatable controller-driven coverage, use each revision's own sessions with
`tje-session replay --mode check`, as described in [SESSIONS.md](SESSIONS.md).
A session whose revision does not match the imported ROM is rejected. Do not
expect a REV 00 trace to equal a REV 02 trace across the documented changes:
compare each port run with **its own original**, then separately establish the
expected revision difference. [PORTING.md](PORTING.md) documents the promotion and
execution-class proof contracts.

[g-revisions]: https://github.com/mschienbein/toejam-earl-decomp/blob/main/docs/REVISIONS.md
[g-data]: https://github.com/mschienbein/toejam-earl-decomp/blob/main/docs/DATA.md
[map-idle]: https://github.com/mschienbein/toejam-earl-decomp/blob/main/src/map/map_drawing_and_controller.c#L510-L513
[inventory-idle]: https://github.com/mschienbein/toejam-earl-decomp/blob/main/src/ui/inventory.c#L220-L222
[terrain-mask]: https://github.com/mschienbein/toejam-earl-decomp/blob/main/src/player/entity_motion_terrain.c#L54-L61
[sleep-guard]: https://github.com/mschienbein/toejam-earl-decomp/blob/main/src/player/sleep.c#L102-L107
[skates-guard]: https://github.com/mschienbein/toejam-earl-decomp/blob/main/src/presents/present_dispatch.c#L144-L153
[present-pose]: https://github.com/mschienbein/toejam-earl-decomp/blob/main/src/presents/present_dispatch.c#L179-L183
[land-toejam]: https://github.com/mschienbein/toejam-earl-decomp/blob/main/src/map/land_processing.c#L542-L546
[land-earl-shared]: https://github.com/mschienbein/toejam-earl-decomp/blob/main/src/map/land_processing.c#L563-L567
[land-earl-split]: https://github.com/mschienbein/toejam-earl-decomp/blob/main/src/map/land_processing.c#L578-L582
[title-time]: https://github.com/mschienbein/toejam-earl-decomp/blob/main/src/cutscenes/title_and_transition.c#L282-L314
[sega-time]: https://github.com/mschienbein/toejam-earl-decomp/blob/main/src/cutscenes/sega_logo.c#L58-L63
