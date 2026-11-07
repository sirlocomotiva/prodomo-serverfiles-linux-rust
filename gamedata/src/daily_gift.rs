//! The daily-gift weeks: `daily_gift.txt` in the Locale folder.
//!
//! `LoadLocaleFile` reads the file after the Locale strings
//! (`G/input_db.cpp:892-963`); a file it cannot load is a `sys_err` and no
//! groups, not a shutdown. `ReadDailyGiftItemGroup`
//! (`G/item_manager_read_tables.cpp:957-1026`) walks the child groups through
//! `CTextFileLoader` ([`crate::text_file`]): each group names its `week` with
//! `GetTokenInteger` (`str_to_number`, 0 when missing), and the day lines `1`
//! to 7 carry vnum, count and cost as their three values. A missing day line
//! ends the week there; a day with a zero vnum, count or cost is left out
//! (`CDailyGiftItemGroup::AddItem`, `G/item_manager.h:33-39`). A group whose
//! week repeats an earlier one keeps the first (`std::map::insert`).

use std::collections::BTreeMap;
use std::path::Path;

use crate::mob_proto::{strtol, strtoul};
use crate::text_file::{self, TextFileError};

/// The daily-gift file, under the Locale folder.
pub const DAILY_GIFT_FILE: &str = "daily_gift.txt";

/// `DAILY_GIFT_WEEK_DAYS` (`C/length.h`): the day lines each week reads.
pub const DAILY_GIFT_DAY_COUNT: usize = 7;

/// One day's gift: what the week pays and what unlocking it early costs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DailyGiftDay {
    /// The gift's vnum.
    pub vnum: u32,
    /// How many the gift pays.
    pub count: u32,
    /// The cash that unlocks the day early.
    pub cost: u32,
}

/// One `CDailyGiftItemGroup`: its week and its days in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DailyGiftWeek {
    /// The `week` line's number.
    pub week: u8,
    /// The days the file gave, up to seven. A missing day line ends the week.
    pub days: Vec<DailyGiftDay>,
}

/// The weeks of the Locale, keyed by week number.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DailyGiftWeeks {
    /// The weeks in week order. A repeated week keeps its first group.
    pub weeks: BTreeMap<u8, DailyGiftWeek>,
}

impl DailyGiftWeeks {
    /// Read the weeks from the Locale folder's `daily_gift.txt`.
    ///
    /// # Errors
    ///
    /// Only [`TextFileError::GroupNameHasSpace`], which legacy answers with
    /// `exit(1)`. A file that cannot be opened is no weeks, as legacy's
    /// `sys_err` is.
    pub fn load(locale_dir: &Path) -> Result<Self, TextFileError> {
        let path = locale_dir.join(DAILY_GIFT_FILE);
        let Ok(data) = std::fs::read(&path) else {
            return Ok(Self::default());
        };
        let file = text_file::parse(&data)?;
        let mut weeks = BTreeMap::new();
        for group in &file.groups {
            let week = group
                .get(b"week")
                .and_then(|values| values.first())
                .map_or(0, |first| strtol(first));
            let week = u8::try_from(week).unwrap_or(0);
            let mut days = Vec::new();
            for day in 1..=DAILY_GIFT_DAY_COUNT {
                let key = day.to_string().into_bytes();
                let Some(values) = group.get(&key) else {
                    break;
                };
                let mut values = values.iter();
                let (Some(vnum), Some(count), Some(cost)) =
                    (values.next(), values.next(), values.next())
                else {
                    break;
                };
                let (vnum, count, cost) = (strtoul(vnum), strtoul(count), strtoul(cost));
                if vnum != 0 && count != 0 && cost != 0 {
                    days.push(DailyGiftDay { vnum, count, cost });
                }
            }
            weeks.entry(week).or_insert(DailyGiftWeek { week, days });
        }
        Ok(Self { weeks })
    }

    /// The week's group, if the file gave one (`GetDailyGiftItemGroup`).
    #[must_use]
    pub fn group(&self, week: u8) -> Option<&DailyGiftWeek> {
        self.weeks.get(&week)
    }

    /// The weeks the file gave (`GetAllDailyGiftGroupSize`).
    #[must_use]
    pub fn len(&self) -> usize {
        self.weeks.len()
    }

    /// Whether the file gave no week.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.weeks.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::{DailyGiftDay, DailyGiftWeeks, DAILY_GIFT_DAY_COUNT, DAILY_GIFT_FILE};
    use std::path::PathBuf;

    /// The owner's weeks: four full groups, 1 to 4.
    #[test]
    fn the_owner_weeks_load_with_seven_days_each() {
        let locale: PathBuf = [
            env!("CARGO_MANIFEST_DIR"),
            "..",
            "legacy",
            "gamedata",
            "locale",
            "europe",
        ]
        .iter()
        .collect();
        let weeks = DailyGiftWeeks::load(&locale).expect("the owner's file parses");
        assert_eq!(weeks.len(), 4);
        for week in 1..=4u8 {
            let group = weeks.group(week).expect("the week loads");
            assert_eq!(group.days.len(), DAILY_GIFT_DAY_COUNT, "week {week}");
        }
        assert_eq!(
            weeks.group(1).expect("week 1").days[0],
            DailyGiftDay {
                vnum: 55401,
                count: 1,
                cost: 200
            }
        );
        assert!(weeks.group(5).is_none());
    }

    /// A missing file is no weeks, as legacy's `sys_err` is.
    #[test]
    fn a_missing_file_is_no_weeks() {
        let weeks = DailyGiftWeeks::load(PathBuf::from("no-such-dir").as_path())
            .expect("no file is no weeks");
        assert!(weeks.is_empty());
    }

    /// A missing day line ends the week, a zero day is left out, and a repeated
    /// week keeps its first group.
    #[test]
    fn short_zero_and_repeated_weeks_read_as_legacy_does() {
        let dir: PathBuf = std::env::temp_dir().join(format!("daily-gift-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a scratch dir");
        std::fs::write(
            dir.join(DAILY_GIFT_FILE),
            b"Group First\n{\nWeek 1\n1 10 2 30\n2 0 1 5\n3 11 1 40\n}\n\
              Group Again\n{\nWeek 1\n1 99 1 1\n}\n\
              Group Short\n{\nWeek 2\n1 12 1 50\n}\n",
        )
        .expect("the scratch file");
        let weeks = DailyGiftWeeks::load(&dir).expect("the scratch file parses");
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(weeks.len(), 2);
        let first = weeks.group(1).expect("week 1");
        assert_eq!(
            first.days,
            vec![
                DailyGiftDay {
                    vnum: 10,
                    count: 2,
                    cost: 30
                },
                DailyGiftDay {
                    vnum: 11,
                    count: 1,
                    cost: 40
                },
            ]
        );
        let short = weeks.group(2).expect("week 2");
        assert_eq!(short.days.len(), 1);
    }
}
