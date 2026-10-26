# Enhanced mode

Enhanced mode is an explicit, default-off simulation profile. Ordinary
`Machine::step` / `play_frame` do not consult its registry. The compiled cartridge
remains the fallback for every unselected entry. Enhanced code is not a timing
proof, and does not enter the proven/readable production tables.

## Resumable overrides, sidecars and input (format 3)

Design agreed with the snapshot owner before implementation, 2026-10-25.
Faithful machine snapshot format **2** and its exact bytes/hash remain unchanged.
Enhanced profiles use an enhanced-only **3** envelope, returned as the same
`MachineSnapshot` type so local saves, rewind and rollback hashing retain one
atomic unit. The existing header retains its ROM SHA1, frame count, CPU and
profile identity; the hardware payload is followed by a versioned enhanced
payload (schema 2) and its length, covered by the existing whole-image checksum. A plain
machine restore rejects this envelope: callers must use `EnhancedMode::restore`.
That method decodes and validates *all* extra state before restoring hardware,
then commits the decoded sidecars/scheduler without another fallible operation.
Faithful/enhanced/profile/sidecar-schema/config mismatches are errors, not
best-effort conversions. Snapshots are private local data, never public assets.

### Authoring and scheduling

Existing `Translation` and `FramedTranslation` callbacks remain bounded,
synchronous Atomic/Functional RTS replacements. They cannot suspend inside their
Rust call stack. `EnhancedProfile::with_resumable` adds explicit
`ResumableTranslation` entries; these may replace classified TimeAware RTS
functions as well. Their function takes `&mut dyn enhanced::Context` and
`&mut Continuation`, returning `OverrideStep`:

- `Return(Option<u32>)`: ordinary synthetic RTS, optionally setting D0.
- `Wait(Wait::Frames(n))`: suspend for at least `n` completed hardware frames.
- `Wait(Wait::Interrupt)`: suspend until an unmasked interrupt is acknowledged
  and its real handler has returned to this invocation.
- `Wait(Wait::Poll)`: yield one bounded clock quantum before checking a polling
  condition again. A VBlank RAM-counter loop reads the actual counter, yields,
  then rechecks; no readiness bit is fabricated.
- `CallOriginal { function, arguments }`: materialize an actual emulated call
  frame, run the compiled original in the frame scheduler, then resume the Rust
  state machine with its register effects intact. The original root's exact
  PC/SP stays bypassed, including polling backedges to its entry; nested calls
  at another SP still consult the enhanced registry.
- `ContinueOriginal`: a bounded before-only observer; enter the current compiled
  original once without keeping a callback/return obligation. Its deferred
  bypass is snapshot-safe even if an interrupt arrives before the first opcode.
  Use this for roots whose callees may reset or perform a nonlocal transfer.

The continuation is explicit serializable data (`stage` plus sixteen `u32`
locals), not a host future, closure, thread, pointer or borrowed Rust frame.
Larger per-invocation state belongs in a sidecar. Each callback invocation must
terminate. A positive clock charge after each transition prevents all-Rust
loops from monopolizing the frontend. Waiting advances real devices, admits and
runs real interrupts, and returns to the frontend at each hardware frame.
Original calls are likewise stepped, never synchronously drained across frames.
Suspended interrupt handlers may themselves enter resumable overrides.

Only classified RTS roots may be selected: RTE/reset/nonlocal-transfer roots are
not override entries. Managed `CallOriginal` transitions require an actual RTS;
use `ContinueOriginal` when no return obligation is appropriate. Synchronous
legacy callbacks cannot call a resumable override; use an explicit
`CallOriginal` transition instead. Nesting is bounded at 32 active invocations;
legacy original calls retain their 1,000,000-step/32-depth operational budget. Failure
latches the driver, with no fallback/retry of partially applied effects; only a
successful atomic restore or reset clears it.
Synchronous children release at their validated actual RTS, even if that step
made an interrupt pending. Pending device/IRQ state is preserved for the outer
driver; the bridge neither drains an IRQ/caller past the return boundary nor
fabricates readiness while completing a bounded callback.

### Typed sidecars

An extra defines a `Sidecar` type with bincode 2 `Encode` / `Decode<()>`, `Send`,
a stable unique `KEY`, and a nonzero schema `VERSION`. Register its initial state through
`mode.sidecars_mut().register(initial_state)` before playback. Both host and
callback can use `sidecars().get::<T>()` / `sidecars_mut().get_mut::<T>()`.
Missing registrations and duplicate keys fail closed. The registry is ordered
by key, not hash-table iteration or host type identifiers. Every entry's key,
version, canonical initial-state SHA1 and complete current value are serialized.
Restore requires the same registered schema and initial-state identity. For
immutable course/configuration fields, override
`Sidecar::configuration_identity() -> &[u8]` (at most 128 bytes; empty by default).
Its trusted registered value is compared against decoded/current state before
restore or capture; even a checksum-rewritten archive cannot replace the
configuration by retaining the old initial-identity claim. A stored digest must
be authenticated against the normalized immutable data by `validate`.
Reset restores registered initial values. At most 64 keys (1–128 bytes each) and
4 MiB of enhanced snapshot payload are supported; no arbitrary unregistered
state is silently initialized. Optional `Sidecar::validate` checks invariants
when registering, capturing and restoring a value.

All deterministic state belongs here: extra seats, course data and progress,
extra RNGs, pending audio requests, publication state and gameplay settings.
Codecs must be deterministic and round-trip their values. Do not encode host
pointers, wall time, unordered-map iteration or platform-specific native sizes.
The complete enhanced image (including pending continuations and input phase)
is the rewind/save/netplay divergence hash. Existing frontend netplay remains
faithful-only until its enhanced-profile transport is separately enabled.

### Immutable imported resources

Decoded ROM programs/catalogues are immutable derivative data, not an excuse to
serialize GPU caches or copy the world per callback. Build a `Resources` set of
`Resource` values (bounded unique key, nonzero version, identity authenticated
against actual imported ROM/config values), then consume a fresh selected mode
through `with_resources`. Installation is one-shot and rejects after execution
or restore; reset retains the same immutable resources.

Only a small canonical resource manifest is registered as a configuration-bound
sidecar. The existing format-3 transaction therefore rejects a different
resource key/version/identity before hardware restore, without serializing host
pointers or decoded resource payloads. Existing profiles with no resource set
keep their prior payload; faithful format 2 is unchanged.

`Resource::validate_restore` receives decoded `PreparedSidecars` and a borrowed
`SnapshotView` of the **captured** RAM/VRAM/CRAM before either hardware or sidecar
commit. Use it for catalogue/contract and actual device-pattern cross-checks;
never validate only after hardware changed. Bounded typed relational animation
contracts may accompany the aggregate as its authenticated manifest. Decoded
catalogue programs/art stay off-snapshot, and shareable controller recordings
strip the relational contracts as well.

Callbacks use `Context::state_and_memory` for disjoint mutable sidecars and a
readonly cartridge/RAM view, then `Sidecars::state_and_resource<T, R>` or
`state_and_resources<T, A, B>` to borrow the aggregate and one/two immutable
catalogues without cloning them. This readonly view does not replace real device
reads/waits: audio, input and interrupt progress still use the resumable scheduler
and approved original device calls.


### Four-pad routing and integration

`EnhancedInputs { pads: [GenesisController; 4] }` carries four complete controller
states; `From<GenesisInputs>` supplies disconnected seats three/four. Enhanced
`step` / `play_frame` accept either. Only the first two pads reach original
hardware, unchanged. All four are available through `enhanced::Context::inputs`
and are included in enhanced snapshots; faithful mode ignores extra seats and
delegates exactly to the original two-pad API. Native/browser enhanced builds
also provide separate, default-disabled seat-three/four binding preparation
controls and strict four-pad conversion; this does not select a gameplay profile
or change original input RAM. Faithful recordings/netplay remain two-pad.

Use `mode.snapshot(engine)`, `snapshot_into(engine, image)` and
`restore(engine, image)` for enhanced saves/rewind/rollback. Native and browser
adapters use these APIs whenever built with enhanced mode, including when the
selection is off (where they delegate to exact faithful format 2). Profile IDs
must change when override behavior changes. Immutable configuration/course data
must also participate in registered initial-state identity.
Rollback adapters also use the atomic capture/reusable-capture/restore/hash
boundary (and the selected driver for advancement); none drops an enhanced
envelope to obtain a bare hardware hash. The existing faithful-only connection
admission guard remains in place.

## Fixed v1

`fixed/1` is a default-off profile containing the catalogue's twenty recommended
entries: B001, B002, B004–B008, B011, B014, B015, B017, B018 and R001–R008.
The authoritative ordered manifest is `tje_extras::fixed::MANIFEST`.
R001–R008 install no overrides on REV 02; selecting them on REV 00 explicitly
offers the corresponding repairs without changing faithful revision selection.
No first-set fix requires or is deferred to the shared simulation cutover.

Native: build with `--features enhanced-mode`, enable **Fixed profile** in
Settings, or use `--fixed [--disable-fix B001] [--enable-fix R003]`.
Both fix switches are repeatable and reject unknown IDs. Browser: build with
`TJE_ENHANCED=1`, open **Fixed v1**, select fixes, then re-import the cartridge or
reload. Native selection changes reset the cartridge; browser selection changes
are saved for the next import, leaving the current session unchanged. B004–B006
are one checked-result interface migration: toggling any of these toggles all
three, preventing a bounded search from feeding unchecked original consumers.
Every other fix can be toggled independently.

Fixed is **exclusive** with enhanced debug, Maker, extended view and Jam Out.
Composing Fixed with extended view and Jam Out comes later through a **single
composed adapter**, not stacked drivers with competing hook/scene ownership.
Host-only shaders, scaling, audio controls, save states, rewind and fast-forward
keep their existing adapters and remain available. Fixed snapshots use format 3,
`fixed/1` and the immutable four-byte fix mask in a registered configuration
sidecar; an otherwise valid image from another mask is rejected before restoring
hardware. Revision is already bound by the outer cartridge identity.

Fixed runs are not faithful recordings. Current controller-only recording/replay
and report replay cannot encode enhanced profiles, so the native frontend rejects
recording/replaying them instead of writing a misleading faithful recording.
Both frontends omit controller-only replay timelines from enhanced bug reports,
including after a native hard reset. Netplay remains faithful-only.

### Corrected outcomes and boundaries

* B004 counts all ten random candidates, then scans all 494 cells once from a
  deterministic random origin. Every coordinate result is checked.
* B005 has an explicit **no-placement error** if that finite search fails. It
  changes no player coordinates, latches the enhanced session and asks the user
  to reset or restore. It neither invents `(0,0)` nor continues on an unvalidated
  old position. It does not add disconnected-island prevention (P004).
* B006 omits an unplaceable item. Large-item spacing and telephone view exclusion
  have ten deterministic retries; failure leaves the telephone record unchanged.
  Item-bank ownership/seed restoration still complete. B007 supplies the explicit
  small-item policy at its recovery caller.
* B008 owns a record only after one of the two bounded scans succeeds. A full
  bank declines the spawn, never accesses its neighbour. B011 frees a tile run
  only when that allocation acquired it.
* B014/B015 use revision-bound original callsites as narrow Rust argument
  adapters. They invoke real original speech/dialogue through the resumable
  scheduler with explicit speaker slots; unrelated callers pass through.
  Reunion dialogue 1/3 starts with ToeJam and dialogue 4 with Earl; ship collection
  uses the collector. Original descriptor-forced speakers still take precedence.
* B001 independently clamps the final X/Y skate speeds after original steering.
  B002 caps both recipients and conserves the actual pre-clap health sum, including
  odd totals, when it fits the combined capacities; only over-capacity excess is
  discarded. B017 stops at rank eight before reading its sentinel threshold.
  B018 immediately draws the defined empty bar for ineligible animators without
  temporarily changing actual health.
* R006/R007 use a documented **entry-reset policy**: reset the scene counter once
  before original bounded setup, rather than transplanting REV 02 addresses/code.
  All other scene/device waits remain real. R001 uses the fall guard and bit-27
  pose refresh; R002 keeps both whirlwind states; R003 gates idle sleep; R004/R005
  reset only the opener's idle time; R008 pauses lean terrain motion.

ROM-free regressions run canonical faithful Rust bodies against corrected
callbacks with labelled authored callees; they do not claim hardware proof.
The private `profile_session` example (`tje-extras/preview`) accepts a matching
cartridge and committed controller session as an **input source** for local
playtests, emits per-frame video/audio hashes, and checks snapshot round-trip and
fix-mask mismatch rejection. Its Fixed playtest is not an enhanced recording.
Its `fixed-pilot` selection instead uses deterministic live-RAM feedback: Start
edges accept the default two-controller menus, then short left/right walking stays near spawn.
It writes no RAM, requires at least 600 live-world frames and observed movement,
and reports those denominators. This is necessary when repaired REV 00 scene
timing makes an old faithful menu-input timeline miss its intended menu.
