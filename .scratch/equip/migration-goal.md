# Goal: the Rust rewrite, one ledger section at a time (resumes at ledger 229g, 2026-10-14)

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

Keep porting the legacy server in the build order of `docs/STATUS.md`, one ledger section at a time,
until the goal below holds. A finished ledger section is not a place to stop. Commit it, push it,
rewrite `.scratch/equip/progress.md`, and start the next one.

## Where it starts

- main is at `4be92117`:
  - `a5bf49d8` is ledger 229f, the monsters standing on the maps.
  - `4be92117` sets the example config to Channel 1 plus 99.
- Tests: 3280 pass with and without `DATABASE_URL`. There are 87 parity scenarios.
- Parity: 121 ported, 126 partial, 1336 missing, 6 unused and 56 codec, out of 1645 rows.
- Owner questions: 26 are open in `.scratch/equip/owner-questions.md` (Q1 to Q27, less Q14).
- `progress.md` has the details, and supersedes this section once it is newer.

## Topology (owner, 2026-10-14)

- **Channel 1 runs. Channels 2-4 are off.** In the example config they are commented-out tables,
  so they can be turned on again later. Do not remove the code that hosts them.
- **The Shared Channel (99) stays on.** Channel 1's players reach its maps through three routes:
  - the OX event, to map 113;
  - the guild battle and flag wars, to maps 110 and 111;
  - the kingdom and siege war buttons, to map 103.

  The research is in ledger 229f.5. Never turn the Shared Channel off without the owner.
- Measure boot time, memory and the cost per Pulse on Channel 1 plus 99. At 229f that is 65,632
  characters and 291 MB on the release build.

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

## Next: 229g onwards, then the rest of step 4

1. **The rest of 229, the monsters (ledger 229f.4 lists what is not ported).**
   - The regen event and respawn. Each map's `Regen` list and each entry's `first_fire` already
     wait for it.
   - `CG_TARGET`.
   - The monster state machine (`CFSM`) and wandering.
   - Melee, then death and restart.
     - `restart_now`, `restart_here` and `restart_town` are `NotPorted`.
     - Port `ForgetMyAttacker` and the `ReviveInvisible(5)` calls with them.
     - Keep legacy's reset `m_dwKillerPID = 0`.
   - Drops and exp, and the mob's carried items.
   - The group's party and follow.
   - The ore vein's destruction.
   - The monster `.msa` clips and `motlist.txt`.
   - The walk-duration recomputes after a `Goto` move (V2 in 229e).
   - The cost per Pulse on Channel 1 plus 99, once monsters act.
   - In the 229g ledger section, correct the one sentence in 229f.5 that speaks of "the owner's live
     server". Because the ledger is append-only, the fix goes in the new section.
2. Skills and affects.
3. Party, guild, messenger, and the cross-Channel bus that Channel 1 and 99 need.
4. Dungeons, events, guild war and OX.
   - `pc.warp`, `pc.warp_to_guild_war_observer_position` and the war buttons open the Shared
     Channel routes above. The return from a Shared Channel map to Channel 1 (`docs/STATUS.md`,
     Divergences) lands with them.
   - The OX-map login Warp belongs here (`sys.event.ox`).
5. The Prodomo custom systems: sash, aura, pets, battle pass, switchbot, item shop, premium shop and
   the rest.
6. GM commands, with the real GM levels of `interpret`, then the adminpage and the logs.
7. Step 5: finish the importers, fix what the full data set breaks, and write `compose.yaml` and the
   README's from-scratch setup.

## Before each ledger section

- Read `AGENTS.md`, `CONTEXT.md`, `docs/STATUS.md` and the ADRs in `docs/adr/`.
- Read the last section of `docs/REWRITE_LEDGER.md` and `.scratch/equip/progress.md`.
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
4. **Update `docs/STATUS.md`:**
   - the "Last reviewed" line;
   - the summary;
   - the build-order row;
   - the Divergence and Defect rows.
5. **Run the mutation sweep.**
   - Write `.scratch/tmp/tools/mutantsNNN.py` and run it with `mutate_v2.py`, in a tar copy of the
     tree, never in the main tree.
   - Check first with `CHECK=1`.
   - Run the shards one after another, because they share one test database.
   - Raise the `PARITY_OK` floor in `mutate_v2.py` when scenarios are added.
   - Close every survivor with a named test, or show it is equivalent and test the class of real
     defect it hides.
   - Report compile-kills separately from semantic kills.
6. **Run the gates**, then append the ledger section with its receipt:
   - the test counts, by file;
   - the gate table;
   - the Rust files and lines;
   - the toolchain;
   - the parity counts before and after.

   Copy the receipts to `.scratch/tmp/lNNN/`.
7. **Commit and push.**
   - Subject: `Ledger NNN: <system> (<the legacy parts>)`.
   - Run `git push origin main`.
   - Rewrite `progress.md`.

## When something needs the owner

The owner may be away. Never stop to wait.

- **A new or upgraded crate** needs an online `cargo fetch` and the owner's yes. Do not add it.
  Record it in `owner-questions.md`, port what does not need it, and mark the rest blocked.
- **Other decisions:** a Quirk versus a Defect, a Divergence players would see, an ADR to accept, or
  a Game data source that needs fixing.
  - Take the option that follows legacy, unless that option is a Defect.
  - Mark it provisional in STATUS and the ledger.
  - Add it to `owner-questions.md` and continue.
- **Never edit `server/` or `legacy/`** to get past a blocker.
- **Ask the owner** with AskUserQuestion only when every remaining row is blocked.

## progress.md

After each push, rewrite `.scratch/equip/progress.md` with:

- the last ledger section and its commit;
- the Parity counts;
- the next system and why;
- the number of open owner questions;
- anything left running or left uncommitted on purpose.

## Standing rules

**Git**
- Commit on main and push right after each section. Never branch.
- End every commit message with the `Claude-Session:` trailer. Never add a Co-Authored-By trailer.
- Stage with `git add -A -- . ':!.scratch/tmp' ':!.scratch/equip'`.
- Never run an unrestricted `git clean`, `git checkout .` or `git stash -u`.
- Only append ledger sections. Never touch `server/` or `legacy/`.

**Rust 1.85.1**
- clippy pedantic with `-D warnings`.
- No new `#[allow]`, no `unsafe`, no let-chains, no items after statements.
- 100 columns, and `try_from` instead of `as`.
- No new crate without the owner.

**Tools**
- Call `/usr/bin/cargo-fmt fmt --all` and
  `/usr/bin/cargo-clippy clippy -p <crate> --all-targets --locked --offline -- -D warnings`. The
  rustup 1.98 in `~/.cargo/bin` shadows the cargo subcommands.
- Use `command grep -a`.
- Gates: run `mkdir -p $J`, then
  `J=$J DATABASE_URL=postgres://prodomo:prodomo-test@127.0.0.1:55432/prodomo .scratch/tmp/tools/gates.sh`
  in the background, then read `$J/gates.summary`. It covers fmt, build, clippy, the tests with
  and without `DATABASE_URL`, the doc-tests, rustdoc, leftover scratch databases and core files.
  Never run it while a sweep runs.
- Use psql only through `podman exec prodomo-pg18 psql -U prodomo -d prodomo`.
- Keep scratch files in the job's `tmp` directory, not in `/tmp`.

**Secrets**
- Never log a secret, and keep credentials out of the repo.
- `prodomo.toml` is ignored at any depth.

**Code**
- Every server line goes through `prodomo::chat_line::chat_packet`.
- A quest API name that no ported system backs stays "not ported" and has a STATUS row.
- A test that runs the binary reads its stdout and stderr while it waits. A pipe here can hold
  8 KiB (229f.7).
