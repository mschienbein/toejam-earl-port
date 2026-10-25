# Jam Out in normal play

Status: initial two-player implementation, opt-in and default off. The `tje-jam-out` crate owns the bounded deterministic gesture/audio sidecar and enhanced original-call hooks. Native and browser settings expose the extra; HD art and four-player gameplay remain excluded.

`G` is the [matching decompilation](https://github.com/mschienbein/toejam-earl-decomp), researched at commit `57fc83fc54781af8e992bbc3b32afe788c1dbd01`; `G/...` paths are relative to it, and `P/...` paths are relative to this repository. REV 02 addresses come from `G/config/rev02/names.json`.

## Original evidence

| Ref | Source and finding |
| --- | --- |
| J1 | [G `src/jamout/jamout.c:29–145`](../../../toejam-earl-decomp/src/jamout/jamout.c#L29): `StartJamOut` (`0x03A7A4`) replaces `gVBlankFunc`, uploads different graphics/palettes, changes scroll/layout, allocates dancers and uses a blocking scene loop. It does **not** run the world/player loop. It is not a safe overworld entry point. |
| J2 | [G `jamout.c:147–220`](../../../toejam-earl-decomp/src/jamout/jamout.c#L147): `JamOutVBlank` (`0x03AC1C`) services a song change or the sequencer, increments the frame counter, alternates controller polling/routes, and forwards a changed **whole input byte**, not just rising button edges. ToeJam's route requests PCM; Earl's requests FM templates. Solo Earl swaps audio routes, not animation ownership. |
| J3 | [G `jamout.c:222–307`](../../../toejam-earl-decomp/src/jamout/jamout.c#L222): random-sample mode polls both pads, selects among 42 PCM pointers and separately cycles songs/template banks. Six sample indices are mutable state. |
| J4 | [G `jamout.c:83–128,309–349,572–606`](../../../toejam-earl-decomp/src/jamout/jamout.c#L83): A/B choose alternating drum sequences, C selects a locked clap, direction/Start a locked vocal/scratch pose; periods are 6/6/3/2. Idle-transition/idle-loop completion unlocks. `JamOutSetToejamSequence`/`JamOutSetEarlSequence` (`0x03B836`/`0x03B880`) operate on dedicated lead handles, not the ordinary player actors. |
| J5 | [G `jamout.c:441–503`](../../../toejam-earl-decomp/src/jamout/jamout.c#L441), [G `include/assets_jam.h:6–18`](../../../toejam-earl-decomp/include/assets_jam.h#L6): nine copies of each character; ToeJam reserves 16 tiles/one piece, Earl 19 tiles/two pieces; followers share lead tiles. Frames and sequences must be resolved from the user's ROM. The overworld does not need eighteen extra objects. |
| J6 | [G `include/ram_jam.h:24–53`](../../../toejam-earl-decomp/include/ram_jam.h#L24): layout/scroll words, two nine-handle arrays, sequence/frame/lock/input words, song/bank cycle and sample indices. REV 02 handle arrays start at `0xFFE41A`/`0xFFE42C`, alternate-bank flag at `0xFFE476`; these are existing scene state, not spare RAM. |
| J7 | [G `src/core/startup_and_main_loop.c:175–254,321–365,677–705`](../../../toejam-earl-decomp/src/core/startup_and_main_loop.c#L175): ordinary VBlank owns graphics, music and alternating physical-pad reads. Input dispatch uses B for inventory, C for the blocking map and Start for pause. Player input/state runs on one of the alternating entity phases, not every VBlank. Installing either Jam VBlank would bypass required world services. |
| J8 | [G `docs/subsystems/audio.md:38–76,94–117,147–184`](../../../toejam-earl-decomp/docs/subsystems/audio.md#L38), `src/audio/{play_sound,play_sound_effect,request_template_effect}.c` and `src/asm/sound*.s`: six hardware FM owners, auxiliary effect records, one pending PCM pointer, one pending PSG request and shared priority/cooldown gates. Later accepted requests can replace pending effects. A song bank change consumes that service slot; do not call the sequencer twice. |
| J9 | [G `include/entities.h:7–24,60–107`](../../../toejam-earl-decomp/include/entities.h#L7), [G `include/ram_player.h:22–79`](../../../toejam-earl-decomp/include/ram_player.h#L22): two player actors in a shared forty-by-128-byte object pool; ordinary rendering uses 26 records, inventories use 26–39, and gameplay has twenty dynamically allocated records 6–25. Animation group bits affect gameplay, not just appearance. |
| J10 | [G `include/ram_sprites.h:6–43,56–79`](../../../toejam-earl-decomp/include/ram_sprites.h#L6), [G `src/asm/vdp.s:6–49,150–166`](../../../toejam-earl-decomp/src/asm/vdp.s#L6): 80 hardware entries, 54 allocatable viewport slots, 80 groups of eight sprite tiles beginning at tile 640, a nominal 112-tile frame budget, 64 KiB VRAM and four sixteen-color palette slots. The 8192-byte upload span is inferred and is **not** a checked allocator capacity. |

The H40 hardware model additionally limits a scanline to twenty sprite entries and forty sprite tiles (320 fetched sprite pixels): [P vendored `vdp/registers.rs:72–104`](vendor/jgenesis/backend/genesis-components/src/vdp/registers.rs#L72). That is hardware-model evidence, not a new fact attributed to G. Work RAM is 64 KiB, cleared by [G `src/asm/interrupts.s:10–25`](../../../toejam-earl-decomp/src/asm/interrupts.s#L10).

## Player-facing behavior

- One master setting, **Jam Out in play**, defaults off. Enabling exposes poses, instruments and a backing-music choice; the normal soundtrack remains selected until the player chooses a Jam backing track. Performance sounds and backing music can each be muted without disabling the feature.
- A remappable `JamModifier` (default unused shoulder; conflict-free V / right Shift on native and V / N in the browser) temporarily routes A/B/C to drum/snare/clap, direction-plus-action to vocals, and A+B to scratch. Start always remains pause. This is an enhanced action channel, not a ninth bit inserted into the original input byte.
- For a three-button-only device, an explicitly enabled B+C chord arms Jam after ten **input-phase updates**; while arming, B/C are withheld, release cancels, and A triggers the selected gesture. The UI explains the delay and lost immediate B/C access. A modern binding is the default; no existing mapping changes when the feature is off.
- Dancing is stationary. It gives no damage, healing, distraction, invulnerability, collision change, item pickup extension or score. Hurt, terrain fall, death, elevator, sleep, Hula, active-power motion or opening a menu immediately wins over the pose. Unsafe ground/air/water states cannot start a dance.
- Sound can be requested while a pose is unavailable, if explicitly enabled; it still obeys the sound policy below. Dances are not the existing health-sharing clap and must not call that mechanic.
- In four-player mode, logical seats 0/2 use ToeJam animation families and 1/3 Earl families by default; physical controller order never determines audio routing. Seat order breaks simultaneous-request ties.

These are deliberate enhanced policies, not claims about the original Jam scene.

## Integration with EnhancedMode

EnhancedMode's coordinated contract is a non-default `enhanced-mode` feature, a separate `EnhancedProfile(name, &'static [Translation], &'static [FramedTranslation])`, and existing `Context` callback signatures. Selected nested `call_at` targets reenter that registry; unselected targets execute compiled originals. Atomic/Functional callbacks have a synthetic RTS charge, **not** original cycle equivalence or source-proof authority. The existing opt-in `NativeCompiledOriginalCallees` profile ([P `docs/ENGINE.md:437–506`](docs/ENGINE.md#L437)) is the dispatch/physical-frame precedent, not permission to label enhanced behavior faithful.

The resumable driver now provides typed, session-owned `Sidecar` storage, full enhanced controller inputs and scheduler-driven `CallOriginal`/`ContinueOriginal` actions. Jam's immutable initial configuration participates in the snapshot identity; mutable settings, release latches, pose clocks, pending requests and cooldowns are restored atomically with the machine. No Jam state is stored in guessed free 68000 RAM.

The initial implementation keeps the original player actors and all their gameplay animation/motion fields intact. `GetPlayerInput` runs its compiled original before consuming explicit enhanced gestures. `ProcessPlayers` and the graphics-queue phase run their complete original bodies before Jam eligibility/animation work. The host presenter resolves the selected Jam frame from the imported cartridge without a new VDP allocation.

The modifier is a separate enhanced action (the unused controller `Mode` field in the enhanced input envelope), not a ninth bit in a three-button wire byte. Conflict-free default keyboard bindings are V for player one and right Shift (native) / N (browser) for player two: J is already an ordinary player-two button in both frontends. Existing keyboard/gamepad mappings are never silently changed. Modifier bindings are remappable; conflicting bindings are rejected before activation.

### Override boundaries

| Original selector | Enhanced responsibility |
| --- | --- |
| `GetPlayerInput` (`0x000938`) / `ProcessInput` (`0x000A6E`) | Preserve the original readable input filtering/dispatch for ordinary actions; consume only explicitly mapped enhanced gestures before inventory/map dispatch. Return ordinary actions through the same phase. A replacement must invoke a factored readable helper, **not** recursively call its own selected FunctionId. |
| `ProcessPlayers` (`0x0111E8`) | Preserve the ordered original update chain, then derive dance eligibility and advance authoritative gesture state once at this phase. Actual movement, hurt and falls stay original. Queue at most one pending request per actor/input phase. |
| `SetSprite` (`0x00CE82`) and sprite-frame presentation boundary (`PrepareEntitySpriteFrame` / ExtendedView equivalent) | Keep canonical gameplay sprite/group state intact. Clear a Jam presentation override on a gameplay state change; the enhanced draw packet can use Jam frames without passing Jam animation motion to player physics. Do not install Jam sprite states into the ordinary state/group mask. |
| `PlaySound`, `PlaySoundEffect`, `AudioRequestChannelTemplateEffect` request-admission boundary | Preserve normal game effects and compare with queued Jam requests under one deterministic arbiter. Do not directly drive the YM/Z80 from a frontend callback. Music service remains the existing original cadence. |

Do **not** select `StartJamOut`, either `JamOut*VBlank`, the layout routines, `JamOutCreateCharacterCopies` or `JamOutSynchronizeEntityCopies` for this extra. Their scene/handle/global-scroll assumptions are not reusable world behavior. Reuse their ROM resource identities and readable selection logic instead.

### State and lifetime

Proposed bounded state (fixed-capacity, allocated at session start):

- Session: ruleset/profile version, enable flags, selected backing-song logical ID, normal song to resume, current/pending bank transition, request sequence counter and separate Jam RNG seeded from session identity.
- Per active seat, at most four: modifier/chord latch, previous enhanced actions, selected gesture, logical sequence/frame/timer, lock, pose eligibility, and one pending instrument request. Store logical ROM resource IDs, never serialized process pointers.
- Derivative decoded tiles, recolored palettes and GPU handles are renderer caches, not authoritative state. Dance completion uses engine phase ticks, never redraw count or host wall time.
- Budget: less than 2 KiB authoritative Jam state; decode/cache only referenced frames. Original two gameplay actors already reserve forty tiles each; the smaller Jam reservations are evidence of feasibility, **not** proof that every existing allocation combination or upload path is safe. Prefer host rendering over extra VDP allocations.
- Entry/exit during a session is a recorded configuration event at a safe phase. Exiting clears gestures/requests and requests the appropriate normal song; it does not reset all sound RAM or restore an old device snapshot over ongoing gameplay.

## Audio and palette policy

Original six-channel FM ownership and the single PCM stream cannot provide four simultaneous independent performers. Initial design intentionally preserves that limit: normal gameplay sounds outrank Jam; PCM Jam requests are rate-limited to one accepted request per six audio service ticks; same-tick requests use increasing seat then sequence order and drop the rest, rather than accumulating latency. FM requests use one bounded auxiliary-effect request per service slot. Cooldowns and pending/drop state are snapshot state. Exact original Jam priority 90 is evidence, not a mandate to preempt hurt/ship/elevator sounds in this mode.

A backing-music change goes through the existing song/template selection and only during the normal music service slot. Pause, boombox and cutscene ownership suspend user backing selection; on release the arbiter reapplies the chosen setting. Do not repeatedly force `gCurrentMusicSelection` mismatches. No new sample mixer, new music assets or arbitrary audio-file loading is needed.

Character variants use runtime palette transforms of the ROM's decoded player palette, retaining transparent index, palette-index semantics and readability. Never reserve four new global CRAM banks: world/HUD already use the four slots. The host renderer supplies per-actor palette bindings; two-player VDP-only rendering may keep original character colors rather than corrupt scenery. No palette words or decoded art are checked into P or share codes.

## Balance levers and risks

- Pose interruptibility and zero gameplay benefit are invariants; audiovisual volume, sound cooldown, gesture duration and backing-song choice are configurable levers.
- Risk: input consumption opens map/inventory on chord release. Maintain separate original and enhanced release latches, including focus loss/reconnect.
- Risk: Jam frames reaching physics change terrain travel. Keep pose animation independent of `object_state.process`, movement frame multipliers and group masks; assert this in tests.
- Risk: request preemption hides essential cues; test mixed PCM/PSG/FM arbitration and never change original admission with the master toggle off.
- Risk: a zero-time selected callback changes the faithful engine's elapsed clocks. All enabled runs are enhanced and have separate replay identities; no enabled-run fidelity claim.

## Test plan and phase sizes

Estimates are design judgments [INFERENCE], single-implementer engineering days, after EnhancedMode/sidecar/render prerequisites; not measured throughput or release dates.

| Phase | Scope / exit condition | Estimate |
| --- | --- | --- |
| J-A | Typed Jam state, input/chord arbitration, snapshot identity, two-player pose state machine without gameplay effects | 2–3 days |
| J-B | Runtime ROM frame resolution, independent presentation clock, interruption and two/four-seat palette support | 2–4 days |
| J-C | Single-cadence sound/backing arbitration, native/browser fixtures and user settings | 3–5 days |
| J-D | Deterministic replay, mixed-mode regression and usability tuning | 2–3 days |

Tests required before shipping:

1. With the toggle absent/off and with an installed-but-disabled enhanced profile, replay the complete existing native/browser engine session corpus for REV 00 and REV 02; compare existing frame **and audio** hashes, not just final RAM. A default frontend config must not construct/select the Jam profile.
2. Hold/change/release every action combination through pause, map, inventory, reconnect and chord timeout; compare edge traces and ensure no double-dispatch. Repeated render calls cannot consume input or advance music.
3. For every pose/facing and both character families, compare resolved animation/frame choice against the dedicated Jam scene at equivalent sequence progress; verify pixel transparency, elevation/priority, sprite-capacity exhaustion and interrupts mid-pose. This is a resource/sequence comparison, not a claim the two modes have identical world frames.
4. Damage, push, sleep, Hula, death, fall, elevator and powerup cancel dance without immunity or saved-velocity restoration. A pose-only/sounds-muted run keeps world positions, health, score, inventories and original RNG unchanged from equivalent consumed normal input.
5. Simultaneous four-seat requests, boombox/cutscene changes, FM ownership, PCM cooldown and pause resume have repeatable request order. Capture/hash enhanced audio separately from faithful baselines.
6. Snapshot/restore then replay identical inputs reproduces Jam state, pixels and audio; renderer cache destruction/rebuild gives the same result. Netplay cannot enable this until the enhanced envelope is negotiated.

The initial implementation uses the existing sequencer and bank-loader cadence, not either Jam scene VBlank. The audio arbiter runs once per original service frame, drops losing requests and retains ordinary PCM/PSG/FM ownership. ToeJam uses PCM resources, Earl uses the original FM effect identities. Backing choice 0 preserves the normal soundtrack; choices 1–6 remap existing template loads to Jam banks. A bank transition consumes a normal music-service slot instead of calling the sequencer twice. Normal selection is resumed when the extra/backing choice is disabled.

Authored pause/map/menu/elevator/ship/ending boundaries suspend Jam through before-only hooks; ordinary world input resumes ownership without reviving an old pose. The first frontend enable explicitly restarts the cartridge at power-on to activate the enhanced driver without discarding a suspended physical frame. Later settings changes are applied and counted at deterministic input/audio phases without restarting. Disabling clears the extra but never relabels the already-enhanced trajectory as faithful. A new default/off session never constructs the Jam profile. Controller-only recording/replay and netplay cannot opt into these unnegotiated enhanced inputs.

Unit coverage includes every original input byte with the extra off, chord timing, independent release latches, pause priority, unsafe-state cancellation, runtime sequence stepping, draw-count independence, muted poses, original-audio priority, cooldown/drop order and sidecar roundtrips. The opt-in `tje-jam-out/native-tests` private-ROM test drives both characters through compiled original gameplay in both revisions, then compares snapshot-replayed frame/audio/presentation hashes. Private execution evidence and the exact gate results are kept in `scratch/JamOutPlay/report.md`, not published cartridge-derived captures.

Four-seat gameplay/palettes, enhanced input-session file recording and negotiated enhanced netplay are outside this initial two-player phase; the existing default faithful paths remain unchanged.
