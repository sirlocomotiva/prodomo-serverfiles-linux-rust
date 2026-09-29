//! Game enums ported from C++ length.h and item_length.h
//!
//! All enums use `#[repr(...)]` matching the original C++ size.

// ============================================================================
// EWearPositions - u8 (values 0-64)
// ============================================================================

/// Equipment wear positions
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EWearPositions {
    /// Body armor slot
    Body = 0,
    /// Head slot
    Head = 1,
    /// Feet slot
    Foots = 2,
    /// Wrist slot
    Wrist = 3,
    /// Weapon slot
    Weapon = 4,
    /// Neck slot
    Neck = 5,
    /// Ear slot
    Ear = 6,
    /// Unique accessory 1
    Unique1 = 7,
    /// Unique accessory 2
    Unique2 = 8,
    /// Arrow slot
    Arrow = 9,
    /// Shield slot
    Shield = 10,
    /// Ability slot 1
    Ability1 = 11,
    /// Ability slot 2
    Ability2 = 12,
    /// Ability slot 3
    Ability3 = 13,
    /// Ability slot 4
    Ability4 = 14,
    /// Ability slot 5
    Ability5 = 15,
    /// Ability slot 6
    Ability6 = 16,
    /// Ability slot 7
    Ability7 = 17,
    /// Ability slot 8
    Ability8 = 18,
    /// Costume body slot
    CostumeBody = 19,
    /// Costume hair slot
    CostumeHair = 20,
    /// Costume mount slot
    CostumeMount = 21,
    /// Costume weapon slot
    CostumeWeapon = 22,
    /// Costume sash slot
    CostumeSash = 23,
    /// Costume aura slot
    CostumeAura = 24,
    /// Ring slot 1
    Ring1 = 25,
    /// Ring slot 2
    Ring2 = 26,
    /// Belt slot
    Belt = 27,
    /// Costume pet slot
    CostumePet = 28,
    /// Costume sash skin slot
    CostumeSashSkin = 29,
    /// Fire talisman slot
    TalismanFire = 30,
    /// Ice talisman slot
    TalismanIce = 31,
    /// Earth talisman slot
    TalismanEarth = 32,
    /// Dark talisman slot
    TalismanDark = 33,
    /// Wind talisman slot
    TalismanWind = 34,
    /// Electric talisman slot
    TalismanElec = 35,
    /// Maximum wear position
    Max = 64,
}

// ============================================================================
// EDragonSoulDeckType - u8
// ============================================================================

/// Dragon soul deck types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EDragonSoulDeckType {
    /// Deck 0
    Deck0 = 0,
    /// Deck 1
    Deck1 = 1,
}

/// Dragon soul deck max count
pub const DRAGON_SOUL_DECK_MAX_NUM: u8 = 2;
/// Dragon soul deck reserved max count
pub const DRAGON_SOUL_DECK_RESERVED_MAX_NUM: u8 = 3;

// ============================================================================
// ESex - u8
// ============================================================================

/// Character sex
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ESex {
    /// Male
    Male = 0,
    /// Female
    Female = 1,
}

// ============================================================================
// EDirection - u8
// ============================================================================

/// Movement directions
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EDirection {
    /// North
    North = 0,
    /// Northeast
    Northeast = 1,
    /// East
    East = 2,
    /// Southeast
    Southeast = 3,
    /// South
    South = 4,
    /// Southwest
    Southwest = 5,
    /// West
    West = 6,
    /// Northwest
    Northwest = 7,
    /// Max direction count
    MaxNum = 8,
}

// ============================================================================
// EAbilityDifficulty - u8
// ============================================================================

/// Ability difficulty levels
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EAbilityDifficulty {
    /// Easy
    Easy = 0,
    /// Normal
    Normal = 1,
    /// Hard
    Hard = 2,
    /// Very hard
    VeryHard = 3,
    /// Number of difficulty types
    NumTypes = 4,
}

// ============================================================================
// EAbilityCategory - u8
// ============================================================================

/// Ability categories
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EAbilityCategory {
    /// Physical category
    Physical = 0,
    /// Mental category
    Mental = 1,
    /// Attribute category
    Attribute = 2,
    /// Number of category types
    NumTypes = 3,
}

// ============================================================================
// EJobs - u8
// ============================================================================

/// Character job classes
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EJobs {
    /// Warrior
    Warrior = 0,
    /// Assassin
    Assassin = 1,
    /// Sura
    Sura = 2,
    /// Shaman
    Shaman = 3,
    /// Maximum job count
    MaxNum = 4,
}

/// Skill group max count
pub const SKILL_GROUP_MAX_NUM: u8 = 2;

// ============================================================================
// ERaceFlags - u32 (bit flags up to 1 << 19)
// ============================================================================

/// Race flags (bitmask)
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ERaceFlags {
    /// Animal race
    Animal = 1 << 0,
    /// Undead race
    Undead = 1 << 1,
    /// Devil race
    Devil = 1 << 2,
    /// Human race
    Human = 1 << 3,
    /// Orc race
    Orc = 1 << 4,
    /// Milgyo race
    Milgyo = 1 << 5,
    /// Insect race
    Insect = 1 << 6,
    /// Fire element
    Fire = 1 << 7,
    /// Ice element
    Ice = 1 << 8,
    /// Desert element
    Desert = 1 << 9,
    /// Tree race
    Tree = 1 << 10,
    /// Electric attack
    AttElec = 1 << 11,
    /// Fire attack
    AttFire = 1 << 12,
    /// Ice attack
    AttIce = 1 << 13,
    /// Wind attack
    AttWind = 1 << 14,
    /// Earth attack
    AttEarth = 1 << 15,
    /// Dark attack
    AttDark = 1 << 16,
    /// Metin stone
    Metin = 1 << 17,
    /// Boss
    Boss = 1 << 18,
    /// CZ (Combat Zone)
    Cz = 1 << 19,
}

// ============================================================================
// ELoads - u8
// ============================================================================

/// Equipment load types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ELoads {
    /// No load
    None = 0,
    /// Light load
    Light = 1,
    /// Normal load
    Normal = 2,
    /// Heavy load
    Heavy = 3,
    /// Massive load
    Massive = 4,
}

// ============================================================================
// EQuickSlotType - u8
// ============================================================================

/// Quickslot types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EQuickSlotType {
    /// Empty slot
    None = 0,
    /// Item slot
    Item = 1,
    /// Skill slot
    Skill = 2,
    /// Command slot
    Command = 3,
    /// Max slot type count
    MaxNum = 4,
}

// ============================================================================
// EParts - u8
// ============================================================================

/// Character parts
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EParts {
    /// Main body part
    Main = 0,
    /// Weapon part
    Weapon = 1,
    /// Head part
    Head = 2,
    /// Hair part
    Hair = 3,
    /// Sash part
    Sash = 4,
    /// Aura part
    Aura = 5,
    /// Max part count
    MaxNum = 6,
    /// Sub weapon part
    WeaponSub = 7,
}

// ============================================================================
// EEmpire - u8
// ============================================================================

/// Empire types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EEmpire {
    /// All empires (neutral)
    All = 0,
    /// Shinsoo empire
    Shinsoo = 1,
    /// Chunjo empire
    Chunjo = 2,
    /// Jinno empire
    Jinno = 3,
}

// ============================================================================
// EChatType - u8
// ============================================================================

/// Chat message types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EChatType {
    /// Normal talking
    Talking = 0,
    /// Information message
    Info = 1,
    /// Notice message
    Notice = 2,
    /// Party chat
    Party = 3,
    /// Guild chat
    Guild = 4,
    /// Command
    Command = 5,
    /// Shout
    Shout = 6,
    /// Whisper
    Whisper = 7,
    /// Big notice
    BigNotice = 8,
    /// Monarch notice
    MonarchNotice = 9,
    /// Max chat type count
    MaxNum = 10,
}

// ============================================================================
// EWhisperType - u8
// ============================================================================

/// Whisper response types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EWhisperType {
    /// Normal whisper
    Normal = 0,
    /// Target does not exist
    NotExist = 1,
    /// Target blocked sender
    TargetBlocked = 2,
    /// Sender blocked target
    SenderBlocked = 3,
    /// Error
    Error = 4,
    /// GM whisper
    Gm = 5,
    /// System message (0xFF)
    System = 0xFF,
}

// ============================================================================
// ECharacterPosition - u8
// ============================================================================

/// Character position states
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ECharacterPosition {
    /// General standing
    General = 0,
    /// In battle
    Battle = 1,
    /// Dying
    Dying = 2,
    /// Sitting on chair
    SittingChair = 3,
    /// Sitting on ground
    SittingGround = 4,
    /// Intro state
    Intro = 5,
    /// Max position count
    MaxNum = 6,
}

// ============================================================================
// EGMLevels - u8
// ============================================================================

/// GM (Game Master) levels
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EGMLevels {
    /// Regular player
    Player = 0,
    /// Low wizard
    LowWizard = 1,
    /// Wizard
    Wizard = 2,
    /// High wizard
    HighWizard = 3,
    /// God level
    God = 4,
    /// Implementor
    Implementor = 5,
    /// Disabled
    Disable = 6,
}

// ============================================================================
// EMobRank - u8
// ============================================================================

/// Monster rank types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EMobRank {
    /// Pawn (basic mob)
    Pawn = 0,
    /// S-Pawn (stronger pawn)
    SPawn = 1,
    /// Knight
    Knight = 2,
    /// S-Knight (stronger knight)
    SKnight = 3,
    /// Boss
    Boss = 4,
    /// King
    King = 5,
    /// Max rank count
    MaxNum = 6,
}

// ============================================================================
// ECharType - u8
// ============================================================================

/// Character types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ECharType {
    /// Monster
    Monster = 0,
    /// NPC
    Npc = 1,
    /// Metin stone
    Stone = 2,
    /// Warp point
    Warp = 3,
    /// Door
    Door = 4,
    /// Building
    Building = 5,
    /// Player character
    Pc = 6,
    /// Polymorphed player
    PolymorphPc = 7,
    /// Horse
    Horse = 8,
    /// Go-to point
    Goto = 9,
}

// ============================================================================
// EBattleType - u8
// ============================================================================

/// Battle types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EBattleType {
    /// Melee combat
    Melee = 0,
    /// Range combat
    Range = 1,
    /// Magic combat
    Magic = 2,
    /// Special combat
    Special = 3,
    /// Power type
    Power = 4,
    /// Tanker type
    Tanker = 5,
    /// Super power type
    SuperPower = 6,
    /// Super tanker type
    SuperTanker = 7,
    /// Max battle type count
    MaxNum = 8,
}

// ============================================================================
// EOnClickEvents - u8
// ============================================================================

/// NPC click event types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EOnClickEvents {
    /// No action
    None = 0,
    /// Open shop
    Shop = 1,
    /// Start talk
    Talk = 2,
    /// Max click event count
    MaxNum = 3,
}

// ============================================================================
// EOnIdleEvents - u8
// ============================================================================

/// NPC idle event types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EOnIdleEvents {
    /// No idle action
    None = 0,
    /// General idle
    General = 1,
    /// Max idle event count
    MaxNum = 2,
}

// ============================================================================
// EMobSizes - u8
// ============================================================================

/// Monster sizes
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EMobSizes {
    /// Reserved
    Reserved = 0,
    /// Small size
    Small = 1,
    /// Medium size
    Medium = 2,
    /// Big size
    Big = 3,
}

// ============================================================================
// EAIFlags - u16 (bit flags up to 1 << 11)
// ============================================================================

/// AI behavior flags (bitmask)
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EAIFlags {
    /// Aggressive behavior
    Aggressive = 1 << 0,
    /// Cannot move
    NoMove = 1 << 1,
    /// Cowardly behavior
    Coward = 1 << 2,
    /// Do not attack Shinsoo
    NoAttackShinsu = 1 << 3,
    /// Do not attack Jinno
    NoAttackJinno = 1 << 4,
    /// Do not attack Chunjo
    NoAttackChunjo = 1 << 5,
    /// Attack other mobs
    AttackMob = 1 << 6,
    /// Berserk mode
    Berserk = 1 << 7,
    /// Stone skin
    StoneSkin = 1 << 8,
    /// God speed
    GodSpeed = 1 << 9,
    /// Death blow
    DeathBlow = 1 << 10,
    /// Revive after death
    Revive = 1 << 11,
}

// ============================================================================
// EMobStatType - u8
// ============================================================================

/// Monster stat types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EMobStatType {
    /// Power type
    Power = 0,
    /// Tanker type
    Tanker = 1,
    /// Super power type
    SuperPower = 2,
    /// Super tanker type
    SuperTanker = 3,
    /// Range type
    Range = 4,
    /// Magic type
    Magic = 5,
    /// Max stat type count
    MaxNum = 6,
}

// ============================================================================
// EImmuneFlags - u8 (bit flags up to 1 << 6)
// ============================================================================

/// Immunity flags (bitmask)
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EImmuneFlags {
    /// Stun immunity
    Stun = 1 << 0,
    /// Slow immunity
    Slow = 1 << 1,
    /// Fall immunity
    Fall = 1 << 2,
    /// Curse immunity
    Curse = 1 << 3,
    /// Poison immunity
    Poison = 1 << 4,
    /// Terror immunity
    Terror = 1 << 5,
    /// Reflect immunity
    Reflect = 1 << 6,
}

// ============================================================================
// EMobEnchants - u8
// ============================================================================

/// Monster enchant types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EMobEnchants {
    /// Curse enchant
    Curse = 0,
    /// Slow enchant
    Slow = 1,
    /// Poison enchant
    Poison = 2,
    /// Stun enchant
    Stun = 3,
    /// Critical enchant
    Critical = 4,
    /// Penetrate enchant
    Penetrate = 5,
    /// Max enchant count
    MaxNum = 6,
}

// ============================================================================
// EMobResists - u8
// ============================================================================

/// Monster resistance types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EMobResists {
    /// Sword resistance
    Sword = 0,
    /// Two-hand resistance
    Twohand = 1,
    /// Dagger resistance
    Dagger = 2,
    /// Bell resistance
    Bell = 3,
    /// Fan resistance
    Fan = 4,
    /// Bow resistance
    Bow = 5,
    /// Fire resistance
    Fire = 6,
    /// Electric resistance
    Elect = 7,
    /// Magic resistance
    Magic = 8,
    /// Wind resistance
    Wind = 9,
    /// Poison resistance
    Poison = 10,
    /// Max resistance count
    MaxNum = 11,
}

// ============================================================================
// ESkillAttrType - u8
// ============================================================================

/// Skill attribute types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ESkillAttrType {
    /// Normal skill
    Normal = 1,
    /// Melee skill
    Melee = 2,
    /// Range skill
    Range = 3,
    /// Magic skill
    Magic = 4,
}

// ============================================================================
// ESkillLevel - u8
// ============================================================================

/// Skill mastery levels
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ESkillLevel {
    /// Normal skill level
    Normal = 0,
    /// Master level
    Master = 1,
    /// Grand master level
    GrandMaster = 2,
    /// Perfect master level
    PerfectMaster = 3,
}

// ============================================================================
// EGuildWarType - u8
// ============================================================================

/// Guild war types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EGuildWarType {
    /// Field war
    Field = 0,
    /// Battle war
    Battle = 1,
    /// Flag war
    Flag = 2,
    /// Max war type count
    MaxNum = 3,
}

// ============================================================================
// EGuildWarState - u8
// ============================================================================

/// Guild war states
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EGuildWarState {
    /// No war
    None = 0,
    /// Send declare
    SendDeclare = 1,
    /// Refuse war
    Refuse = 2,
    /// Receive declare
    RecvDeclare = 3,
    /// Wait for start
    WaitStart = 4,
    /// Cancel war
    Cancel = 5,
    /// War in progress
    OnWar = 6,
    /// War ended
    End = 7,
    /// War over
    Over = 8,
    /// War reserved
    Reserve = 9,
}

// ============================================================================
// EAttributeSet - u8
// ============================================================================

/// Item attribute sets
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EAttributeSet {
    /// Weapon attributes
    Weapon = 0,
    /// Body armor attributes
    Body = 1,
    /// Wrist attributes
    Wrist = 2,
    /// Feet attributes
    Foots = 3,
    /// Neck attributes
    Neck = 4,
    /// Head attributes
    Head = 5,
    /// Shield attributes
    Shield = 6,
    /// Ear attributes
    Ear = 7,
    /// Talisman attributes
    Talisman = 8,
    /// Max attribute set count
    MaxNum = 9,
}

// ============================================================================
// EPrivType - u8
// ============================================================================

/// Privilege types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EPrivType {
    /// No privilege
    None = 0,
    /// Item drop bonus
    ItemDrop = 1,
    /// Gold drop bonus
    GoldDrop = 2,
    /// Gold x10 drop bonus
    Gold10Drop = 3,
    /// EXP percent bonus
    ExpPct = 4,
    /// Max privilege count
    MaxNum = 5,
}

// ============================================================================
// EMoneyLogType - u8
// ============================================================================

/// Money log types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EMoneyLogType {
    /// Reserved
    Reserved = 0,
    /// Monster drop
    Monster = 1,
    /// Shop transaction
    Shop = 2,
    /// Refine cost
    Refine = 3,
    /// Quest reward
    Quest = 4,
    /// Guild transaction
    Guild = 5,
    /// Miscellaneous
    Misc = 6,
    /// Monster kill reward
    MonsterKill = 7,
    /// Item drop
    Drop = 8,
    /// Max log type count
    MaxNum = 9,
}

// ============================================================================
// EPremiumTypes - u8
// ============================================================================

/// Premium account types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EPremiumTypes {
    /// EXP bonus
    Exp = 0,
    /// Item drop bonus
    Item = 1,
    /// Safebox expansion
    Safebox = 2,
    /// Auto loot
    AutoLoot = 3,
    /// Fish mind
    FishMind = 4,
    /// Marriage fast
    MarriageFast = 5,
    /// Gold bonus
    Gold = 6,
    /// Max premium count
    MaxNum = 9,
}

// ============================================================================
// ESpecialEffect - u8
// ============================================================================

/// Special visual effects
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ESpecialEffect {
    /// No effect
    None = 0,
    /// HP up red
    HpUpRed = 1,
    /// SP up blue
    SpUpBlue = 2,
    /// Speed up green
    SpeedUpGreen = 3,
    /// DX up purple
    DxUpPurple = 4,
    /// Critical effect
    Critical = 5,
    /// Penetrate effect
    Penetrate = 6,
    /// Block effect
    Block = 7,
    /// Dodge effect
    Dodge = 8,
    /// China firework
    ChinaFirework = 9,
    /// Spin top
    SpinTop = 10,
    /// Success effect
    Success = 11,
    /// Fail effect
    Fail = 12,
    /// FR success
    FrSuccess = 13,
    /// Level up 14+ for Germany
    LevelupOn14ForGermany = 14,
    /// Level up under 15 for Germany
    LevelupUnder15ForGermany = 15,
    /// Percent damage 1
    PercentDamage1 = 16,
    /// Percent damage 2
    PercentDamage2 = 17,
    /// Percent damage 3
    PercentDamage3 = 18,
    /// Auto HP up
    AutoHpUp = 19,
    /// Auto SP up
    AutoSpUp = 20,
    /// Equip ramadan ring
    EquipRamadanRing = 21,
    /// Equip halloween candy
    EquipHalloweenCandy = 22,
    /// Equip happiness ring
    EquipHappinessRing = 23,
    /// Equip love pendant
    EquipLovePendant = 24,
}

// ============================================================================
// EShopCoinType - u8
// ============================================================================

/// Shop coin types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EShopCoinType {
    /// Gold (default)
    Gold = 0,
    /// Secondary coin
    SecondaryCoin = 1,
}

// ============================================================================
// EShopSearchMode - u8
// ============================================================================

/// Shop search modes
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EShopSearchMode {
    /// No mode
    None = 0,
    /// Looking mode
    Looking = 1,
    /// Trading mode
    Trading = 2,
}

// ============================================================================
// EPrivateShopState - u8
// ============================================================================

/// Private shop states
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EPrivateShopState {
    /// Unavailable
    Unavailable = 0,
    /// Closed
    Closed = 1,
    /// Open
    Open = 2,
    /// Modifying
    Modify = 3,
}

// ============================================================================
// EPrivateShopSearchState - u8
// ============================================================================

/// Private shop search states
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EPrivateShopSearchState {
    /// Non-existent
    NonExistent = 0,
    /// Removed
    Removed = 1,
    /// Available
    Available = 2,
    /// Restricted
    Restricted = 3,
}

// ============================================================================
// EFishEventShape - u8
// ============================================================================

/// Fish event shape types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EFishEventShape {
    /// No shape
    None = 0,
    /// Shape 1
    Shape1 = 1,
    /// Shape 2
    Shape2 = 2,
    /// Shape 3
    Shape3 = 3,
    /// Shape 4
    Shape4 = 4,
    /// Shape 5
    Shape5 = 5,
    /// Shape 6
    Shape6 = 6,
    /// Shape 7
    Shape7 = 7,
    /// Max shape count
    MaxNum = 8,
}

// ============================================================================
// ECostumeOptionFlags - u8 (bit flags)
// ============================================================================

/// Costume hide option flags (bitmask)
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ECostumeOptionFlags {
    /// Hide head costume
    Head = 1 << 0,
    /// Hide body costume
    Body = 1 << 1,
    /// Hide weapon costume
    Weapon = 1 << 2,
    /// Hide sash costume
    Sash = 1 << 3,
}

// ============================================================================
// EDragonSoulRefineWindowSize - u8
// ============================================================================

/// Dragon soul refine window size
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EDragonSoulRefineWindowSize {
    /// Grid max size
    GridMax = 15,
}

// ============================================================================
// EItemTypes - u8 (from item_length.h)
// ============================================================================

/// Item types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EItemTypes {
    /// No item type
    None = 0,
    /// Weapon
    Weapon = 1,
    /// Armor
    Armor = 2,
    /// Use/consumable
    Use = 3,
    /// Auto-use
    AutoUse = 4,
    /// Material
    Material = 5,
    /// Special item
    Special = 6,
    /// Tool
    Tool = 7,
    /// Lottery
    Lottery = 8,
    /// Elk (currency)
    Elk = 9,
    /// Metin stone
    Metin = 10,
    /// Container
    Container = 11,
    /// Fish
    Fish = 12,
    /// Fishing rod
    Rod = 13,
    /// Resource
    Resource = 14,
    /// Campfire
    Campfire = 15,
    /// Unique item
    Unique = 16,
    /// Skill book
    Skillbook = 17,
    /// Quest item
    Quest = 18,
    /// Polymorph
    Polymorph = 19,
    /// Treasure box
    TreasureBox = 20,
    /// Treasure key
    TreasureKey = 21,
    /// Skill forget book
    SkillForget = 22,
    /// Gift box
    GiftBox = 23,
    /// Pickaxe
    Pick = 24,
    /// Hair item
    Hair = 25,
    /// Totem
    Totem = 26,
    /// Blend
    Blend = 27,
    /// Costume
    Costume = 28,
    /// Dragon soul
    Ds = 29,
    /// Special dragon soul
    SpecialDs = 30,
    /// Extract
    Extract = 31,
    /// Secondary coin
    SecondaryCoin = 32,
    /// Ring
    Ring = 33,
    /// Belt
    Belt = 34,
    /// Talisman
    Talisman = 35,
    /// Toggle item
    Toggle = 36,
}

// ============================================================================
// EMetinSubTypes - u8
// ============================================================================

/// Metin stone subtypes
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EMetinSubTypes {
    /// Normal metin
    Normal = 0,
    /// Gold metin
    Gold = 1,
}

// ============================================================================
// EToggleSubTypes - u8
// ============================================================================

/// Toggle item subtypes
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EToggleSubTypes {
    /// Shaman toggle
    Shaman = 0,
}

// ============================================================================
// EWeaponSubTypes - u8
// ============================================================================

/// Weapon subtypes
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EWeaponSubTypes {
    /// Sword
    Sword = 0,
    /// Dagger
    Dagger = 1,
    /// Bow
    Bow = 2,
    /// Two-handed weapon
    TwoHanded = 3,
    /// Bell
    Bell = 4,
    /// Fan
    Fan = 5,
    /// Arrow
    Arrow = 6,
    /// Mount spear
    MountSpear = 7,
    /// Number of weapon types
    NumTypes = 8,
}

// ============================================================================
// EArmorSubTypes - u8
// ============================================================================

/// Armor subtypes
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EArmorSubTypes {
    /// Body armor
    Body = 0,
    /// Head armor
    Head = 1,
    /// Shield
    Shield = 2,
    /// Wrist armor
    Wrist = 3,
    /// Feet armor
    Foots = 4,
    /// Neck armor
    Neck = 5,
    /// Ear armor
    Ear = 6,
    /// Number of armor types
    NumTypes = 7,
}

// ============================================================================
// ECostumeSubTypes - u8
// ============================================================================

/// Costume subtypes
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ECostumeSubTypes {
    /// Body costume (matches `ARMOR_BODY` = 0)
    Body = 0,
    /// Hair costume (matches `ARMOR_HEAD` = 1)
    Hair = 1,
    /// Mount costume
    Mount = 2,
    /// Sash costume
    Sash = 3,
    /// Weapon costume
    Weapon = 4,
    /// Aura costume
    Aura = 5,
    /// Pet costume
    Pet = 6,
    /// Sash skin costume
    SashSkin = 7,
    /// Number of costume types
    NumTypes = 8,
}

// ============================================================================
// EDragonSoulSubType - u8
// ============================================================================

/// Dragon soul slot subtypes
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EDragonSoulSubType {
    /// Slot 1
    Slot1 = 0,
    /// Slot 2
    Slot2 = 1,
    /// Slot 3
    Slot3 = 2,
    /// Slot 4
    Slot4 = 3,
    /// Slot 5
    Slot5 = 4,
    /// Slot 6
    Slot6 = 5,
    /// Max slot count
    Max = 6,
}

// ============================================================================
// EDragonSoulGradeTypes - u8
// ============================================================================

/// Dragon soul grade types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EDragonSoulGradeTypes {
    /// Normal grade
    Normal = 0,
    /// Brilliant grade
    Brilliant = 1,
    /// Rare grade
    Rare = 2,
    /// Ancient grade
    Ancient = 3,
    /// Legendary grade
    Legendary = 4,
    /// Max grade count
    Max = 5,
}

// ============================================================================
// EDragonSoulStepTypes - u8
// ============================================================================

/// Dragon soul step types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EDragonSoulStepTypes {
    /// Lowest step
    Lowest = 0,
    /// Low step
    Low = 1,
    /// Mid step
    Mid = 2,
    /// High step
    High = 3,
    /// Highest step
    Highest = 4,
    /// Max step count
    Max = 5,
}

// ============================================================================
// EFishSubTypes - u8
// ============================================================================

/// Fish subtypes
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EFishSubTypes {
    /// Alive fish
    Alive = 0,
    /// Dead fish
    Dead = 1,
}

// ============================================================================
// EResourceSubTypes - u8
// ============================================================================

/// Resource subtypes
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EResourceSubTypes {
    /// Fishbone
    Fishbone = 0,
    /// Water stone piece
    WaterStonePiece = 1,
    /// Water stone
    WaterStone = 2,
    /// Blood pearl
    BloodPearl = 3,
    /// Blue pearl
    BluePearl = 4,
    /// White pearl
    WhitePearl = 5,
    /// Bucket
    Bucket = 6,
    /// Crystal
    Crystal = 7,
    /// Gem
    Gem = 8,
    /// Stone
    Stone = 9,
    /// Metin resource
    Metin = 10,
    /// Ore
    Ore = 11,
}

// ============================================================================
// EUniqueSubTypes - u8
// ============================================================================

/// Unique item subtypes
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EUniqueSubTypes {
    /// None
    None = 0,
    /// Book
    Book = 1,
    /// Special ride
    SpecialRide = 2,
    /// Special mount ride
    SpecialMountRide = 3,
}

// ============================================================================
// EUseSubTypes - u8
// ============================================================================

/// Use item subtypes
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EUseSubTypes {
    /// Potion
    Potion = 0,
    /// Talisman
    Talisman = 1,
    /// Tuning scroll
    Tuning = 2,
    /// Move scroll
    Move = 3,
    /// Treasure box
    TreasureBox = 4,
    /// Money bag
    Moneybag = 5,
    /// Fishing bait
    Bait = 6,
    /// Ability up
    AbilityUp = 7,
    /// Affect item
    Affect = 8,
    /// Create stone
    CreateStone = 9,
    /// Special use
    Special = 10,
    /// Potion no delay
    PotionNoDelay = 11,
    /// Clear affect
    Clear = 12,
    /// Invisibility
    Invisibility = 13,
    /// Detachment
    Detachment = 14,
    /// Bucket
    Bucket = 15,
    /// Continuous potion
    PotionContinue = 16,
    /// Clean socket
    CleanSocket = 17,
    /// Change attribute
    ChangeAttribute = 18,
    /// Add attribute
    AddAttribute = 19,
    /// Add accessory socket
    AddAccessorySocket = 20,
    /// Put into accessory socket
    PutIntoAccessorySocket = 21,
    /// Add attribute 2
    AddAttribute2 = 22,
    /// Recipe
    Recipe = 23,
    /// Change attribute 2
    ChangeAttribute2 = 24,
    /// Bind item
    Bind = 25,
    /// Unbind item
    Unbind = 26,
    /// Time charge percent
    TimeChargePer = 27,
    /// Time charge fixed
    TimeChargeFix = 28,
    /// Put into belt socket
    PutIntoBeltSocket = 29,
    /// Put into ring socket
    PutIntoRingSocket = 30,
    /// Change costume attribute
    ChangeCostumeAttr = 31,
    /// Reset costume attribute
    ResetCostumeAttr = 32,
    /// Unknown 33
    Unk33 = 33,
    /// Change attribute plus
    ChangeAttributePlus = 34,
    /// Set attribute costume
    SetAttCostume = 39,
    /// Set attribute pet
    SetAttPet = 40,
    /// Set attribute mount
    SetAttMount = 41,
    /// Set attribute costume weapon
    SetAttCostumeWeapon = 42,
    /// Add attribute talisman
    AddAttributeTalisman = 43,
    /// Change attribute talisman
    ChangeAttributeTalisman = 44,
    /// Add attribute glove
    AddAttributeGlove = 45,
    /// Change attribute glove
    ChangeAttributeGlove = 46,
}

// ============================================================================
// EExtractSubTypes - u8
// ============================================================================

/// Extract subtypes
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EExtractSubTypes {
    /// Dragon soul extract
    DragonSoul = 0,
    /// Dragon heart extract
    DragonHeart = 1,
}

// ============================================================================
// EAutoUseSubTypes - u8
// ============================================================================

/// Auto-use item subtypes
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EAutoUseSubTypes {
    /// Auto potion
    Potion = 0,
    /// Auto ability up
    AbilityUp = 1,
    /// Auto bomb
    Bomb = 2,
    /// Auto gold
    Gold = 3,
    /// Auto moneybag
    Moneybag = 4,
    /// Auto treasure box
    TreasureBox = 5,
}

// ============================================================================
// EMaterialSubTypes - u8
// ============================================================================

/// Material subtypes
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EMaterialSubTypes {
    /// Leather
    Leather = 0,
    /// Blood
    Blood = 1,
    /// Root
    Root = 2,
    /// Needle
    Needle = 3,
    /// Jewel
    Jewel = 4,
    /// DS refine normal
    DsRefineNormal = 5,
    /// DS refine blessed
    DsRefineBlessed = 6,
    /// DS refine holly
    DsRefineHolly = 7,
}

// ============================================================================
// ESpecialSubTypes - u8
// ============================================================================

/// Special item subtypes
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ESpecialSubTypes {
    /// Map
    Map = 0,
    /// Key
    Key = 1,
    /// Document
    Doc = 2,
    /// Spirit
    Spirit = 3,
}

// ============================================================================
// EToolSubTypes - u8
// ============================================================================

/// Tool subtypes
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EToolSubTypes {
    /// Fishing rod
    FishingRod = 0,
}

// ============================================================================
// ELotterySubTypes - u8
// ============================================================================

/// Lottery subtypes
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ELotterySubTypes {
    /// Lottery ticket
    Ticket = 0,
    /// Instant lottery
    Instant = 1,
}

// ============================================================================
// EItemFlag - u16 (bit flags up to 1 << 14)
// ============================================================================

/// Item flags (bitmask)
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EItemFlag {
    /// Can be refined
    Refineable = 1 << 0,
    /// Save on logout
    Save = 1 << 1,
    /// Stackable
    Stackable = 1 << 2,
    /// Count per 1 gold
    CountPer1Gold = 1 << 3,
    /// Slow query item
    SlowQuery = 1 << 4,
    /// Unused flag
    Unused01 = 1 << 5,
    /// Unique item
    Unique = 1 << 6,
    /// Has make count
    MakeCount = 1 << 7,
    /// Cannot be removed
    Irremovable = 1 << 8,
    /// Confirm when use
    ConfirmWhenUse = 1 << 9,
    /// Quest use item
    QuestUse = 1 << 10,
    /// Quest use multiple
    QuestUseMultiple = 1 << 11,
    /// Quest give item
    QuestGive = 1 << 12,
    /// Log item
    Log = 1 << 13,
    /// Applicable item
    Applicable = 1 << 14,
}

// ============================================================================
// EItemAntiFlag - u32 (bit flags up to 1 << 17)
// ============================================================================

/// Item anti-flags (bitmask) - restrictions on item usage
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EItemAntiFlag {
    /// Female cannot use
    Female = 1 << 0,
    /// Male cannot use
    Male = 1 << 1,
    /// Warrior cannot use
    Warrior = 1 << 2,
    /// Assassin cannot use
    Assassin = 1 << 3,
    /// Sura cannot use
    Sura = 1 << 4,
    /// Shaman cannot use
    Shaman = 1 << 5,
    /// Cannot get
    Get = 1 << 6,
    /// Cannot drop
    Drop = 1 << 7,
    /// Cannot sell
    Sell = 1 << 8,
    /// Empire A cannot use
    EmpireA = 1 << 9,
    /// Empire B cannot use
    EmpireB = 1 << 10,
    /// Empire C cannot use
    EmpireC = 1 << 11,
    /// Cannot save
    Save = 1 << 12,
    /// Cannot give/trade
    Give = 1 << 13,
    /// PK drop
    PkDrop = 1 << 14,
    /// Cannot stack
    Stack = 1 << 15,
    /// Cannot put in my shop
    MyShop = 1 << 16,
    /// Cannot put in safebox
    Safebox = 1 << 17,
}

// ============================================================================
// EItemWearableFlag - u32 (bit flags up to 1 << 20)
// ============================================================================

/// Item wearable flags (bitmask) - where item can be equipped
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EItemWearableFlag {
    /// Body slot
    Body = 1 << 0,
    /// Head slot
    Head = 1 << 1,
    /// Feet slot
    Foots = 1 << 2,
    /// Wrist slot
    Wrist = 1 << 3,
    /// Weapon slot
    Weapon = 1 << 4,
    /// Neck slot
    Neck = 1 << 5,
    /// Ear slot
    Ear = 1 << 6,
    /// Unique slot
    Unique = 1 << 7,
    /// Shield slot
    Shield = 1 << 8,
    /// Arrow slot
    Arrow = 1 << 9,
    /// Hair slot
    Hair = 1 << 10,
    /// Ability slot
    Ability = 1 << 11,
    /// Costume sash slot
    CostumeSash = 1 << 12,
    /// Fire talisman slot
    Fire = 1 << 13,
    /// Ice talisman slot
    Ice = 1 << 14,
    /// Earth talisman slot
    Earth = 1 << 15,
    /// Dark talisman slot
    Dark = 1 << 16,
    /// Wind talisman slot
    Wind = 1 << 17,
    /// Electric talisman slot
    Elec = 1 << 18,
    /// Sash skin slot
    SashSkin = 1 << 20,
}

// ============================================================================
// ELimitTypes - u8
// ============================================================================

/// Item limit types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ELimitTypes {
    /// No limit
    None = 0,
    /// Level requirement
    Level = 1,
    /// Strength requirement
    Str = 2,
    /// Dexterity requirement
    Dex = 3,
    /// Intelligence requirement
    Int = 4,
    /// Constitution requirement
    Con = 5,
    /// Real time expiration
    RealTime = 6,
    /// Timer starts on first use
    RealTimeStartFirstUse = 7,
    /// Timer based on wear time
    TimerBasedOnWear = 8,
    /// Champion only
    Champion = 9,
    /// Max limit type count
    MaxNum = 10,
}

// ============================================================================
// EAttrAddonTypes - i8 (has negative value)
// ============================================================================

/// Attribute addon types
#[repr(i8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EAttrAddonTypes {
    /// No addon
    None = 0,
    /// Damage addon (negative value)
    Damage = -1,
}

// ============================================================================
// ERefineType - u8
// ============================================================================

/// Refine types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ERefineType {
    /// Normal refine
    Normal = 0,
    /// Not used
    NotUsed1 = 1,
    /// Scroll refine
    Scroll = 2,
    /// Hyuniron refine
    HyunIron = 3,
    /// Money only refine
    MoneyOnly = 4,
    /// Musin refine
    Musin = 5,
    /// Black dragon refine
    BDragon = 6,
}

// ============================================================================
// EAuraWindowType - u8
// ============================================================================

/// Aura window types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EAuraWindowType {
    /// Absorb window
    Absorb = 0,
    /// Growth window
    Growth = 1,
    /// Evolve window
    Evolve = 2,
    /// Max window type count
    Max = 3,
}

// ============================================================================
// EAuraSlotType - u8
// ============================================================================

/// Aura slot types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EAuraSlotType {
    /// Main slot
    Main = 0,
    /// Sub slot
    Sub = 1,
    /// Result slot
    Result = 2,
    /// Max slot count
    Max = 3,
}

// ============================================================================
// EAuraGradeType - u8
// ============================================================================

/// Aura grade types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EAuraGradeType {
    /// No grade
    None = 0,
    /// Ordinary grade
    Ordinary = 1,
    /// Simple grade
    Simple = 2,
    /// Noble grade
    Noble = 3,
    /// Sparkling grade
    Sparkling = 4,
    /// Magnificent grade
    Magnificent = 5,
    /// Radiant grade
    Radiant = 6,
    /// Max grade count
    MaxNum = 7,
}

// ============================================================================
// EAuraRefineInfoType - u8
// ============================================================================

/// Aura refine info types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EAuraRefineInfoType {
    /// Step info
    Step = 0,
    /// Level min info
    LevelMin = 1,
    /// Level max info
    LevelMax = 2,
    /// Need exp info
    NeedExp = 3,
    /// Material vnum info
    MaterialVnum = 4,
    /// Material count info
    MaterialCount = 5,
    /// Need gold info
    NeedGold = 6,
    /// Evolve percent info
    EvolvePct = 7,
    /// Max info count
    Max = 8,
}

// ============================================================================
// ERefineInfoSlotType - u8
// ============================================================================

/// Refine info slot types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ERefineInfoSlotType {
    /// Current slot
    Current = 0,
    /// Next slot
    Next = 1,
    /// Evolved slot
    Evolved = 2,
    /// Max slot count
    Max = 3,
}

// ============================================================================
// ERefineElementCategory - u8
// ============================================================================

/// Refine element categories
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ERefineElementCategory {
    /// No category
    None = 0,
    /// Electric element
    Elect = 1,
    /// Fire element
    Fire = 2,
    /// Ice element
    Ice = 3,
    /// Wind element
    Wind = 4,
    /// Earth element
    Earth = 5,
    /// Dark element
    Dark = 6,
    /// Max category count
    Max = 7,
}

// ============================================================================
// EDailyGiftState - u8
// ============================================================================

/// Daily gift states
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EDailyGiftState {
    /// Close state
    Close = 0,
    /// Open state
    Open = 1,
    /// Collect state
    Collect = 2,
    /// Collect with ticket
    CollectUseTicket = 3,
}

// ============================================================================
// EDailyGiftCollectState - u8
// ============================================================================

/// Daily gift collect states
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EDailyGiftCollectState {
    /// Locked
    Lock = 0,
    /// Waiting
    Wait = 1,
    /// Success
    Success = 2,
}

// ============================================================================
// ELocale - u8
// ============================================================================

/// Locale/language types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ELocale {
    /// Korea (YMIR)
    Ymir = 0,
    /// English
    En = 1,
    /// Portuguese
    Pt = 2,
    /// Spanish
    Es = 3,
    /// French
    Fr = 4,
    /// German
    De = 5,
    /// Romanian
    Ro = 6,
    /// Polish
    Pl = 7,
    /// Italian
    It = 8,
    /// Czech
    Cz = 9,
    /// Hungarian
    Hu = 10,
    /// Turkish
    Tr = 11,
    /// Max locale count
    MaxNum = 12,
}

/// Default locale
pub const LOCALE_DEFAULT: ELocale = ELocale::En;

// ============================================================================
// EStageType - u8
// ============================================================================

/// Stage event types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EStageType {
    /// Solo stage
    Solo = 0,
    /// Empire stage
    Empire = 1,
    /// HWID stage
    Hwid = 2,
    /// IP stage
    Ip = 3,
    /// Guild stage
    Guild = 4,
    /// Max type count
    Max = 5,
}

// ============================================================================
// EStageMission - u8
// ============================================================================

/// Stage mission types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EStageMission {
    /// Kill mob mission
    Mob = 0,
    /// Buy mission
    Buy = 1,
    /// Earn mission
    Earn = 2,
    /// Giving mission
    Giving = 3,
    /// EXP mission
    Exp = 4,
    /// Max mission count
    Max = 5,
}

// ============================================================================
// EStageReward - u8
// ============================================================================

/// Stage reward types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EStageReward {
    /// Yang reward
    Yang = 0,
    /// Bonus reward
    Bonus = 1,
    /// Item reward
    Item = 2,
    /// Taxes reward
    Taxes = 3,
    /// Max reward count
    Max = 4,
}

// ============================================================================
// ERewardMission - u8
// ============================================================================

/// Reward mission types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ERewardMission {
    /// Level up
    LevelUp = 0,
    /// Pet level up
    LevelUpPet = 1,
    /// Skill upgrade
    SkillUpgrade = 2,
    /// Kill monster
    KillMonster = 3,
    /// Kill stone
    KillStone = 4,
    /// Kill boss
    KillBoss = 5,
    /// Inventory slot
    InventorySlot = 6,
    /// Offlineshop slot
    OfflineshopSlot = 7,
    /// Average bonus
    AverageBonus = 8,
    /// Battlepass
    Battlepass = 9,
    /// First item
    FirstItem = 10,
    /// Custom sash
    CustomSash = 11,
    /// Biologist
    Biologist = 12,
    /// Passive skill complete
    PassiveSkillComplete = 13,
    /// Dungeon
    Dungeon = 14,
    /// Use item
    UseItem = 15,
    /// Sell item
    SellItem = 16,
    /// Buy item
    BuyItem = 17,
    /// Playtime
    Playtime = 18,
    /// Complete skill
    CompleteSkill = 19,
}
