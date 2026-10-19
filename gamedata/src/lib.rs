//! Legacy Game data table records, the row rules that build them, and the
//! readers that parse the owner's text files.
//!
//! A row rule turns one legacy query row, as the one-time importer reads it
//! from the owner's MySQL dump, into the typed record the legacy loader built.
//!
//! A reader turns a file under `legacy/gamedata` into the same typed record,
//! byte-exactly, and is added with the system that first needs it. [`text_file`]
//! is the `CTextFileLoader` port that most of them sit on.

pub mod banword;
pub mod belt_inventory;
pub mod csv_table;
pub mod drop;
pub mod event;
/// The guild skill rows, 151-162.
pub mod guild_skill_data;
pub mod item_attr;
pub mod item_custom_category;
pub mod item_kind;
pub mod item_mask;
pub mod item_proto;
pub mod item_proto_value;
pub mod land;
pub mod locale_string;
pub mod lzo;
pub mod map_atlas;
pub mod mob_group;
pub mod mob_locale_names;
pub mod mob_names;
pub mod mob_proto;
/// The hosted mob skill rows, the mob power row, and clip splash data.
pub mod mob_skill_data;
pub mod npc_shop;
pub mod object_proto;
pub mod pc_motion;
/// The PC skill rows, the eight power rows, and the `CanUseSkill` gates.
pub mod pc_skill_data;
pub mod records;
pub mod refine;
pub mod regen;
pub mod renewal_shop;
pub mod server_attr;
pub mod shop;
pub mod skill;
pub mod special_item_group;
pub mod sql_dump;
pub mod text_file;
