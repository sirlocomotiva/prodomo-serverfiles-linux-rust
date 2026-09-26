//! Legacy Game data table records and the row rules that build them.
//!
//! A row rule turns one legacy query row, as the one-time importer reads it
//! from the owner's MySQL dump, into the typed record the legacy loader built.

pub mod banword;
pub mod event;
pub mod item_attr;
pub mod land;
pub mod object_proto;
pub mod records;
pub mod refine;
pub mod renewal_shop;
pub mod shop;
pub mod skill;
