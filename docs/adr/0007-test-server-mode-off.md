# The Rewrite ships with the test-server mode off

Status: accepted (owner, 2026-11-15)

Legacy compiles `test_server` on (`G/config.cpp:72`) and none of the owner's `CONFIG` files turns
it off, so the legacy deployment ran in the test-server mode: `GetGMLevel()` answered
`GM_IMPLEMENTOR` for every character (`G/char.cpp:6354-6358`, `G/gm.cpp:53`), the restarts skipped
their waits, and test-only lines were sent. Parity alone would keep all of that. The owner decided
otherwise: `[game] test_server` defaults to off, and game-master authority comes only from a GM
grant (`prodomo gm`). A server where every player is an implementor is a Defect of the deployment,
not behaviour to preserve.

## Considered options

- **On, with legacy's full meaning.** What the owner's players had. Refused, because any player
  could run any GM command.
- **On, with the partial meaning the Rewrite had grown** (provisional since ledger 228, owner
  question 5). Some gates read the mode on their own (the whisper type, the messenger's GM level,
  the build lines, `/reload`) and others read the stored GM level (a quest's `pc.is_gm`, V26), so
  nothing said what the mode granted.

## Consequences

- This is a Divergence from the owner's legacy deployment. The mode stays a `[game]` key; when
  it is on it means what legacy means in every gate, through one function that answers a
  character's GM level. The gates that read the mode on their own move to that function.
- The scripted-client scenarios were written with the mode on. The harness (`parity::Server`)
  keeps it on for a scenario that does not name the key, so no scenario changed meaning in the
  change that flipped the default. Each scenario moves to the shipped mode, with a GM grant where
  it plays a game master, and the harness default is removed when the last one has moved. Until
  then a refusal that an ordinary player hears is proved over the wire only by a scenario that
  sets `test_server = false`.
- The production arms that legacy skipped under the mode (the restart waits and their `IsHack`
  checks, V18i) stop being provisional: they are the behaviour.
- `GameState` still starts its own unit-test worlds with the mode on; that default moves with the
  scenarios.
