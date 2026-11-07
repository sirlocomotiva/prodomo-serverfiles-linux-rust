//! The protected inventory (`__ENABLE_INVENTORY_PROTECTED_SYSTEM__`,
//! `G/char.cpp:11866-11974`).
//!
//! The lock is the points' two fields (`POINT_SECURED_STATE` and
//! `POINT_SECURED_PASSWORD`, stored with the row): 0 open, anything else locked.
//! A fresh character is open with the password `999_999` (`player.sql:872-873`).
//! Every guard chats its line and sends nothing; the activate and the change
//! land through the point arms, so the change records ride the caller's outbox.
//!
//! # Defects not reproduced
//!
//! - The change checks the digits of the two first bytes, not the two passwords
//!   (`has_only_digits(strFindFirstForNow)` at `:11941`, where the activate
//!   passes the whole password at `:11878`). The Rewrite checks the wholes.
//!
//! `G/` = `server/server/game/`, `C/` = `server/server/common/`.

use common::point_slot;
use common::vid::Vid;
use protocol::gc_chat::CHAT_TYPE_COMMAND;

use super::GameState;
use crate::chat_line::Arg;
use crate::item_move::{MoveItemRefused, MovedItems, Mover};

/// `SUBHEADER_CG_INVENTORY_PROTECTED_ACTIVATE`: lock or unlock with the password.
pub const PROTECTED_ACTIVATE: u8 = 0;
/// `SUBHEADER_CG_INVENTORY_PROTECTED_PASSWORD_CHANGE`: swap the password.
pub const PROTECTED_PASSWORD_CHANGE: u8 = 1;

/// The longest password, in digits (`INVENTORY_PROTECTED_PASSWORD_MAX_LEN`).
pub const PROTECTED_PASSWORD_MAX_LEN: usize = 6;

/// The password a fresh character carries (`player.sql:873`).
pub const PROTECTED_FRESH_PASSWORD: i32 = 999_999;

/// One decoded `CG_INVENTORY_PROTECTED` request, as
/// `RecvActivateProtectedSystem` reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtectedStep {
    /// `SUBHEADER_CG_INVENTORY_PROTECTED_ACTIVATE`: lock (`activated`) or
    /// unlock with `password`.
    Activate {
        /// The `szPasswordNow` bytes, C-stringed.
        password: [u8; 7],
        /// The `bActivated` byte.
        activated: bool,
    },
    /// `SUBHEADER_CG_INVENTORY_PROTECTED_PASSWORD_CHANGE`: swap `password` for
    /// `replacement`.
    PasswordChange {
        /// The `szPasswordNow` bytes, C-stringed.
        password: [u8; 7],
        /// The `szPasswordNew` bytes, C-stringed.
        replacement: [u8; 7],
    },
}

impl GameState {
    /// One `CG_INVENTORY_PROTECTED` for the character under `vid`: the lock or
    /// the unlock, or the password change. Answers the rows it changed, as an
    /// item step does, so the descriptor stores the lock with the row.
    ///
    /// # Errors
    ///
    /// [`MoveItemRefused::NoSuchCharacter`] when nobody is online under `vid`.
    pub fn protected(
        &mut self,
        vid: Vid,
        step: ProtectedStep,
        actor: Mover,
    ) -> Result<MovedItems, MoveItemRefused> {
        let character = self
            .characters
            .find_by_vid(vid)
            .map_err(|_| MoveItemRefused::NoSuchCharacter { vid })?;
        let pid = character.player_id();
        let moved = MovedItems::new(
            pid,
            vid.raw(),
            world::character::MoveDone {
                kind: world::character::MoveKind::Given,
                records: Vec::new(),
                changes: Vec::new(),
            },
            actor,
            &self.protos,
            &self.locale,
        );
        let moved = match step {
            ProtectedStep::Activate {
                password,
                activated,
            } => self.protected_activate(vid, password, activated, moved, actor),
            ProtectedStep::PasswordChange {
                password,
                replacement,
            } => self.protected_change(vid, password, replacement, moved, actor),
        };
        Ok(moved)
    }

    /// `ActivateProtectedSystem` (`G/char.cpp:11867-11925`): lock or unlock.
    fn protected_activate(
        &mut self,
        vid: Vid,
        password: [u8; 7],
        activated: bool,
        moved: MovedItems,
        actor: Mover,
    ) -> MovedItems {
        let text = c_string(&password);
        if text.first() == Some(&b'0') {
            self.protected_tell(
                vid,
                actor,
                b"|cffd93c3c[Err]:|h|r_Parola_nu_poate_incepe_cu_'0'.",
            );
            return moved;
        }
        if !all_digits(text) {
            self.protected_tell(
                vid,
                actor,
                b"|cffd93c3c[Err]:|h|r_Parola_trebuie_sa_contina_doar_cifre.",
            );
            return moved;
        }
        if text.len() > PROTECTED_PASSWORD_MAX_LEN {
            self.protected_tell(
                vid,
                actor,
                b"|cffd93c3c[Err]:|h|r_Parola_nu_poate_fi_mai_mare_de_6_cifre.",
            );
            return moved;
        }
        let typed = atoi(text);
        let (locked, stored) = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.points())
            .map_or((false, PROTECTED_FRESH_PASSWORD), |points| {
                (points.is_secured(), points.secured_password())
            });
        if activated {
            if locked {
                self.protected_tell(
                    vid,
                    actor,
                    b"|cffd93c3c[Err]:|h|r_Securitatea_este_deja_activa!",
                );
                return moved;
            }
            if stored != typed {
                self.protected_tell(
                    vid,
                    actor,
                    b"|cffd93c3c[Err]:|h|r_Parola_gresita._Securitatea_nu_poate_fi_activata!",
                );
                return moved;
            }
            self.change_pc_point(vid, point_slot::POINT_SECURED_STATE, 1, true);
            self.protected_tell(
                vid,
                actor,
                b"|cff3cd95e[Info]:|h|r_Securitatea_a_fost_activata!",
            );
        } else {
            if !locked {
                self.protected_tell(
                    vid,
                    actor,
                    b"|cffd93c3c[Err]:|h|r_Securitatea_este_deja_dezactivata!",
                );
                return moved;
            }
            if stored != typed {
                self.protected_tell(
                    vid,
                    actor,
                    b"|cffd93c3c[Err]:|h|r_Parola_gresita._Securitatea_nu_poate_fi_dezactivata!",
                );
                return moved;
            }
            self.change_pc_point(vid, point_slot::POINT_SECURED_STATE, 0, true);
            self.protected_tell(
                vid,
                actor,
                b"|cff3cd95e[Info]:|h|r_Securitatea_a_fost_dezactivata!",
            );
        }
        moved
    }

    /// `ChangeProtectedSystemPassword` (`G/char.cpp:11927-11974`): swap the password.
    fn protected_change(
        &mut self,
        vid: Vid,
        password: [u8; 7],
        replacement: [u8; 7],
        moved: MovedItems,
        actor: Mover,
    ) -> MovedItems {
        let now = c_string(&password);
        let fresh = c_string(&replacement);
        if now.first() == Some(&b'0') || fresh.first() == Some(&b'0') {
            self.protected_tell(
                vid,
                actor,
                b"|cffd93c3c[Err]:|h|r_Parola_nu_poate_incepe_cu_'0'.",
            );
            return moved;
        }
        if !all_digits(now) || !all_digits(fresh) {
            self.protected_tell(
                vid,
                actor,
                b"|cffd93c3c[Err]:|h|r_Parola_trebuie_sa_contina_doar_cifre.",
            );
            return moved;
        }
        if now.len() > PROTECTED_PASSWORD_MAX_LEN || fresh.len() > PROTECTED_PASSWORD_MAX_LEN {
            self.protected_tell(
                vid,
                actor,
                b"|cffd93c3c[Err]:|h|r_Parola_nu_poate_fi_mai_mare_de_6_cifre.",
            );
            return moved;
        }
        let typed = atoi(now);
        let changed = atoi(fresh);
        let stored = self
            .characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.points())
            .map_or(
                PROTECTED_FRESH_PASSWORD,
                world::character::Points::secured_password,
            );
        if stored != typed {
            self.protected_tell(
                vid,
                actor,
                b"|cffd93c3c[Err]:|h|r_Parola_actuala_este_gresita!",
            );
            return moved;
        }
        if typed == changed {
            self.protected_tell(
                vid,
                actor,
                b"|cffd93c3c[Err]:|h|r_Parola_noua_trebuie_sa_fie_diferita_de_cea_veche!",
            );
            return moved;
        }
        self.change_pc_point(vid, point_slot::POINT_SECURED_PASSWORD, changed, true);
        self.protected_tell(
            vid,
            actor,
            b"|cff3cd95e[Info]:|h|r_Parola_a_fost_schimbata_cu_succes!",
        );
        moved
    }

    /// Whether the character under `vid` locks its inventory (`IsSecured`).
    pub(super) fn is_secured(&self, vid: Vid) -> bool {
        self.characters
            .find_by_vid(vid)
            .ok()
            .and_then(|character| character.points())
            .is_some_and(world::character::Points::is_secured)
    }

    /// `ChatPacket(CHAT_TYPE_COMMAND, "BINARY_PopupMessage %s",
    /// LC_TEXT("|cffd93c3c[Err]:|h|r_Acest_cont_este_securizat!"))`: the popup a
    /// gate chats, encoded.
    pub(super) fn protected_popup(&self, actor: Mover) -> Vec<u8> {
        crate::chat_line::chat_packet(
            actor.recipient(&self.locale),
            CHAT_TYPE_COMMAND,
            b"BINARY_PopupMessage %s",
            &[Arg::Text(
                b"|cffd93c3c[Err]:|h|r_Acest_cont_este_securizat!",
            )],
        )
    }

    /// `ChatPacket(CHAT_TYPE_COMMAND, "BINARY_PopupMessage %s", LC_TEXT(text))`.
    fn protected_tell(&self, vid: Vid, actor: Mover, text: &[u8]) {
        self.tell(
            vid,
            actor,
            CHAT_TYPE_COMMAND,
            b"BINARY_PopupMessage %s",
            &[Arg::Text(text)],
        );
    }
}

/// The bytes before the first NUL: the legacy `std::string` constructor reads
/// the 7-byte field as a C string.
fn c_string(field: &[u8; 7]) -> &[u8] {
    let end = field.iter().position(|byte| *byte == 0).unwrap_or(7);
    &field[..end]
}

/// `has_only_digits` (`G/utils.cpp:358-361`): every byte a digit. Empty holds,
/// as the legacy `find_first_not_of` does.
fn all_digits(text: &[u8]) -> bool {
    text.iter().all(u8::is_ascii_digit)
}

/// `atoi` on a digits-only slice, at most six digits: the value fits.
fn atoi(text: &[u8]) -> i32 {
    text.iter()
        .fold(0, |value, byte| value * 10 + i32::from(byte - b'0'))
}
