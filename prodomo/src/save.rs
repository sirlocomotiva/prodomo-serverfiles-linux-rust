//! When a character's row is written, as legacy's save cycle and disconnect decide it.
//!
//! There is no game thread yet, so the two things that scheduled a save in legacy are modelled
//! as decisions rather than as a scheduler: [`judge_save_real`](crate::save::judge_save_real) is
//! `CHARACTER::SaveReal`'s two guards, and
//! [`judge_disconnect_save`](crate::save::judge_disconnect_save) is the `FlushDelayedSave` branch
//! at the end of `CHARACTER::Disconnect`.
//! [`event_period`](crate::save::event_period) and
//! [`drain_period`](crate::save::drain_period) convert legacy's Pulse counts
//! into the wall-clock intervals the Rewrite's timers use, which is the one conversion
//! `AGENTS.md` allows: the `[game]` key is written in seconds and legacy multiplied it into
//! Pulses while parsing, so dividing back recovers the value the configuration actually carried.

use std::time::Duration;

use db::players::{Character, PlayerSave};

/// The legacy Pulse rate, `passes_per_sec` (`G/config.cpp:27`). It is 25 in every legacy
/// deployment and is configurable only to break the client.
pub const PASSES_PER_SEC: u32 = 25;

/// How `CHARACTER::SaveReal` ended, in source order (`G/char.cpp:1656-1664`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveRealOutcome {
    /// `m_bSkipSave` is set. `Disconnect` sets it as its last statement (`G/char.cpp:1800`,
    /// "after this, do not save any more") and never clears it, so a character past that point
    /// writes nothing. Nothing is sent and no row changes.
    Skipped,
    /// `!GetDesc()`. Legacy logs `Character::Save : no descriptor when saving` and returns,
    /// so a character with no descriptor is not written. The Rewrite is stricter: it never
    /// reaches this arm, because a row is only written from a descriptor that still holds one.
    NoDescriptor,
    /// The row is written.
    Write,
}

/// `CHARACTER::SaveReal` as a decision.
///
/// `skip_save` is `m_bSkipSave` and `has_descriptor` is `!GetDesc() == false`.
pub const fn judge_save_real(skip_save: bool, has_descriptor: bool) -> SaveRealOutcome {
    if skip_save {
        SaveRealOutcome::Skipped
    } else if !has_descriptor {
        SaveRealOutcome::NoDescriptor
    } else {
        SaveRealOutcome::Write
    }
}

/// How the save at the end of `CHARACTER::Disconnect` was reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisconnectSave {
    /// `FlushDelayedSave` found the character queued, so it removed it from the queue and
    /// called `SaveReal` (`G/char_manager.cpp:803-813`). The character is no longer in the
    /// queue the drain will sweep.
    Flushed,
    /// `FlushDelayedSave` returned `false`, so the character was not queued and `Disconnect`
    /// called `SaveReal` itself (`G/char.cpp:1786-1789`). The row is still written; only the
    /// route differs, and legacy's log line is the only way to tell them apart.
    Unqueued,
}

/// The `if (!CHARACTER_MANAGER::instance().FlushDelayedSave(this))` branch
/// (`G/char.cpp:1786-1789`) as a decision.
///
/// Both arms write the row, so this is not a behaviour difference. It is kept because the
/// queue has to be drained either way, and a Rewrite that recorded only the `false` arm would
/// leave the queued entry for the drain to write a second time.
pub const fn judge_disconnect_save(queued: bool) -> DisconnectSave {
    if queued {
        DisconnectSave::Flushed
    } else {
        DisconnectSave::Unqueued
    }
}

/// The interval `StartSaveEvent` uses: `event_create(save_event, info, save_event_second_cycle)`
/// (`G/char.cpp:5092`).
///
/// Legacy stores the cycle as `cycle * passes_per_sec` Pulses (`G/config.cpp:934`) and the
/// event fires on a Pulse boundary, so this divides it back. The result is the same wall-clock
/// interval either way, and it keeps the `[game]` key in the unit the configuration file was
/// written in, which `AGENTS.md` requires.
///
/// A zero cycle is refused here rather than turned into a busy loop. `ServerConfig::validate`
/// already refuses `save_event_second_cycle = 0` (`common/src/config.rs:516`), so this arm is
/// the last line of defence for a caller that is not the configuration.
pub const fn event_period(cycle_seconds: u32, passes_per_sec: u32) -> Option<Duration> {
    if cycle_seconds == 0 || passes_per_sec == 0 {
        return None;
    }
    // Legacy holds the cycle as `cycle * passes_per_sec` Pulses and the event fires on a Pulse
    // boundary, so the interval is the cycle in seconds. The multiply is still performed, and
    // still refused on overflow, because that is the value the configuration is constrained to
    // fit.
    match cycle_seconds.checked_mul(passes_per_sec) {
        Some(_) => Some(Duration::from_secs(cycle_seconds as u64)),
        None => None,
    }
}

/// The interval the save queue is drained on: `pulse % (passes_per_sec + 4)`
/// (`G/main.cpp:276`).
///
/// The `+ 4` is legacy's, not a rounding of the Rewrite's. It is the drift the 29-Pulse period
/// adds against a 25-Hz game loop, and keeping it keeps the drain at 1.16 seconds rather than a
/// clean second, which is the "at most a few seconds late" ADR-0003 describes.
pub const fn drain_period(passes_per_sec: u32) -> Option<Duration> {
    if passes_per_sec == 0 {
        return None;
    }
    match passes_per_sec.checked_add(4) {
        Some(pulses) => match pulses.checked_mul(1000) {
            Some(millis) if millis / passes_per_sec > 0 => {
                Some(Duration::from_millis((millis / passes_per_sec) as u64))
            }
            _ => None,
        },
        None => None,
    }
}

/// The playtime a save writes, and the remainder the next session starts from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Playtime {
    /// `POINT_PLAYTIME`, the whole minutes the character has played.
    pub minutes: i32,
    /// `dwPlayedTime % 60000`, which `ResetPlayTime` hands back so the sub-minute part is not
    /// lost. It is not stored: it is folded into the next session's elapsed time by
    /// `m_dwPlayStartTime = get_dword_time() - dwTimeRemain`.
    pub remainder_ms: u32,
}

/// `POINT_PLAYTIME` as `CreatePlayerProto` accumulates it (`G/char.cpp:1519-1550`).
///
/// `dwPlayedTime` is `get_dword_time() - m_dwPlayStartTime`, and `get_dword_time` is
/// milliseconds since boot in a `DWORD` (`server/server/libthecore/utils.cpp:467-472`), so the
/// elapsed time is an unsigned 32-bit millisecond count and the subtraction wraps every ~49.7
/// days. The guard is `> 60000`, strictly greater, so a session of exactly 60.000 seconds banks
/// nothing and one of 60.001 banks a minute.
pub fn playtime(stored_minutes: i32, elapsed_ms: u32) -> Playtime {
    if elapsed_ms > 60_000 {
        Playtime {
            minutes: stored_minutes.wrapping_add(banked_minutes(elapsed_ms)),
            remainder_ms: elapsed_ms % 60_000,
        }
    } else {
        Playtime {
            minutes: stored_minutes,
            remainder_ms: elapsed_ms,
        }
    }
}

/// `elapsed_ms / 60_000` as an `i32`.
///
/// The largest quotient a 32-bit millisecond count can produce is 71 582, so the conversion
/// cannot fail and the fallback arm is unreachable; it is written rather than asserted because a
/// cast here would be a silent wrap on a value nobody has measured the range of. The legacy type
/// is a `DWORD` minutes counter too, so a real overflow is legacy's own and is left to `wrapping_add`
/// on the sum.
fn banked_minutes(elapsed_ms: u32) -> i32 {
    i32::try_from(elapsed_ms / 60_000).unwrap_or(i32::MAX)
}

/// The position a save writes, which is the avatar's live position and not the row's.
///
/// Legacy writes `m_posWarp` when either axis is nonzero and the live position otherwise
/// (`G/char.cpp:1551-1565`), so a character with a pending Warp is stored at its destination
/// and a character without one at where it stands. The Rewrite has no pending Warp outside the
/// login home move of ledger 189, which writes its own row, so the live position is the whole
/// rule here and the pending case is left to the Warp that sets one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SavePosition {
    /// `GetX()`.
    pub x: i32,
    /// `GetY()`.
    pub y: i32,
}

/// The row one save writes, built from what the loading phase read and where the avatar stands.
///
/// Only the position is taken from the avatar. Every other column is the value the load
/// returned, because nothing in the Rewrite changes it yet: no inventory, no skill, no stat.
/// A save therefore rewrites the row it read, which is what legacy does on an idle cycle, and
/// the next slice that changes a column changes this function with it.
pub fn player_save(character: &Character, position: SavePosition, played: Playtime) -> PlayerSave {
    PlayerSave {
        level: character.level,
        exp: character.exp,
        conqueror_level: character.conqueror_level,
        conqueror_exp: character.conqueror_exp,
        st: character.st,
        ht: character.ht,
        dx: character.dx,
        iq: character.iq,
        hp: character.hp,
        sp: character.sp,
        stamina: character.stamina,
        gold: character.gold,
        voice: character.voice,
        part_base: character.part_base,
        main_part: character.main_part,
        hair_part: character.hair_part,
        sash_part: character.sash_part,
        x: position.x,
        y: position.y,
        skill_group: character.skill_group,
        playtime_minutes: played.minutes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A character with a value in every column, built the way `load_character` returns one.
    fn hero() -> Character {
        Character {
            id: 7,
            slot: 1,
            name: "Arges".to_string(),
            empire: 2,
            job: 5,
            level: 9,
            exp: 11,
            conqueror_level: 3,
            conqueror_exp: 13,
            sungma_str: 53,
            sungma_hp: 59,
            sungma_move: 61,
            sungma_immune: 67,
            st: 17,
            ht: 19,
            dx: 23,
            iq: 29,
            hp: 31,
            sp: 37,
            stamina: 41,
            gold: 43,
            voice: 47,
            part_base: 2,
            main_part: 1001,
            hair_part: 1002,
            sash_part: 1003,
            x: 469_300,
            y: 964_200,
            skill_group: 5,
            playtime_minutes: 12,
            change_name: false,
        }
    }

    #[test]
    fn a_save_takes_the_position_from_the_avatar_and_the_rest_from_the_row() {
        let save = player_save(
            &hero(),
            SavePosition {
                x: 470_100,
                y: 950_100,
            },
            Playtime {
                minutes: 13,
                remainder_ms: 20_000,
            },
        );
        // The avatar stands somewhere other than where the row was written, and the save follows
        // the avatar: that is the whole point of a save.
        assert_eq!((save.x, save.y), (470_100, 950_100));
        assert_ne!((save.x, save.y), (469_300, 964_200));
        assert_eq!(save.playtime_minutes, 13);
        // Every other column is the row's own value, copied through.
        assert_eq!(save.level, 9);
        assert_eq!(save.exp, 11);
        assert_eq!(save.conqueror_level, 3);
        assert_eq!(save.conqueror_exp, 13);
        assert_eq!(save.st, 17);
        assert_eq!(save.ht, 19);
        assert_eq!(save.dx, 23);
        assert_eq!(save.iq, 29);
        assert_eq!(save.hp, 31);
        assert_eq!(save.sp, 37);
        assert_eq!(save.stamina, 41);
        assert_eq!(save.gold, 43);
        assert_eq!(save.voice, 47);
        assert_eq!(save.part_base, 2);
        assert_eq!(save.main_part, 1001);
        assert_eq!(save.hair_part, 1002);
        assert_eq!(save.sash_part, 1003);
        assert_eq!(save.skill_group, 5);
    }

    #[test]
    fn a_skipped_character_is_never_written() {
        assert_eq!(judge_save_real(true, true), SaveRealOutcome::Skipped);
        // `m_bSkipSave` is checked first, so a skipped character with no descriptor reports the
        // skip, exactly as `SaveReal`'s first `if` does.
        assert_eq!(judge_save_real(true, false), SaveRealOutcome::Skipped);
    }

    #[test]
    fn a_character_with_no_descriptor_is_not_written() {
        assert_eq!(judge_save_real(false, false), SaveRealOutcome::NoDescriptor);
    }

    #[test]
    fn a_live_character_is_written() {
        assert_eq!(judge_save_real(false, true), SaveRealOutcome::Write);
    }

    #[test]
    fn a_queued_character_is_flushed_and_an_unqueued_one_saved_directly() {
        assert_eq!(judge_disconnect_save(true), DisconnectSave::Flushed);
        assert_eq!(judge_disconnect_save(false), DisconnectSave::Unqueued);
    }

    #[test]
    fn a_session_of_sixty_seconds_exactly_banks_nothing() {
        // The guard is `> 60000`, not `>=`, so exactly sixty seconds is not a minute yet.
        assert_eq!(
            playtime(7, 60_000),
            Playtime {
                minutes: 7,
                remainder_ms: 60_000
            }
        );
    }

    #[test]
    fn a_millisecond_over_sixty_seconds_banks_a_minute_and_carries_the_rest() {
        assert_eq!(
            playtime(7, 60_001),
            Playtime {
                minutes: 8,
                remainder_ms: 1
            }
        );
        assert_eq!(
            playtime(0, 150_000),
            Playtime {
                minutes: 2,
                remainder_ms: 30_000
            }
        );
    }

    #[test]
    fn a_session_under_a_minute_carries_its_whole_elapsed_time() {
        assert_eq!(
            playtime(3, 59_999),
            Playtime {
                minutes: 3,
                remainder_ms: 59_999
            }
        );
        assert_eq!(playtime(3, 0).minutes, 3);
    }

    #[test]
    fn the_remainder_is_what_carries_a_short_session_into_the_next_one() {
        // Two 40-second sessions bank nothing on their own. Legacy keeps the second session's
        // `m_dwPlayStartTime` at `now - remainder`, so the next elapsed time is 80 seconds and
        // banks the minute the pair earned.
        let first = playtime(0, 40_000);
        assert_eq!(first.minutes, 0);
        let second = playtime(first.minutes, 40_000 + first.remainder_ms);
        assert_eq!(second.minutes, 1);
        assert_eq!(second.remainder_ms, 20_000);
    }

    #[test]
    fn the_save_event_period_is_the_configured_number_of_seconds() {
        // Legacy multiplies by 25 Pulses while parsing (`G/config.cpp:934`) and the event fires
        // on a Pulse boundary, so the Rewrite's interval is the same number of seconds.
        assert_eq!(
            event_period(120, PASSES_PER_SEC),
            Some(Duration::from_secs(120))
        );
        assert_eq!(
            event_period(180, PASSES_PER_SEC),
            Some(Duration::from_secs(180))
        );
        assert_eq!(
            event_period(1, PASSES_PER_SEC),
            Some(Duration::from_secs(1))
        );
    }

    #[test]
    fn a_zero_or_unusable_save_event_cycle_is_refused() {
        assert_eq!(event_period(0, PASSES_PER_SEC), None);
        assert_eq!(event_period(1, 0), None);
        assert_eq!(event_period(u32::MAX, PASSES_PER_SEC), None);
    }

    #[test]
    fn the_drain_is_the_legacy_twenty_nine_pulses() {
        // `pulse % (passes_per_sec + 4)` at 25 Pulses a second is 29/25 s, which is the
        // 1.16 seconds the ledger records rather than a clean second.
        assert_eq!(
            drain_period(PASSES_PER_SEC),
            Some(Duration::from_millis(1160))
        );
    }

    #[test]
    fn a_drain_rate_of_zero_is_refused_rather_than_dividing() {
        assert_eq!(drain_period(0), None);
    }

    #[test]
    fn a_drain_rate_that_overflows_the_addition_is_refused() {
        assert_eq!(drain_period(u32::MAX), None);
    }
}
