//! The mob motion clips: each monster folder's `WALK` and `RUN` speed.
//!
//! Legacy `CMotionManager::Build` (`server/server/game/motion.cpp:326-354`) inserts a motion
//! set for every proto row whose folder is not empty, then loads `MOTION_WALK`, `MOTION_RUN`
//! and `MOTION_NORMAL_ATTACK` into it through `LoadMotion` (`:133-157`). Each file comes from
//! `GetMotionFileName` (`:73-131`): the first `motlist.txt` line of four or more cells whose
//! second cell matches the motion's field case-insensitively (`WALK`, `RUN`, `NORMAL_ATTACK`),
//! a missing `motlist.txt` naming nothing. A RUN clip whose `Accumulation` y is 0 is read and
//! dropped, never inserted (`:146-148`); the sixteen owner folders whose RUN clip has no
//! carry read the same.
//!
//! A speed reads two of the clips (`GetMoveMotionSpeed`, `G/char.cpp:3555-3577`): a player
//! with no mount reads its race's walk or run, and every non-player reads the run's (`IsPC`
//! gates the walk). The clip's speed is `-accY / duration`, the same [`crate::pc_motion`]
//! number. This module loads the two clips' speeds per folder the protos name; the attack
//! duration and skill splash already live in [`crate::mob_skill_data::MobSkillClips`].

use std::collections::BTreeMap;
use std::path::Path;

use crate::mob_proto::MobProtos;
use crate::pc_motion::parse_clip;

/// The `motlist.txt` field of [`MobMoveClips::walk_s`].
const WALK_FIELD: &str = "WALK";
/// The `motlist.txt` field of [`MobMoveClips::run_s`].
const RUN_FIELD: &str = "RUN";

/// The WALK and RUN speeds of every monster folder: `CMotionManager`'s move table
/// (`G/motion.cpp:326-354`).
#[derive(Debug, Clone, Default)]
pub struct MobMoveClips {
    /// The speeds per folder, `(walk, run)`, each `None` without the line or the clip.
    by_folder: BTreeMap<String, (Option<f32>, Option<f32>)>,
    /// Every clip that failed to parse (warned, empty — legacy's `sys_err` arm).
    pub skipped: Vec<String>,
}

impl MobMoveClips {
    /// Load the move clips of the folders `protos` names: each folder's `WALK`
    /// and `RUN` file from its `motlist.txt`, parsed like a player clip. A
    /// missing line or file leaves the speed empty; a malformed clip is skipped
    /// and recorded (legacy logs `sys_err` and keeps the empty motion — except
    /// a clip the text reader refuses, which exits legacy's boot where this
    /// skips like the skill clips do). A RUN clip with a zero `Accumulation` y
    /// is dropped, as legacy drops it, and a clip with no `MotionDuration` is
    /// not loaded.
    pub fn load(monster_dir: &Path, protos: &MobProtos) -> Self {
        let mut clips = Self::default();
        for proto in protos.rows() {
            let folder = String::from_utf8_lossy(&proto.table.folder)
                .trim_end_matches('\0')
                .to_string();
            if folder.is_empty() || clips.by_folder.contains_key(&folder) {
                continue;
            }
            let motlist = monster_dir.join(&folder).join("motlist.txt");
            let text =
                std::fs::read(&motlist).map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
            let (mut walk, mut run) = (None, None);
            if let Ok(text) = text {
                let lines: Vec<Vec<&str>> = text
                    .lines()
                    .map(|line| line.split_whitespace().collect())
                    .collect();
                for (field, slot, zero_dropped) in
                    [(WALK_FIELD, &mut walk, false), (RUN_FIELD, &mut run, true)]
                {
                    let file = lines.iter().find_map(|cells| {
                        (cells.len() >= 4 && cells[1].eq_ignore_ascii_case(field)).then(|| cells[2])
                    });
                    let Some(file) = file else { continue };
                    let path = monster_dir.join(&folder).join(file);
                    let parsed = std::fs::read(&path).map_err(|_| ()).and_then(|bytes| {
                        parse_clip(&bytes)
                            .map_err(|_| ())
                            .and_then(|clip| clip.ok_or(()))
                    });
                    match parsed {
                        Ok(clip) if zero_dropped && clip.accumulation_y == 0.0 => {}
                        Ok(clip) => {
                            *slot = Some(clip.speed());
                        }
                        Err(()) => {
                            clips.skipped.push(path.display().to_string());
                        }
                    }
                }
            }
            clips.by_folder.insert(folder, (walk, run));
        }
        clips
    }

    /// The folder's `WALK` speed, or `None` without the line or the clip.
    #[must_use]
    pub fn walk_s(&self, folder: &str) -> Option<f32> {
        self.by_folder.get(folder).and_then(|&(walk, _)| walk)
    }

    /// The folder's `RUN` speed, or `None` without the line, the clip, or its carry.
    #[must_use]
    pub fn run_s(&self, folder: &str) -> Option<f32> {
        self.by_folder.get(folder).and_then(|&(_, run)| run)
    }

    /// Whether any folder loaded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_folder.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mob_proto::MobProto;
    use crate::records::MobTableRecord;

    fn protos_of(folders: &[&str]) -> MobProtos {
        let rows = folders
            .iter()
            .enumerate()
            .map(|(i, folder)| {
                let mut table = MobTableRecord {
                    vnum: 100 + u32::try_from(i).unwrap_or(u32::MAX),
                    ..MobTableRecord::default()
                };
                let bytes = folder.as_bytes();
                table.folder[..bytes.len()].copy_from_slice(bytes);
                MobProto { line: i + 2, table }
            })
            .collect();
        MobProtos::from_rows(rows)
    }

    fn owners() -> MobProtos {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/proto");
        MobProtos::load(&dir).expect("the owner's protos read")
    }

    fn monster_dir() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../legacy/gamedata/data/monster")
    }

    #[test]
    fn the_dog_walks_and_runs_at_its_clips_speed() {
        let clips = MobMoveClips::load(&monster_dir(), &protos_of(&["stray_dog"]));
        let walk = clips
            .walk_s("stray_dog")
            .expect("the WALK line names 02.msa");
        assert!((walk - 47.73 / 0.733_333).abs() < 1e-2, "{walk}");
        let run = clips.run_s("stray_dog").expect("the RUN line names 03.msa");
        assert!((run - 219.19 / 0.5).abs() < 1e-2, "{run}");
        assert!(clips.skipped.is_empty());
    }

    #[test]
    fn a_run_clip_with_no_carry_reads_empty() {
        let clips = MobMoveClips::load(&monster_dir(), &protos_of(&["metinstone"]));
        assert_eq!(clips.run_s("metinstone"), None);
        assert!(clips.skipped.is_empty(), "{:?}", clips.skipped);
    }

    #[test]
    fn a_folder_with_no_motlist_reads_empty() {
        let clips = MobMoveClips::load(&monster_dir(), &protos_of(&["yamachun"]));
        assert_eq!(clips.walk_s("yamachun"), None);
        assert_eq!(clips.run_s("yamachun"), None);
    }

    #[test]
    fn the_match_is_case_insensitive_and_needs_four_cells() {
        let dir = std::env::temp_dir().join(format!("mob-motion-fields-{}", std::process::id()));
        let folder = dir.join("fieldy");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("motlist.txt"),
            "GENERAL walk walk.msa 100\nGENERAL RUN run.msa\nGENERAL Run run.msa 100\n",
        )
        .unwrap();
        std::fs::write(
            folder.join("walk.msa"),
            b"MotionDuration 1.0\nAccumulation 0 -100 0\n",
        )
        .unwrap();
        std::fs::write(
            folder.join("run.msa"),
            b"MotionDuration 0.5\nAccumulation 0 -200 0\n",
        )
        .unwrap();
        let clips = MobMoveClips::load(&dir, &protos_of(&["fieldy"]));
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(clips.walk_s("fieldy"), Some(100.0));
        assert_eq!(clips.run_s("fieldy"), Some(400.0));
    }

    #[test]
    fn a_malformed_clip_is_skipped_and_recorded() {
        let dir = std::env::temp_dir().join(format!("mob-motion-bad-{}", std::process::id()));
        let folder = dir.join("baddy");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("motlist.txt"), "GENERAL RUN run.msa 100\n").unwrap();
        std::fs::write(folder.join("run.msa"), b"group a b\n").unwrap();
        let clips = MobMoveClips::load(&dir, &protos_of(&["baddy"]));
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(clips.run_s("baddy"), None);
        assert_eq!(
            clips.skipped,
            vec![folder.join("run.msa").display().to_string()]
        );
    }

    #[test]
    fn the_owners_folders_load() {
        let clips = MobMoveClips::load(&monster_dir(), &owners());
        assert!(clips.walk_s("horse").is_some());
        assert!(clips.run_s("horse").is_some());
        assert!(clips.run_s("thiefboss3").is_some());
        assert_eq!(clips.walk_s("misterious_diseased_egg"), None);
        assert!(clips.skipped.is_empty(), "{:?}", clips.skipped);
    }
}
