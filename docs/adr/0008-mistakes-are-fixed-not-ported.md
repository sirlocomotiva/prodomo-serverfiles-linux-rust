# Parity carries the game over; legacy's mistakes are fixed, not ported

Status: accepted (owner, 2026-11-15; first stated 2026-10-14)

The Rewrite stays close to the Legacy server for one reason: so that what reaches the Reference
client is the actual game and not an approximation of it. Staying close is not a duty to copy.
The owner decided that the port should improve the old source: where legacy's behaviour is a
mistake, the Rewrite fixes it as the code is ported, without asking first, and records the fix.

A mistake is any of these:

- **Unsafe behaviour:** a crash, memory corruption, lost or duplicated items or gold, injection,
  a length, index or slot taken from the client unchecked.
- **A backdoor or an open door:** a default credential, an authority nobody granted, a command
  or path that skips its own check.
- **An inconsistency:** code that contradicts what it plainly means to do. A value that is parsed
  and never used, a variable named for one thing and filled with another, a line said when
  nothing happened, a dialog that cannot reach its end, two paths to one result that charge
  differently by accident.

What stays as it is:

- **Odd but deliberate behaviour is the game.** A rule the code states on purpose, or a switch
  the owner's build defines, is reproduced as a Quirk. Changing it is a change of design, and
  design is the owner's.
- **A difference the Rewrite's own design causes is not an improvement.** An order of records
  that differs because the world runs after the script, or a wait counted differently, is
  either brought back to legacy or argued as a Divergence on its merits. This decision does not
  cover it.

## Considered options

- **Bug-for-bug Parity.** Rejected: it would carry known holes and dead dialogs into a new
  server, and the play test would spend its time on bugs both servers share.
- **Free redesign.** Rejected: without the Legacy server as the reference nothing shows that
  the game was carried over.

## Consequences

- No question is asked before a mistake is fixed. The fix is recorded where the next reader
  looks: a Defect row in `docs/STATUS.md` with the legacy lines, a Divergence row when a player
  or the client can tell the difference, the ledger section, and a test of the fixed behaviour.
  A fix nobody recorded is drift, not improvement.
- A fix that changes numbers every player meets (a hit or evasion formula, a spawn or drop
  weight) is made the same way and is also listed under "Waiting on the owner" in
  `docs/STATUS.md`, so the owner can name it a Quirk. The work does not wait for the answer.
- "Reproduced, provisional" is no longer a resting state for a mistake. The agents' rule "take
  the option that follows legacy and ask" now applies only to deliberate behaviour.
- A quest source with a mistake that stops a script may be fixed under the rule ADR-0004 now
  carries; nothing else in a quest source is edited.
- A security rule needs no legacy precedent. A limit, a deadline or a check that legacy lacks is
  added because the server needs it (package C in `docs/STATUS.md`).
