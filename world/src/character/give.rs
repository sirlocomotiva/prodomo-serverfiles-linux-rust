//! Giving an item to an NPC (`CHARACTER::GiveItem`, `G/char_item.cpp:9226-9248`).
//!
//! The giver's checks (`CanHandleItem`, the waiting script, the cell, the offered item)
//! run on the caller; this module judges the victim's half: `CanReceiveItem`
//! (`:9250-9420`) and the arm of `ReceiveItem` (`:9422-9572`) the gift runs. A player
//! victim refuses, as does an NPC past the 2000 `DISTANCE_APPROX` bound
//! (`TOO_LONG_DISTANCE_EXCHANGE_BUG_FIX`). The race switch answers per race, and every
//! race it does not name answers yes: the `ITEM_FLAG_QUEST_GIVE` test is commented out,
//! so the gift falls through to the quest's `take` scripts.
//!
//! `G/` = `server/server/game/`.

use gamedata::item_kind::{
    ARMOR_BODY, ARMOR_HEAD, ARMOR_SHIELD, FISH_ALIVE, FISH_DEAD, ITEM_ARMOR, ITEM_FISH, ITEM_WEAPON,
};
use gamedata::item_proto::ItemProto;

/// `TOO_LONG_DISTANCE_EXCHANGE_BUG_FIX` (`G/char_item.cpp:9257-9259`): the `DISTANCE_APPROX`
/// past which an NPC refuses a gift.
pub const GIVE_DISTANCE: i32 = 2000;

/// `fishing::CAMPFIRE_MOB` (`G/fishing.h:10`): a fish is grilled, anything else is taken.
pub const CAMPFIRE_MOB: u32 = 12_000;
/// `fishing::FISHER_MOB` (`G/fishing.h:11`): a rod is taken.
pub const FISHER_MOB: u32 = 9009;
/// `BLACKSMITH_MOB` (`G/refine.h:8`): a refinable item opens the normal refine.
pub const BLACKSMITH_MOB: u32 = 20_016;
/// `ALCHEMIST_MOB` (`G/refine.h:9`): a refinable item is taken.
pub const ALCHEMIST_MOB: u32 = 20_001;
/// `BLACKSMITH_WEAPON_MOB` (`G/refine.h:11`): a refinable weapon opens the normal refine.
pub const BLACKSMITH_WEAPON_MOB: u32 = 20_044;
/// `BLACKSMITH_ARMOR_MOB` (`G/refine.h:12`): a refinable body, head or shield opens it.
pub const BLACKSMITH_ARMOR_MOB: u32 = 20_045;
/// `BLACKSMITH_ACCESSORY_MOB` (`G/refine.h:13`): any other refinable armour opens it.
pub const BLACKSMITH_ACCESSORY_MOB: u32 = 20_046;
/// `DEVILTOWER_BLACKSMITH_WEAPON_MOB` (`G/refine.h:15`): a refinable weapon opens the
/// money-only refine.
pub const DEVILTOWER_BLACKSMITH_WEAPON_MOB: u32 = 20_074;
/// `DEVILTOWER_BLACKSMITH_ARMOR_MOB` (`G/refine.h:16`): a refinable body, head or shield
/// opens it.
pub const DEVILTOWER_BLACKSMITH_ARMOR_MOB: u32 = 20_075;
/// `DEVILTOWER_BLACKSMITH_ACCESSORY_MOB` (`G/refine.h:17`): any other refinable armour
/// opens it.
pub const DEVILTOWER_BLACKSMITH_ACCESSORY_MOB: u32 = 20_076;
/// `BLACKSMITH2_MOB` (`G/refine.h:19`): a refine set at or past 500 opens the normal refine.
pub const BLACKSMITH2_MOB: u32 = 20_091;
/// `MeleyLair::STATUE_VNUM` (`G/MeleyLair.h:25`): the gift feeds the statue.
pub const MELEY_STATUE_VNUM: u32 = 6118;

/// The horse NPCs (`G/char_item.cpp:9331-9408`): the young, the grown and the war horse.
pub const HORSE_VNUMS: std::ops::RangeInclusive<u32> = 20_101..=20_109;
/// `ITEM_REVIVE_HORSE_1` (`G/unique_item.h:68`): the young horse's revive herb.
pub const ITEM_REVIVE_HORSE_1: u32 = 50_057;
/// `ITEM_REVIVE_HORSE_2`: the grown horse's revive herb.
pub const ITEM_REVIVE_HORSE_2: u32 = 50_058;
/// `ITEM_REVIVE_HORSE_3`: the war horse's revive herb.
pub const ITEM_REVIVE_HORSE_3: u32 = 50_059;
/// `ITEM_HORSE_FOOD_1` (`G/unique_item.h:65`): the young horse's feed.
pub const ITEM_HORSE_FOOD_1: u32 = 50_054;
/// `ITEM_HORSE_FOOD_2`: the grown horse's feed.
pub const ITEM_HORSE_FOOD_2: u32 = 50_055;
/// `ITEM_HORSE_FOOD_3`: the war horse's feed.
pub const ITEM_HORSE_FOOD_3: u32 = 50_056;

/// `[LS;452]`: the revive herb's line, chatted when a living horse is offered one and when
/// a dead one is revived.
pub const HORSE_REVIVE_LINE: &[u8] = b"[LS;452]";
/// `[LS;453]`: the feed's line, chatted when a dead horse is offered feed and when a living
/// one is fed.
pub const HORSE_FEED_LINE: &[u8] = b"[LS;453]";

/// What `CanReceiveItem` answers for an NPC, in the order the race switch judges
/// (`G/char_item.cpp:9261-9420`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GiveAnswer {
    /// A fish at the campfire: `fishing::Grill`.
    Grill,
    /// Anything else at the campfire, a rod at the fisher's, and every unnamed race: the
    /// quest's `take` scripts.
    Take,
    /// A refinable item at a town smith: `RefineInformation` with `REFINE_TYPE_NORMAL`.
    RefineNormal,
    /// A refinable item at a devil-tower smith: `RefineInformation` with
    /// `REFINE_TYPE_MONEY_ONLY`.
    RefineMoneyOnly,
    /// A revive herb at a dead horse of its grade.
    HorseRevive,
    /// Feed at a living horse.
    HorseFeed,
    /// A gift at the Meley statue.
    MeleyStatue,
    /// The gift is refused silently.
    Refused,
    /// The gift is refused with this line chatted to the giver.
    RefusedWith(&'static [u8]),
}

/// A [`GiveAnswer`] arm this build does not serve yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GiveUnported {
    /// The campfire grill (`fishing::Grill`).
    Grill,
    /// The Meley statue.
    MeleyStatue,
}

/// The unported system a [`GiveAnswer`] arm needs, for the arms this build does not serve.
#[must_use]
pub fn unported_of(answer: GiveAnswer) -> Option<GiveUnported> {
    match answer {
        GiveAnswer::Grill => Some(GiveUnported::Grill),
        GiveAnswer::MeleyStatue => Some(GiveUnported::MeleyStatue),
        GiveAnswer::Take
        | GiveAnswer::Refused
        | GiveAnswer::RefusedWith(_)
        | GiveAnswer::RefineNormal
        | GiveAnswer::RefineMoneyOnly
        | GiveAnswer::HorseRevive
        | GiveAnswer::HorseFeed => None,
    }
}

/// `CHARACTER::CanReceiveItem` for an NPC of `race`, dead when `npc_dead`, offered the item
/// of `vnum` with `proto` (`G/char_item.cpp:9250-9420`). The player-victim and distance arms
/// run on the caller.
#[must_use]
pub fn can_receive(race: u32, npc_dead: bool, vnum: u32, proto: &ItemProto) -> GiveAnswer {
    // `BLACKSMITH_MOB` also refuses a refine set at or past 500 (`:9292-9302`).
    if race == BLACKSMITH_MOB {
        return if proto.refined_vnum != 0 && proto.refine_set < 500 {
            GiveAnswer::RefineNormal
        } else {
            GiveAnswer::Refused
        };
    }
    // `BLACKSMITH2_MOB` is the mirror: only a refine set at or past 500 (`:9304-9312`).
    if race == BLACKSMITH2_MOB {
        return if proto.refine_set >= 500 {
            GiveAnswer::RefineNormal
        } else {
            GiveAnswer::Refused
        };
    }
    // The alchemist takes a refinable item, and anything else falls through the switch to the
    // quest (`:9314-9317`); both run the `take` scripts, as the fisher's rod does (`:9269`).
    if race == ALCHEMIST_MOB || race == FISHER_MOB {
        return GiveAnswer::Take;
    }
    if HORSE_VNUMS.contains(&race) {
        return horse_answer(race, npc_dead, vnum);
    }
    match race {
        CAMPFIRE_MOB => {
            if proto.item_type == ITEM_FISH
                && (proto.sub_type == FISH_ALIVE || proto.sub_type == FISH_DEAD)
            {
                GiveAnswer::Grill
            } else {
                GiveAnswer::Take
            }
        }
        BLACKSMITH_WEAPON_MOB | DEVILTOWER_BLACKSMITH_WEAPON_MOB => {
            if proto.item_type == ITEM_WEAPON && proto.refined_vnum != 0 {
                refine_of(race)
            } else {
                GiveAnswer::Refused
            }
        }
        BLACKSMITH_ARMOR_MOB | DEVILTOWER_BLACKSMITH_ARMOR_MOB => {
            if proto.item_type == ITEM_ARMOR
                && is_body_head_or_shield(proto.sub_type)
                && proto.refined_vnum != 0
            {
                refine_of(race)
            } else {
                GiveAnswer::Refused
            }
        }
        BLACKSMITH_ACCESSORY_MOB | DEVILTOWER_BLACKSMITH_ACCESSORY_MOB => {
            if proto.item_type == ITEM_ARMOR
                && !is_body_head_or_shield(proto.sub_type)
                && proto.refined_vnum != 0
            {
                refine_of(race)
            } else {
                GiveAnswer::Refused
            }
        }
        MELEY_STATUE_VNUM => GiveAnswer::MeleyStatue,
        _ => GiveAnswer::Take,
    }
}

/// Whether the armour sub type is one the armour smith takes: the body, the head or the
/// shield (`G/char_item.cpp:9281`).
fn is_body_head_or_shield(sub_type: i32) -> bool {
    sub_type == ARMOR_BODY || sub_type == ARMOR_SHIELD || sub_type == ARMOR_HEAD
}

/// The refine arm of a smith race: the devil-tower smiths open the money-only refine, the
/// town smiths the normal one.
fn refine_of(race: u32) -> GiveAnswer {
    match race {
        DEVILTOWER_BLACKSMITH_WEAPON_MOB
        | DEVILTOWER_BLACKSMITH_ARMOR_MOB
        | DEVILTOWER_BLACKSMITH_ACCESSORY_MOB => GiveAnswer::RefineMoneyOnly,
        _ => GiveAnswer::RefineNormal,
    }
}

/// The horse arms of `CanReceiveItem` (`G/char_item.cpp:9331-9408`): each grade takes its
/// own revive herb when dead and its own feed when living, chatting the herb's line at a
/// living horse and the feed's line at a dead one. Another grade's herb or feed is refused,
/// and anything else falls through to the quest.
fn horse_answer(race: u32, npc_dead: bool, vnum: u32) -> GiveAnswer {
    let (revive, feed) = match race {
        20_101..=20_103 => (ITEM_REVIVE_HORSE_1, ITEM_HORSE_FOOD_1),
        20_104..=20_106 => (ITEM_REVIVE_HORSE_2, ITEM_HORSE_FOOD_2),
        _ => (ITEM_REVIVE_HORSE_3, ITEM_HORSE_FOOD_3),
    };
    if vnum == revive {
        if npc_dead {
            return GiveAnswer::HorseRevive;
        }
        return GiveAnswer::RefusedWith(HORSE_REVIVE_LINE);
    }
    if vnum == feed {
        if npc_dead {
            return GiveAnswer::RefusedWith(HORSE_FEED_LINE);
        }
        return GiveAnswer::HorseFeed;
    }
    // Another grade's herb or feed is refused; anything else falls through (`:9352-9354`).
    if matches!(
        vnum,
        ITEM_REVIVE_HORSE_1
            | ITEM_REVIVE_HORSE_2
            | ITEM_REVIVE_HORSE_3
            | ITEM_HORSE_FOOD_1
            | ITEM_HORSE_FOOD_2
            | ITEM_HORSE_FOOD_3
    ) {
        return GiveAnswer::Refused;
    }
    GiveAnswer::Take
}

#[cfg(test)]
mod tests {
    use super::*;
    use gamedata::item_kind::{ITEM_ROD, ITEM_USE};

    fn proto(item_type: i32, sub_type: i32) -> ItemProto {
        ItemProto::for_category_rule(19, item_type, sub_type)
    }

    #[test]
    fn the_campfire_grills_a_fish_and_takes_anything_else() {
        let alive = proto(ITEM_FISH, FISH_ALIVE);
        assert_eq!(
            can_receive(CAMPFIRE_MOB, false, 27_801, &alive),
            GiveAnswer::Grill
        );
        let dead = proto(ITEM_FISH, FISH_DEAD);
        assert_eq!(
            can_receive(CAMPFIRE_MOB, false, 27_801, &dead),
            GiveAnswer::Grill
        );
        let sword = proto(ITEM_WEAPON, 0);
        assert_eq!(
            can_receive(CAMPFIRE_MOB, false, 19, &sword),
            GiveAnswer::Take
        );
    }

    #[test]
    fn the_smiths_take_what_they_refine_and_refuse_the_rest() {
        let mut sword = proto(ITEM_WEAPON, 0);
        sword.refined_vnum = 20;
        assert_eq!(
            can_receive(BLACKSMITH_WEAPON_MOB, false, 19, &sword),
            GiveAnswer::RefineNormal
        );
        assert_eq!(
            can_receive(DEVILTOWER_BLACKSMITH_WEAPON_MOB, false, 19, &sword),
            GiveAnswer::RefineMoneyOnly
        );
        let plain = proto(ITEM_WEAPON, 0);
        assert_eq!(
            can_receive(BLACKSMITH_WEAPON_MOB, false, 19, &plain),
            GiveAnswer::Refused
        );
        let mut body = proto(ITEM_ARMOR, ARMOR_BODY);
        body.refined_vnum = 21;
        assert_eq!(
            can_receive(BLACKSMITH_ARMOR_MOB, false, 20, &body),
            GiveAnswer::RefineNormal
        );
        assert_eq!(
            can_receive(BLACKSMITH_ACCESSORY_MOB, false, 20, &body),
            GiveAnswer::Refused
        );
        let mut wrist = proto(ITEM_ARMOR, 3);
        wrist.refined_vnum = 22;
        assert_eq!(
            can_receive(BLACKSMITH_ACCESSORY_MOB, false, 21, &wrist),
            GiveAnswer::RefineNormal
        );
        let mut set = proto(ITEM_WEAPON, 0);
        set.refined_vnum = 20;
        set.refine_set = 500;
        assert_eq!(
            can_receive(BLACKSMITH_MOB, false, 19, &set),
            GiveAnswer::Refused
        );
        assert_eq!(
            can_receive(BLACKSMITH2_MOB, false, 19, &set),
            GiveAnswer::RefineNormal
        );
        set.refine_set = 499;
        assert_eq!(
            can_receive(BLACKSMITH_MOB, false, 19, &set),
            GiveAnswer::RefineNormal
        );
        assert_eq!(
            can_receive(BLACKSMITH2_MOB, false, 19, &set),
            GiveAnswer::Refused
        );
    }

    #[test]
    fn the_horses_take_their_own_herb_and_feed_and_chat_the_rest() {
        let herb = proto(ITEM_USE, 0);
        assert_eq!(
            can_receive(20_101, true, ITEM_REVIVE_HORSE_1, &herb),
            GiveAnswer::HorseRevive
        );
        assert_eq!(
            can_receive(20_101, false, ITEM_REVIVE_HORSE_1, &herb),
            GiveAnswer::RefusedWith(HORSE_REVIVE_LINE)
        );
        assert_eq!(
            can_receive(20_101, false, ITEM_HORSE_FOOD_1, &herb),
            GiveAnswer::HorseFeed
        );
        assert_eq!(
            can_receive(20_101, true, ITEM_HORSE_FOOD_1, &herb),
            GiveAnswer::RefusedWith(HORSE_FEED_LINE)
        );
        assert_eq!(
            can_receive(20_101, true, ITEM_REVIVE_HORSE_2, &herb),
            GiveAnswer::Refused
        );
        assert_eq!(can_receive(20_109, false, 19, &herb), GiveAnswer::Take);
    }

    #[test]
    fn the_fisher_the_alchemist_the_statue_and_the_rest_take() {
        let rod = proto(ITEM_ROD, 0);
        assert_eq!(
            can_receive(FISHER_MOB, false, 28_001, &rod),
            GiveAnswer::Take
        );
        let sword = proto(ITEM_WEAPON, 0);
        assert_eq!(can_receive(FISHER_MOB, false, 19, &sword), GiveAnswer::Take);
        assert_eq!(
            can_receive(ALCHEMIST_MOB, false, 19, &sword),
            GiveAnswer::Take
        );
        assert_eq!(
            can_receive(MELEY_STATUE_VNUM, false, 19, &sword),
            GiveAnswer::MeleyStatue
        );
        assert_eq!(can_receive(20005, false, 19, &sword), GiveAnswer::Take);
    }

    #[test]
    fn the_unserved_arms_name_their_system_and_the_rest_name_none() {
        assert_eq!(unported_of(GiveAnswer::Grill), Some(GiveUnported::Grill));
        assert_eq!(unported_of(GiveAnswer::RefineNormal), None);
        assert_eq!(unported_of(GiveAnswer::RefineMoneyOnly), None);
        assert_eq!(unported_of(GiveAnswer::HorseRevive), None);
        assert_eq!(unported_of(GiveAnswer::HorseFeed), None);
        assert_eq!(
            unported_of(GiveAnswer::MeleyStatue),
            Some(GiveUnported::MeleyStatue)
        );
        assert_eq!(unported_of(GiveAnswer::Take), None);
        assert_eq!(unported_of(GiveAnswer::Refused), None);
        assert_eq!(unported_of(GiveAnswer::RefusedWith(b"x")), None);
    }
}
