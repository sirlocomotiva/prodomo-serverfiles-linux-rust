# Goal: the Rust rewrite, package by package (re-cut by the direction review, ledger 679, 2026-11-15)

Prodomo is being rewritten from scratch. There is no live server, no deployment and no player data
(ADR-0001, ADR-0003):

- There is no account, character or item to migrate, and no running config to fix.
- The owner has no `prodomo.toml`. The first one will be copied from `config/prodomo.toml.example`,
  so a fix to the setup belongs in the example, the README or the code.
- The PostgreSQL at `127.0.0.1:55432` is a fresh test database.
- The owner's only inputs are the legacy Game data in `legacy/` (protos, maps, quests, locale
  strings, tables) and the frozen C++ source in `server/`, which is the reference for behaviour.
- Never write as if a server is running. When asking the owner about behaviour, ask what should
  happen when the server first opens, not what the live server does.

Keep porting the legacy server by the packages of `docs/STATUS.md` ("Next steps"), in their order,
until the goal below holds. A finished ledger section is not a place to stop. Commit it, push it,
rewrite `.scratch/migration/progress.md`, and start the next one.

## Where it starts

- Ledger 679 is the direction review. It ported nothing; it set `test_server = false` and
  `save_event_second_cycle = 10` as the defaults, amended the ADRs, and re-cut the work.
- Ledger 680 applied the owner's answers: the cross compiler, `change_empire` loaded, the drop
  limit, the game logs to keep, and `compose.yaml`.
- Ledger 681 is ADR-0008: legacy's mistakes are fixed while porting, not ported. Seven fixes are
  queued as package C10.
- Ledger 682 is the cleanup: this folder was `.scratch/equip/`, the gates are `tools/gates.sh`,
  the probes are in `.scratch/probes/`, and finished notes and issues were deleted.
- Ledger 683 is the test hygiene: a killed test run leaves nothing (`parity::scratch`,
  `db/tests/support/lease.rs`), `tools/gates.sh` fails on anything left, and the machine's
  leftovers, worktrees and `.scratch/tmp/` were removed.
- Ledger 684 is the toolchain: Rust 1.100.0 pinned in `rust-toolchain.toml`, the new Clippy's
  findings fixed, and a dependency no target uses fails the build.
- Ledger 685 is the scenario timing: every scenario that lost a full run had its cause in the
  scenario, all 519 were swept for the same classes, and `AGENTS.md` ("SCENARIO TIMING") has
  the rules a new scenario follows.
- Ledger 686 made the tail countable: every `partial` and `missing` note ends in `Left:`, and
  the large switches are generated arm tables.
- Ledger 687 is the first package B slice (the sibling vnums of four `USE_SPECIAL` families).
- Tests: 5,695 pass in 46 targets with `DATABASE_URL`. There are 554 parity scenarios.
- Parity: 1,656 ported, 158 partial, 512 unused, 5 missing and 0 codec, out of 2,331 rows;
  101 items left.
- Owner questions: `.scratch/migration/owner-questions.md`; the ones that block work are in
  `docs/STATUS.md`, "Waiting on the owner".
- `progress.md` has the details, and supersedes this section once it is newer.

## Topology (owner, 2026-10-14)

- **Channel 1 runs. Channels 2-4 are off.** In the example config they are commented-out tables,
  so they can be turned on again later. Do not remove the code that hosts them.
- **The Shared Channel (99) stays on.** Channel 1's players reach its maps through three routes:
  - the OX event, to map 113;
  - the guild battle and flag wars, to maps 110 and 111;
  - the kingdom and siege war buttons, to map 103.

  The research is in ledger 229f.5. Never turn the Shared Channel off without the owner.
- Measure boot time, memory and the cost per Pulse on Channel 1 plus 99. At 679 that is 65,604
  NPCs and monsters, 595 MB and 0.05 ms per idle Pulse on the release build; the cost with
  players in the world is package C1.

## The goal holds when

All three are true:

1. Every row of the nine generated tables in `.scratch/parity/` is in one of these states:
   - `ported`;
   - `unused`, with its evidence and controls;
   - blocked on the owner, and listed in `owner-questions.md` with the question it waits on.

   A row counts as blocked only when a new crate, an owner decision or the owner's play test is
   the one thing missing. A large row is not blocked.
2. `docs/STATUS.md` shows steps 3 and 4 done, apart from blocked rows. It also shows step 5 ready for
   the owner's first play test:
   - the Game data readers and importers are finished, and the full data set loads;
   - `compose.yaml` is written;
   - the README's from-scratch setup is written: PostgreSQL, a `prodomo.toml` copied from the
     example, `prodomo account` and `prodomo gm` for the first accounts, then `serve`.

   No importer for player or account data is built, because none exists.
3. Every ledger section is committed and pushed on main.

## Next

`docs/STATUS.md`, "Next steps", is the list. In short:

1. **Package A first.** Give every `partial` row a `Left:` list that can be checked against the
   legacy source, and generate the list where a row hides a switch (`UseItemEx`'s `case` labels,
   the skill switches, `PointChange` and `ApplyPoint`). Until the number of items left exists, do
   not port another arm.
2. **Package B**, by its sub-packages B1 to B8, a family of arms per ledger section.
3. **Package C alongside B**, all of it before D: the Pulse budget, the quest host's limit, slow
   and silent clients, one meaning for the test-server mode, the two `text` columns, the leaking
   harness, mutation sweeps by risk, and the widths.
4. **Package D**, the play-test kit. Package E is the owner's.

## Before each ledger section

- Read `AGENTS.md`, `CONTEXT.md`, `docs/STATUS.md` and the ADRs in `docs/adr/`.
- Read the last section of `docs/REWRITE_LEDGER.md` and `.scratch/migration/progress.md`.
- After a context compaction, read `progress.md` first.

## Each ledger section

Track the steps as tasks. Split a large system into sub-steps (229g, 229h and so on). If a system
grows past what one commit can review, it becomes two ledger sections.

1. **Research** the legacy source.
   - Every count and every absence gets a positive control and a nonsense control.
   - A new decision gets an ADR, marked proposed, since only the owner accepts one.
2. **Port** in dependency order. Back each quest API name once its system lands, and update its row
   in `quest-api.md`.
3. **Prove it** with a scripted-client scenario in `prodomo/tests/parity.rs`. Only a `ported` row
   fills its scenario cell; a `partial` row names its scenario in the note. Set each row's status,
   scenario and note cells.
4. **Update `docs/STATUS.md`:** the numbers in "Where the Rewrite stands", the "Next steps" list,
   and the Divergence and Defect rows. Never add a per-section paragraph; the story is the ledger
   section.
5. **Mutants.** Kill the section's own mutants by hand, as sections 640 to 678 did, and say which
   test kills each. The full sweeps run by risk under package C7 with `tools/mutate.py`, in a
   copy of the tree under `.scratch/tmp/` and never in the main tree; remove the copy when the
   sweep ends.
6. **Run the gates**, with `DATABASE_URL` and without it, then append the ledger section with its
   receipt:
   - both runs' test counts and target counts;
   - every test that failed on a first run, by name, with its cause read against the legacy
     rule (`AGENTS.md`, "SCENARIO TIMING") and its fix, or an issue under `.scratch/` that
     keeps the failure's output; "passes alone" does not close it;
   - the toolchain, and whether a width was measured;
   - the parity counts and the number of `Left:` items before and after;
   - the `temp-files`, `servers`, `scratch-dbs` and `cores` lines of the summary, each 0
     (`AGENTS.md`, "TEST HYGIENE"); a run that left something is not a receipt;
   - the `allows` line of the summary, which may not be higher than the last receipt's
     (`docs/STATUS.md`, "Code-quality backlog").
7. **Commit and push.**
   - Subject: `Ledger NNN: <system> (<the legacy parts>)`.
   - Run `git push origin main`.
   - Rewrite `progress.md`.

## When something needs the owner

The owner may be away. Never stop to wait.

- **A new or upgraded crate** needs an online `cargo fetch` and the owner's yes. Do not add it.
  Record it in `owner-questions.md`, port what does not need it, and mark the rest blocked.
- **A legacy mistake** (a bug, an inconsistency, a vulnerability, a backdoor) is not a question:
  fix it as you port the code and record it (ADR-0008): a Defect row, a Divergence row when a
  player can tell, the ledger, and a test of the fixed behaviour. Never port it "provisionally".
  When the fix changes numbers every player meets, also add it to the table under "Waiting on
  the owner" in `docs/STATUS.md`, and continue.
- **Odd but deliberate legacy behaviour** (a rule the code states on purpose, a switch the
  owner's build defines) is a Quirk: port it as it is. If you think it should change, add it to
  `owner-questions.md` and continue.
- **A difference the Rewrite's design causes** is not an improvement. Match legacy, or record
  the Divergence with its reason.
- **Other decisions:** a Divergence players would see for another reason, or an ADR to accept:
  take the option that follows legacy, mark it provisional in STATUS and the ledger, add it to
  `owner-questions.md` and continue.
- **Never edit `server/`.** Edit `legacy/` only to fix a quest source whose mistake stops a
  script, under ADR-0004's rule; never to get past any other blocker.
- **Ask the owner** with AskUserQuestion only when every remaining row is blocked.

## progress.md

After each push, rewrite `.scratch/migration/progress.md` with:

- the last ledger section and its commit;
- the Parity counts;
- the next system and why;
- the number of open owner questions;
- anything left running or left uncommitted on purpose.

## Standing rules

**Git**
- Commit on main and push right after each section. Never branch.
- End every commit message with the `Claude-Session:` trailer. Never add a Co-Authored-By trailer.
- Stage with `git add -A -- . ':!.scratch/tmp'`. The files of `.scratch/migration/` are part of
  each commit: left out of the staging, `progress.md` once stood 450 sections behind in Git.
- Never run an unrestricted `git clean`, `git checkout .` or `git stash -u`.
- Only append ledger sections. Never touch `server/`. Touch `legacy/` only for a quest source
  whose mistake stops a script (ADR-0004's rule, "When something needs the owner" above).

**Rust 1.100.0** (pinned in `rust-toolchain.toml`, ledger 684)
- clippy pedantic with `-D warnings`.
- No new `#[allow]`, no `unsafe`, no items after statements.
- 100 columns, and `try_from` instead of `as`.
- No new crate without the owner.
- The edition is 2021. Edition 2024 (and with it let-chains) is the owner's decision, not a
  section's.

**Tools**
- `cargo` is rustup's proxy (`~/.cargo/bin/cargo`), which reads the pin. `/usr/bin/cargo` is the
  distribution's 1.85.1 and must not build this tree; check `cargo --version` says 1.100.0.
- Use `command grep -a`.
- Gates: run `DATABASE_URL=postgres://prodomo:prodomo-test@127.0.0.1:55432/prodomo
  tools/gates.sh $J` in the background, then read `$J/summary.txt`. It covers fmt, build, clippy,
  the tests without and with `DATABASE_URL` (naming every test that failed), the doc-tests,
  rustdoc, and what the tests left behind (temporary files, servers, scratch databases, core
  files), which fails the run. Never run it beside another test run or a sweep.
- Use psql only through `podman exec prodomo-pg18 psql -U prodomo -d prodomo`.
- Keep scratch files in `.scratch/tmp/`, not in `/tmp`, and delete them when the section lands.
  Stop a test run with Ctrl-C or SIGTERM to `cargo`, never `kill -9` on a test binary.

**Secrets**
- Never log a secret, and keep credentials out of the repo.
- `prodomo.toml` is ignored at any depth.

**Code**
- Every server line goes through `prodomo::chat_line::chat_packet`.
- A quest API name that no ported system backs stays "not ported" and has a STATUS row.
- A test that runs the binary reads its stdout and stderr while it waits. A pipe here can hold
  8 KiB (229f.7).
- A new scenario runs with `test_server = false` and grants the GM level it needs (ADR-0007).
- A new scenario follows `AGENTS.md`, "SCENARIO TIMING": enough tries for a draw, whole
  seconds from a fresh second, nothing unrelated inside a legacy window, and the save line
  before SQL on a saved column.
- A rule that needs only a character, a map and the Game data goes in `world`, not in
  `prodomo/src/game_state/` (`docs/STATUS.md`, package F).
