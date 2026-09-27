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
pub mod csv_table;
pub mod event;
pub mod item_attr;
pub mod item_custom_category;
pub mod item_proto;
pub mod item_proto_value;
pub mod land;
pub mod map_atlas;
pub mod mob_names;
pub mod object_proto;
pub mod records;
pub mod refine;
pub mod renewal_shop;
pub mod shop;
pub mod skill;
pub mod special_item_group;
pub mod sql_dump;
pub mod text_file;
