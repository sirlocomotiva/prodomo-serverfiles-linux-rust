# 01. Scenarios that fail in the full suite and pass alone

Status: ready-for-agent

`docs/STATUS.md`, next steps A2 and C7: a scenario that fails in the full suite is fixed or filed
here; "passes alone" does not close it. More than 100 ledger receipts before 679 record such a
loss without a cause.

## What was seen (2026-11-15, 16 cores, `DATABASE_URL` set, default test threads)

Every full run but one lost one scenario, a different one most times. Each passed alone
afterwards. Ledger 683 read each failure against the legacy rule the scenario tests, and three of
the four scenarios had a cause in the scenario itself, none of them the load:

| run | lost scenario | cause | state |
|---|---|---|---|
| 679, before any change | `a_premium_cast_sees_the_biting_fish_before_the_take` | not known: the failure's output was not kept | open |
| 679, after the default changes | none | | |
| 680, third run; 683, second run | `a_cape_of_courage_gathers_the_dogs_around` | **The rule's own dice.** `AggregateMonster` starts each monster on a one-in-two draw (`G/char_battle.cpp:3663`); the scenario placed five dogs, so none came in one run of 32 and the read for the swing timed out (`parity/src/client.rs:80`). | fixed in 683: sixteen dogs, here and in `a_gm_aggregates_and_forgets_the_dogs_around`, which had the same five |
| 680, fourth run; 682 | `an_auto_potion_lays_its_row_pays_each_tick_and_toggles_off` | **The rule's own clock.** The use throttle compares whole seconds of the wall clock (`get_global_time() - last_use_time < 1`, `G/char_item.cpp:5370`); the scenario sent its two uses at any moment, and the server handles them about one Pulse apart, so when they straddled a second the second use toggled the potion off instead of hearing the wait line. Reproduced on purpose: aimed 30 ms before a second's end, five failures of six with the same message. | fixed in 683: the pair starts early in a second (`wait_for_a_fresh_second`) |
| 683, first run | `a_hide_is_refused_in_the_move_window_then_stored_and_restored_on_relogin` | **Unrelated work inside a legacy window.** The first `/hide_costume 1` must land inside the 1.5-second move window that starts at Alpha's Show; the scenario logged a second character in first, about 1.2 s on an idle machine and more in a full run. | fixed in 683: the second character enters first |

In the second 680 run `the_quest_window_runs_the_letters_info_and_the_quest_moves_on` failed at
the `client.rs:80` read, but that run also held a wrong quest number in the scenario, so it is
not counted.

## What the load is, measured (ledger 683)

One scenario alone (`the_exchange_scroll_moves_alpha_to_the_yellow_empire`) takes 5.8 s of wall
time and 4.4 s of CPU. Its server needs 3.45 s from "Configuration loaded" to "Accepting clients"
in the debug build, and each login spends 0.31 s in the argon2id check. 519 scenarios are about
2,300 CPU-seconds, so the 16 cores are full for the whole of the 490-second run. No Pulse duration
is recorded yet (C1), so nothing shows whether a game thread runs late under that load; no loss
so far needed that to explain it.

## What to do

1. For each new loss, keep the failure's output (`tools/gates.sh` keeps `test-db.log`) and read
   it against the legacy rule first. Look for:
   - a rule with a draw, tested with too few tries for the draw to be sure;
   - a rule counted in whole seconds of the wall clock, with steps sent at any moment;
   - a legacy window that starts at an enter, a Show or a use, with a login, a `quiet` or a
     `sleep` between that event and the step that needs the window.
2. Sweep the scenarios for the same three classes before they lose a run. Where ledger 683
   left it:
   - draws: the only two regen lines with five monsters beside `AggregateMonster` are fixed;
     other draws were not looked for;
   - whole seconds: `get_global_time() - x < n` occurs five times in the legacy game source
     (`char.cpp:9735`, `char_item.cpp:4946`, `:5370`, `:5564`, `desc_client.cpp:78`); only
     `:5370` is fixed. `parity.rs` has 18 `send_record` pairs with no read between them, four
     of them item uses (the two potions, two firework pairs); none but the auto potion was
     read against its rule;
   - windows: look for `enter_world` between a character's enter and an "inside the window"
     step.
3. Only a loss none of these explains is a load question; it waits for C1's Pulse duration
   record.
4. A receipt keeps naming each first-run loss. This issue closes when five full runs in a row
   lose none.
