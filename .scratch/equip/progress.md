# Migration progress

Read this first after a compaction or in a new session. The goal is
`.scratch/equip/migration-goal.md`.

## Where it stands

- main is at `2183d8fa`, **Ledger 229k** (the player's death and restart), pushed to
  `origin/main`. Its section is `## 229k.` in `docs/REWRITE_LEDGER.md`. 229j is `04519297`.
- **Owner decision, 2026-10-15 (chat):** "move forward with the implementations, and resume
  testing when we'll be on a more powerful machine; let's maximize implementation as much as
  possible."
  - **Deferred:** mutation sweeps and the full `gates.sh` runs. Do not start them until the owner
    says the stronger machine is ready.
  - **Kept for each section:** fmt check, clippy `-D warnings` over the workspace, the touched
    crates' unit tests, and the section's new or changed parity scenarios run alone.
  - **Bookkeeping:** each ledger section lists what it owes. The running list is "Deferred
    testing" below.
- **Owner decision, 2026-10-14 (chat):** an in-memory (world or unit) test is enough to prove a
  row `ported` when a binary scenario is impractical. Name the world test in the scenario cell.
  Parity matters, but work smart: fix the bugs and **backdoors** found in the original directly
  (record them as Defects) rather than carrying them over.
- **The host:** an EC2 `t2.2xlarge`, `i-00bb600a4547d5188` in eu-central-1, with standard CPU
  credits. The credits have been at 0 since 2026-10-15 11:36, so it runs about 5x slower. Do not
  change the instance without the owner; switching to unlimited credits was offered and not
  answered.
- Left uncommitted on purpose: `.scratch/equip/`. The staging rule keeps `owner-questions.md` out
  of commits, and the other files here are control notes.
- Topology (owner, 2026-10-14):
  - Channel 1 runs, and Channels 2-4 stay off.
  - The Shared Channel (99) stays on, because Channel 1 players need it. They reach it through
    the OX event (map 113), the guild battle and flag wars (maps 110 and 111) and the kingdom and
    siege war buttons (map 103). None of these warps runs until `pc.warp` lands.
  - Size the load and boot checks for Channel 1 plus 99: 65,632 characters and 291 MB on the
    release build.
- The owner has no `prodomo.toml` and no live server. The store at `127.0.0.1:55432` is a fresh
  test database, and the owner's only data is the legacy Game data in `legacy/` (2026-10-14).
- Open for the owner, unchanged:
  - `.scratch/tmp/tools/` is cited by the ledger but ignored.
  - `.scratch/survey195-storage.md` is tracked, and no repo path cites it.

## Deferred testing (run on the stronger machine, in this order)

1. **229k's sweep, the rest.** 229k.7 has the details; the scripts are in
   `.scratch/tmp/l229k/sweep/`.
   - Run part e's other 45 mutants in a fresh tar copy: `ONLY=` every name of
     `mutants229k_e.py` except `dg_restart_not_run`, with `BASELINE=0` once the baseline passes.
   - Then run the closing pass: `make_f.py`, then `run2.sh`. Every mutant must be killed.
   - `dm_restart_gold_unstored` and `dm_boot_recall_unwired` are carried to 229l's scenario of a
     player killed by a monster, which must kill both.
2. **The full gates for 229k and every later section,** with and without `DATABASE_URL`. 229k's
   expected count is 3472.
3. **Each later section's sweep,** from its own mutant files, written when the section is
   written.

On a host without the credit throttle, run the parity suite on its default threads. The 120 s
`START_TIMEOUT` stays (229k.7).

## 229g, done

On the final tree (committed in `9ebee194`):

- fmt, build (0 warnings), clippy, and rustdoc with `-D warnings` are clean.
- 3297 tests pass with and without `DATABASE_URL`, in 38 suites (3280 before: +14 `target.rs`,
  +1 `points.rs`, +1 `npc.rs`, +1 parity scenario). Doc-tests: 1 passed, 19 ignored.
- Parity scenarios: 87 (the test count in `parity.rs` is 88 `#[test]` markers; the suite runs 87).
  The `PARITY_OK` floor in `mutate_v2.py` is 87.
- Sweep: `mutants229g.py`, 42 mutants, all killed; two closed by added assertions (229g.8).
- Two old test gaps closed (229g.7): the loop test waits for its first Pulse, and the parity harness
  skips the stamina refill (`is_stamina_refill`), which a slow run let land inside chat scenarios.
- 254 Rust files and 191,473 lines outside `server/` and `.scratch/`.
- Receipts in `.scratch/tmp/l229g/`: `gates/`, `gates-first/`, `gates-second/` and `sweep/`.

## 229h, done

- 3312 tests pass with and without `DATABASE_URL` (38 suites); parity suite runs 89 (floor 89 in
  `mutate_v2.py`). 255 Rust files, 192,596 lines.
- Sweep: `mutants229h.py`, 33 mutants, all killed (229h.8). Receipts in `.scratch/tmp/l229h/`.
- `game_state::regen`: `Regens`, `run_regens`, `fire_regen`, `destroy_npc` (non-player Destroy).
  `world::npc::Respawner`. Boot installs it in `main.rs` `stand_up_npcs`.
- A `ported` row may name a world test as `path::name` (spec.md, `scenario_problem` in parity.rs).
- Harness: `quiet_for` sizes a point change (only a stamina refill passes); `read_a_move_within`
  skips the revive's end (`is_revive_end`). Three parity suites at once still overload boot and the
  5 s window scenarios; one suite at a time is fine.
- Sweep copies: `target/` moved between directories keeps stale `CARGO_MANIFEST_DIR`; run
  `cargo clean -p <the nine workspace crates>` in the copy after a move.

## 229i, done

- 3332 tests pass with and without `DATABASE_URL` (38 suites); the parity suite runs 91 (floor 91 in
  `mutate_v2.py`). 256 Rust files, 194,744 lines. Receipts in `.scratch/tmp/l229i/`.
- `game_state::mob_ai`: `Minds`, the state set, `Wake`, Idle and Move, `Goto` (`FUNC_WAIT`),
  `CowardEscape`, `MonsterChat`. `view.rs` keeps the sectrees' player count (`pc_counts`).
- Sweep: 108 mutants in three files (`mutants229i.py`, `_b`, `_c`); all survivors closed or
  equivalent. The driver gained a `quest` suite.
- **Harness, after 229i:** NPCs near a player walk and talk at random (the world's dice are seeded
  per run). Reads, drains, `still` and `closes` skip NPC chatter (`is_npc_chatter`). A test that
  needs a still NPC gives its proto `AIFLAG_NOMOVE`. `login_key_in` retries `ALREADY` like
  `login_by_key_when_free`. A long scenario must expect the revive's end at 5 s
  (`Keyed::timer_record`).
- **Sweep copies:** after re-tarring main over a copy, run `cargo clean --offline -p` on the nine
  workspace crates (protocol common net db gamedata world quest prodomo parity). The tar gives the
  sources main's older mtimes, so a crate last built from a mutant looks fresh.
- **Gates:** under full load a parity scenario can flake; run the parity suite with
  `DATABASE_URL` a few times after harness changes before the receipt run.

## 229j, done

- 3397 tests pass with and without `DATABASE_URL` (38 suites); the parity suite runs 95 (floor 95 in
  `mutate_v2.py`). 259 Rust files, 198,673 lines. Receipts in `.scratch/tmp/l229j/` (`gates/`,
  `gates-wrong-cargo/`, `sweep/`, `plan.md`).
- `prodomo::game_state::fight` (attack, `npc_on`, `after_damage`, `stun_npc`, `dead_npc`,
  `recovery_event`; the dues `Due::Stun`, `Due::Dead`, `Due::Recovery` on the regen schedule) and
  `world::character::combat::melee` (`melee_reach`, `calc_melee_damage`, `strike_npc`).
  `world::npc::fight::MobFight::of` gives a proto's battle points. `Regens::holds` is a test-only
  reader. `Vitality` and `apply_damage` are still there; 229k deletes them.
- Sweep: 142 mutants in five files (`mutants229j_a` to `_e`), 136 killed in the first pass; the 6
  survivors closed by named tests (229j.7).
- **Gates toolchain:** the first run built with rustup's cargo 1.98.1 and clippy failed E0514.
  `gates.sh` now exports `PATH=/usr/bin:$PATH` and writes `toolchain.txt`; check it says cargo
  1.85.1.
- Legacy facts to keep: `StartRecoveryEvent` runs when `dam > 0`; `Dead` takes the hitter as killer
  before the PID the stun kept; `test_server` defaults to 1, so its toggles run in production.

## 229k, done (testing cut short; see "Deferred testing")

- Cheap checks on the final tree, all passing:
  - fmt and clippy across the workspace;
  - `world` lib: 434;
  - `world` combat: 3;
  - `prodomo` lib: 832;
  - `prodomo` bin: 11;
  - `game_loop_thread`: 13;
  - the 5 new or changed parity scenarios: 5, on one thread, after the `still` fix.
- 261 Rust files, 202,621 lines. Receipts are in `.scratch/tmp/l229k/`.
- **Sweep:** 225 of 270 mutants ran, and 189 were killed. Of the 36 survivors:
  - 2 are equivalent;
  - 2 are carried to 229l;
  - 32 are answered by tests in the tree, 30 of them verified one at a time.
- `prodomo::game_state::death`:
  - `PcCombat` with `damage_pc`; `damage_pc` and `stun_pc` are `#[cfg(test)]` until 229l's
    monster melee calls them;
  - `dead_pc`, `dead_event`, the three restarts and `DeathPenalty`;
  - `ReviveInvisible(5)`, `CheckTarget` and the recall.
- `world::character::death_penalty`.
- **The phase:** `PHASE_DEAD` is 6, written as a real `GC_PHASE`, and the descriptor installs it.
- **The harness:** `START_TIMEOUT` is 120 s for the server's gate. `mutate_v2.py` gained
  `PARITY_THREADS` and `FLAKY_DIR`, and its `PARITY_OK` floor is 97.

## Next: 229l onwards

**Paused by the owner on 2026-10-15 at about 22:30 UTC** ("after this workflow finishes, let's
stop, and we'll continue later"), once 229l's plan was final. Nothing of 229l is in the tree
yet: main is still `2183d8fa`, clean apart from `.scratch/equip/`. **Resume with the 229l
port** (task 17), from `plan.md` §9.2 "Order of work".

In order:
- **229l, the monster fights back.** The plan is final and is the input for the port:
  `.scratch/tmp/l229l/research/plan.md` (ignored; copied there from the job's `tmp/r229l/`,
  which a new job will not have).
  - The same folder holds the five research notes (`legacy-battle.md`, `legacy-hit.md`,
    `rust-map.md`, `rows-client.md`, `forget-heal.md`), both critiques (`critic-legacy.md`: 1
    blocker, 3 fix, 9 note; `critic-design.md`: 2 blockers, 13 fix, 7 note) and their tools.
  - Plan §10 lists each critic finding and how the plan takes it: 32 taken, 3 in part (L4
    stun/slow stay draw-only until 229m; D7 one heal tick, not 57; D12 `created` seeds
    `last_attacked` only).
  - **Scope (plan §1):** 229l is the whole melee fight. 229m is the rest of the monster's fight
    (aggro and the damage map, RANGE/MAGIC, guards, summoners, mob skills, the `.msa` clip
    loader, reflect, the party arms). 229n is drops and exp, which needs 229m's damage map.
  - **New ids:** Defects B26-B30, Divergence V19.
  - **Scenarios:** P1, P2, P3a and P3b are written now but run later (deferred testing). P3a and
    P3b (a monster kills a player, who restarts) kill the two carried 229k survivors:
    - they read `GC_PHASE{6}`;
    - they check that the store's gold drops on `/restart_now` (`dm_restart_gold_unstored`);
    - they check that `/restart_town` warps to town (`dm_boot_recall_unwired`).
  - **Cheap checks per plan §9.2 step 9:** fmt, clippy `--workspace --all-targets`, and
    `cargo test -p common -p gamedata -p world --lib` plus `cargo test -p prodomo --lib`.
- **229n, drops and exp.** The research is done; no plan or critic yet (task 18).
  `.scratch/tmp/l229n/research/` holds `legacy-exp.md`, `legacy-drops.md`, `rust-map.md` and
  `rows-client.md`.
  - **Findings:**
    - On Channel 1 plus 99 the owner's data drops gold only: three drop tables are empty, and
      `mob_drop_item.txt` has one group, for mob 2493, which is not spawned.
    - Every kill still makes about 18 draws.
    - `Reward` runs `DistributeExp` first and stops when it returns NULL.
    - Ownership lasts 30 s.
  - **Defects to plan:**
    - the GiveExp int overflow;
    - the `iRandRange` precedence bug (the owner's Item-Drop Boost Event never worked);
    - the target-info preview, which creates items it never destroys (a backdoor);
    - race 5001's 25-35 empty gold piles.
  - **The research's proposed split:** the level-up chain first (no monster needed), then the
    solo exp reward, then gold on the ground and ownership, then `Reward`/`CreateDropItem` after
    the damage map.
- Then:
  - the group's party and follow;
  - the monster `.msa` clips and `motlist.txt`;
  - the V2 walk-duration recomputes (`G/char_skill.cpp:1827`, `:1945`, `G/char_battle.cpp:3741`,
    `G/BlueDragon_Skill.h:327`);
  - the cost per Pulse on Channel 1 plus 99.

  After those, follow the order in `migration-goal.md`.
- **Open items to carry:**
  - X6 / Q26: the AI flag names are swapped, and 229f reads only bit 0;
  - the event flags that ungate 5001, 5002 and 5004.

## Owner questions

- 31 are open in `.scratch/equip/owner-questions.md`: Q1 to Q32, less Q14, which was resolved in 229e.
- 229k added Q29 (the automatic restart's penalty), Q30 (no exp loss at the top), Q31 (drop the
  five `cg.dead.*` rows) and Q32 (when `[LS;901]` is said), and addenda to Q5 and Q18.
- 229j added Q28: evasion reads the attacker's level (`G/battle.cpp:282`, B5), kept provisionally.
- 229f added Q22 to Q27:
  - Q22: the group-group probabilities.
  - Q23: a group entry at a single point.
  - Q24: `AFF_SPAWN` stays set.
  - Q25: one dice stream for the whole process.
  - Q26: the AI flag names.
  - Q27: which Shared Channel routes open at launch, and the `event_panel` button that does
    not check GM level.
- Still to add when a section reaches it: `test_server` (question 5).

## ADRs

229b to 229k changed no ADR and proposed none. ADR-0006 (the quest dialect and host) is still proposed
and waits on the owner (question 1).

## Parity counts

- After 229a: 119 ported, 125 partial, 1338 missing, 6 unused, 57 codec.
- After 229b and after 229c: 120 ported, 125 partial, 1338 missing, 6 unused, 56 codec.
- After 229d: 121 ported, 124 partial, 1338 missing, 6 unused, 56 codec.
- After 229e: 121 ported, 125 partial, 1337 missing, 6 unused, 56 codec.
- After 229f: 121 ported, 126 partial, 1336 missing, 6 unused, 56 codec. Total 1645 rows.
- After 229g: 125 ported, 127 partial, 1332 missing, 6 unused, 55 codec.
- After 229h: 127 ported, 127 partial, 1330 missing, 6 unused, 55 codec.
- After 229i: 127 ported, 128 partial, 1329 missing, 6 unused, 55 codec.
- After 229j: 133 ported, 136 partial, 1319 missing, 6 unused, 51 codec.
- After 229k: 137 ported, 136 partial, 1311 missing, 11 unused, 50 codec.

Recount with `python3 .scratch/tmp/tools/parity_counts.py`.

## Toolchain, gates and sweeps

**Toolchain**
- rustup 1.98 in `~/.cargo/bin` shadows `cargo` itself and `cargo clippy` and `cargo fmt`. Always
  `export PATH=/usr/bin:$PATH`, and use `/usr/bin/cargo-clippy clippy` and `/usr/bin/cargo-fmt`;
  `gates.sh` does both since 229j.
- Rust 1.85.1, `clippy 0.1.85`, `rustfmt 1.8.0`.
- The gates' Rust line counts outside `server/` and `.scratch/` (fixed in 229c; the first 229c run
  counted 570 files because it did not exclude `.scratch/`).

**Gates**
- `J=<dir> DATABASE_URL=postgres://prodomo:prodomo-test@127.0.0.1:55432/prodomo
  .scratch/tmp/tools/gates.sh`. It writes `<dir>/gates.summary`. Run it in the background, and never
  while a sweep runs. The 229e final run is `gates229e4` in the job's `tmp`.
- A database test with `DATABASE_URL` unset prints a notice and passes, and a skipped test still counts
  as passed, so equal counts with and without the variable prove nothing. To check the store is used,
  run one scenario both ways and look for the `skipped` notice. For 229c this was done in
  `.scratch/tmp/l229/gates229c_final/ctl/`; for 229e, `an_operator_command_names_the_store_it_cannot_use`
  (unset: `skipped`, 0.00 s; set: no notice, 0.06 s).

**The sweep driver.** `.scratch/tmp/tools/mutate_v2.py` takes `ROOT`, `MUTANTS`, `ONLY`,
`SHARD=k/n`, `CHECK=1` and `BASELINE=0`. It:
- asserts that the copy has `.scratch/parity`;
- runs an unmutated baseline (`BASELINE=0` skips it, which is safe only when the copy's code equals
  main's);
- runs the suites world, common, gamed (added in 229e), prodomo lib, prodomo bin (`main.rs` only), and
  parity, and quest for `quest/` mutants; `PARITY_OK` is a floor of 95 passing parity tests, to raise as scenarios are added;
- confirms each parity kill by running the scenario alone;
- for a `main.rs` mutant, also runs the binary's own tests.

It has no timeout on `subprocess.run`, so a test that hangs blocks it. A killed driver skips its
`finally`, so the copy keeps the last mutant applied. Stop the hung test binary, then compare the
copy's file with main's sha256 and restore it. It writes its log only at the end. For one mutant by
hand, use `.scratch/tmp/tools/one_mutant_any.py`.

Run the shards one at a time. Two sweeps at once share the 8 cores, and two earlier 229d attempts
failed their baseline on chat scenarios when they overlapped (229d.7).

**The copies.** `wt2`, `wt3` and `wt4` under `.scratch/tmp/l229/` are scratch copies of the tree.
Run sweeps only in a tar copy, never in the main tree, and never copy a tree while a sweep runs in it.
