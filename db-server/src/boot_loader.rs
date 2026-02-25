//! SQL acquisition and composition of the complete `DG_BOOT` table set.
//!
//! The legacy DB server loads every table into memory once, at startup, before
//! it answers any `QUERY_BOOT`; a boot request then re-reads those in-memory
//! caches. This module reproduces that shape: [`BootTableLoader`] reads the
//! tables, and [`LoadedTables`] holds the result.
//!
//! # What this module owns
//!
//! - which database each table is read from;
//! - the `TABLE_POSTFIX` boundary, validated once by [`TablePostfix`];
//! - composing the sections into a [`BootSnapshot`] and a [`BootMonarchInfo`].
//!
//! # What this module deliberately does not own
//!
//! - **The record limits.** Each loader is built with its own
//!   `*Limits::default()`, which is defined in terms of that table's own
//!   verified `*_MAX_RECORDS` / `*_MAX_SECTION_BYTES` constants. No limit is
//!   retyped here, so this module cannot drift from the widths the tested pure
//!   decoders enforce.
//! - **Item-ID ranges.** Legacy `GetRange` consumes one pair per boot *request*,
//!   so a range is not table data. [`LoadedTables::compose`] takes the pair as
//!   arguments instead of storing it.
//! - **The feature profile.** The caller supplies it. Nothing here infers which
//!   optional sections exist from payload bytes.
//! - **The clock.** `global_time` and the market-price `unix_seconds` are
//!   supplied, so a test can freeze both.
//! - **GM hosts and administrators.** Legacy resolves `gmhost` and `gmlist`
//!   inside each `QUERY_BOOT` call, not once at startup, and filters the
//!   administrator list on the requesting peer's address. They are read per
//!   request by [`BootTableLoader::load_gm_tail`] and passed to
//!   [`LoadedTables::compose`] as a [`BootGmTail`]. Monarch **candidacy** is
//!   different: legacy has no `SELECT` for it anywhere, so that section is
//!   always empty and no constructor here can present an unloaded list as a
//!   loaded one.
//!
//! # Postfix handling is per-table
//!
//! The legacy postfix is appended verbatim to a table name by `snprintf`, and
//! it is not applied to every table. `shop`, `shopex`, and `banword` take **no**
//! postfix in the legacy source, and this module matches that: only
//! [`ShopTableLoader`], [`BanwordSectionLimits`], and the postfix-free
//! [`RenewalShopTableLoader`] are built without a [`TablePostfix`]. Applying one
//! postfix to all fourteen would be wrong.
//!
//! # Failure policy
//!
//! A table that fails to load is a **hard** error. Returning a partial boot
//! would hand a game server a world with, say, no monsters and no items while
//! reporting success, and the game server cannot tell an empty table from an
//! absent one. Legacy behaves the same way: a failed `SetCache` aborts startup.
//!
//! # A deliberate divergence: the proto tables are always read from SQL
//!
//! Legacy branches on a **runtime** flag. `ClientManagerBoot.cpp:18-30` reads
//! `mob_proto` and `item_proto` from the **text files** `mob_proto.txt` and
//! `item_proto.txt` unless the config key `PROTO_FROM_DB` is set non-zero;
//! `bIsProtoReadFromDB` defaults to `false` at `ClientManager.cpp:71`. Only
//! `PROTO_FROM_DB=1` selects the SQL statements this module implements.
//!
//! The wire format is identical either way, so the client cannot tell which
//! source was used, but the **data** can differ and would differ silently if
//! the text files and the SQL tables had drifted apart.
//!
//! This module always reads SQL. That is a recorded divergence rather than an
//! oversight, and it is forced: neither text file is checked into this
//! repository and there is no Rust text-table reader. To match a legacy
//! deployment exactly, that deployment must have been running with
//! `PROTO_FROM_DB=1`.
//!
//! The related `ENABLE_AUTODETECT_VNUMRANGE` is defined at
//! `ClientManagerBoot.cpp:1372`, inside the `ENABLE_PROTO_FROM_DB` block, so
//! the `item_proto` statement selects 34 columns and **not** `vnum_range`.

use std::error::Error;
use std::fmt;

use db::pool::ConnectionPool;
use protocol::db_boot::{
    BootAdminInfo, BootAdminSection, BootFeatureProfile, BootGmHost, BootGmHostSection,
    BootItemIdRange, BootItemIdRanges, BootMonarchCandidacySection, BootMonarchInfo, BootSection,
    ADMIN_INFO_WIRE_SIZE, GM_HOST_WIRE_SIZE, ITEM_ID_RANGE_WIRE_SIZE, MONARCH_CANDIDACY_WIRE_SIZE,
};

use crate::banword::BanwordSectionLimits;
use crate::boot_composition::LoadedBootSections;
use crate::boot_snapshot::{BootSnapshot, BootSnapshotParts};
use crate::event::{EventQuery, EventSectionLimits};
use crate::gm::{AdminQuery, GmSectionLimits};
use crate::item_attr::ItemAttrSectionLimits;
use crate::item_proto::{ItemProtoLoader, ItemProtoSectionLimits};
use crate::land::LandSectionLimits;
use crate::market_price::{MarketPriceQuery, MarketPriceSectionLimits};
use crate::mob_proto::{MobProtoLoader, MobProtoSectionLimits};
use crate::monarch::{MonarchLimits, MonarchLoader, MONARCH_MAX_SOURCE_ROWS};
use crate::object::{ObjectSectionLimits, ObjectTableLoader};
use crate::object_proto::{ObjectProtoSectionLimits, ObjectProtoTableLoader};
use crate::postfix::{
    ItemAttrLoader, ItemRareLoader, LandTableLoader, RefineProtoLoader, RefineSectionLimits,
    SkillSectionLimits, SkillTableLoader, TablePostfix,
};
use crate::renewal_shop::RenewalShopTableLoader;
use crate::shop::ShopTableLoader;

/// A failure while loading the boot table set.
#[derive(Debug)]
pub enum BootTableLoadError {
    /// The `TABLE_POSTFIX` value is not a valid table identifier.
    Postfix(String),
    /// A table query could not be assembled.
    Query {
        /// Which table failed to build its query.
        table: &'static str,
        /// The underlying build failure.
        detail: String,
    },
    /// SQL acquisition failed.
    Database {
        /// Which table could not be read.
        table: &'static str,
        /// The underlying database failure.
        detail: String,
    },
    /// The sections could not be composed into a boot snapshot.
    Compose(String),
}

impl fmt::Display for BootTableLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Postfix(detail) => {
                write!(formatter, "TABLE_POSTFIX is invalid: {detail}")
            }
            Self::Query { table, detail } => {
                write!(formatter, "the {table} query could not be built: {detail}")
            }
            Self::Database { table, detail } => {
                write!(formatter, "the {table} table could not be read: {detail}")
            }
            Self::Compose(detail) => write!(formatter, "boot composition failed: {detail}"),
        }
    }
}

impl Error for BootTableLoadError {}

/// Attach a table name to a query-build failure.
fn query_error(table: &'static str, error: impl fmt::Display) -> BootTableLoadError {
    BootTableLoadError::Query {
        table,
        detail: error.to_string(),
    }
}

/// Attach a table name to an acquisition failure.
fn database_error(table: &'static str, error: impl fmt::Display) -> BootTableLoadError {
    BootTableLoadError::Database {
        table,
        detail: error.to_string(),
    }
}

/// The databases the boot loader reads from.
///
/// All fourteen boot tables live in the player database, which is what the
/// legacy `DirectQuery(..., SQL_PLAYER)` default slot resolves to. The `gmhost`
/// and `gmlist` tail tables are the exception: legacy passes `SQL_COMMON`
/// explicitly for both, so they come from `common`.
///
/// The `account` pool is carried but unread. The account handlers are separate
/// requests, and nothing in the boot path queries that schema.
#[derive(Debug, Clone, Default)]
pub struct BootDataSources {
    /// The player database. Holds all fourteen boot tables and the monarch join.
    pub player: Option<ConnectionPool>,
    /// The account database. Unused; the account handlers are separate requests.
    pub account: Option<ConnectionPool>,
    /// The common database. Holds `gmhost` and `gmlist`.
    pub common: Option<ConnectionPool>,
}

/// The per-request GM tail of a boot payload.
///
/// Legacy calls `__GetHostInfo` and `__GetAdminInfo` inside `QUERY_BOOT`, not
/// once at startup, so this is resolved per request rather than cached with the
/// tables. The host list does not depend on the requester, but the
/// administrator list does, so both are passed together to keep the
/// request-scoped pairing visible.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BootGmTail {
    /// Packed `gmhost` rows in fetch order.
    pub hosts: Vec<BootGmHost>,
    /// Packed `gmlist` rows for this request's address.
    pub admins: Vec<BootAdminInfo>,
}

impl BootGmTail {
    /// A tail with both lists empty.
    ///
    /// This is the honest value for a server with no configured common pool. It
    /// is *not* the same as a successful read that happened to return nothing,
    /// which is why the loader refuses rather than falling back to it.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            hosts: Vec::new(),
            admins: Vec::new(),
        }
    }

    /// The `gmhost` count, narrowed to the wire `WORD`.
    ///
    /// # Errors
    ///
    /// Returns [`BootTableLoadError::Compose`] when the count does not fit,
    /// which the pure limits already prevent for a real load.
    fn host_count_u16(&self) -> Result<u16, BootTableLoadError> {
        u16::try_from(self.hosts.len())
            .map_err(|_| BootTableLoadError::Compose(format!("{} GM hosts", self.hosts.len())))
    }

    /// The `gmlist` count, narrowed to the wire `WORD`.
    ///
    /// # Errors
    ///
    /// Returns [`BootTableLoadError::Compose`] when the count does not fit.
    fn admin_count_u16(&self) -> Result<u16, BootTableLoadError> {
        u16::try_from(self.admins.len()).map_err(|_| {
            BootTableLoadError::Compose(format!("{} GM administrators", self.admins.len()))
        })
    }
}

/// A complete, validated set of boot tables, held after one load.
#[derive(Debug, Clone)]
pub struct LoadedTables {
    profile: BootFeatureProfile,
    sections: Vec<BootSection>,
}

impl LoadedTables {
    /// Build a loaded set from sections that are already in hand.
    ///
    /// The set is validated before it is accepted: every profile section must
    /// be present exactly once, with the width the SQL adapters enforce. That
    /// makes a hand-built or restored set indistinguishable from a loaded one,
    /// which is the point: a caller cannot publish a set that the loader could
    /// never have produced.
    ///
    /// [`BootTableLoader::load`] uses this to publish its own result, so the two paths
    /// cannot diverge.
    ///
    /// # Errors
    ///
    /// Returns [`BootTableLoadError::Compose`] for a duplicate, a section the
    /// profile disables, a wrong width, a count that disagrees with the data
    /// length, or a missing section.
    pub fn new(
        profile: BootFeatureProfile,
        sections: Vec<BootSection>,
    ) -> Result<Self, BootTableLoadError> {
        let mut loaded = LoadedBootSections::try_new(profile)
            .map_err(|error| BootTableLoadError::Compose(error.to_string()))?;
        for section in sections {
            loaded
                .insert(section)
                .map_err(|error| BootTableLoadError::Compose(error.to_string()))?;
        }
        if !loaded.is_complete() {
            return Err(BootTableLoadError::Compose(format!(
                "the profile needs {} sections but the set is incomplete",
                loaded.slot_count()
            )));
        }
        let sections = loaded
            .into_ordered()
            .map_err(|error| BootTableLoadError::Compose(error.to_string()))?;
        Ok(Self { profile, sections })
    }

    /// The feature profile these sections were selected for.
    #[must_use]
    pub const fn profile(&self) -> BootFeatureProfile {
        self.profile
    }

    /// The loaded sections, in the profile's wire order.
    #[must_use]
    pub fn sections(&self) -> &[BootSection] {
        &self.sections
    }

    /// How many records the loaded sections hold in total.
    #[must_use]
    pub fn record_count(&self) -> usize {
        self.sections
            .iter()
            .map(|section| usize::from(section.count))
            .sum()
    }

    /// The declared record count of one section, or `None` if it is absent.
    ///
    /// An absent optional section is not a zero-count section. A test that wants
    /// to prove a table loaded must ask for its count, not the section count.
    #[must_use]
    pub fn section_count(&self, kind: protocol::db_boot::BootSectionKind) -> Option<u16> {
        self.sections
            .iter()
            .find(|section| section.kind == kind)
            .map(|section| section.count)
    }

    /// Compose the boot payload for one request.
    ///
    /// The tail clock is an argument, not a field, because the legacy
    /// `QUERY_BOOT` calls `time(0)` on every request while the tables stay
    /// resident in memory. Freezing the clock at load time would serve a stale
    /// timestamp for the process lifetime.
    ///
    /// # Errors
    ///
    /// Returns the composition failure.
    pub fn compose(
        &self,
        monarch: BootMonarchInfo,
        global_time: i32,
        active: BootItemIdRange,
        spare: BootItemIdRange,
        gm: &BootGmTail,
    ) -> Result<BootSnapshot, BootTableLoadError> {
        let range_width = |size: usize| {
            u16::try_from(size).map_err(|_| {
                BootTableLoadError::Compose(format!("wire width {size} overflows u16"))
            })
        };
        let parts = BootSnapshotParts {
            sections: self.sections.clone(),
            global_time,
            item_id_ranges: BootItemIdRanges {
                record_size: range_width(ITEM_ID_RANGE_WIRE_SIZE)?,
                declared_count: 1,
                active,
                spare,
            },
            gm_hosts: BootGmHostSection {
                record_size: range_width(GM_HOST_WIRE_SIZE)?,
                count: gm.host_count_u16()?,
                hosts: gm.hosts.clone(),
            },
            admins: BootAdminSection {
                record_size: range_width(ADMIN_INFO_WIRE_SIZE)?,
                count: gm.admin_count_u16()?,
                admins: gm.admins.clone(),
            },
            monarch,
            monarch_candidacy: BootMonarchCandidacySection {
                record_size: range_width(MONARCH_CANDIDACY_WIRE_SIZE)?,
                count: 0,
                candidates: Vec::new(),
            },
        };
        BootSnapshot::compose(self.profile, parts)
            .map_err(|error| BootTableLoadError::Compose(error.to_string()))
    }
}

/// Reads the boot tables from SQL and composes them into profile sections.
#[derive(Debug, Clone)]
pub struct BootTableLoader {
    postfix: TablePostfix,
    profile: BootFeatureProfile,
    /// The load-time `time(0)` used by the premium market-price filter.
    ///
    /// This is a *query* parameter, not wire data. Legacy
    /// `InitializePrivateShopMarketItemPrice` passes `time(0)` into
    /// `FROM_UNIXTIME(%u)`, so it is read once when the tables load. The boot
    /// tail's own `time_t` is a different value: legacy `QUERY_BOOT` calls
    /// `time(0)` again per request, and
    /// [`LoadedTables::compose`] therefore takes that one as an argument.
    unix_seconds: u32,
}

impl BootTableLoader {
    /// Build a loader for one postfix, profile, and load-time clock reading.
    ///
    /// `unix_seconds` must be the current Unix time, not a placeholder. It is
    /// interpolated into the market-price `FROM_UNIXTIME(%u)`, so passing zero
    /// does not fail loudly — it silently selects every row as older than the
    /// day interval and loads an empty price table.
    ///
    /// # Errors
    ///
    /// Returns [`BootTableLoadError::Postfix`] when the configured value is not a
    /// valid table identifier. A bad postfix fails here, once, rather than
    /// becoming an SQL identifier interpolated fourteen times.
    pub fn new(
        configured_postfix: Option<&str>,
        profile: BootFeatureProfile,
        unix_seconds: u32,
    ) -> Result<Self, BootTableLoadError> {
        let postfix = TablePostfix::from_config(configured_postfix)
            .map_err(|error| BootTableLoadError::Postfix(error.to_string()))?;
        Ok(Self {
            postfix,
            profile,
            unix_seconds,
        })
    }

    /// The validated postfix every postfixed table query will use.
    #[must_use]
    pub const fn postfix(&self) -> &TablePostfix {
        &self.postfix
    }

    /// The feature profile sections are selected for.
    #[must_use]
    pub const fn profile(&self) -> BootFeatureProfile {
        self.profile
    }

    /// Read the `gmhost` and `gmlist` tail for one requesting address.
    ///
    /// Legacy resolves both inside `QUERY_BOOT` rather than once at startup, so
    /// this is a per-request read. The administrator list is filtered on the
    /// requester's `szIP`, which is why the address is an argument.
    ///
    /// A missing common pool is an error, not an empty list. Legacy always
    /// queries `SQL_COMMON` here, so silently returning zero rows would turn a
    /// configuration defect into a game server with no GM hosts and no
    /// administrators, which looks like a deliberate setting.
    ///
    /// # Errors
    ///
    /// Returns [`BootTableLoadError::Database`] when the common pool is absent
    /// or a query fails, and [`BootTableLoadError::Query`] when the requesting
    /// address cannot be interpolated into the legacy statement.
    pub async fn load_gm_tail(
        &self,
        sources: &BootDataSources,
        request_ip: Option<&str>,
    ) -> Result<BootGmTail, BootTableLoadError> {
        let common = sources
            .common
            .as_ref()
            .ok_or_else(|| BootTableLoadError::Database {
                table: "gmhost",
                detail: "the common database is not configured, so the GM host and \
                         administrator lists cannot be read"
                    .to_owned(),
            })?;
        let limits = GmSectionLimits::default();
        let query = AdminQuery::new(request_ip).map_err(|error| BootTableLoadError::Query {
            table: "gmlist",
            detail: error.to_string(),
        })?;
        let hosts = crate::gm_sqlx::load_gm_hosts_sqlx(common, limits)
            .await
            .map_err(|error| database_error("gmhost", error))?;
        let admins = crate::gm_sqlx::load_admins_sqlx(common, &query, limits)
            .await
            .map_err(|error| database_error("gmlist", error))?;
        Ok(BootGmTail { hosts, admins })
    }

    /// The load-time Unix time used by the market-price filter.
    #[must_use]
    pub const fn unix_seconds(&self) -> u32 {
        self.unix_seconds
    }

    /// Read every profile-selected table and compose the sections.
    ///
    /// The section order below is the legacy **wire** order from
    /// `CClientManager::QUERY_BOOT`, which is not the legacy load order
    /// (`InitializeTables` loads event third and the market price last). The
    /// registry sorts by profile order regardless, so the order here is for
    /// readability and for matching the legacy failure messages, not for wire
    /// correctness.
    ///
    /// # Errors
    ///
    /// Returns the first table failure. A partial result is never returned.
    pub async fn load(
        &self,
        sources: &BootDataSources,
    ) -> Result<LoadedTables, BootTableLoadError> {
        let player = sources
            .player
            .as_ref()
            .ok_or_else(|| BootTableLoadError::Database {
                table: "player",
                detail: "the player database is not configured, so no boot table can be read"
                    .to_owned(),
            })?;
        let mut loaded = LoadedBootSections::try_new(self.profile)
            .map_err(|error| BootTableLoadError::Compose(error.to_string()))?;

        self.load_unconditional_tables(player, &mut loaded).await?;
        self.load_profile_tables(player, &mut loaded).await?;

        let sections = loaded
            .into_ordered()
            .map_err(|error| BootTableLoadError::Compose(error.to_string()))?;
        LoadedTables::new(self.profile, sections)
    }

    /// Load the tables every profile sends, in legacy wire order.
    ///
    /// # Errors
    ///
    /// Returns the first table failure. A partial result is never returned.
    async fn load_unconditional_tables(
        &self,
        player: &ConnectionPool,
        loaded: &mut LoadedBootSections,
    ) -> Result<(), BootTableLoadError> {
        // mob_proto
        let mob_loader = MobProtoLoader::new(&self.postfix, MobProtoSectionLimits::default())
            .map_err(|error| query_error("mob_proto", error))?;
        let section = crate::mob_proto_sqlx::load_mob_proto_table_section_sqlx(player, &mob_loader)
            .await
            .map_err(|error| database_error("mob_proto", error))?;
        loaded
            .insert(section)
            .map_err(|error| BootTableLoadError::Compose(error.to_string()))?;

        // item_proto
        let item_loader = ItemProtoLoader::new(&self.postfix, ItemProtoSectionLimits::default())
            .map_err(|error| query_error("item_proto", error))?;
        let section =
            crate::item_proto_sqlx::load_item_proto_table_section_sqlx(player, &item_loader)
                .await
                .map_err(|error| database_error("item_proto", error))?;
        loaded
            .insert(section)
            .map_err(|error| BootTableLoadError::Compose(error.to_string()))?;

        // shop: no TABLE_POSTFIX in the legacy query.
        let section = crate::shop_sqlx::load_shop_table_section_sqlx(
            player,
            &ShopTableLoader::default_with_limits(),
        )
        .await
        .map_err(|error| database_error("shop", error))?;
        loaded
            .insert(section)
            .map_err(|error| BootTableLoadError::Compose(error.to_string()))?;

        // skill_proto
        let skill_loader = SkillTableLoader::new(&self.postfix, SkillSectionLimits::default())
            .map_err(|error| query_error("skill", error))?;
        let section = crate::skill_sqlx::load_skill_table_section_sqlx(player, &skill_loader)
            .await
            .map_err(|error| database_error("skill", error))?;
        loaded
            .insert(section)
            .map_err(|error| BootTableLoadError::Compose(error.to_string()))?;

        // refine_proto
        let refine_loader = RefineProtoLoader::new(&self.postfix, RefineSectionLimits::default())
            .map_err(|error| query_error("refine_proto", error))?;
        let section = crate::refine_sqlx::load_refine_proto_section_sqlx(player, &refine_loader)
            .await
            .map_err(|error| database_error("refine_proto", error))?;
        loaded
            .insert(section)
            .map_err(|error| BootTableLoadError::Compose(error.to_string()))?;

        // item_attr
        let item_attr_loader = ItemAttrLoader::new(&self.postfix, ItemAttrSectionLimits::default())
            .map_err(|error| query_error("item_attr", error))?;
        let section =
            crate::item_attr_sqlx::load_item_attr_table_section_sqlx(player, &item_attr_loader)
                .await
                .map_err(|error| database_error("item_attr", error))?;
        loaded
            .insert(section)
            .map_err(|error| BootTableLoadError::Compose(error.to_string()))?;

        // item_attr_rare: 14 source columns, so its own loader and limit.
        let item_rare_loader = ItemRareLoader::new(&self.postfix, ItemAttrSectionLimits::default())
            .map_err(|error| query_error("item_attr_rare", error))?;
        let section =
            crate::item_attr_sqlx::load_item_rare_table_section_sqlx(player, &item_rare_loader)
                .await
                .map_err(|error| database_error("item_attr_rare", error))?;
        loaded
            .insert(section)
            .map_err(|error| BootTableLoadError::Compose(error.to_string()))?;

        // banword: no TABLE_POSTFIX in the legacy query.
        let section =
            crate::banword_sqlx::load_banword_section_sqlx(player, BanwordSectionLimits::default())
                .await
                .map_err(|error| database_error("banword", error))?;
        loaded
            .insert(section)
            .map_err(|error| BootTableLoadError::Compose(error.to_string()))?;

        // land
        let land_loader = LandTableLoader::new(&self.postfix, LandSectionLimits::default())
            .map_err(|error| query_error("land", error))?;
        let section = crate::land_sqlx::load_land_table_section_sqlx(player, &land_loader)
            .await
            .map_err(|error| database_error("land", error))?;
        loaded
            .insert(section)
            .map_err(|error| BootTableLoadError::Compose(error.to_string()))?;

        // object_proto
        let object_proto_loader =
            ObjectProtoTableLoader::new(&self.postfix, ObjectProtoSectionLimits::default())
                .map_err(|error| query_error("object_proto", error))?;
        let section = crate::object_proto_sqlx::load_object_proto_table_section_sqlx(
            player,
            &object_proto_loader,
        )
        .await
        .map_err(|error| database_error("object_proto", error))?;
        loaded
            .insert(section)
            .map_err(|error| BootTableLoadError::Compose(error.to_string()))?;

        // object
        let object_loader = ObjectTableLoader::new(&self.postfix, ObjectSectionLimits::default())
            .map_err(|error| query_error("object", error))?;
        let section = crate::object_sqlx::load_object_table_section_sqlx(player, &object_loader)
            .await
            .map_err(|error| database_error("object", error))?;
        loaded
            .insert(section)
            .map_err(|error| BootTableLoadError::Compose(error.to_string()))?;

        Ok(())
    }

    /// Load the tables the active feature profile selects.
    ///
    /// These three are absent from a profile that does not enable the matching
    /// feature, so a minimal profile legitimately loads fewer tables.
    ///
    /// # Errors
    ///
    /// Returns the first table failure. A partial result is never returned.
    async fn load_profile_tables(
        &self,
        player: &ConnectionPool,
        loaded: &mut LoadedBootSections,
    ) -> Result<(), BootTableLoadError> {
        // shopex: profile-selected, and no TABLE_POSTFIX in the legacy query.
        if self.profile.renewal_shop_ex {
            let section = crate::renewal_shop_sqlx::load_renewal_shop_table_section_sqlx(
                player,
                &RenewalShopTableLoader::default_with_limits(),
            )
            .await
            .map_err(|error| database_error("shopex", error))?;
            loaded
                .insert(section)
                .map_err(|error| BootTableLoadError::Compose(error.to_string()))?;
        }

        // event: profile-selected.
        if self.profile.event_manager {
            let query =
                EventQuery::new(&self.postfix).map_err(|error| query_error("event", error))?;
            let section = crate::event_sqlx::load_event_table_section_sqlx(
                player,
                &query,
                EventSectionLimits::default(),
            )
            .await
            .map_err(|error| database_error("event", error))?;
            loaded
                .insert(section)
                .map_err(|error| BootTableLoadError::Compose(error.to_string()))?;
        }

        // private_shop_sale_history: profile-selected, filtered by clock.
        if self.profile.premium_market_price {
            let query = MarketPriceQuery::new(&self.postfix, self.unix_seconds)
                .map_err(|error| query_error("premium_market_price", error))?;
            let section = crate::market_price_sqlx::load_market_price_section_sqlx(
                player,
                &query,
                MarketPriceSectionLimits::default(),
            )
            .await
            .map_err(|error| database_error("premium_market_price", error))?;
            loaded
                .insert(section)
                .map_err(|error| BootTableLoadError::Compose(error.to_string()))?;
        }

        Ok(())
    }
    /// Read the monarch rows and select the incumbent monarch.
    ///
    /// Kept out of the table load because the monarch record is boot *tail* data
    /// rather than a table section, and the legacy code fetches it after the
    /// tables. The legacy query is a `monarch` JOIN `player%s`, so the postfix
    /// belongs to the joined player table, not to `monarch` itself. There is
    /// still no `SELECT` for `monarch_candidacy` in the legacy source, so this
    /// returns only the incumbent.
    ///
    /// # Errors
    ///
    /// Returns the acquisition failure, including the fail-closed empty result.
    pub async fn load_monarch(
        &self,
        sources: &BootDataSources,
    ) -> Result<BootMonarchInfo, BootTableLoadError> {
        let player = sources
            .player
            .as_ref()
            .ok_or_else(|| BootTableLoadError::Database {
                table: "monarch",
                detail: "the player database is not configured".to_owned(),
            })?;
        let monarch_loader = MonarchLoader::new(&self.postfix, MonarchLimits::default())
            .map_err(|error| query_error("monarch", error))?;
        crate::monarch_sqlx::load_monarch_info_sqlx(player, &monarch_loader)
            .await
            .map_err(|error| database_error("monarch", error))
    }

    /// The legacy four-row monarch source bound.
    #[must_use]
    pub const fn monarch_row_limit(&self) -> usize {
        MONARCH_MAX_SOURCE_ROWS
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::banword::BANWORD_QUERY as BANWORD_QUERY_TEXT;
    use crate::postfix::TablePostfixError;
    use protocol::db_boot::BootSectionKind;

    fn loader(profile: BootFeatureProfile) -> BootTableLoader {
        BootTableLoader::new(Some("_test"), profile, 1_700_000_000)
            .expect("an identifier-shaped postfix is valid")
    }

    /// A loader with no SQL available at all.
    fn offline_sources() -> BootDataSources {
        BootDataSources::default()
    }

    // --- postfix validation ------------------------------------------------

    #[test]
    fn an_invalid_postfix_fails_once_before_any_query_is_built() {
        let error = BootTableLoader::new(
            Some("_bad; DROP TABLE player"),
            BootFeatureProfile::active(),
            0,
        )
        .expect_err("a semicolon is not a table identifier");
        assert!(
            matches!(error, BootTableLoadError::Postfix(_)),
            "a bad postfix is reported as a postfix failure, got {error:?}"
        );
        assert!(
            error.to_string().contains("TABLE_POSTFIX"),
            "the message names the configuration key: {error}"
        );
    }

    #[test]
    fn an_absent_or_empty_postfix_is_the_legacy_default() {
        for value in [None, Some("")] {
            let loader = BootTableLoader::new(value, BootFeatureProfile::active(), 0)
                .expect("an absent or empty postfix is the legacy default");
            assert!(loader.postfix().is_empty());
            assert_eq!(loader.postfix().as_str(), "");
        }
    }

    #[test]
    fn a_validated_postfix_is_reported_verbatim() {
        // The legacy code appends the postfix with no dot or separator, so the
        // value must survive byte for byte, including a leading underscore.
        let loader = loader(BootFeatureProfile::active());
        assert_eq!(loader.postfix().as_str(), "_test");
    }

    #[test]
    fn the_postfix_error_names_the_offending_byte() {
        // The failure has to be diagnosable from the message alone, or an
        // operator cannot tell which character to fix.
        let error = TablePostfix::from_config(Some("bad-name")).expect_err("a dash is invalid");
        match error {
            TablePostfixError::InvalidCharacter { index, byte } => {
                assert_eq!(index, 3);
                assert_eq!(byte, b'-');
            }
            other @ TablePostfixError::TooLong { .. } => {
                panic!("expected an invalid-character error, got {other:?}")
            }
        }
    }

    // --- the market-price clock is a real clock ---------------------------

    #[test]
    fn the_market_price_filter_uses_the_supplied_unix_time() {
        // The filter is `DATEDIFF(time, FROM_UNIXTIME(%u)) < N`. A zero here
        // does not fail; it makes every row older than N days from the epoch
        // and silently loads an empty price table. Legacy passes `time(0)`, so
        // the loader has to be built with the current time.
        let loader = BootTableLoader::new(Some(""), BootFeatureProfile::active(), 1_700_000_000)
            .expect("a valid postfix builds a loader");
        let query = MarketPriceQuery::new(loader.postfix(), loader.unix_seconds())
            .expect("the market-price query builds");
        assert_eq!(query.unix_seconds(), 1_700_000_000);
        assert!(
            query.as_str().contains("FROM_UNIXTIME(1700000000)"),
            "the supplied time must reach the statement, got {:?}",
            query.as_str()
        );
        assert!(
            !query.as_str().contains("FROM_UNIXTIME(0)"),
            "an epoch filter would load an empty price table"
        );
    }

    // --- postfix is per-table, not global ---------------------------------

    #[test]
    fn only_the_legacy_postfixed_tables_take_a_postfix() {
        // The audit result, restated as a test. The legacy source appends
        // TABLE_POSTFIX to eleven of the fourteen boot tables; `shop`, `shopex`,
        // and `banword` take none. A single global postfix would be wrong for
        // exactly those three, so this test names all fourteen and pins which
        // side of the split each one is on.
        const POSTFIXED: [(&str, BootSectionKind); 11] = [
            ("mob_proto", BootSectionKind::Mob),
            ("item_proto", BootSectionKind::Item),
            ("skill_proto", BootSectionKind::Skill),
            ("refine_proto", BootSectionKind::Refine),
            ("item_attr", BootSectionKind::ItemAttr),
            ("item_attr_rare", BootSectionKind::ItemRare),
            ("land", BootSectionKind::Land),
            ("object_proto", BootSectionKind::ObjectProto),
            ("object", BootSectionKind::Object),
            ("event", BootSectionKind::Event),
            (
                "private_shop_sale_history",
                BootSectionKind::PremiumMarketPrice,
            ),
        ];
        const POSTFIX_FREE: [(&str, BootSectionKind); 3] = [
            ("shop", BootSectionKind::Shop),
            ("shopex", BootSectionKind::RenewalShop),
            ("banword", BootSectionKind::Banword),
        ];

        // Every one of the fourteen must be classified, and the two lists must
        // together cover the active profile exactly, or a future table could
        // be added to one list and never to the other.
        let classified: Vec<BootSectionKind> = POSTFIXED
            .iter()
            .chain(POSTFIX_FREE.iter())
            .map(|(_, kind)| *kind)
            .collect();
        for kind in BootFeatureProfile::active().section_kinds() {
            assert!(
                classified.contains(kind),
                "{kind:?} is not classified as postfixed or postfix-free"
            );
        }
        assert_eq!(classified.len(), 14);
        assert_eq!(POSTFIXED.len() + POSTFIX_FREE.len(), classified.len());

        // The three postfix-free tables must be readable by a loader that was
        // built with no postfix at all, which is how this module calls them.
        for (table, _kind) in POSTFIX_FREE {
            // The legacy statement is a fixed string, so the postfix cannot be
            // interpolated anywhere in it. Matching the `FROM <table>` clause
            // rather than the whole text matters: the shopex statement contains
            // `attrtype0..6` columns, and a naive substring test for a postfix
            // beginning with `_t` would match `attrtype` and report a false
            // failure on a column name.
            let (text, from_table) = match table {
                "shop" => (crate::shop::SHOP_TABLE_QUERY, "shop"),
                "shopex" => (crate::renewal_shop::RENEWAL_SHOP_TABLE_QUERY, "shopex"),
                "banword" => (BANWORD_QUERY_TEXT, "banword"),
                other => panic!("{other} is not a postfix-free boot table"),
            };
            let clause = text
                .split("FROM ")
                .nth(1)
                .unwrap_or_else(|| panic!("the {table} statement has no FROM clause"));
            let referenced = clause.split_whitespace().next().unwrap_or_default();
            assert_eq!(
                referenced, from_table,
                "the {table} statement must name {from_table} with no postfix"
            );
        }
    }

    // --- profile selection -------------------------------------------------

    #[test]
    fn the_active_profile_selects_fourteen_sections() {
        let profile = BootFeatureProfile::active();
        assert_eq!(profile.section_kinds().len(), 14);
    }

    #[test]
    fn the_minimal_profile_omits_all_three_optional_sections() {
        // The three optional tables are the only difference between the two
        // profiles, so a load under `minimal` must not query for them.
        let minimal = BootFeatureProfile::minimal();
        assert!(!minimal.renewal_shop_ex);
        assert!(!minimal.event_manager);
        assert!(!minimal.premium_market_price);
        for kind in [
            BootSectionKind::RenewalShop,
            BootSectionKind::Event,
            BootSectionKind::PremiumMarketPrice,
        ] {
            assert!(
                !minimal.section_kinds().contains(&kind),
                "{kind:?} must not be selected by the minimal profile"
            );
        }
        assert!(BootFeatureProfile::active()
            .section_kinds()
            .contains(&BootSectionKind::RenewalShop));
    }

    #[test]
    fn the_loader_keeps_the_profile_it_was_given() {
        let loader = loader(BootFeatureProfile::minimal());
        assert_eq!(loader.profile(), BootFeatureProfile::minimal());
        assert_eq!(loader.unix_seconds(), 1_700_000_000);
    }

    // --- failure policy ---------------------------------------------------

    #[tokio::test]
    async fn a_missing_player_database_is_a_hard_error() {
        // Returning a partial boot here would hand a game server a world with no
        // monsters and no items while reporting success.
        let error = loader(BootFeatureProfile::active())
            .load(&offline_sources())
            .await
            .expect_err("without a player database no table can be read");
        match error {
            BootTableLoadError::Database { table, detail } => {
                assert_eq!(table, "player");
                assert!(
                    detail.contains("not configured"),
                    "the message explains the cause: {detail}"
                );
            }
            other => panic!("expected an acquisition failure, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn the_monarch_load_also_requires_the_player_database() {
        let error = loader(BootFeatureProfile::active())
            .load_monarch(&offline_sources())
            .await
            .expect_err("the monarch join needs the player database");
        assert!(
            matches!(
                error,
                BootTableLoadError::Database {
                    table: "monarch",
                    ..
                }
            ),
            "the monarch failure names the monarch table, got {error:?}"
        );
    }

    #[test]
    fn an_unreachable_table_error_names_its_table() {
        // An operator reading one log line must learn which table failed.
        let error = database_error("land", "connection refused");
        assert_eq!(
            error.to_string(),
            "the land table could not be read: connection refused"
        );
    }

    #[test]
    fn every_error_variant_mentions_its_table_or_its_boundary() {
        let cases: [BootTableLoadError; 4] = [
            query_error("shop", "bad name"),
            database_error("banword", "no such table"),
            BootTableLoadError::Compose("registry incomplete".to_owned()),
            BootTableLoadError::Postfix("bad;name".to_owned()),
        ];
        let rendered: Vec<String> = cases.iter().map(ToString::to_string).collect();
        assert!(rendered[0].contains("shop"));
        assert!(rendered[0].contains("query"));
        assert!(rendered[1].contains("banword"));
        assert!(rendered[2].contains("composition"));
        assert!(rendered[3].contains("TABLE_POSTFIX"));
        // Distinct failures must not collapse into one indistinguishable line.
        assert_eq!(
            rendered
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            4
        );
    }

    // --- the cache fail-closed boundary is exercised in `boot_cache` -------
    // The load itself needs a database, so the composition and the cache are
    // covered by `boot_cache`'s tests, which inject a hand-built table set.

    /// Every boot table statement, checked against the legacy loader in one place.
    ///
    /// Each expected string is transcribed from the legacy source named beside
    /// it, with `%s` replaced by the empty postfix used here. Collecting them in
    /// one test means a change to any statement shows up as one failure that
    /// names the table, instead of as a silent divergence in one of fourteen
    /// separate tests. The per-table tests already pin the individual strings;
    /// this one is the cross-check that none of them drifted together.
    ///
    /// The postfix is empty so the expected text is readable as plain SQL. The
    /// interpolated form is checked by the postfix tests.
    #[test]
    fn every_boot_statement_matches_the_legacy_loader() {
        use crate::postfix::TablePostfix;

        let postfix = TablePostfix::default();
        let default_postfix = postfix.as_str();
        assert_eq!(
            default_postfix, "",
            "this test reads better with no postfix"
        );

        // `ClientManagerBoot.cpp:1395-1405` (ENABLE_PROTO_FROM_DB path).
        let mob = crate::mob_proto::MobProtoQuery::new(
            &postfix,
            &crate::mob_proto::MobProtoLocaleColumn::default(),
        )
        .expect("the fixed mob statement builds");
        assert_eq!(
            mob.as_str(),
            "SELECT vnum, name, name, type, rank, battle_type, level, size+0, ai_flag+0, setRaceFlag+0, setImmuneFlag+0, on_click, empire, drop_item, resurrection_vnum, folder, st, dx, ht, iq, damage_min, damage_max, max_hp, regen_cycle, regen_percent, exp, gold_min, gold_max, def, attack_speed, move_speed, aggressive_hp_pct, aggressive_sight, attack_range, polymorph_item, enchant_curse, enchant_slow, enchant_poison, enchant_stun, enchant_critical, enchant_penetrate, resist_sword, resist_twohand, resist_dagger, resist_bell, resist_fan, resist_bow, resist_fire, resist_elect, resist_magic, resist_wind, resist_poison, dam_multiply, summon, drain_sp, skill_vnum0, skill_level0, skill_vnum1, skill_level1, skill_vnum2, skill_level2, skill_vnum3, skill_level3, skill_vnum4, skill_level4, sp_berserk, sp_stoneskin, sp_godspeed, sp_deathblow, sp_revive FROM mob_proto ORDER BY vnum;"
        );

        // `ClientManagerBoot.cpp:1553-1561` (ENABLE_PROTO_FROM_DB path).
        let item = crate::item_proto::ItemProtoQuery::new(
            &postfix,
            &crate::item_proto::ItemProtoLocaleColumn::default(),
        )
        .expect("the fixed item statement builds");
        assert_eq!(
            item.as_str(),
            "SELECT vnum, type, subtype, name, name, gold, shop_buy_price, weight, size, flag, wearflag, antiflag, immuneflag+0, refined_vnum, refine_set, magic_pct, socket_pct, addon_type, limittype0, limitvalue0, limittype1, limitvalue1, applytype0, applyvalue0, applytype1, applyvalue1, applytype2, applyvalue2, value0, value1, value2, value3, value4, value5 FROM item_proto ORDER BY vnum;"
        );

        // `ClientManagerBoot.cpp:305-310`. No postfix and a LEFT JOIN, so a
        // shop with no items still produces a row.
        assert_eq!(
            crate::shop::ShopTableQuery::new().as_str(),
            "SELECT shop.vnum, shop.npc_vnum, shop_item.item_vnum, shop_item.count FROM shop LEFT JOIN shop_item ON shop.vnum = shop_item.shop_vnum ORDER BY shop.vnum, shop_item.item_vnum"
        );

        // `ClientManagerBoot.cpp:659-665`.
        let skill = crate::postfix::SkillTableQuery::new(&postfix)
            .expect("the fixed skill statement builds");
        assert!(skill.as_str().starts_with(
            "SELECT dwVnum, szName, bType, bMaxLevel, dwSplashRange, szPointOn, szPointPoly, szSPCostPoly, szDurationPoly, szDurationSPCostPoly, szCooldownPoly, szMasterBonusPoly, setFlag+0, setAffectFlag+0, szPointOn2, szPointPoly2, szDurationPoly2, setAffectFlag2+0, szPointOn3, szPointPoly3, szDurationPoly3, szGrandMasterAddSPCostPoly, bLevelStep, bLevelLimit, prerequisiteSkillVnum, prerequisiteSkillLevel, iMaxHit, szSplashAroundDamageAdjustPoly, eSkillType+0, dwTargetRange FROM skill_proto ORDER BY dwVnum"
        ), "got {}", skill.as_str());

        // `ClientManagerBoot.cpp:156`. Note the **double space** before
        // `vnum3` and the **absence of any ORDER BY**: the legacy statement is
        // not sorted, so a sorted rewrite would be a behavior change.
        let refine = crate::postfix::RefineProtoQuery::new(&postfix)
            .expect("the fixed refine statement builds");
        assert_eq!(
            refine.as_str(),
            "SELECT id, cost, prob, vnum0, count0, vnum1, count1, vnum2, count2,  vnum3, count3, vnum4, count4 FROM refine_proto",
        );

        // `ClientManagerBoot.cpp:777` and `:851`. The rare table selects
        // fourteen columns, not eighteen, so it has its own statement. These
        // two are the source-fixed `%s` templates rather than a built
        // statement, so the placeholder is compared literally; the postfix
        // tests cover the interpolation.
        assert_eq!(
            crate::item_attr::ITEM_ATTR_QUERY_SQL,
            "SELECT apply, apply+0, prob, lv1, lv2, lv3, lv4, lv5, weapon, body, wrist, foots, neck, head, shield, ear, talisman, glove FROM item_attr%s ORDER BY apply"
        );
        assert_eq!(
            crate::item_attr::ITEM_ATTR_RARE_QUERY_SQL,
            "SELECT apply, apply+0, prob, lv1, lv2, lv3, lv4, lv5, weapon, body, wrist, foots, neck, head, shield, ear FROM item_attr_rare%s ORDER BY apply"
        );

        // `ClientManagerBoot.cpp:750`. No postfix.
        assert_eq!(
            crate::banword::BanwordQuery.as_str(),
            "SELECT word FROM banword"
        );

        // `ClientManagerBoot.cpp:937-938`. The `enable='YES'` filter is part of
        // the statement, so a disabled land never reaches the client.
        let land =
            crate::postfix::LandTableQuery::new(&postfix).expect("the fixed land statement builds");
        assert_eq!(
            land.as_str(),
            "SELECT id, map_index, x, y, width, height, guild_id, guild_level_limit, price FROM land WHERE enable='YES' ORDER BY id"
        );

        // `ClientManagerBoot.cpp:1040-1041`.
        let object_proto = crate::object_proto::ObjectProtoTableQuery::new(&postfix)
            .expect("the fixed object_proto statement builds");
        assert_eq!(
            object_proto.as_str(),
            "SELECT vnum, price, materials, upgrade_vnum, upgrade_limit_time, life, reg_1, reg_2, reg_3, reg_4, npc, group_vnum, dependent_group FROM object_proto ORDER BY vnum"
        );

        // `ClientManagerBoot.cpp:1113`.
        let object = crate::object::ObjectTableQuery::new(&postfix)
            .expect("the fixed object statement builds");
        assert_eq!(
            object.as_str(),
            "SELECT id, land_id, vnum, map_index, x, y, x_rot, y_rot, z_rot, life FROM object ORDER BY id"
        );

        // `ClientManagerBoot.cpp:384+` (ENABLE_RENEWAL_SHOPEX). No postfix.
        assert!(
            crate::renewal_shop::RenewalShopTableQuery::new().as_str()
                .starts_with("SELECT shopex.vnum, shopex.name, shopex.npc_vnum, shopex_item.item_vnum, shopex_item.count, shopex_item.price, shopex_item.price_vnum, shopex_item.price_type+0, socket0, socket1, socket2, socket3, socket4, socket5, "),
            "got {}",
            crate::renewal_shop::RenewalShopTableQuery::new().as_str()
        );

        // `ClientManagerBoot.cpp:1695`. Sorted by **start**, not by id.
        let event = crate::event::EventQuery::new(&postfix).expect("the event statement builds");
        assert_eq!(
            event.as_str(),
            "SELECT id, type, UNIX_TIMESTAMP(start), UNIX_TIMESTAMP(end), value0, value1, completed FROM event ORDER BY start"
        );

        // `ClientManagerPrivateShop.cpp:852-853`. The interval is the
        // fixed `VALID_MARKET_PRICE_DAY_INTERVAL`, and the second `%u` is
        // `time(0)` at the moment the statement is built.
        let market = crate::market_price::MarketPriceQuery::new(&postfix, 1_700_000_000_u32)
            .expect("the market-price statement builds");
        assert_eq!(
            market.as_str(),
            "SELECT vnum, gold, cheque FROM private_shop_sale_history WHERE DATEDIFF(time, FROM_UNIXTIME(1700000000)) < 3"
        );
    }
}
