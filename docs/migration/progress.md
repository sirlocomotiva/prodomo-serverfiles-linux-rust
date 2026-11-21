# Migration progress

Read this first after a compaction or in a new session. The goal is
`docs/migration/migration-goal.md`; this folder was `.scratch/migration/` until ledger 745. This
file is rewritten after every section and goes into the section's commit.

## Where it stands

- The last section is **ledger 745**, reorganizing the repository layout (`crates/`, root `docker-compose.yml`, `docs/parity/`, `docs/migration/`, `tools/probes/`, `.scratch/` removed, stale `.overlord/` worktrees cleaned). **Packages A–D are all closed** (ledger 744 finished Package D). Package B
  so far: 687 (sibling vnums, the gold bar cap), 688 (the ability potions), 689 (the apply types),
  690 (the arena's potion gate), 691 (the ward and aid climb gate, and the refused `/setskill`
  sending byte 76), 692 (each skill type's point on a climb and a forget), 693 (each skill's hit cap on one victim), 694 (the master arms of a climb by point), 695 (each automatic potion's kind of row), 696 (the basic books and two book-read bugs), 697 (the passive books' ladder), 698 (the sibling capes and peach), 699 (the white flag and the treasure box), 700 (the zen bean), 701 (the no-op item types), 702 (the seasonal boxes), 703 (a unique worn by its use), 704 (the ramadan candy and the marriage ring), 705 (a fish's use), 706 (the wearable types worn by their use), 707 (a use refused by its limits), 708 (the craft, mining and horse books), 709 (the shaman unlock line), 710 (a use with no arm), 711 (the bonus change and the order of a scroll's records), 712 (the fifth bonus, the rare change and the belt jewel), 713 (the pet and mount reinforcements), 714 (the giftbox with no group and the element change), 715 (the stone scroll and the shaman unlock), 716 (the tuning and detachment scrolls), 717 (the metin stone failure and wear checks), 718 (the recipe refusal), 719 (the plain blend), 720 (the guild skills as personal skills), 721 (the unworn dragon-soul stone), 722 (the capped bonuses and immunities), 723 (the item-use quest and special checks), 724 (the 508xx potions and potion-nodelay), 725 (the detectors and the portal payment), 726 (the clock-fix scenarios), 727 (the horse skills and `skill-arms.md` closure), 728 (the fish mind pill, dragon-soul pull-out and `item-use.md` closure). Before them: 686 (the tail made countable: three generated arm tables and every `partial` and `missing` note ending in `Left:`), 685 (the scenario timing), 684 (Rust 1.100.0), 683 (the test hygiene), 682 (the cleanup), 681 (ADR-0008), 680 (the owner's answers) and 679 (the direction review).
- Parity: 1,812 `ported`, 0 `partial`, 520 `unused`, 0 `missing`, 0 `codec`, of 2,332 rows.
  **0 open rows left** (all 12 inventory files and B1–B9 are closed).
- Tests: 5,797 pass in 46 targets with `DATABASE_URL`; 608 are scenarios.
- **What is next is in `docs/STATUS.md`, "Next steps"**: package E (the owner's play test with the Reference client following `docs/PLAYTEST.md`, plus native Docker verification on a Docker host).

## Where things are

- `AGENTS.md` ("WORKSPACE") has the map of everything outside the crates; "TEST HYGIENE" has the
  rules a test and a session follow so nothing is left behind; "SCENARIO TIMING" has the rules
  that keep a scenario passing in a full run.
- Gates: `tools/gates.sh <output-dir>`, then read `<output-dir>/summary.txt`. It fails on a
  failed gate and on anything the tests left (`temp-files`, `servers`, `scratch-dbs`, `cores`),
  and prints the `allows` count, which may not rise.
- Mutation sweeps: `tools/mutate.py`, on a copy of the tree under `.tmp/`.
- Probes: `tools/probes/` (its README says how to build one).
- History, never a rule: `docs/STATUS_HISTORY.md`, `docs/PROTOCOL_NOTES_HISTORY.md`.
- A finished issue, a research note or a one-off script is not kept in the tree. `.tmp/` is
  ignored and is emptied when a section lands; nothing tracked may cite a file in it.

## Owner decisions in force

- 2026-10-14: an in-memory test may prove a row when a binary scenario is impractical; Defects
  and backdoors are fixed, not carried over; Channel 1 and the Shared Channel run.
- 2026-10-15: mutation sweeps wait for a stronger machine. Answered 2026-11-15: this 16-core
  machine is the one, so package C7's sweeps by risk may start. The 450 deferred per-section
  sweeps are **not** owed any more.
- 2026-11-15: the play test is last and out of scope until the owner opens it; the test-server
  mode ships off (ADR-0007); the save cycle is 10 seconds; the `unused` rows stand; ADR-0006 is
  accepted; `change_empire.lua` is fixed, the one edit to a quest source (ADR-0004); the drop
  limit is 1000 ms; the game logs to keep are in `docs/STATUS.md`.
- 2026-11-15 (ADR-0008): a legacy bug, inconsistency, vulnerability or backdoor is fixed while
  porting, without a question, and recorded; never ported "provisionally". Odd but deliberate
  behaviour stays as a Quirk. A difference the Rewrite's own design causes is not an
  improvement. A quest source is edited only where a mistake stops a script (ADR-0004).

## The machine

- 16 cores, 30 GB. Rust 1.100.0 comes from rustup through `rust-toolchain.toml`: use
  `~/.cargo/bin/cargo` (put `~/.cargo/bin` first in `PATH`). `/usr/bin/cargo` is the
  distribution's 1.85.1 and must not build this tree; `tools/gates.sh` refuses it.
- PostgreSQL 18 is the Podman container `prodomo-pg18` on `127.0.0.1:55432`. There is no `psql`
  on the host: use `podman exec prodomo-pg18 psql -U prodomo -d prodomo`.
- `i686-linux-gnu-g++-12` 12.4.0 is installed (ledger 680); package C8 probes the records.
- `docker-compose.yml` builds and starts with Podman or Docker (ledgers 680 & 745). The images
  `rust:1.100.0-bookworm`, `debian:bookworm-slim` and `localhost/prodomo:local` are on the
  machine. With Podman, never `podman compose restart`: it ended the server without the SIGTERM.
- rustup has the one toolchain, `1.100.0`; the 1.85.1 image and toolchain are removed (685).
- Clean since ledger 683 (and ledger 745): no scratch database, no test server, no `prodomo-*`
  entry in `/tmp`, no Git worktree but the main one, and no `.tmp/` or `.scratch/` leftovers.

## Left running or uncommitted

Nothing is left running. Ledger 745 is committed and pushed.
