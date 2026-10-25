# Original bugs, exploits, and compatibility quirks

**Faithful is the default. Nothing in this catalogue authorizes changing faithful
code or the shipped original-engine behaviour.** This is the consolidated list
for the explicitly selected **Fixed v1** profile on the extras branch. Source
evidence and implementation status are separate: a synthetic trigger is not a
claim that every adverse state occurred in a retail playthrough.

## Summary

There are **97 catalogue entries** (families, not individual callsites): **48
source defects/failure paths**, **8 REV 00 behaviours repaired by REV 02**, **13
additional compatibility hazards/quirks**, **13 public player-facing exploits or
secrets**, and **15 invisible/proof-only quirks**.

| Primary impact | Entries | Qualification |
| --- | ---: | --- |
| None / invisible | 15 | Includes discarded EXT.W high lanes, ABI spellings, and the ISR-evidence qualification; not gameplay-fix candidates. |
| Cosmetic | 21 | Includes audio, animation, rendering, and map-presentation differences. |
| Gameplay-affecting | 50 | Includes intentional exploits and synthetic invalid-state hazards, not just defects reachable in ordinary play. |
| Softlock | 7 | Conditional nontermination or inability to progress; public disconnected-island report included. |
| Crash | 4 | **Potential, source-derived [INFERENCE]**, not four reproduced natural crashes. Unsafe pointer, frame-overflow, and decoder-overrun cases are identified below. |
| **Total** | **97** | One primary impact per entry; duplicates and private-port defects are not counted twice. |

**Recommended first fixed-profile set:**

1. Stop rank recomputation at the last real rank (**B017**); make the immediate
   health-bar constructor respect non-progressing animator states (**B018**).
2. Make placement terminate and propagate failure instead of consuming residual
   coordinates (**B004–B006**); do not silently place at `(0,0)`. A deterministic
   generation retry/explicit no-placement outcome belongs to the fixed profile.
3. Check both Earthling-bank exhaustion paths (**B008**), and stop freeing an
   unassigned shared tile (**B011**).
4. Pass the actual player to promotion speech (**B014**) and supply the omitted
   dialogue/placement arguments at their callers (**B007**, **B015**).
5. Correct the two-axis skate clamp and health-sharing cap (**B001–B002**).
6. Offer the **REV 02 repairs R001–R008** when a REV 00 cartridge is selected;
   they must not turn faithful REV 00 into REV 02 implicitly.

Keep the pause secret, Total Bummer rescue, dropped-present Randomizer strategy,
Santa farming, and deliberate traversal/combat affordances available (**P001–P013**).
Do **not** spend this first pass “fixing” EXT.W locals, harmless return-view
spellings, saved ISR registers, or normal 68000 arithmetic. Disconnected-island
prevention (**P004**) is valuable but changes level generation and needs its own
policy and seed/recording domain rather than being bundled into a small safety fix.

## Implementation status — Fixed v1

The recommended first set is implemented in `crates/tje-extras/src/fixed/`;
its versioned manifest and immutable mask are shared by native/browser settings.
See [selection and outcome policies](ENHANCED_MODE.md#fixed-v1). No first-set
entry is deferred to the shared simulation cutover. “Not yet” means absent from
this profile, not authorization to change a kept exploit or proof-only quirk.

| Entry | Status |
| --- | --- |
| B001 | Fixed in profile v1 |
| B002 | Fixed in profile v1 |
| B003 | Not yet |
| B004 | Fixed in profile v1 (linked B004–B006) |
| B005 | Fixed in profile v1 (explicit no-placement error) |
| B006 | Fixed in profile v1 (absent item / unchanged telephone) |
| B007 | Fixed in profile v1 |
| B008 | Fixed in profile v1 |
| B009 | Not yet |
| B010 | Not yet |
| B011 | Fixed in profile v1 |
| B012 | Not yet |
| B013 | Not yet |
| B014 | Fixed in profile v1 |
| B015 | Fixed in profile v1 |
| B016 | Not yet |
| B017 | Fixed in profile v1 |
| B018 | Fixed in profile v1 |
| B019 | Not yet |
| B020 | Not yet |
| B021 | Not yet |
| B022 | Not yet |
| B023 | Not yet |
| B024 | Not yet |
| B025 | Not yet |
| B026 | Not yet |
| B027 | Not yet |
| B028 | Not yet |
| B029 | Not yet |
| B030 | Not yet |
| B031 | Not yet |
| B032 | Not yet |
| B033 | Not yet |
| B034 | Not yet |
| B035 | Not yet |
| B036 | Not yet |
| B037 | Not yet |
| B038 | Not yet |
| B039 | Not yet |
| B040 | Not yet |
| B041 | Not yet |
| B042 | Not yet |
| B043 | Not yet |
| B044 | Not yet |
| B045 | Not yet |
| B046 | Not yet |
| B047 | Not yet |
| B048 | Not yet |
| R001 | Fixed in profile v1, REV 00 only |
| R002 | Fixed in profile v1, REV 00 only |
| R003 | Fixed in profile v1, REV 00 only |
| R004 | Fixed in profile v1, REV 00 only |
| R005 | Fixed in profile v1, REV 00 only |
| R006 | Fixed in profile v1, REV 00 only (entry-reset policy) |
| R007 | Fixed in profile v1, REV 00 only (entry-reset policy) |
| R008 | Fixed in profile v1, REV 00 only |
| H001 | Not yet |
| H002 | Not yet |
| H003 | Not yet |
| H004 | Not yet |
| H005 | Not yet |
| H006 | Not yet |
| H007 | Not yet |
| H008 | Not yet |
| H009 | Not yet (keep) |
| H010 | Not yet |
| H011 | Not yet |
| H012 | Not yet |
| H013 | Not yet (keep) |
| P001 | Not yet (keep) |
| P002 | Not yet (keep) |
| P003 | Not yet (keep) |
| P004 | Not yet (separate generation policy) |
| P005 | Not yet (keep) |
| P006 | Not yet (keep) |
| P007 | Not yet (keep) |
| P008 | Not yet (keep) |
| P009 | Not yet (keep) |
| P010 | Not yet (keep) |
| P011 | Not yet (keep) |
| P012 | Not yet (keep) |
| P013 | Not yet (keep) |
| Q001 | Not yet (keep) |
| Q002 | Not yet (keep) |
| Q003 | Not yet (keep) |
| Q004 | Not yet (keep) |
| Q005 | Not yet (keep) |
| Q006 | Not yet (keep) |
| Q007 | Not yet (keep) |
| Q008 | Not yet (keep) |
| Q009 | Not yet (keep) |
| Q010 | Not yet (keep) |
| Q011 | Not yet (keep) |
| Q012 | Not yet (keep) |
| Q013 | Not yet (keep) |
| Q014 | Not yet (keep) |
| Q015 | Not yet (keep) |


## Reading the evidence and proposed fixes

### Source identities and notation

* **G** is [the decompilation](https://github.com/mschienbein/toejam-earl-decomp),
  inspected at `57fc83fc54781af8e992bbc3b32afe788c1dbd01`.
  `G:src/...:line` is a file and inclusive line/range in that checkout.
  `G:asm/c_foo.s:line` abbreviates
  `G/build/{rev00,rev02}/asm/c_foo.s:line` unless a revision is written explicitly.
  Those private build listings are cited, never copied into this document.
* **R** means `P/crates/tje-game/src/`; thus `R:player/rocket.rs:149–151`
  is an exact Rust file/line citation. Rust anchors are at port baseline
  **`888fab32`**, the parent of this documentation branch. Function names are
  stable search keys when subsequent integration moves line numbers.
* **L** means an untracked/private lane report below `P/scratch/`. These are
  supplemental provenance, not public assets or new verification performed by
  this catalogue. Report line numbers refer to the inspected 2026-10-25 copy;
  named commits disambiguate later report updates. A report's later annotation
  is not assumed to have landed in the baseline Rust file.
* **00+02** means the affected source path exists in **both REV 00 and REV 02**.
  It does not claim a public FAQ author identified their cartridge revision.
  The eight changed functions are independently enumerated in
  [G's revision comparison](https://github.com/mschienbein/toejam-earl-decomp/blob/57fc83fc54781af8e992bbc3b32afe788c1dbd01/docs/REVISIONS.md#L16-L38)
  (`G:docs/REVISIONS.md:16–38`). All other paired logic is unchanged there;
  data content is unchanged apart from header/fill/address relocation (:40–47).
* **Normal** means the trigger uses ordinary controls/game states, not that this
  catalogue measured its frequency. **Boundary** means a malformed/out-of-range
  input or exhausted resource state whose natural reachability is unestablished.
  **Reported** distinguishes public or lane observations from this document's
  source inspection. An uncertain historical intention, causal match, or adverse
  consequence is explicitly **[INFERENCE]**.

### Enhanced-profile and recording contract

Every fix sketch below means **an opt-in enhanced override**, never an edit to
G or the faithful `tje-game` implementation. The existing extras design provides
`EnhancedProfile` selections, `ResumableTranslation`, serializable continuations,
and profile-owned sidecars; see `L:EnhancedMode2/report.md:23–61,63–111` (handoff
`3aa9576`). An RTS function can be overridden directly; interactive/waiting roots
need the resumable form and real original calls/device progress. RTE wrappers
must not be treated as ordinary RTS overrides. A before-only observer or a
returning C-handler override is the appropriate boundary if such a change is
actually wanted. Dispositions remain design sketches; the implementation-status
table above identifies the subset shipped in Fixed v1.

Each disposition includes a compatibility tag:

* **S** — may change simulation RAM, RNG use, control flow, placement, timing,
  or subsequent events. **Can break faithful replays and desynchronize mixed
  netplay.** Use fixed-profile-only recordings and exact matching configuration.
* **V** — intended as a visual/audio-only repair, but can still change saved
  state, allocation, timing, and hashes. **Can invalidate replays/mixed netplay**;
  do not assume presentation code is outside the deterministic machine.
* **N** — no ordinary visible benefit established. Leave faithful behaviour
  alone; a hypothetical normalization still changes machine/proof state and must
  have its own fixed-profile recordings, never mixed-profile netplay.

These tags apply to **each row** below. “Fixable: yes” describes technical
feasibility, not a proof of safety or an instruction to enable it. Recommendations
are **fix**, **optional**, or **keep** (including iconic intentional behaviours).
No enhanced network-session compatibility is claimed: that handoff explicitly
keeps existing connection admission faithful-only (:108–111). Fixed profiles must
bind revision, enabled fixes and immutable configuration into their recording,
save and peer-admission identity before such sessions are supported.

## A. Source defects and failure paths

These change a result or can fail under their stated preconditions. A boundary
entry is not evidence of a common retail gameplay bug.

### Player motion, items, and placement

| ID / title | Where it lives | Description, trigger, and normal-play reachability | Primary impact / revisions | Fixed-profile disposition |
| --- | --- | --- | --- | --- |
| **B001 — Rocket-skate clamp skips Y** | G:src/player/rocket_motion.c:157–169; R:player/rocket.rs:149–151 | The X-speed clamp is followed by an `else if` Y clamp. If both components exceed their limits, only X is clamped. Ordinary skate/motion state can enter the routine; a natural simultaneous-over-limit occurrence is not established here. | Gameplay-affecting; 00+02. | Fixable: yes, override `PlayerApplyRocketSkatesInput` with independent axis clamps. **fix; S**. |
| **B002 — High-five health cap is not a cap** | G:src/player/edge_hop_and_clap.c:78–92; R:player/cooperation.rs:62–66 | On completion, an average above one player's maximum is assigned unchanged to that player, while the partner subtraction uses the maximum. Normal co-op high-fives with unequal capacities/health can trigger it; later HUD clamping does not make the assignment correct. | Gameplay-affecting; 00+02. | Fixable: yes, override the completion calculation, cap the recipient and conserve/distribute the intended remaining health. **fix; S**. |
| **B003 — Full-health food disappears** | G:src/presents/item_updates.c:198–207; R:presents/items.rs:154–158 | The positive-food/full-health branch leaves `collected` set, so it removes food despite providing no health. Reachable by normally walking over food with a full bar. | Gameplay-affecting; 00+02. | Fixable: yes, override collection so the no-benefit path leaves the item and does not publish a pickup. **optional; S** (some players expect auto-consumption). |
| **B004 — Wholly rejected placement maps never exhaust attempts** | G:src/map/terrain_placement.c:110–119,157–159; G:src/presents/level_item_setup.c:174–183; R:map/placement.rs:72–76; R:presents/placement_frames.rs:54–90; R:presents/mod.rs:349–353 | Rejected marked/excluded metatiles decrement the attempt counter. A map where every random choice is rejected never reaches the exhaustive scan. This is genuine source nontermination, not a hardware wait. An all-rejected ordinary generated map has not been demonstrated; authored/adverse maps satisfy the trigger. | Softlock; 00+02. | Fixable: yes, override both placement searches, count every random draw and then perform a finite scan with an explicit failure result handled by profile-owned callers. **fix; S**. |
| **B005 — Exhausted player placement consumes old coordinates** | G:src/map/terrain_placement.c:137–169; G:asm/c_terrain_placement.s:650–673; R:map/placement.rs:72–76; L:MapLevels/report.md:31 | After a complete failed scan, `set_position` still writes X/Y. They may be old successful outputs or genuinely unwritten own-frame words; neither is a valid placement result. A lane fixture reports a consumed dependency; normal generation reachability is not established. Distinct from B004: this path actually finishes scanning. | Gameplay-affecting; 00+02. | Fixable: yes, override search and its placement consumer to require success; deterministically retry generation or retain a known valid placement, not an invented zero. **fix; S**. |
| **B006 — Item/telephone callers consume unwritten failed-search outputs** | G:src/presents/level_item_setup.c:152–221,238–240,354–362; G:src/presents/telephone_open.c:57–65; R:presents/placement_frames.rs:9–220,223–551,557–668; L:MixedWordSweep/report.md:71–83 | The void placement helper can return without writing either output; large-item, item-bank and telephone callers continue using local coordinates. A prior iteration may hide the first-read defect. Natural exhausted-first-search reachability remains unproved, not “safe because the callee usually writes.” | Gameplay-affecting; 00+02. | Fixable: yes, profile overrides give the search a checked success outcome and migrate all three caller families together. **fix; S**. |
| **B007 — Small-item recovery omits the placement flag** | G:src/presents/item_updates.c:75–78; R:presents/item_frames.rs:10–87; R:presents/placement_frames.rs:43–50; G:docs/subsystems/presents.md:306–307; L:Presents/report.md:180–183 | Allocation failure calls the four-argument placement function with only three arguments. Its `large` word comes from the saved incoming A2 slot; the visibility caller leaves A2 pointing at the item. Ordinary resource pressure can reach recovery; no universal zero/default flag exists. | Gameplay-affecting; 00+02. | Fixable: yes, override `EntityCreateLevelItem` and pass an explicit small-item flag in the real fourth slot; do not globally mask the callee's input. **fix; S**. |
| **B008 — Earthling present scans past a full bank** | G:src/earthlings/spawning_and_banks.c:505–524; R:earthlings/open_present.rs:5–9; L:Earthlings/report.md:242–245; R:earthlings/live_fixtures.rs:696–699 | Exhausting the unused-record scan dereferences the one-past record; exhausting the inactive-record fallback still writes through it. Bank zero's next record is bank one's first. Opening an Earthling present is normal; both banks exhausted is a pressure/boundary condition, explicitly exercised by authored fixtures, not a measured natural frequency. | Gameplay-affecting; 00+02. | Fixable: yes, override `OpenEarthlingPresent`, check both scans and decline/defer spawning when no owned record is available. **fix; S**. |
| **B009 — Negative terrain coordinates index before the map** | G:src/map/terrain_at_position.c:13–43; R:map/terrain.rs:4–6; L:ReviewA/report.md:100 | Zero and upper bounds are rejected, but negative map coordinates are not. The original reads neighbouring emulated storage. Whether a normal caller supplies such coordinates is explicitly unknown in G; negative-coordinate fixtures are not proof of normal reachability. | Gameplay-affecting; 00+02. | Fixable: yes, an enhanced terrain-query override rejects lower as well as upper bounds using a documented level-specific water/void result. **optional; S**. |
| **B010 — A path cell can select a shape from stale D6** | G:src/map/land_processing.c:146–200; G:asm/c_land_processing.s:463–540; R:map/consume.rs:4–7; R:map/land.rs:460–466 | If no pending/adjacent direction supplies an edge, `shape` was never assigned and uses entry D6.B. The parent normally carries its current row there. The marked-path updater is ordinary gameplay; feasibility of the no-edge configuration in natural generation is not established. | Gameplay-affecting; 00+02. | Fixable: yes, override the consumer with a defined no-edge outcome and make the updater honor it rather than selecting a random-looking shape. **optional; S**. |

### Sprites, speech, and resource accounting

| ID / title | Where it lives | Description, trigger, and normal-play reachability | Primary impact / revisions | Fixed-profile disposition |
| --- | --- | --- | --- | --- |
| **B011 — Shared-tile allocation failure frees stale D4** | G:src/sprites/sprite_slots.c:162–190; G:asm/c_sprite_slots.s:390–456; R:sprites/lifecycle.rs:52–55; R:camera/animations.rs:163–166; R:speech/dialogue.rs:140–144; L:ReviewHazards/report.md:50 | Shared tiles + no resident viewport + selector failure reaches `FreeSpriteTileRun` without assigning `tile`. Actual entry D4.W is used: examples include a speech slot, animation index, caller Y, or object index. Normal allocation pressure can reach this; never substitute zero in faithful code. | Gameplay-affecting; 00+02. | Fixable: yes, override allocator cleanup so it releases a tile run only if this invocation acquired one. Cover all callers through that single boundary. **fix; S**. |
| **B012 — Map recovery forgets the third allocation** | G:src/map/map_drawing_and_controller.c:297–321; R:map/display.rs:247–249; L:MapScreens/replay-prep.md:11 | After two eight-tile allocation failures, stronger eviction runs but the allocator is never retried. The map sprites proceed with tile base zero. Normal map opening plus resource exhaustion is the trigger; a natural double-failure frequency is unknown. | Cosmetic; 00+02. | Fixable: yes, override map sprite initialization to retry after stronger recovery and fail explicitly if still unavailable. **fix; V**. |
| **B013 — Not-ready sprites are counted only when already over budget** | G:src/sprites/entity_sprite_rendering.c:444–486; R:sprites/updates.rs:283–285; L:ReadableReview/SpritesCore.md:63 | The not-ready branch adds tile count only when the total would exceed 112, the reverse of useful reservation accounting. Ordinary visible not-ready entities reach this branch; downstream animation/motion callbacks make this more than a cosmetic counter. | Gameplay-affecting; 00+02. | Fixable: yes, override the scheduler's reservation predicate with consistent budget accounting and preserve callback ordering for admitted entries. **optional; S**. |
| **B014 — Promotion passes an object pointer as speaker** | G:src/presents/present_effects.c:371–387; R:presents/effects.rs:503–506; R:speech/dialogue.rs:31–34,51–62; L:PresentsB/replay-prep.md:22 | The immediate/no-display promotion fallback passes `&gPlayerInfo[player]` where `GenerateSpeech` expects a player number. Its low word is then used in word-scaled object addressing. Trigger: existing promotion display or failed display allocation, followed by promotion. This is the original **pointer-as-speaker bug**, not an audio-channel/panning defect. | Cosmetic (missing/misattributed speech; further bad-address effects are [INFERENCE]); 00+02. | Fixable: yes, override the fallback caller to pass the explicit player number. **fix; V**. |
| **B015 — Reunion and ship collection omit the speaker** | G:src/speech/speech_and_dialogue.c:297–305; G:src/ship/ship_screen_and_collection.c:605–609; R:speech/dialogue.rs:230–238,319; R:ship/collection.rs:297; R:ship/mod.rs:131,277–278; L:ShipSpeech/replay-prep.md:55,59; P:docs/TRANSLATION.md:509–510 | Three reunion calls and the shared/ToeJam ship-collection call supply no second argument. `StartDialogue` reads the caller's saved entry D2 word. Descriptor-forced speakers can hide the error; descriptor-dependent replies need not. These are normal reunion/collection paths, not an invented “always player zero” interface. | Cosmetic; 00+02. | Fixable: yes, override both callers with explicit initiating speaker/partner policy for every dialogue. **fix; V**. |
| **B016 — Control-remap dialogue only visits ToeJam** | G:src/speech/speech_and_dialogue.c:308–318; R:speech/dialogue.rs:462–464; G:docs/subsystems/speech.md:178–181 | The loop bound is one, so Earl's alternate branch is unreachable. Normal co-op remap/proximity/random conditions can expose the asymmetry. | Cosmetic; 00+02. | Fixable: yes, override ambient dialogue to inspect both players with an explicit priority and RNG-consumption rule. **optional; V**. |
| **B017 — Rank reset loops at 880+ points** | G:src/ui/hud.c:665–676; R:ui/menus/resources.rs:73–76; L:UiHud/replay-prep.md:74 | Recompute starts at the initial rank and has no rank-eight stop; the zero threshold drives endless recomputation when points are at least 880. Points at that scale are attainable, but the reset/resource-rebuild call must also occur: merely reaching 880 during ordinary promotion is not asserted to freeze the game. | Softlock; 00+02. | Fixable: yes, override `PlayerResetRankAndResources` with a last-real-rank bound before threshold lookup. **fix; S**. |
| **B018 — Immediate health construction can never finish** | G:src/ui/hud.c:248–257,327–344; R:ui/hud_all/health.rs:11–15; L:UiHud/replay-prep.md:72; R:ui/hud_all/check/main.rs:370–372 | The constructor repeatedly animates until displayed health reaches positive actual health. The animator returns without progress for absent/ghost/final-level players. Trigger requires the immediate constructor with that inconsistent/non-progressing state; normal valid constructors are not all nonterminating. | Softlock; 00+02. | Fixable: yes, override construction to use the animator's eligibility/target-health policy and finish a defined bar directly for ineligible states. **fix; S**. |
| **B019 — Missing-piece hint has no all-collected exit** | G:src/ship/ship_screen_and_collection.c:650–661; R:ship/collection.rs:356–361; L:ShipSpeech/replay-prep.md:56 | `ShowRandomUncollectedShipPiece` cycles forever if all ten assignments are zero. Normal calls should have an uncollected piece; a late/repeated hint, cheat interaction, or authored state must be distinguished from a proven natural occurrence. | Softlock; 00+02. | Fixable: yes, override the hint root to return a defined no-missing-piece outcome before drawing RNG/searching. **fix; S**. |
| **B020 — Invalid ship-piece name uses stale D3 as a pointer** | G:src/ship/ship_screen_and_collection.c:308–333; R:ship/background.rs:314–318; R:ship/screen.rs:141–144; L:ShipSpeech/replay-prep.md:51 | No default switch assignment exists for a piece outside 0–9, so the string scan follows entry D3. Valid piece collection does not trigger it. Authored retained-RAM-string cases are finite; arbitrary residue can address invalid/odd memory or scan indefinitely. | Crash, potential [INFERENCE]; 00+02. | Fixable: yes, override the name entry to reject invalid piece IDs before touching any string; do not invent a faithful fallback string. **fix; S**. |

### Residual registers, allocation lifetimes, and scene defects

| ID / title | Where it lives | Description, trigger, and normal-play reachability | Primary impact / revisions | Fixed-profile disposition |
| --- | --- | --- | --- | --- |
| **B021 — Invalid departure level returns no value** | G:src/elevator/elevator_scene.c:97–105; R:elevator/management.rs:4–7; G:docs/subsystems/elevator.md:202–208 | An out-of-range ordinary departure request returns from a `char` function without defining its result. The compiled result is the sign-extended low byte left by the player-pointer load, not a portable error code. Normal departure callers are expected to supply valid levels; boundary reachability is unestablished. | Gameplay-affecting; 00+02. | Fixable: yes, an enhanced `SetupElevator` override returns a defined failure and profile-owned consumers check it before indexing. **optional; S**. |
| **B022 — No-rider elevator advance retains D3/A3** | G:src/elevator/elevator_scene.c:652–698; R:elevator/ride.rs:582–587; P:docs/TRANSLATION.md:511,519; L:Elevator/replay-prep.md:18–19,49–51 | Without a matching rider, destination level can remain entry D3.B; the cleanup path can use an unassigned elevator pointer (entry A3) when a callback is present. Step1 may leave the last centered player pointer in D3, not always its old state. Normal correct rides initialize the locals; no-rider/invalid-state paths are boundaries. | Gameplay-affecting; 00+02; unsafe-address escalation is [INFERENCE]. | Fixable: yes, override advance with an explicit rider/destination record and only clean up a ride actually begun. **optional; S**. |
| **B023 — Bouncing Earthling damage queries entry D1's map** | G:src/player/earthling_damage_reactions.c:283–310; G:asm/c_earthling_damage_reactions.s:651–670; R:player/damage.rs:178–181; R:earthlings/animation.rs:142–145 | If neither loaded map level matches the object level, `map_number` is never assigned. The terrain query consumes entry D1.W; its upstream timed-movement caller preserves that residue. Requires an off-bank/stale object-level relation, not every ordinary bounce. | Gameplay-affecting; 00+02. | Fixable: yes, override the damage/terrain decision to resolve an actual loaded bank or defer off-bank terrain processing. **optional; S**. |
| **B024 — No eligible AI target still selects residual deltas/A3** | G:src/earthlings/targeting.c:24–28,71–121; G:asm/c_targeting.s:3–427; R:earthlings/framed.rs:402–406,597–651; R:earthlings/generic.rs:93–95; L:Earthlings/replay-prep.md:47–49; P:docs/TRANSLATION.md:521 | Both distances can stay at the `0x7fff` sentinel while the expanded maximum threshold admits them; the tie chooses Earl and reads unwritten delta words. The no-candidate shark event can also use entry A3 as `target`. Authored producer fixtures link previous `SetEarthlingState` saves to those words; natural-session last-writer attribution is not established. | Gameplay-affecting; 00+02. | Fixable: yes, override targeting to require an eligible candidate before selection/event/steering; no zero-delta “target” substitute. **fix; S**. |
| **B025 — Intro attached copies can dereference stale A3** | G:src/cutscenes/collision_scene_entities.c:10–59; G:asm/c_collision_scene_entities.s:17,81; R:cutscenes/collision_scene_entities.rs:4–8; L:CutscenesA/report.md:91 | If the parent allocation fails but a later attached allocation succeeds, `parent` is uninitialized. IntroPart8's compiled A3 holds a background-draw function address. A static atomic exhausted pool cannot regain a slot between these calls; the lane explicitly did not generate the exceptional interleaving. Do not call it a common intro crash. | Crash, potential [INFERENCE] from an invalid parent dereference; 00+02. | Fixable: yes, override constructor to require a valid parent before making shared copies and roll back owned partial allocations. **fix; S**. |
| **B026 — Stationary composite stores a table, not a frame** | G:src/cutscenes/impact_and_planet_entities.c:77–88; R:cutscenes/impact_and_planet_entities.rs:93–96; L:CutscenesB/report.md:27,169 | `current_frame` receives the frame-table address rather than its first entry. Reachable by the normal scene constructor; the exact on-screen artefact depends on subsequent preparation/use, not a newly observed screenshot here. | Cosmetic; 00+02. | Fixable: yes, override constructor to use the first actual frame descriptor. **optional; V**. |
| **B027 — Ending bounce acceleration starts at residual phase** | G:src/cutscenes/ending.c:505–508; G:asm/c_ending.s:1116–1117,1193–1194; R:cutscenes/ending.rs:809; L:CutscenesA/report.md:92; L:CutscenesA/replay-prep.md:135–140; L:Ending/report.md:40 | The first increment reads the untouched movement-frame word at entry SP−6. On the uninterrupted ending path the preceding descending scene's saved A2.low supplies it. This is a genuinely consumed phase, unlike an EXT-discarded high byte. Normal ending entry reaches the routine; exact phase depends on its real parent frame. | Cosmetic; 00+02. | Fixable: yes, enhanced bouncing-scene state initializes phase to a specified value and uses serializable continuation state. **optional; V**. |
| **B028 — Intro flight variants 2 and 3 cannot occur** | G:src/cutscenes/flying_entities.c:102–116; R:cutscenes/flying_entities.rs:82–85; L:CutscenesB/report.md:169 | Random variant is masked with one, but subsequent branches describe variants two/three and another frame table. Normal intro flights can only select zero/one. | Cosmetic; 00+02. | Fixable: yes, override selection with the intended four-way domain **only if** deliberately restoring those variants; the intended distribution is [INFERENCE]. **optional; V**. |
| **B029 — Decimal formatting mishandles −32768** | G:src/ui/inventory.c:710–733; R:ui/menus/draws.rs:28–31; G:docs/subsystems/ui.md:220 | Negation narrows to a signed short and remains negative; decimal extraction no longer formats the magnitude correctly. Normal scores/bucks are not known to reach this signed input; direct formatting boundary is clear. | Cosmetic; 00+02. | Fixable: yes, override formatting with a wider magnitude calculation before digit extraction. **optional; V**. |
| **B030 — Missing transition shape reuses an old choice word** | G:src/map/level_generation_and_changes.c:292–330; G:asm/c_level_generation_and_changes.s:1518–1566; R:map/alternatives.rs:5–9; L:MixedWordSweep/report.md:65–67 | If a transition descriptor does not contain the current shape, the alternative index is read before a choice store. A later cell may inherit an earlier choice; a first such cell reads entry SP−8 residue. Original descriptor/generation consistency is expected; natural mismatch is unproved. | Gameplay-affecting; 00+02. | Fixable: yes, override alternatives selection to treat a missing shape as a defined generation failure before indexing. **optional; S**. |
| **B031 — Unsupported region kind leaves stop probability unset** | G:src/map/random_regions_and_cleanup.c:293–325,383–395; G:asm/c_random_regions_and_cleanup.s:966–983,1235; R:map/growth.rs:5–9; L:MixedWordSweep/report.md:67 | Only boundary/filled kinds initialize `stop_chance`; unsupported kinds can consume the word at entry SP−18. Original callers use the two supported kinds, so this is an invalid-input boundary, not an asserted natural generator defect. | Gameplay-affecting; 00+02. | Fixable: yes, enhanced generation rejects unsupported kinds at this entry rather than inventing a probability. **optional; S**. |
| **B032 — Jam Out copies inherit an uncreated lead** | G:src/jamout/jamout.c:442–503; G:asm/c_jamout.s:1149–1155,1198–1204; R:jamout/entities.rs:32–38; P:docs/TRANSLATION.md:518; L:MixedWordSweep/report.md:8,65 | If a non-lead copy succeeds after slot zero failed, it consumes an unwritten lead-tile word (entry SP−6) and entry D3.B parent number, or an earlier group's retained lead. `StartJamOut` carries its song in D3. Requires allocation/interleaving conditions; a normal observed lead failure is not claimed. | Cosmetic; 00+02. | Fixable: yes, override construction to create copies only after a valid lead exists, with group-local explicit ownership. **optional; V**. |
| **B033 — Failed scene/indicator allocation writes entity zero** | G:src/cutscenes/rocketship_allocate.c:10–38; G:src/ship/ship_screen_and_collection.c:627–645; R:cutscenes/rocketship_allocate.rs:4–7; R:ship/collection.rs:15; L:ReadableReview/AudioJamShipSpeech.md:102 | These constructors dereference an allocation result without a nonzero guard. Entity zero is ToeJam, not a null object. Resource failure can therefore overwrite an existing player record; ordinary constructor calls are reachable, natural complete exhaustion not measured. | Gameplay-affecting; 00+02. | Fixable: yes, override both constructors to publish/init only successful handles and undo only allocations owned by this attempt. **fix; S**. |
| **B034 — Santa keeps a pointer after relocation releases its object** | G:src/earthlings/mole_santa.c:95–99,150–156,197–199; G:src/earthlings/spawning_and_banks.c:322; R:earthlings/santa.rs:25–28; R:earthlings/state.rs:244; L:ReadableReview/ReviewEarthlings.md:43–51 | `santa` is captured before relocation, then its X/Y and possibly frame are still written after the record relocation frees/changes ownership. Normal off-screen Santa flight reaches relocation. In flat RAM this is stale-slot access, not a host-language dangling reference; an actual reused-slot symptom is not established. | Gameplay-affecting; 00+02. | Fixable: yes, override relocation consumer to stop using the released handle and resume only with a current owned object. **optional; S**. |
| **B035 — Decoy removal can release twice** | G:src/presents/present_effects.c:140–160; R:presents/effects.rs:156–159; L:Presents/report.md:185–186 | Level-change and three-hit checks are independent. If both apply in the same update, the second branch frees the same old handle and reads its coordinates for an effect. Ordinary decoys can change level and accrue hits; coincidence frequency is unmeasured. It is not a demonstrated host double-free crash. | Gameplay-affecting; 00+02. | Fixable: yes, override cleanup to choose one removal reason, capture needed effect coordinates before release, and release once. **fix; S**. |
| **B036 — Retiring a zero terrain-effect handle rewrites ToeJam** | G:src/presents/projectile_effects.c:515–533; R:presents/projectiles.rs:448–467; L:Presents/report.md:186 | The retirement entry accepts active handle zero and then installs the retiring animation on entity zero. Known updater callers guard their active handle (G:568–587), so the ordinary path does not establish a zero-handle call. Boundary/reentrant use remains unguarded. | Gameplay-affecting; 00+02. | Fixable: yes, override retirement to return before animation stores when no owned active effect exists. **optional; S**. |

### Capacity, cleanup, arithmetic, and audio edge failures

| ID / title | Where it lives | Description, trigger, and normal-play reachability | Primary impact / revisions | Fixed-profile disposition |
| --- | --- | --- | --- | --- |
| **B037 — Viewport link arrays overflow after 54 handles** | G:src/sprites/entity_sprite_rendering.c:134–168; R:sprites/rendering.rs:328–332; R:sprites/check_frames.rs:130–131; L:W0Sprites/report.md:67–72; R:elevator/ride.rs:221–222 | Two unchecked 54-word arrays collect pieces, shadows and inventory pieces. Primary handles 55/56 overwrite saved old A6, not the return PC. Authored production-caller fixtures establish that corruption is preserved; a natural >54-handle draw list is not established. Later use of corrupted A6 can fail. | Crash, potential [INFERENCE]; 00+02. | Fixable: yes, override link building with a deterministic hardware-capacity admission policy and bounded profile-local storage; do not merely hide the original overflow in faithful Rust. **fix; S**. |
| **B038 — Sprite selector's twelve-byte locals can overflow** | G:src/sprites/sprite_slots.c:10–59; R:sprites/slots.rs:94–98; L:W0Sprites/report.md:67 | `free_slots`/`secondary_slots` have twelve entries but a larger requested count can fill beyond them; zero-count requests also do not behave like a simple empty request. Counts 13/15 are authored boundary cases. Normal shipped entity requests have not been shown to violate capacity. | Gameplay-affecting; 00+02. | Fixable: yes, override the selector to reject impossible counts or use a bounded full-pool selection policy with defined zero-count success. **optional; S**. |
| **B039 — Pause secret indexes assignment ten when none remain** | G:src/ui/pause.c:142–160; R:ui/menus/pause.rs:182–184 | With all ten assignments already zero, the secret's first scan leaves `minimum_entry == 10`; its later read/write accesses past the array. Normal secret use with missing pieces is P001; entering it again after completion is this boundary bug. | Gameplay-affecting; 00+02. | Fixable: yes, override the secret handler to acknowledge/no-op an empty assignment set before selecting a minimum. Preserve the actual secret. **fix; S**. |
| **B040 — Invalid growth direction can make no progress forever** | G:src/map/random_regions_and_cleanup.c:182–228,234–276; R:map/random_generation.rs:201–202; R:map/regions.rs:4–6; L:W0Map/report.md:84–85 | Only the four cardinal direction values move the origin. A different value on an unchanged fully matching region can keep the scan in place indefinitely. Original normal callers choose cardinal directions; this is an invalid-input hazard. | Softlock; 00+02. | Fixable: yes, enhanced growth-origin overrides reject a non-cardinal direction before scanning. **optional; S**. |
| **B041 — Flagged cutscene cleanup tests and deletes different records** | G:src/cutscenes/title_and_transition.c:234–246; R:cutscenes/title_and_transition.rs:357–362; G:docs/subsystems/cutscenes.md:256–258 | The pointer starts at record zero while deletion indices start at six and end at 39. A flag on record N controls deletion of N+6. Normal cleanup reaches this loop; an unwanted deletion's natural visibility is not established. | Cosmetic; 00+02. | Fixable: yes, override cleanup to align tested record and deleted handle after choosing the intended range; intended historical range is [INFERENCE]. **optional; V**. |
| **B042 — Driving-part height bias accumulates** | G:src/cutscenes/large_player_parts.c:179–208; R:cutscenes/large_player_parts.rs:190–195; L:CutscenesB/report.md:169 | Each placement sets X/Y but adds 100/101 to existing height for the two offset parts. Repeated placement without a reset accumulates height. The routine is a normal intro helper; repeated-trigger frequency is not established. | Cosmetic; 00+02. | Fixable: yes, override placement with a defined base height plus the part bias, not repeated addition. **optional; V**. |
| **B043 — Planet release leaves a stale selector** | G:src/cutscenes/impact_and_planet_entities.c:69–74; R:cutscenes/impact_and_planet_entities.rs:154–159; L:CutscenesB/report.md:42,169 | The object is freed but its global handle is retained. A repeated release or later stale use can address a recycled slot. Normal release is safe if no reuse occurs; repeated/reordered use is the hazard, not proof every release corrupts a scene. | Cosmetic; 00+02. | Fixable: yes, enhanced release clears the handle after releasing the owned object. **optional; V**. |
| **B044 — Earl-only ending reads the first player's handle anyway** | G:src/cutscenes/ending_player_release.c:18–31; R:cutscenes/mod.rs:1048–1053 | Either live handle enables the frame query, but both branches index the first handle. With only Earl's ending entity, the query reads entity zero's frame. Earl-only play is normal; the resulting dialogue/mouth animation difference is source-derived. | Cosmetic; 00+02. | Fixable: yes, override `EndingGetFirstPlayerAnimationFrame` to use the surviving handle selected by its guard. **fix; V**. |
| **B045 — Ship collection proceeds with piece index ten** | G:src/ship/ship_screen_and_collection.c:592–609; R:ship/collection.rs:297; R:ship/mod.rs:131; L:ShipSpeech/replay-prep.md:55; G:docs/subsystems/ship.md:208–210 | The level-assignment scan can exhaust ten entries, yet the world object is freed and `ShowShipPieces(10,...)` is called. Requires a ship record without a matching assigned piece; ordinary valid assignments avoid it. This can feed B020 but is a separate unchecked producer. | Gameplay-affecting; 00+02. | Fixable: yes, override collection to establish the matching piece before mutating/freeing the world record. **fix; S**. |
| **B046 — Inventory service phase starts at entry D2** | G:src/ui/inventory.c:491–507; R:ui/input.rs:527–530; P:docs/TRANSLATION.md:520 | `CheckBButton` increments an uninitialized short before modulo-three servicing. It begins at actual entry D2.low; mail-order/identification wrappers commonly leave the player number there, other callers their compiled residue. Normal modal use reaches it; this affects initial service cadence, not an EXT-discarded lane. | Cosmetic; 00+02. | Fixable: yes, initialize the service phase in an enhanced modal continuation. **optional; V**; ordinary cadence is not a reason to alter faithful callers. |
| **B047 — Minimum RNG seed remains negative** | G:src/map/random_regions_and_cleanup.c:25–42; R:map/mod.rs:107–111; L:ReviewA/report.md:96 | Negating the minimum signed 32-bit seed wraps to itself, so normalization fails to make it positive. Normal seeds are not shown to reach that exact value; direct seed/import boundary is clear. Return value remains the previous generator state by design. | Gameplay-affecting; 00+02. | Fixable: yes, enhanced seeding uses a wider calculation and a specified positive Park–Miller domain mapping. **optional; S** (changes every downstream draw for that seed). |
| **B048 — Invalid PSG ID leaves PSG-active latched** | G:src/z80/sound_driver.s:134–144; R:audio/guard_fixtures.rs:39–42; G:src/asm/controllers.s:32,60; G:docs/subsystems/audio.md:216–219 | Driver sets active before validating the ID, then returns to polling without clearing it for out-of-range IDs. Controller reads mirror the real flag; PCM admission can be inhibited. An authored ID-30 driver path is documented; a normal invalid producer has not been established. The original Z80 driver is not a new Rust `tje-game` body. | Cosmetic (audio loss/admission); 00+02. | Fixable: yes, enhanced sound submission rejects invalid IDs before dispatch, or an explicitly versioned enhanced Z80/service replacement clears active on rejection. Do not fake the hardware mirror. **fix; V**. |

## B. REV 00 defects already repaired in REV 02

These eight entries follow the **eight changed functions**, not eight claims of
independent player-visible symptoms. R004/R005/R003 overlap in the sleep symptom;
R001 contains two source changes. The faithful revision switch must retain them.

| ID / title | Where it lives | Description, trigger, and normal-play reachability | Primary impact / affected revisions | Fixed-profile disposition |
| --- | --- | --- | --- | --- |
| **R001 — Falling skates and incomplete active-pose mask** | G:src/presents/present_dispatch.c:117; G:docs/REVISIONS.md:26; R:presents/dispatch.rs:6–8 | REV 00 can open rocket skates during falling; one animation-group bit also fails to allow active-present pose replacement. Normal present use during those states is the trigger. | Gameplay-affecting; **REV 00 affected; REV 02 adds fall guard and bit 27**. | Fixable: yes, enhanced REV 00 dispatch adopts the two explicit REV 02 conditions. No additional REV 02 repair needed. **fix; S**. |
| **R002 — Changing land makes a whirlwind rider fall** | G:src/map/land_processing.c:527–590; G:docs/REVISIONS.md:27; R:map/land.rs:569–570 | Turning land to void can start falling even while a player is inside/recovering from a whirlwind. Normal dynamic-land/whirlwind coincidence can trigger it for either player/shared or split screen. | Gameplay-affecting; **REV 00 affected; REV 02 excludes both whirlwind states**. | Fixable: yes, enhanced `ProcessLand` applies the REV 02 exclusions to REV 00. **fix; S**. |
| **R003 — Choice prompts count toward idle sleep** | G:src/player/sleep.c:34; G:docs/REVISIONS.md:28,35–38; R:player/sleep.rs:4–6 | REV 00 advances idle sleep while an interaction choice is open. Reachable by lingering at an ordinary prompt. | Gameplay-affecting; **REV 00 affected; REV 02 gates on interaction-choice state**. | Fixable: yes, enhanced sleep override adopts that gate. **fix; S**. |
| **R004 — Map opening retains idle ticks** | G:src/map/map_drawing_and_controller.c:501; G:docs/REVISIONS.md:29,35–38; R:map/screen_controller.rs:60–65 | Old idle time survives map entry and contributes to dozing after/menu interactions. Normal map use suffices; this works with R003/R005 rather than being a new sleep algorithm. | Gameplay-affecting; **REV 00 affected; REV 02 resets the opener's idle counter**. | Fixable: yes, an enhanced REV 00 map-entry override resets that counter once at entry. **fix; S**. |
| **R005 — Inventory opening retains idle ticks** | G:src/ui/inventory.c:197–223; G:docs/REVISIONS.md:30,35–38; R:ui/control.rs:302–307 | Same retained-idle issue for ordinary inventory entry. | Gameplay-affecting; **REV 00 affected; REV 02 resets the opener's idle counter**. | Fixable: yes, enhanced REV 00 inventory entry adopts the reset, not a continuous anti-sleep patch. **fix; S**. |
| **R006 — Title wait inherits the old counter** | G:src/cutscenes/title_and_transition.c:282; G:docs/REVISIONS.md:31,35–38; R:cutscenes/title_and_transition.rs:527–532 | The 1,200-tick wait is measured against the entry counter instead of restarting it. Normal attract/menu cycling changes the entry value; exact visible timing consequence depends on path. | Cosmetic; **REV 00 affected; REV 02 resets gFrameCounter**. | Fixable: yes, enhanced REV 00 title entry adopts the reset or a wrap-safe relative deadline with a specified profile policy. **fix; V**. |
| **R007 — Sega fade deadline can become unreachable at counter wrap** | G:src/cutscenes/sega_logo.c:36–122; G:docs/REVISIONS.md:32; R:cutscenes/sega_logo.rs:103–107; L:CutscenesSega/replay-prep.md:82,94 | REV 00 compares the zero-extended 16-bit counter with entry start plus 120; near wrap the promoted deadline can exceed every counter value. That boundary can leave the loop nonterminating. Ordinary startup is not near wrap; repeated/reentered scene state is the condition, not a reproduced boot hang. | Softlock; **REV 00 affected; REV 02 restarts the counter before the fade**. | Fixable: yes, enhanced REV 00 logo entry uses the REV 02 reset (or an explicitly profiled wrap-safe elapsed test). **fix; S**. |
| **R008 — Lean animations still receive terrain motion** | G:src/player/entity_motion_terrain.c:71; G:docs/REVISIONS.md:33; R:player/terrain_motion.rs:5–14 | REV 00's terrain-motion pause mask omits the lean group. Normal leaning on moving/affected terrain can trigger motion during the pose. | Gameplay-affecting; **REV 00 affected; REV 02 includes PLAYER_GROUP_LEAN**. | Fixable: yes, enhanced REV 00 motion adopts the wider pause mask. **fix; S**. |

## C. Additional compatibility hazards and intentional-looking code quirks

These are observed semantics, **not automatically original gameplay defects**.
Keep their reachability qualification when selecting optional hardening or mod
support. None authorizes clamping the faithful memory model.

| ID / title | Where it lives | Description, trigger, and normal-play reachability | Primary impact / revisions | Fixed-profile disposition |
| --- | --- | --- | --- | --- |
| **H001 — Encoded DMA length zero means 65,536 words** | G:src/asm/dma.s:15–74; R:platform/video.rs:176–179; R:sprites/dma_upload.rs:23–25; R:sprites/check_dma.rs:71–76; L:Elevator/replay-prep.md:237–240,267–269,318–319; L:PlatformTime/report.md:99 | DMA's 16-bit count wraps; zero is a full 65,536-word transfer, not an empty copy. RAM-wide sources can observe saved stack/frame bytes and overwrite VRAM. **Queue count zero executes no run**. Shipped action-icon runs are nonzero, prepared-frame queues suppress zero sums, and the ten ship frames are nonempty; a retained run is not evidence of a naturally retained zero-length run. | Cosmetic (VRAM corruption/large transfer); 00+02. | Fixable: yes, enhanced producer/upload overrides omit a logically empty run and reserve an explicit full-transfer operation for intentional use. Do not globally redefine the hardware count. **optional; V**. |
| **H002 — Decoder zero counts underflow DBRA** | G:src/asm/rle.s:22–132,139–203,210–228; G:docs/subsystems/platform.md:139–165; R:sprites/unpack.rs:32–34,149–151 | Raw zero tile count processes 65,536 tiles; zero literal/fill token counts run 65,536 iterations. Interleaved alternating zero/one counts produce huge pair counts. Actual retail assets reaching these invalid encodings are not established. Overwrites can include live RAM/stack. | Crash, potential [INFERENCE]; 00+02. | Fixable: yes, enhanced asset-decoder entry validates token minima, extents and destination quota, rejecting invalid content. A mod/import validator can do this without altering valid retail streams. **optional; S**. |
| **H003 — Zero shadow depth reads the preceding ROM byte** | G:src/sprites/entity_depth.c:40–51; R:sprites/updates.rs:44–46,123–135; R:sprites/leaf_tests.rs:669 | Depth is masked then shifted; when it becomes zero and differs from old depth, `lookup[depth-1]` under-reads. Heights 112–127 are within the computation's normal domain. The byte is stable ROM data, not random host memory; intended shadow appearance is unestablished. | Cosmetic; 00+02. | Fixable: yes, enhanced shadow override defines the zero-depth appearance explicitly. **optional; V**. |
| **H004 — Camera row publishes a fixed count after early edge exit** | G:src/camera/camera_tiles_and_scroll.c:68–206; R:camera/tiles.rs:19–22 | On nonwrapping level edges the world-row loop can stop before its fixed 45-word published run is filled, retaining old remainder data. Levels one/final deliberately wrap. Ordinary camera edges reach the routine; a visible stale-tail artefact is not newly demonstrated here. | Cosmetic; 00+02. | Fixable: yes, enhanced builder fills the complete published row with the intended blank edge or publishes the actual count consistently with its consumer. **optional; V**. |
| **H005 — Edge widening remembers a label across a gap** | G:src/map/random_regions_and_cleanup.c:478–483,484–606; R:map/regions.rs:112–114,177–179; G:docs/subsystems/map.md:237–239 | Nonqualifying cells reset run length to one but retain the prior terrain label. One cell, a gap, then three matching cells can hit the widening threshold. Generator reachability is ordinary; that this was unintended is explicitly [INFERENCE], not an ORIGINAL BUG assertion. | Gameplay-affecting; 00+02. | Fixable: yes, enhanced generation resets count and remembered label together. This changes maps for existing seeds. **optional; S**. |
| **H006 — Transition-list zero indexes before the descriptor table** | G:src/map/transition_paths.c:10–20; R:map/paths.rs:4–8 | Subtraction of one occurs without validating the unsigned-byte list number; zero selects the original underflow address. Catalogued normal transition callers use positive list IDs; direct malformed input is the trigger. | Gameplay-affecting; 00+02. | Fixable: yes, enhanced query rejects zero/out-of-domain list IDs before pointer resolution. **optional; S**. |
| **H007 — Path-class neighbour traversal has no edge checks** | G:src/map/map_population.c:12–65; G:src/map/level_generation_and_changes.c:15–61; R:map/paths.rs:24–27; L:ReviewA/report.md:98; L:W0Map/report.md:84 | Ordinary region traversal has boundary guards, but path-class branches recurse through direction bits without them; shape refinement also intentionally reads neighbours unchecked. Edge-direction/malformed map states can address adjacent storage. Normal generator invariants may exclude them; a natural violating map is not established. | Gameplay-affecting; 00+02. | Fixable: yes, enhanced generation/traversal validates edge direction bits or bounds the affected neighbour reads. **optional; S**; do not label every border read an observed bug. |
| **H008 — Unchecked handles and aliases can make compiler saves semantic** | G:src/sprites/sprite_slots.c:79–90; G:src/speech/speech_and_dialogue.c:19–45,173–190; G:src/jamout/jamout.c:310–349,418–439,525–606; R:sprites/slots.rs:5; R:speech/mod.rs:175,209; R:jamout/mod.rs:223,289,327,339,377,398; L:ReadableReview/SpritesCore.md:18–58; L:ReadableReview/AudioJamShipSpeech.md:17–25,35–93 | Negative/out-of-range handles, full-width indexes, or stack-backed data can read/write real save slots, mutate restored registers, and affect later object/VDP/music state. This is the flat original machine's unchecked entry-state contract, not a newly discovered natural exploit. The reviews' missing-save bugs were **port bugs**, not bugs to patch in the original. | Gameplay-affecting on authored boundary inputs; 00+02. | Fixable: technically yes through enhanced boundary validation/mod policy, but **keep; S** for ordinary compatibility. Do not add defensive host slices or sanitize aliases in faithful mode. |
| **H009 — Zero horizontal dead-zone delta still moves right** | G:src/earthlings/steering_helpers.c:9–32; R:earthlings/mod.rs:665–667; L:W0Actors/report.md:52 | The vertical-dead-zone helper chooses right for zero X delta and returns that directional state. Normal AI alignment can trigger it; this is a deterministic bias, not an uninitialized read. | Gameplay-affecting; 00+02. | Fixable: yes, enhanced steering could stop horizontal motion at exact alignment. **keep; S** (recognizable AI behaviour; no demonstrated defect requiring repair). |
| **H010 — Biting mailbox setup retains old contact bytes** | G:src/earthlings/mailbox_earthling.c:24–42; R:earthlings/banks.rs:45–47 | The mail-order branch clears contacts; the biting branch does not. Reconfiguring a bank into biting mode retains old contact state. Normal bank reuse reaches setup, but an adverse visible contact consequence is not established. | Gameplay-affecting, conditional; 00+02. | Fixable: yes, enhanced mailbox-bank setup can reset both contacts for every new level/mode. **optional; S**. |
| **H011 — Absent elevator records can still exclude placement** | G:src/elevator/elevator_scene.c:309–323; R:elevator/geometry.rs:113–115 | `ElevatorContainsPosition` filters by level/position but not absent/blocked state. A same-level retained record can therefore reject positions even when its elevator is not displayed. Normal placement calls use this helper; a harmful retained-record configuration is not established. | Gameplay-affecting, conditional; 00+02. | Fixable: yes, enhanced placement predicate can require a relevant active elevator state as well as level. **optional; S**; do not assume every absent record is semantically irrelevant in the original. |
| **H012 — Arrival push-out checks only the last eligible non-rider** | G:src/elevator/elevator_transitions.c:206–223; R:elevator/transitions.rs:8–11 | If both same-level players are non-riders, Earl overwrites the candidate pointer selected for ToeJam. Only that last candidate is tested and moved clear of the arrival door. Normal co-op arrival processing can select the policy; the both-non-rider/overlap configuration is not established as a natural failure. | Gameplay-affecting; 00+02. | Fixable: yes, enhanced arrival processing can apply push-out independently to both eligible players with deterministic order. **optional; S**. |
| **H013 — Ending state seven immediately consumes state eight's tick** | G:src/cutscenes/ending.c:156–166; R:cutscenes/ending.rs:643–646; L:CutscenesA/replay-prep.md:14 | After setting a 51-tick caption countdown, state seven increments the state and deliberately falls through to the decrement in state eight, leaving 50 in that same update. Reached by the normal ending. No missing-break historical intent is asserted. | Cosmetic; 00+02. | Fixable/changeable: yes, enhanced scene state could separate entry from tick processing. **keep; V** (authentic scene cadence, not a demonstrated defect). |

## D. Public player-facing exploits, secrets, and quirks

Public evidence is not a substitute for a code trace. Source matches below say
whether they establish the mechanism or only identify a candidate. In particular,
**intentionally helpful or funny behaviour is not classified as a defect merely
because it is exploitable**.

Public sources consulted:

* **[RG]** Ron Glenn, [Guide and Walkthrough, v1.0 (2001-03-15)](https://gamefaqs.gamespot.com/genesis/563350-toejam-and-earl/faqs/11002),
  especially **VI, “Gameplay Quirks and Bugs”**, IV (Santa), and VII (secrets).
* **[FH]** falsehead, [Guide and Walkthrough, Final (2001-09-10)](https://gamefaqs.gamespot.com/genesis/563350-toejam-and-earl/faqs/10905),
  sections VIII (lives) and X (“Tips and Cheats”). The guide identifies its play
  context as PAL, not a verified REV 00/02 ROM identity.
* **[GF]** [GameFAQs Genesis cheats/secrets](https://gamefaqs.gamespot.com/genesis/563350-toejam-and-earl/cheats),
  named entries “Ultimate Cheat,” “Free Presents,” “Level 0,” “Present Island,”
  and “Secret Trixie Island on Planet Funkotron.”
* **[DH]** DHarris, [FAQ v1 (2001-02-05)](https://gamefaqs.gamespot.com/genesis/563350-toejam-and-earl/faqs/10584),
  present descriptions and “Other Stuff/Tips,” used as corroboration, not proof
  of every FAQ claim.

The TCRF game page and its parse endpoint both returned HTTP 403 during this
research. **No inaccessible TCRF statement is treated as evidence**. No later-game,
modern-remake, emulator-configuration bug, or unsourced folklore is silently
imported as a Genesis defect. For example, [RG]'s old-emulator H-interrupt setup
warning is not an original game's bug.

| ID / title | Public evidence and code match | Trigger and normal-play reachability | Primary impact / revisions | Fixed-profile disposition |
| --- | --- | --- | --- | --- |
| **P001 — The iconic pause secret** | [GF] “Ultimate Cheat”; [FH] X; [RG] VII.B. Exact handler: G:src/ui/pause.c:127–162; R:ui/menus/pause.rs:182–184 and GamePaused body. L:CutscenesA/replay-prep.md:95 and L:Elevator/replay-prep.md:106–109 distinguish secret-ending recordings from natural campaigns. | While paused, enter **Up+A+B+C, Right+A, Down+B, Left+C** (mapped input). It clears all but the earliest remaining piece and, unless that piece is already on player one's current level, puts it on the next level. Thus the FAQ's “always next level” wording has a source-visible exception. Normal controls; deliberately hidden cheat, not accidental corruption. Empty-set bug is B039. | Gameplay-affecting; 00+02, handler source-confirmed. Public cartridge revision unspecified. | Fixable/removable: yes, override GamePaused's secret stage or gate it as a named cheat option. **keep; S**. Record its use honestly; do not label completion through it a natural ten-piece campaign. |
| **P002 — Survive Total Bummer by opening a healing present** | [RG] VI, Total Bummer paragraph. Candidate mechanism: G:src/presents/present_effects.c:395–440; G:src/ui/inventory.c:403–488; R:presents/effects.rs:665; R:ui/input.rs:196. The exact input-to-death race is **reported**, not newly reproduced/code-proven end to end here. | Hold the presents button during the final drain animation and immediately use health-restoring food/root beer/sundae when the menu opens. Normal inventory strategy, dependent on timing and owning a healing present. | Gameplay-affecting; relevant source paths 00+02; public revision unspecified, both-revision exploit timing remains unverified. | Fixable: yes, enhanced death/resource state could commit the fatal result before allowing inventory, if intentionally desired. **keep; S** (famous resourceful escape, not a first-pass safety fix). |
| **P003 — Protect dropped presents from Randomizer** | [RG] VI, final paragraph; [DH] Randomizer description. Mechanism: G:src/ui/inventory.c:355–399 changes inventories/identities, not world-item kinds; G:src/presents/item_updates.c:84–128 recreates world display; R:ui/menus/transactions.rs:79; R:ui/input.rs:11; R:presents/item_frames.rs:127. | Drop valued presents, leave their display range, use Randomizer, then return and collect the same contents in reassigned/unidentified boxes. [RG] also reports already-visible boxes do not refresh until leaving/reentering. Ordinary strategy; no corrupt state required. | Gameplay-affecting; 00+02 mechanism; public revision unspecified. | Fixable: yes, enhanced reroll can include world items or refresh visible labels, with a clearly chosen rule for previously dropped items. **keep; S** for content preservation; label refresh alone could be optional and still needs a distinct profile. |
| **P004 — Generated start, ship piece, or exit on a disconnected island** | [RG] VI, long islands paragraph (first-person report). Candidate generation/validation chain: G:src/map/level_generation_and_changes.c:435–640; G:src/map/map_population.c:12–77; G:src/map/terrain_placement.c:80–169; R:map/generation_stages.rs:6; R:map/paths.rs:27; R:map/placement.rs:76. No exact retail seed or proven single bad predicate is available from that report. | Rare random level strands a required position on an island inaccessible with current presents. Wings/shoes/doorway or farming Santa may recover; without resources it can block progression. **Reported softlock**, not proof every disconnected land patch is invalid. | Softlock, resource-dependent; generation logic shared 00+02; report revision unspecified. | Fixable: yes, an enhanced generation override validates reachability among spawn/exit/ship objectives under an explicit traversal rule and deterministically retries. **fix; S**, but separate from initial small fixes because it changes seeded maps and may remove intentional optional islands. |
| **P005 — Mini-map gaps look larger than traversable gaps** | [RG] VI, map paragraph. Candidate rendering/terrain split: G:src/map/map_drawing_and_controller.c:59–91; G:src/map/terrain_at_position.c:17–76; R:map/display.rs:157–158; R:map/terrain.rs:6. The specific public gap has not been matched to one metatile. | Walk across a narrow gap represented as wider void on the coarse map; the marker can appear over empty map space. Normal terrain/map comparison, public report rather than a new reproduction. | Cosmetic; shared code 00+02, specific metatile/revision unverified. | Fixable: yes, enhanced map drawing could depict collision-support detail more accurately. **optional; V**; keep original map art in faithful presentation. |
| **P006 — Split-screen enemies track beyond the visible half-screen** | [RG] VI, split-screen paragraph. Candidate mechanism: G:src/earthlings/targeting.c:40–47,78–100; G:src/earthlings/spawning_and_banks.c:426 (LoadInEarthlings); R:earthlings/framed.rs:404; R:earthlings/dispatch.rs:21. Target ranges use world distance, not the reduced viewport height. | Separate the players. An Earthling can track/ambush before entering that player's shorter view. Normal co-op; understandable shared-world policy, not necessarily mistaken AI. | Gameplay-affecting; 00+02 source mechanism; public revision unspecified. | Fixable: yes, enhanced targeting could gate acquisition by per-player visible bounds. **optional; S** (changes difficulty and target/RNG history). |
| **P007 — Airborne/motion-state contact immunity** | [RG] VI, feet-not-touching paragraph; projectile exception noted there. State-mask mechanism: G:src/player/earthling_damage_reactions.c:8–18,40–56; G:src/presents/projectile_effects.c:152–197; R:player/mod.rs:765–766; R:player/reactions.rs:8–11; R:presents/projectiles.rs:789. The public “feet physically touching” explanation is a simplification of state/animation-group tests. | Rocket skates, jumps/bounces, flight and whirlwind states avoid many contact effects; projectiles use a different path and can still hit. Ordinary intended-looking combat/traversal affordance. Do not infer universal invulnerability from height alone. | Gameplay-affecting; 00+02 mechanisms; public revision unspecified. | Fixable: yes, enhanced contact eligibility could include more states, but would rebalance movement items. **keep; S**. |
| **P008 — Skip extra-life immobilization while airborne** | [RG] VI, extra-life paragraph. Direct gate: G:src/presents/present_effects.c:342–369,390–391; R:presents/effects.rs:506 (PerformPromotion). | Gain a life during an excluded airborne/special state and retain motion instead of entering the ground reaction pose; the life/sound/display still occur. Normal present/rank timing. | Gameplay-affecting; 00+02; public revision unspecified. | Fixable: yes, enhanced promotion override can force or consistently avoid immobilization according to a chosen policy. **keep; S** (helpful state-aware affordance, not corrupted life accounting). |
| **P009 — Rain-cloud strike misses swimming states** | [RG] VI, lightning paragraph. Direct strike eligibility: G:src/presents/present_effects.c:499–539; R:presents/effects.rs:72–76. | Enter swimming state during a normal cloud strike. The selected ground/zap and jump groups do not include every water state. **Exception:** inventory/elevator interaction while a strike is active has a separate direct two-health subtraction (:512–516); do not advertise absolute water immunity for that branch. | Gameplay-affecting; 00+02 source path; public revision unspecified. | Fixable: yes, enhanced cloud override can add water-state damage with an explicit rule. **keep; S** (recognizable escape strategy). |
| **P010 — Cupid's shadow can stop Super HiTops** | [RG] VI, Cupid paragraph. Candidate collision/ability chain: G:src/player/process_movement.c:75–95; G:src/player/earthling_damage_reactions.c:245–277; G:src/asm/collision.s:9–11; R:player/movement.rs:7; R:player/damage.rs:130; R:collision/mod.rs:1–4. Exact “shadow” collision cause is **unconfirmed**; the drawn shadow alone is not proven to be the collider. | Run into Cupid's projected position using HiTops. Normal public-reported obstruction, not a reason to assert every shadow is collidable. | Gameplay-affecting; relevant code 00+02; public revision and exact mechanism unverified. | Fixable: technically yes through an enhanced collision/HiTops eligibility override **after** identifying the intended collider/state. **optional; S**; do not change faithful collision from this report alone. |
| **P011 — Mailboxes exclude three powerful gifts** | [RG] VI, first paragraph (the author only observed absence). G:src/ui/inventory.c:175–194, specifically :184, explicitly rejects Extra Life, Promotion and Jackpot by identity; R:ui/menus/transactions.rs:26–31. This confirms all three reported exclusions as a deliberate stock rule, not a probability defect. | Browse normal mail-order offers. No amount of rerolling this offer selector can produce those three gifts through its accepted branch. | Gameplay-affecting stock policy; 00+02 source policy, public revision unspecified. | Fixable: yes, enhanced offer selection can use a separately configured exclusion policy. **keep; S** (balance/content choice, not a safety fix). |
| **P012 — Repeated Santa present farming** | [GF] “Free Presents”; [FH] X; [RG] IV Santa calls him an unlimited source. G:src/earthlings/mole_santa.c:173–200,203–239; R:earthlings/santa.rs:28; R:earthlings/effects.rs:85–87. Drops are two or three presents; Santa relocates and can be approached again. | Sneak up while Santa searches his sack, stop when he looks up, collect drops, locate him again. Ordinary repeatable resource strategy, not a duplicated-inventory memory exploit. | Gameplay-affecting; 00+02 mechanism; public revision unspecified. | Fixable/cappable: yes, enhanced sidecar tracks a per-Santa/per-level reward budget. **keep; S** (iconic helper interaction and recovery route for P004). |
| **P013 — Secret islands, Level 0 return, and Funkotron bonus island** | [GF] named island/Level 0/Trixie secrets; [FH] X; [RG] VII.A. Partial code match: G:src/map/map_drawing_and_controller.c:263; G:src/player/hottub_ability.c:19; G:src/player/ability_helpers.c:61; G:src/player/active_present_falling_and_respawn.c:425–428; R:map/screen.rs:30; R:player/hottub.rs:40; R:player/levels.rs:6. Exact Trixie placement is public-reported, not assigned a guessed code label. | Travel beyond the starting Level 1 islands with appropriate presents; the southwest hole reaches Level 0's hot tub/lemonade, leaving it returns to the highest reached level. [RG] explicitly says lemonade grants a life only once, not infinite lives. Funkotron's southeast island is another intentional secret. Normal exploration, not an underflow-level bug. | Gameplay-affecting; relevant logic/content shared 00+02; exact public revision unspecified. | Fixable/removable: technically yes by enhanced map/level/interaction overrides. **keep; S** (charming, authored secrets; preserve their one-time reward rules). |

## E. Invisible code quirks and proof-only obligations

**These are not gameplay-fix priorities.** A physical read can be real and require
an explicit source witness even when its value cannot affect the result. Conversely,
a genuinely consumed whole-word phase/delta must not be dismissed because a nearby
EXT.W case is harmless. Whole-word dependent cases are B005/B024/B027/B030–B032.

| ID / title | Exact source and Rust location | Trigger, reachability, and why it is invisible | Impact / revisions | Fixed-profile disposition |
| --- | --- | --- | --- | --- |
| **Q001 — Directional mode's mixed word before EXT.W** | G:asm/c_sprite_checks_and_movement.s:286–289; G:src/player/sprite_checks_and_movement.c:103; R:player/directional.rs:83–92 | Routine stores only the low byte of the own word at entry SP−10, physically reads the whole word, sign-extends the byte, then stores a full word. The stale high lane is discarded. Normal directional movement reaches it. | None / invisible; 00+02. | Fixable/normalizable: yes, an enhanced override could initialize the lane or use a host value, but no gameplay repair is justified. **keep; N**. |
| **Q002 — Earthling-present index's mixed word before EXT.W** | G:src/earthlings/spawning_and_banks.c:476–481; G:asm/c_spawning_and_banks.s:1425–1434; R:earthlings/open_present.rs:25–36; L:Earthlings/report.md:242–245 | Own entry SP−10 word gets only the entity-index low byte before a physical word read. EXT.W discards the untouched high byte. Every normal entry can reach it; independent of the real bank-overrun bug B008. | None / invisible; 00+02. | Fixable/normalizable: yes, but do not replace the faithful physical word read with a byte or fabricate a high-byte store. **keep; N**. |
| **Q003 — LoadMapIntoVRAM mixed word** | G:src/map/map_drawing_and_controller.c:214; G:asm/c_map_drawing_and_controller.s:717; R:map/display.rs:108–155; L:MixedWordSweep/report.md:51–58,158 | Compiler reads the word at own entry SP−8 after a byte store, then discards the old high lane with EXT.W. The baseline Rust implementation does not mirror this physical stack-local read, so it needs no gratuitous frame/API declaration. Ordinary map upload is affected only as a proof representation detail. | None / invisible; 00+02. | Fixable/normalizable: yes; **keep; N**. Do not invent stack traffic solely to make source spelling look uniform. |
| **Q004 — PrepareEntitySpriteFrame mixed word** | G:src/sprites/entity_sprite_rendering.c:357–414; G:asm/c_entity_sprite_rendering.s:908; R:sprites/frame_rendering.rs:51–141; L:MixedWordSweep/report.md:51–58,195 | Byte-initialized own entry SP−6 word is read then EXT.W-normalized in the compiler output. Old high lane is not a frame-choice input. As Q003, baseline Rust does not make that physical read. Normal frame preparation can reach it. | None / invisible; 00+02. | Fixable/normalizable: yes; **keep; N**. Preserve observable accesses of the chosen implementation/proof tier, not an invented gameplay fix. |
| **Q005 — First food pickup's mixed word** | G:src/presents/item_updates.c:156; G:asm/c_item_updates.s:415; R:presents/items.rs:252–256; L:MixedWordSweep/report.md:59,188 (Presents annotation `80c1ec5f`) | Own entry SP−10 high byte may be unwritten on the first food iteration; its low kind byte determines the sign-extended food value. Later iterations already own the whole word. Normal first food pickup reaches it. Full-health disappearance is separately B003. | None / invisible; 00+02. | Fixable/normalizable: yes; **keep; N**. A first-only declaration is a proof obligation, not a new food rule. |
| **Q006 — First path-animation cleanup's mixed handle word** | G:asm/c_land_processing.s:1509–1512; G:src/map/land_processing.c:445; R:map/land.rs:388; L:W0Map/replay-prep.md:106–113 | First cleanup writes a handle byte at A6−3 and reads A6−4 (entry SP−8); EXT.W discards the untouched high lane. Later cleanup/bank iterations have a fully written word. Normal path animation may reach the first cleanup. | None / invisible; 00+02. | Fixable/normalizable: yes; **keep; N**. Do not declare every later already-written occurrence uninitialized. |
| **Q007 — First path reservation passes unused uninitialized coordinates** | G:src/map/land_processing.c:405,459–462; G:asm/c_land_processing.s:1437; R:map/land.rs:282,388; L:W0Map/replay-prep.md:109–113 | First reservation consumes the wholly unwritten column at own entry SP−6 and row-register residue. The callee reads but does not use either coordinate. This is a real whole-word read with no coordinate effect, unlike the consumed shape in B010. Normal first reservation reaches it. | None / invisible; 00+02. | Fixable/normalizable: yes, enhanced caller can pass explicit coordinates, but there is no gameplay benefit established. **keep; N**. |
| **Q008 — Startup phase local is uninitialized in source** | G:src/core/startup_and_main_loop.c:607–615; R:core/main_loop.rs:191–194; G:docs/QUALIFICATIONS.md:293–298 | Allocated D2 previously held a shifted frame-counter value with a zero low word. The real startup path therefore supplies the phase residue; source historical reliance is [INFERENCE]. Arbitrary direct-entry register mutation is not normal startup. | None / invisible on the documented startup path; 00+02. | Fixable/normalizable: yes, initialize enhanced phase state explicitly if replacing the outer loop. **keep; N** for faithful; do not claim portable C initialization. |
| **Q009 — NewGameMainMenu reads an omitted incoming argument** | G:src/ui/main_menu.c:26–35; G:src/core/startup_and_main_loop.c:640; R:ui/menus/main_menu.rs:11–15; G:docs/QUALIFICATIONS.md:298 | Entry reads a short slot although the caller supplies no argument; the first menu overwrites it before the selection is used. Normal startup reaches the load, with no established gameplay effect. | None / invisible; 00+02. | Fixable/normalizable: yes, enhanced menu can use a no-input interface. **keep; N**; this is not B007/B015, where omitted arguments are genuinely consumed. |
| **Q010 — Consumer-local int result views / historical declaration uncertainty** | G comments and individual consumers listed below; R counterpart functions in the same ledger. | Shared narrow declarations and ordinary promotion do not reproduce matching output, so G retains consumer-local `int` views. A missing historical declaration is [INFERENCE]. Several real callees explicitly extend D0, so the source-model mismatch alone does not prove any player-visible error. Ordinary calls reach them. | None / invisible as a family absent an independently demonstrated symptom; 00+02. | Fixable/normalizable: yes, enhanced caller can consume a defined-width result, but **keep; N** unless an actual gameplay defect is separately identified. Never globally change faithful return widths from modern C intuition. |
| **Q011 — Impossible ship-screen exit never reads the uninitialized row** | G:src/ship/ship_screen_and_collection.c:265–270; R:ship/background.rs:207–211; G:docs/subsystems/ship.md:194–197 | The `page == 4` test is inside `page < 4`; short-circuiting prevents the row read. Normal initialization executes the loop, not the impossible branch. This is dead logic, not a dynamically consumed uninitialized local. | None / invisible; 00+02. | Fixable/cleanable: yes, omit the dead condition in an enhanced rewrite. **keep; N** in faithful; no need for an uninitialized-read waiver. |
| **Q012 — Unused Earthling record receives residual coordinates** | G:src/earthlings/spawning_and_banks.c:244–255,262; G:asm/c_spawning_and_banks.s:745,762,779,858; R:earthlings/framed.rs:219–223 (EarthlingRetainLevelBank); L:MixedWordSweep/report.md:63,81 | Kind zero skips coordinate producers but still writes X/Y from own entry SP−12/−10. A prior iteration may have initialized them. The record is unused; no later gameplay consumer is established, though exact RAM/proof state differs. | None / invisible on the documented unused-record path; 00+02. | Fixable/normalizable: yes, enhanced bank setup can initialize coordinates for unused records, but **keep; N**; path-dependent output history cannot be waived by a blanket annotation. |
| **Q013 — Interrupted D1/D2 observations are not an established ISR gameplay bug** | G:src/asm/interrupts.s:46–53; G:asm/c_startup_and_main_loop.s:193–196; R:platform/entries.rs:33–40; R:core/mod.rs:354–358; P commit `238889d`; L:W0Core/replay-prep.md:85–93; L:W0Map/replay-prep.md:90–104 | Level6Isr at REV 02 `0x32e` **saves all D0–D7/A0–A6 before calling C and restores them before RTE**. A proof reported consumption of interrupted D1 bits 8–15; Wait's real initial counter load is now materialized. The exact downstream semantic D1 consumer is not established by those notes, so retain “reported by proof work, unconfirmed as gameplay defect.” The Row D2/D3/D4 demand was subsequently traced to generic SSA/ABI overtaint and withdrawn: both Row proofs finalized without a body edit. Normal IRQ save/restore reads register values; that alone is not a leak into game rules. | None / invisible on established evidence; 00+02 wrapper; particular reported Wait observation REV 02. | Fixable: no original gameplay repair justified. An enhanced handler could explicitly define scratch inputs only if a real consumer were identified; do not remove required saves. **keep; N**. **Do not promote the retracted D2 diagnostic into an original bug.** |
| **Q014 — Original machine arithmetic and inclusive-count semantics** | G:src/asm/collision.s:9–11; G:src/asm/rle.s:22–228; G:src/asm/z80bus.s:10–39; R:collision/mod.rs:1–4; R:sprites/unpack.rs:32–34,149–151; R:audio/upload.rs:28–31; P:docs/TRANSLATION.md:486–496 | Word wrapping, N-only collision magnitude tests, narrow SUBQ overflow flags, inclusive DBRA lengths and minimum-short negation differ from naive host arithmetic. Ordinary defined inputs rely on those semantics; they are not bugs simply because a natural Rust rewrite changes them. Individually adverse cases are catalogued separately (B029/B047/H001/H002). | None / invisible for semantics alone; 00+02. | Fixable: not a blanket fix target. A specific enhanced rule may widen arithmetic, but it needs its own defined behaviour and recordings. **keep; N**. |
| **Q015 — Jam Out partner-frame synchronization is immediately overwritten** | G:src/jamout/jamout.c:313–343; R:jamout/mod.rs:220–223,234–246,258–269; L:ReadableReview/AudioJamShipSpeech.md:102 | Completed lead animations may copy the partner frame, then unconditionally store the before-first-frame sentinel before publishing the frame cache. In ordinary nonaliasing state the copied value cannot survive to the next update. Reads/stores still exist for exact access and unchecked-alias contracts (H008). Normal Jam Out lead completion reaches them. | None / invisible on ordinary inputs; 00+02. | Fixable/cleanable: yes, enhanced animation can remove the dead synchronization stores or explicitly implement synchronization as a new feature. **keep; N** in faithful; do not call the ordinary redundant value a new visible animation bug. |

### Q010: exact consumer-local result-view inventory

Each row belongs to **Q010**, sharing that entry's impact, 00+02 revision,
reachability and **keep / N** disposition. These are **all eleven** `ORIGINAL BUG`
consumer-local declaration blocks found in G, not eleven independently observed
player symptoms. Ordinary width-preserving casts/matching qualifications are not
silently added as more original bugs.

| G declaration and consumers | Rust counterpart location |
| --- | --- |
| G:src/cutscenes/composite_entities.c:13–16 — `AllocateEntity`; EndingCreatePoseComposite, EndingCreateSixPieceComposite, EndingCreateRightOffsetComposite, IntroCreateFallingRocketship, EndingCreateElevatedThreePieceComposite | R:cutscenes/composite_entities.rs:6,46,88,131,172 respectively. |
| G:src/cutscenes/ending_player_entities.c:12–15 — `AllocateEntity`; EndingCreateActivePlayerEntities | R:cutscenes/ending_player_entities.rs:6. |
| G:src/cutscenes/ending.c:22–25 — `EndingGetFirstPlayerAnimationFrame`; EndingPlayDialogueAndDeparture | R:cutscenes/ending.rs:458. The separate wrong-handle bug is B044. |
| G:src/earthlings/mole_santa.c:27–30 — `IsEntityVisibleInViewport`; SantaLogic | R:earthlings/santa.rs:25–28. |
| G:src/earthlings/spawning_and_banks.c:35–38 — `GetMapTerrainAtWorldPosition`; OpenEarthlingPresent, EarthlingDamageOnHazardTerrain | R:earthlings/open_present.rs:9; R:earthlings/state.rs:412. |
| G:src/earthlings/spawning_and_banks.c:40–43 — `EarthlingConfigureMailboxLevelBank`; EarthlingRetainLevelBank, OpenEarthlingPresent | R:earthlings/framed.rs:221; R:earthlings/open_present.rs:9. |
| G:src/earthlings/spawning_and_banks.c:45–48 — `GetRandomNumber`; FindSpawnLocationForEarthling, OpenEarthlingPresent | R:earthlings/framed.rs:77; R:earthlings/open_present.rs:9. |
| G:src/earthlings/terrain_movement.c:15–18 — terrain query; EarthlingProbeDirectionalTerrain, EarthlingMoveOnAllowedTerrain, EntityUpdateTerrainDrawFlag, EarthlingMoveWithinMapBounds | R:earthlings/movement.rs:16,78,174,204 respectively. |
| G:src/presents/level_item_setup.c:26–29 — terrain query; MapPlaceLargeLevelItems | R:presents/placement_frames.rs:223–551. |
| G:src/presents/projectile_effects.c:37–40 — terrain query; ProcessTomatoRain, EarthlingUpdateControlRemapProjectiles | R:presents/projectiles.rs:789,706 respectively. |
| G:src/presents/projectile_effects.c:42–45 — `PresentRegisterDecoyHit`; ProcessTomatoRain, EarthlingUpdateControlRemapProjectiles | R:presents/projectiles.rs:789,706 respectively. |

G's own interpretation is explicit in `docs/subsystems/{cutscenes,earthlings,presents}.md`
(cutscenes:264–274; earthlings:270–286; presents:312–323): matching a result view
is not proof of historical intent or an extra visible symptom.

## F. Explicit uninitialized-local declarations and lane-only additions

At baseline `888fab32`, the **gameplay-body** `.uninitialized_local(...)`
declarations are exactly Q001 and Q002. The declarations in
`R:stack_read.rs:252–258` and `R:stack_read/guard.rs:696–702` are synthetic API/guard
test requests, **not extra game bugs**. The DTO constructor at
`R:call_frame.rs:47–56`, Context API at `R:lib.rs:456–462`, and fixture Context
implementations are infrastructure, not additional declarations of historical
behaviour.

The report sweep also captures owner-branch annotations not assumed present at
the frozen baseline. This manifest records **every reported explicit production
or pending-body declaration site**, including first-only/lane-conditional sites,
without upgrading proof status:

| Owner / own entry-SP location | Catalogue entry | Exact declaration/read witness and disposition |
| --- | --- | --- |
| PlayerChooseDirectionalMovement, **−10 word** | Q001 | R:player/directional.rs:86–90; G:asm/c_sprite_checks_and_movement.s:286–289; low byte written, high byte discarded. Present in baseline. |
| OpenEarthlingPresent, **−10 word** | Q002 | R:earthlings/open_present.rs:29–34; G:asm/c_spawning_and_banks.s:1431–1434; present in baseline, Earthlings `edbe1db0`. |
| PlayerCollectNearbyItems, **−10 word**, first food only | Q005 | Baseline R:presents/items.rs:252–256; L:MixedWordSweep/report.md:59 names Presents `80c1ec5f` first-only annotation. Later already-written reads remain ordinary. |
| MapAnimateMarkedLandPaths, **−6 word**, first reservation only | Q007 | L:W0Map/replay-prep.md:109–113, `2e4bf78b`; G:asm/c_land_processing.s:1437. Entire column word is unwritten, not a mixed-high-byte case. |
| MapAnimateMarkedLandPaths, **−8 word**, first cleanup only | Q006 | Same owner report/commit; G:asm/c_land_processing.s:1509–1512. Untouched high lane discarded after the physical word read. |
| EndingPlayBouncingRocketshipScene, **−6 word**, first movement RMW | B027 | L:CutscenesA/replay-prep.md:135–140, `78205356` + `05ea9d80`; G:asm/c_ending.s:1116–1117,1193. Later reads follow the real wrapping word store. Post-callee original-frame witness remains a separate obligation. |
| JamOutCreateCharacterCopies, **−6 word**, either group's pre-lead read | B032 | L:MixedWordSweep/report.md:8,65, `6bbd9025`; G:asm/c_jamout.s:1149–1155,1198–1204. Two conditional source read sites share one local; actual prior writes control annotation eligibility. |
| MapChooseMetatileAlternatives, **−8 word**, before first choice store | B030 | L:MixedWordSweep/report.md:9,66, `3e51048c`; G:asm/c_level_generation_and_changes.s:1518–1566. Whole choice is consumed, not discarded by EXT.W. |
| MapGrowTerrainRegion, **−18 word**, unsupported kind only | B031 | Same report/commit :9,67; G:asm/c_random_regions_and_cleanup.s:966–983,1235. Normal supported-kind initialization must not be bypassed. |

The sweep's **LoadMapIntoVRAM** and **PrepareEntitySpriteFrame** sites are Q003/Q004,
not missing declarations: there is no physical Rust stack read to annotate
(`L:MixedWordSweep/report.md:11,55–58`). B005/B006/Q012 are callee-output/first-path
ambiguities; that report (:71–83) explicitly requires actual write history, not
a blanket uninitialized waiver. A declaration is not permission to import a
callee spill, ignore an actual store, or invent a saved-register value.

## G. Collection coverage, duplicate handling, and evidence limits

### G notes and comments

The collection searched G's README, the complete docs/subsystem guides, and source
comments for `bug`, `quirk`, `uninitialized`/`uninitialised`, overflow/underflow,
stale/undefined/nontermination, missing arguments and suspicious bounds. No
separate BUGS-named file was found. G's `README.md:35–38` and
`docs/REVISIONS.md:16–38` supply the eight revision repairs. The source's **43 C
ORIGINAL BUG blocks plus one Z80 marker** are accounted for by A/E/Q010 (the
lexical count is independently recorded by `L:AddressSpace/report.md:69`). A
marker can describe an ABI family or two scan defects; those are not counted as
43 separate player-facing symptoms.

The guides' explicit bug lists were cross-checked at:

| G guide | Relevant lines / catalogue coverage |
| --- | --- |
| docs/subsystems/core.md | 171–175 → Q008. |
| docs/subsystems/cutscenes.md | 250–285 → B025–B028, B041/B044, Q010; flying mask is also explicit in source. |
| docs/subsystems/earthlings.md | 265–286 → B008 and Q010. |
| docs/subsystems/elevator.md | 202–208 → B021; B022 comes from source/Rust residual ledger. |
| docs/subsystems/map.md | 223–239 → B004/B005/B010/B012/B047, Q007, H005. |
| docs/subsystems/player.md | 313–324 → B001/B002/B023. |
| docs/subsystems/presents.md | 304–324 → B003/B004/B007/B014, Q010. |
| docs/subsystems/ship.md | 190–210 → B019/B020/B045, Q011. |
| docs/subsystems/speech.md | 176–181 → B015/B016. |
| docs/subsystems/sprites.md | 210–215 → B011/B013. |
| docs/subsystems/ui.md | 215–227 → B017/B029/B046, Q009. |
| docs/subsystems/audio.md | 216–219 → B048. |
| docs/subsystems/platform.md | 139–165,221–235 → H002 and arithmetic/codec qualifications, not extra marked bugs. |
| docs/subsystems/camera.md; jamout.md | camera:186–194 and jamout:197–208 explicitly have no marked original bugs; later source/lane findings H004/B032 are kept distinct. |
| docs/QUALIFICATIONS.md | 148–149,293–298 → Q008/Q009 and the important **non-bug** literal-256 qualification below. |

The byte store of literal **256** to `gSinkingHelpSpeechDelay`
(`G:src/player/active_present_falling_and_respawn.c:614–618`,
`G:docs/QUALIFICATIONS.md:149`, `G:docs/subsystems/player.md:246–247`) is explicitly
**not asserted to be an original cooldown bug**. It truncates to zero and is a
matching-spelling qualification. Likewise unused temporaries, `sizeof` frame-layout
coaxing and redundant casts do not each become “bugs” merely because a preservation
comment mentions them.

### Rust and lane/review report reconciliation

Rust preservation comments were collected across `crates/tje-game/src`, including
all `stack_read` users. Normal register preservation, failure propagation,
private-test scaffolding and scheduling-admission comments are not separate
original-game quirks. The cited entries retain the actual semantic oddities
rather than treating every occurrence of “preserve” as a defect.

All matching `P/scratch/*/replay-prep.md`, `P/scratch/*/report.md` and the nine
`P/scratch/ReadableReview/*.md` reports were searched for original-behaviour
findings; relevant findings were followed into G/Rust. The useful report groups
are indexed here so a private historical port failure is not mistaken for a new
original-game bug:

| Private report(s) | What was retained, and what was not reclassified |
| --- | --- |
| Earthlings/{report,replay-prep}.md; Earthlings-width/report.md; Earthlings-held/report.md | B008/B023/B024, Q002/Q012 and result-view evidence. Earlier rejected uninitialized-read checks are historical proof blockers, not extra bugs. |
| CutscenesA/{report,replay-prep}.md; Ending/report.md | B011/B025/B027; real parent-frame producer and later first-read declaration retained. No fabricated natural failed-parent fixture or whole-ending approval. |
| CutscenesB/{report,replay-prep}.md; CutscenesIntro/replay-prep.md; CutscenesSega/replay-prep.md | B026/B028/B041–B044 and R006/R007. Access-order/CCR fixes in Rust are port fixes, not new 1991 defects. |
| Presents/{report,replay-prep}.md; PresentsB/replay-prep.md | B003/B004/B006/B007/B014/B035/B036, Q005. Postincrement/width/read-purpose corrections are faithful-port work, not fixes to the original. |
| W0Map/{report,replay-prep}.md; MapLevels/report.md; MapScreens/replay-prep.md; MixedWordSweep/report.md | Placement failures, B010/B030/B031/B040, Q003/Q006/Q007/Q012 and complete declaration manifest. Dynamic poll/readbacks are proof obligations. The later Row D2 correction **retracts** the earlier diagnostic (Q013). |
| W0Sprites/{report,replay-prep}.md; W0Core/{report,replay-prep}.md; W0World/{report,replay-prep}.md | B011/B037/B038, Q008/Q013, H001/H002. Exact stack/register/call composition remains faithful, not a request to remove original quirks. |
| ShipSpeech/replay-prep.md; UiHud/{report,replay-prep}.md; UiInventory/replay-prep.md; UiMenus/report.md | B015/B017–B020/B039/B045/B046 and Q011; secret-ending and no-progress qualifications retained. |
| Elevator/replay-prep.md; PlatformTime/report.md; W0Platform/{report,replay-prep}.md | B021/B022, H001/H002 and source-count/hardware semantics. Natural nonzero DMA producers distinguish reachability from a seeded zero-run proof. |
| W0Actors/report.md; W0Audio/{report,replay-prep}.md; AudioRetryHunt/report.md; AudioMusic/report.md | H009, Q010/Q014 and audio protocol ownership. Genuine device-ready waits are not CPU-loop bugs or evidence of a softlock under normal functioning hardware. |
| AddressSpace/report.md; ReviewHazards/report.md; ReviewA/report.md; ReviewB/report.md; MemReview/report.md | Original alias, boundary, width and residual semantics. Review failures in translations/proof contracts are not automatically original-game defects. |
| ReadableReview/SpritesCore.md:18–69; AudioJamShipSpeech.md:17–108 | Port save/frame-omission counterexamples inform H008; original overflow/residue notes inform B011/B037/B038/H002/H003. No natural-session reachability was asserted by those reviews. |
| ReadableReview/ReviewPlayer.md:21–43 | Missing **port** divide-exception delivery is not an original player bug. Actual original arithmetic semantics belong to Q014; no new gameplay crash is counted from this port finding. |
| ReadableReview/ReviewCutscenes.md:29–40; ReviewUi.md:31–35 | Pointer ABI metadata and discarded-read provenance errors are **port** findings, not original bugs. |
| ReadableReview/ReviewEarthlings.md:47–53; MapReviewer.md:36–44; ReviewPresents.md:29–47 | Static reviews and qualification of retained quirks, not independent executed reproductions. A no-finding review does not prove malformed states safe. |
| ReadableReview/ReviewWorld.md:18–46 | Port frame-omission counterexamples involving retained zero-length DMA and overlapping rectangle sources inform H001/H008; neither is reclassified as a newly reproduced original gameplay defect. |
| CheckProjection/report.md; HostPerturb/report.md; HostFixture/report.md; GamePilot/report.md; CorpusCheck/report.md | Proof-framework/corpus receipts were used only to qualify reported observations, not turned into 1991 defects or new passing tests. Later explicit source retractions take precedence (Q013). |
| EnhancedMode2/report.md:23–111; Netplay/report.md | Mode/configuration/recording boundaries guide the proposed opt-in design; these are not original bug evidence. |

### What this document does not claim

* No original ROM was changed and no code fix is implemented by this commit.
* No gameplay, replay, netplay or crash reproduction was run for this catalogue.
  Source-level triggers and lane/public reports are labelled as such. Existing
  fixture outcomes remain their owners' evidence, not freshly exercised checks.
* A proof operational ceiling, unsupported input, stale generated link, port panic,
  lost build cache, or emulator missing-HBlank option is not itself an original
  nontermination/crash. Real source loops B004/B017–B019/B040/R007 are separate.
* The list does not claim every latent bug in the game has been discovered.
  It consolidates all explicit findings/quirks in the searched sources, with
  grouped generic machine/alias/ABI obligations and independently qualified
  public reports. New evidence belongs under these stable IDs or a new entry,
  not in a second competing bug list.
