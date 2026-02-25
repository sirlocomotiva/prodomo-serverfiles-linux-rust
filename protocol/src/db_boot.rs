//! Safe parser for the legacy version-6 database boot payload.
//!
//! `CClientManager::QUERY_BOOT` writes a `u32` payload length, a version byte,
//! table sections, and a small fixed tail.  `CInputDB::Boot` consumes that
//! payload on the game side.  The request side is the packed 24-byte
//! `TPacketGDBoot` payload.  This module only decodes and encodes bytes.  It
//! does not load tables, contact a peer, or apply any table data.
//!
//! The legacy item-ID-range header is unusual: it declares one record but
//! writes two `TItemIDRangeTable` values (the active range and the spare
//! range).  The parser preserves and validates that two-record layout rather
//! than treating the second value as the next section.
//!
//! Optional sections are selected by an explicit [`BootFeatureProfile`].  A
//! profile is required because the wire format has no section identifiers and
//! a disabled optional section changes the position of every following field.
//! The parser does not infer a profile from payload bytes.
//!
//! Most ordinary table records remain opaque blobs. Their declared record size
//! and count are checked against the available bytes, but this module does not
//! load or interpret those records. The source-fixed active 255-byte
//! `TMobTable`, active 204-byte `TItemTable`, base and optional renewal
//! `TShopTable`, and other explicitly listed boundaries are decoded for their
//! respective [`BootSectionKind`] values. The source-fixed `TBanwordTable`,
//! `TRefineTable`, `building::TLand`, `building::TObjectProto`,
//! `building::TObject`, optional `TEventTable`, and optional
//! `TMarketItemPrice` boundaries are decoded explicitly below. `TSkillTable`
//! is source-fixed at 1,475 packed bytes, and `TItemAttrTable` normal and rare
//! sections are source-fixed at 71 bytes in the active glove-enabled build.
//! `ENABLE_ITEMSHOP`, when enabled in the legacy sender, emits a separate frame
//! after the boot frame; that frame is not part of this payload and is rejected
//! as trailing data here.

use std::error::Error;
use std::fmt;

use crate::db_records::{
    DbRecordError, EventTableRecord, ItemAttrRecord, ItemTableRecord, LandRecord,
    MarketItemPriceRecord, MobTableRecord, ObjectProtoRecord, ObjectRecord, RefineTableRecord,
    ShopTableRecord, SkillTableRecord, EVENT_TABLE_WIRE_SIZE, ITEM_ATTR_RECORD_WIRE_SIZE,
    ITEM_TABLE_RECORD_WIRE_SIZE, LAND_RECORD_WIRE_SIZE, MARKET_ITEM_PRICE_WIRE_SIZE,
    MOB_TABLE_RECORD_WIRE_SIZE, OBJECT_PROTO_RECORD_WIRE_SIZE, OBJECT_RECORD_WIRE_SIZE,
    REFINE_TABLE_WIRE_SIZE, SHOP_HOST_ITEM_MAX_NUM, SHOP_TABLE_RECORD_WIRE_SIZE,
    SKILL_TABLE_RECORD_WIRE_SIZE,
};
use crate::db_wire::DbFrame;

/// Legacy DB header carrying the boot request from the game side.
pub const HEADER_GD_BOOT: u8 = 9;

/// Legacy DB header carrying the boot response.
pub const HEADER_DG_BOOT: u8 = 43;

/// The legacy boot response is unsolicited and uses a zero peer handle.
pub const DB_BOOT_RESPONSE_HANDLE: u32 = 0;

/// Packed x86 wire size of `TPacketGDBoot` (the GD request payload).
pub const BOOT_REQUEST_WIRE_SIZE: usize = 24;

/// Packed x86 wire size of `TBanwordTable` (`char[BANWORD_MAX_LEN + 1]`).
pub const BANWORD_WIRE_SIZE: usize = 25;

/// The only boot payload version understood by the legacy game code.
pub const DB_BOOT_VERSION: u8 = 6;

/// Wire width of the active x86 `time_t` value in the boot payload.
pub const X86_TIME_T_WIRE_SIZE: usize = 4;

/// Wire width of one packed x86 `TItemIDRangeTable`.
pub const ITEM_ID_RANGE_WIRE_SIZE: usize = 12;

/// Wire width of one GM host string (`char[16]`).
pub const GM_HOST_WIRE_SIZE: usize = 16;

/// Wire width of one packed x86 `tAdminInfo`.
pub const ADMIN_INFO_WIRE_SIZE: usize = 104;

/// Wire width of one packed x86 `TMonarchInfo` in this build.
pub const MONARCH_INFO_WIRE_SIZE: usize = 304;

/// Wire width of one packed x86 `MonarchCandidacy`.
pub const MONARCH_CANDIDACY_WIRE_SIZE: usize = 68;

/// End marker emitted after the monarch tail.
pub const DB_BOOT_END_MARKER: u16 = 0xffff;

/// Conservative default bound for one complete boot payload.
///
/// The wire length is a `u32`, but a peer must not be allowed to make a
/// process reserve an arbitrarily large output copy.  Applications with a
/// larger known boot stream can select their own limit through
/// [`DbBootParser::with_max_payload_size`].
pub const DEFAULT_MAX_DB_BOOT_PAYLOAD_SIZE: usize = 64 * 1024 * 1024;

/// Feature switches which change the ordered boot section list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootFeatureProfile {
    /// Whether `ENABLE_RENEWAL_SHOPEX` adds the second shop section.
    pub renewal_shop_ex: bool,
    /// Whether `__EVENT_MANAGER__` adds the event section.
    pub event_manager: bool,
    /// Whether `__PREMIUM_PRIVATE_SHOP__` adds the market-price section.
    pub premium_market_price: bool,
}

impl BootFeatureProfile {
    /// Construct a profile from its three optional section switches.
    #[must_use]
    pub const fn new(
        renewal_shop_ex: bool,
        event_manager: bool,
        premium_market_price: bool,
    ) -> Self {
        Self {
            renewal_shop_ex,
            event_manager,
            premium_market_price,
        }
    }

    /// The feature set enabled by the current legacy build.
    #[must_use]
    pub const fn active() -> Self {
        Self::new(true, true, true)
    }

    /// The base profile with all three optional sections disabled.
    #[must_use]
    pub const fn minimal() -> Self {
        Self::new(false, false, false)
    }

    /// Return a copy with the renewal shop section enabled or disabled.
    #[must_use]
    pub const fn with_renewal_shop_ex(self, enabled: bool) -> Self {
        Self {
            renewal_shop_ex: enabled,
            ..self
        }
    }

    /// Return a copy with the event section enabled or disabled.
    #[must_use]
    pub const fn with_event_manager(self, enabled: bool) -> Self {
        Self {
            event_manager: enabled,
            ..self
        }
    }

    /// Return a copy with the market-price section enabled or disabled.
    #[must_use]
    pub const fn with_premium_market_price(self, enabled: bool) -> Self {
        Self {
            premium_market_price: enabled,
            ..self
        }
    }

    /// Alias for [`BootFeatureProfile::with_renewal_shop_ex`].
    #[must_use]
    pub const fn with_renewal_shop(self, enabled: bool) -> Self {
        self.with_renewal_shop_ex(enabled)
    }

    /// Alias for [`BootFeatureProfile::with_event_manager`].
    #[must_use]
    pub const fn with_event(self, enabled: bool) -> Self {
        self.with_event_manager(enabled)
    }

    /// Return the ordered section kinds for this profile.
    #[must_use]
    pub const fn section_kinds(&self) -> &'static [BootSectionKind] {
        match (
            self.renewal_shop_ex,
            self.event_manager,
            self.premium_market_price,
        ) {
            (false, false, false) => BASE_SECTION_KINDS,
            (true, false, false) => RENEWAL_SECTION_KINDS,
            (false, true, false) => EVENT_SECTION_KINDS,
            (false, false, true) => MARKET_SECTION_KINDS,
            (true, true, false) => RENEWAL_EVENT_SECTION_KINDS,
            (true, false, true) => RENEWAL_MARKET_SECTION_KINDS,
            (false, true, true) => EVENT_MARKET_SECTION_KINDS,
            (true, true, true) => ACTIVE_SECTION_KINDS,
        }
    }
}

impl Default for BootFeatureProfile {
    fn default() -> Self {
        Self::active()
    }
}

/// Compatibility name for callers that use the DB-specific prefix.
pub type DbBootFeatureProfile = BootFeatureProfile;

/// Names the unlabelled, ordered table sections in a boot payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BootSectionKind {
    /// `TMobTable` records.
    Mob,
    /// `TItemTable` records.
    Item,
    /// The base `TShopTable` records.
    Shop,
    /// The optional renewal `TShopTable` records.
    RenewalShop,
    /// `TSkillTable` records.
    Skill,
    /// `TRefineTable` records.
    Refine,
    /// `TItemAttrTable` records.
    ItemAttr,
    /// Rare `TItemAttrTable` records.
    ItemRare,
    /// `TBanwordTable` records.
    Banword,
    /// `building::TLand` records.
    Land,
    /// `building::TObjectProto` records.
    ObjectProto,
    /// `building::TObject` records.
    Object,
    /// Optional `TEventTable` records.
    Event,
    /// Optional `TMarketItemPrice` records.
    PremiumMarketPrice,
}

impl BootSectionKind {
    /// Return a stable lower-case name for diagnostics.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mob => "mob",
            Self::Item => "item",
            Self::Shop => "shop",
            Self::RenewalShop => "renewal_shop",
            Self::Skill => "skill",
            Self::Refine => "refine",
            Self::ItemAttr => "item_attr",
            Self::ItemRare => "item_rare",
            Self::Banword => "banword",
            Self::Land => "land",
            Self::ObjectProto => "object_proto",
            Self::Object => "object",
            Self::Event => "event",
            Self::PremiumMarketPrice => "premium_market_price",
        }
    }
}

impl fmt::Display for BootSectionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A validated, untyped table section.
///
/// Legacy table records are intentionally not decoded here.  The section
/// retains the exact record size, count, and bytes so a later typed codec can
/// consume it without this parser loading a table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootSection {
    /// Position-derived name of this section.
    pub kind: BootSectionKind,
    /// Size of one record in bytes.
    pub record_size: u16,
    /// Number of records declared by the wire header.
    pub count: u16,
    /// Concatenated record bytes.
    pub data: Vec<u8>,
}

impl BootSection {
    /// Return the number of bytes in the section payload.
    #[must_use]
    pub fn byte_len(&self) -> usize {
        self.data.len()
    }

    /// Borrow the concatenated record bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.data
    }

    /// Iterate over validated record slices.
    ///
    /// Sections produced by this module always have a non-zero record size.
    /// The guard also keeps this method safe for manually constructed values.
    pub fn records(&self) -> impl Iterator<Item = &[u8]> {
        let size = usize::from(self.record_size).max(1);
        self.data
            .chunks_exact(size)
            .filter(|_| self.record_size != 0)
    }

    /// Validate the section metadata and iterate over complete record slices.
    ///
    /// This is the boundary for a future typed table loader. It checks that
    /// the record size is non-zero and that the data length exactly equals
    /// `record_size * count`, with checked `usize` arithmetic.
    ///
    /// # Errors
    ///
    /// Returns [`DbBootError::InvalidRecordSize`] for a zero record size,
    /// [`DbBootError::SizeOverflow`] for an unrepresentable product, or
    /// [`DbBootError::SectionLengthMismatch`] when the data length disagrees
    /// with the declared metadata.
    pub fn try_records(&self) -> Result<impl Iterator<Item = &[u8]>, DbBootError> {
        if self.record_size == 0 {
            return Err(DbBootError::InvalidRecordSize {
                section: self.kind,
                record_size: self.record_size,
            });
        }
        let size = usize::from(self.record_size);
        let expected = checked_product(size, usize::from(self.count))?;
        if self.data.len() != expected {
            return Err(DbBootError::SectionLengthMismatch {
                section: self.kind,
                expected,
                actual: self.data.len(),
            });
        }
        Ok(self.data.chunks_exact(size))
    }

    /// Return one record by zero-based index.
    #[must_use]
    pub fn record(&self, index: usize) -> Option<&[u8]> {
        let size = usize::from(self.record_size);
        if size == 0 {
            return None;
        }
        let start = index.checked_mul(size)?;
        let end = start.checked_add(size)?;
        self.data.get(start..end)
    }

    /// Return whether the section has no records.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
}

/// One packed x86 `TBanwordTable` value.
///
/// This is the first ordinary boot table whose layout is fixed by the
/// active source: it contains only a 25-byte `char` array and has no
/// feature-dependent fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootBanword {
    /// Raw 25-byte word field, including any legacy terminator.
    pub bytes: [u8; BANWORD_WIRE_SIZE],
}

impl BootBanword {
    /// Decode exactly 25 bytes.
    ///
    /// # Errors
    ///
    /// Returns [`DbBootError::InvalidFixedRecordSize`] unless `data` is
    /// exactly 25 bytes.
    pub fn decode(data: &[u8]) -> Result<Self, DbBootError> {
        if data.len() != BANWORD_WIRE_SIZE {
            return Err(DbBootError::InvalidFixedRecordSize {
                field: "banword",
                expected: BANWORD_WIRE_SIZE,
                actual: data.len(),
            });
        }
        let mut bytes = [0_u8; BANWORD_WIRE_SIZE];
        bytes.copy_from_slice(data);
        Ok(Self { bytes })
    }

    /// Return the bytes up to the first NUL, if present.
    #[must_use]
    pub fn as_str(&self) -> &str {
        let end = self
            .bytes
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(self.bytes.len());
        std::str::from_utf8(&self.bytes[..end]).unwrap_or("")
    }
}

/// Decode the typed `TBanwordTable` records in a boot section.
///
/// The section kind, declared record width, count, and byte length are all
/// checked before any record is returned. Other boot table types remain
/// opaque until their feature-dependent x86 layouts are independently
/// verified.
///
/// # Errors
///
/// Returns [`DbBootError`] for a wrong section kind, malformed section
/// metadata, or a non-25-byte record.
pub fn decode_banword_section(section: &BootSection) -> Result<Vec<BootBanword>, DbBootError> {
    if section.kind != BootSectionKind::Banword {
        return Err(DbBootError::UnexpectedDecoderSection {
            expected: BootSectionKind::Banword,
            actual: section.kind,
        });
    }
    if usize::from(section.record_size) != BANWORD_WIRE_SIZE {
        return Err(DbBootError::InvalidFixedRecordSize {
            field: "banword_section",
            expected: BANWORD_WIRE_SIZE,
            actual: usize::from(section.record_size),
        });
    }
    section.try_records()?.map(BootBanword::decode).collect()
}

const BASE_SECTION_KINDS: &[BootSectionKind] = &[
    BootSectionKind::Mob,
    BootSectionKind::Item,
    BootSectionKind::Shop,
    BootSectionKind::Skill,
    BootSectionKind::Refine,
    BootSectionKind::ItemAttr,
    BootSectionKind::ItemRare,
    BootSectionKind::Banword,
    BootSectionKind::Land,
    BootSectionKind::ObjectProto,
    BootSectionKind::Object,
];

const RENEWAL_SECTION_KINDS: &[BootSectionKind] = &[
    BootSectionKind::Mob,
    BootSectionKind::Item,
    BootSectionKind::Shop,
    BootSectionKind::RenewalShop,
    BootSectionKind::Skill,
    BootSectionKind::Refine,
    BootSectionKind::ItemAttr,
    BootSectionKind::ItemRare,
    BootSectionKind::Banword,
    BootSectionKind::Land,
    BootSectionKind::ObjectProto,
    BootSectionKind::Object,
];

const EVENT_SECTION_KINDS: &[BootSectionKind] = &[
    BootSectionKind::Mob,
    BootSectionKind::Item,
    BootSectionKind::Shop,
    BootSectionKind::Skill,
    BootSectionKind::Refine,
    BootSectionKind::ItemAttr,
    BootSectionKind::ItemRare,
    BootSectionKind::Banword,
    BootSectionKind::Land,
    BootSectionKind::ObjectProto,
    BootSectionKind::Object,
    BootSectionKind::Event,
];

const MARKET_SECTION_KINDS: &[BootSectionKind] = &[
    BootSectionKind::Mob,
    BootSectionKind::Item,
    BootSectionKind::Shop,
    BootSectionKind::Skill,
    BootSectionKind::Refine,
    BootSectionKind::ItemAttr,
    BootSectionKind::ItemRare,
    BootSectionKind::Banword,
    BootSectionKind::Land,
    BootSectionKind::ObjectProto,
    BootSectionKind::Object,
    BootSectionKind::PremiumMarketPrice,
];

const RENEWAL_EVENT_SECTION_KINDS: &[BootSectionKind] = &[
    BootSectionKind::Mob,
    BootSectionKind::Item,
    BootSectionKind::Shop,
    BootSectionKind::RenewalShop,
    BootSectionKind::Skill,
    BootSectionKind::Refine,
    BootSectionKind::ItemAttr,
    BootSectionKind::ItemRare,
    BootSectionKind::Banword,
    BootSectionKind::Land,
    BootSectionKind::ObjectProto,
    BootSectionKind::Object,
    BootSectionKind::Event,
];

const RENEWAL_MARKET_SECTION_KINDS: &[BootSectionKind] = &[
    BootSectionKind::Mob,
    BootSectionKind::Item,
    BootSectionKind::Shop,
    BootSectionKind::RenewalShop,
    BootSectionKind::Skill,
    BootSectionKind::Refine,
    BootSectionKind::ItemAttr,
    BootSectionKind::ItemRare,
    BootSectionKind::Banword,
    BootSectionKind::Land,
    BootSectionKind::ObjectProto,
    BootSectionKind::Object,
    BootSectionKind::PremiumMarketPrice,
];

const EVENT_MARKET_SECTION_KINDS: &[BootSectionKind] = &[
    BootSectionKind::Mob,
    BootSectionKind::Item,
    BootSectionKind::Shop,
    BootSectionKind::Skill,
    BootSectionKind::Refine,
    BootSectionKind::ItemAttr,
    BootSectionKind::ItemRare,
    BootSectionKind::Banword,
    BootSectionKind::Land,
    BootSectionKind::ObjectProto,
    BootSectionKind::Object,
    BootSectionKind::Event,
    BootSectionKind::PremiumMarketPrice,
];

const ACTIVE_SECTION_KINDS: &[BootSectionKind] = &[
    BootSectionKind::Mob,
    BootSectionKind::Item,
    BootSectionKind::Shop,
    BootSectionKind::RenewalShop,
    BootSectionKind::Skill,
    BootSectionKind::Refine,
    BootSectionKind::ItemAttr,
    BootSectionKind::ItemRare,
    BootSectionKind::Banword,
    BootSectionKind::Land,
    BootSectionKind::ObjectProto,
    BootSectionKind::Object,
    BootSectionKind::Event,
    BootSectionKind::PremiumMarketPrice,
];

/// A packed x86 `TItemIDRangeTable` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootItemIdRange {
    /// `dwMin`.
    pub min: u32,
    /// `dwMax`.
    pub max: u32,
    /// `dwUsableItemIDMin`.
    pub usable_item_id_min: u32,
}

impl BootItemIdRange {
    /// Decode exactly 12 little-endian bytes.
    ///
    /// # Errors
    ///
    /// Returns [`DbBootError::InvalidFixedRecordSize`] for a value whose
    /// length is not exactly 12 bytes.
    pub fn decode(data: &[u8]) -> Result<Self, DbBootError> {
        if data.len() != ITEM_ID_RANGE_WIRE_SIZE {
            return Err(DbBootError::InvalidFixedRecordSize {
                field: "item_id_range",
                expected: ITEM_ID_RANGE_WIRE_SIZE,
                actual: data.len(),
            });
        }
        Ok(Self {
            min: u32::from_le_bytes([data[0], data[1], data[2], data[3]]),
            max: u32::from_le_bytes([data[4], data[5], data[6], data[7]]),
            usable_item_id_min: u32::from_le_bytes([data[8], data[9], data[10], data[11]]),
        })
    }
}

/// The two item-ID ranges written under one legacy count-1 header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootItemIdRanges {
    /// Wire record size declared by the header.
    pub record_size: u16,
    /// Count declared by the legacy header (always one).
    pub declared_count: u16,
    /// Active item-ID range.
    pub active: BootItemIdRange,
    /// Spare item-ID range.
    pub spare: BootItemIdRange,
}

impl BootItemIdRanges {
    /// Return both ranges in wire order.
    #[must_use]
    pub const fn as_pair(&self) -> [BootItemIdRange; 2] {
        [self.active, self.spare]
    }
}

/// The packed x86 `TPacketGDBoot` request sent by the game side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DbBootRequest {
    /// `dwItemIDRange[0]` and `dwItemIDRange[1]`.
    pub item_id_range: [u32; 2],
    /// `szIP[16]`, normally a NUL-padded IPv4 string.
    pub ip: [u8; 16],
}

impl DbBootRequest {
    /// Construct a request from the two legacy item-range values and IP bytes.
    #[must_use]
    pub const fn new(item_id_range: [u32; 2], ip: [u8; 16]) -> Self {
        Self { item_id_range, ip }
    }

    /// Encode the exact 24-byte little-endian request payload.
    #[must_use]
    pub fn encode(&self) -> [u8; BOOT_REQUEST_WIRE_SIZE] {
        let mut bytes = [0_u8; BOOT_REQUEST_WIRE_SIZE];
        bytes[0..4].copy_from_slice(&self.item_id_range[0].to_le_bytes());
        bytes[4..8].copy_from_slice(&self.item_id_range[1].to_le_bytes());
        bytes[8..24].copy_from_slice(&self.ip);
        bytes
    }

    /// Decode an exact 24-byte request payload.
    ///
    /// # Errors
    ///
    /// Returns [`DbBootError::InvalidFixedRecordSize`] for a short or
    /// overlong payload.
    pub fn decode(data: &[u8]) -> Result<Self, DbBootError> {
        if data.len() != BOOT_REQUEST_WIRE_SIZE {
            return Err(DbBootError::InvalidFixedRecordSize {
                field: "boot_request",
                expected: BOOT_REQUEST_WIRE_SIZE,
                actual: data.len(),
            });
        }
        let mut offset = 0;
        let item_id_range = [
            read_u32_at(data, &mut offset, "boot_item_id_range[0]")?,
            read_u32_at(data, &mut offset, "boot_item_id_range[1]")?,
        ];
        let ip = read_array_at::<16>(data, &mut offset, "boot_ip")?;
        Ok(Self { item_id_range, ip })
    }

    /// The `szIP` bytes as the C string the legacy server would have read.
    ///
    /// Legacy `__GetAdminInfo` interpolates this field into an unescaped
    /// `'%s'`, so a field with no NUL is a malformed request rather than a
    /// sixteen-character address. This returns `None` in that case, which
    /// callers must treat as "no requester address" rather than as the full
    /// sixteen bytes. The result is only meaningful when every byte is ASCII;
    /// a non-ASCII field yields `None` rather than a lossy conversion.
    #[must_use]
    pub fn ip_text(&self) -> Option<String> {
        let end = self.ip.iter().position(|byte| *byte == 0)?;
        let text = &self.ip[..end];
        if text.is_empty() || !text.iter().all(u8::is_ascii) {
            return None;
        }
        std::str::from_utf8(text)
            .ok()
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    }
}

/// One fixed-width GM host value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootGmHost {
    /// Raw 16-byte host field, including any legacy terminator.
    pub bytes: [u8; GM_HOST_WIRE_SIZE],
}

impl BootGmHost {
    /// Decode exactly 16 bytes.
    ///
    /// # Errors
    ///
    /// Returns [`DbBootError::InvalidFixedRecordSize`] unless `data` is
    /// exactly 16 bytes.
    pub fn decode(data: &[u8]) -> Result<Self, DbBootError> {
        if data.len() != GM_HOST_WIRE_SIZE {
            return Err(DbBootError::InvalidFixedRecordSize {
                field: "gm_host",
                expected: GM_HOST_WIRE_SIZE,
                actual: data.len(),
            });
        }
        let mut bytes = [0_u8; GM_HOST_WIRE_SIZE];
        bytes.copy_from_slice(data);
        Ok(Self { bytes })
    }

    /// Return the bytes up to the first NUL, if present.
    #[must_use]
    pub fn as_str(&self) -> &str {
        let end = self
            .bytes
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(self.bytes.len());
        std::str::from_utf8(&self.bytes[..end]).unwrap_or("")
    }
}

/// The GM-host section after the item ranges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootGmHostSection {
    /// Declared record size (always 16).
    pub record_size: u16,
    /// Number of hosts.
    pub count: u16,
    /// Parsed hosts in wire order.
    pub hosts: Vec<BootGmHost>,
}

/// One packed x86 `tAdminInfo` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootAdminInfo {
    /// `m_ID` (`int`, four bytes on x86).
    pub id: i32,
    /// `m_szAccount[32]`.
    pub account: [u8; 32],
    /// `m_szName[32]`.
    pub name: [u8; 32],
    /// `m_szContactIP[16]`.
    pub contact_ip: [u8; 16],
    /// `m_szServerIP[16]`.
    pub server_ip: [u8; 16],
    /// `m_Authority` (`int`, four bytes on x86).
    pub authority: i32,
}

impl BootAdminInfo {
    /// Decode exactly 104 little-endian bytes.
    ///
    /// # Errors
    ///
    /// Returns [`DbBootError::InvalidFixedRecordSize`] unless `data` is
    /// exactly 104 bytes.
    pub fn decode(data: &[u8]) -> Result<Self, DbBootError> {
        if data.len() != ADMIN_INFO_WIRE_SIZE {
            return Err(DbBootError::InvalidFixedRecordSize {
                field: "admin_info",
                expected: ADMIN_INFO_WIRE_SIZE,
                actual: data.len(),
            });
        }
        let mut offset = 0;
        Ok(Self {
            id: read_i32_at(data, &mut offset, "admin_id")?,
            account: read_array_at::<32>(data, &mut offset, "admin_account")?,
            name: read_array_at::<32>(data, &mut offset, "admin_name")?,
            contact_ip: read_array_at::<16>(data, &mut offset, "admin_contact_ip")?,
            server_ip: read_array_at::<16>(data, &mut offset, "admin_server_ip")?,
            authority: read_i32_at(data, &mut offset, "admin_authority")?,
        })
    }

    /// Encode exactly the 104 packed little-endian bytes the legacy sender
    /// writes for one `tAdminInfo`.
    ///
    /// This is the inverse of [`Self::decode`] and the same byte layout the
    /// payload encoder uses inline, so a record can be checked against the wire
    /// without composing a whole boot payload.
    #[must_use]
    pub fn encode(&self) -> [u8; ADMIN_INFO_WIRE_SIZE] {
        let mut out = [0_u8; ADMIN_INFO_WIRE_SIZE];
        out[0..4].copy_from_slice(&self.id.to_le_bytes());
        out[4..36].copy_from_slice(&self.account);
        out[36..68].copy_from_slice(&self.name);
        out[68..84].copy_from_slice(&self.contact_ip);
        out[84..100].copy_from_slice(&self.server_ip);
        out[100..104].copy_from_slice(&self.authority.to_le_bytes());
        out
    }
}

/// The admin section after the GM-host section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootAdminSection {
    /// Declared record size (always 104).
    pub record_size: u16,
    /// Number of administrators.
    pub count: u16,
    /// Parsed administrators in wire order.
    pub admins: Vec<BootAdminInfo>,
}

/// One packed x86 `TMonarchInfo` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootMonarchInfo {
    /// Four monarch player IDs.
    pub pid: [u32; 4],
    /// Four signed 64-bit money values.
    pub money: [i64; 4],
    /// Four 32-byte monarch names.
    pub name: [[u8; 32]; 4],
    /// Four 32-byte election dates.
    pub date: [[u8; 32]; 4],
}

impl BootMonarchInfo {
    /// Decode exactly 304 little-endian bytes.
    ///
    /// # Errors
    ///
    /// Returns [`DbBootError::InvalidFixedRecordSize`] unless `data` is
    /// exactly 304 bytes.
    pub fn decode(data: &[u8]) -> Result<Self, DbBootError> {
        if data.len() != MONARCH_INFO_WIRE_SIZE {
            return Err(DbBootError::InvalidFixedRecordSize {
                field: "monarch_info",
                expected: MONARCH_INFO_WIRE_SIZE,
                actual: data.len(),
            });
        }
        let mut pid = [0_u32; 4];
        let mut money = [0_i64; 4];
        let mut name = [[0_u8; 32]; 4];
        let mut dates = [[0_u8; 32]; 4];
        let mut offset = 0;
        for value in &mut pid {
            *value = read_u32_at(data, &mut offset, "monarch_pid")?;
        }
        for value in &mut money {
            *value = read_i64_at(data, &mut offset, "monarch_money")?;
        }
        for value in &mut name {
            value.copy_from_slice(read_bytes_at(data, &mut offset, 32, "monarch_name")?);
        }
        for value in &mut dates {
            value.copy_from_slice(read_bytes_at(data, &mut offset, 32, "monarch_date")?);
        }
        Ok(Self {
            pid,
            money,
            name,
            date: dates,
        })
    }
}

/// One packed x86 `MonarchCandidacy` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootMonarchCandidacy {
    /// Candidate player ID.
    pub pid: u32,
    /// Candidate name bytes.
    pub name: [u8; 32],
    /// Election date bytes.
    pub date: [u8; 32],
}

impl BootMonarchCandidacy {
    /// Decode exactly 68 little-endian bytes.
    ///
    /// # Errors
    ///
    /// Returns [`DbBootError::InvalidFixedRecordSize`] unless `data` is
    /// exactly 68 bytes.
    pub fn decode(data: &[u8]) -> Result<Self, DbBootError> {
        if data.len() != MONARCH_CANDIDACY_WIRE_SIZE {
            return Err(DbBootError::InvalidFixedRecordSize {
                field: "monarch_candidacy",
                expected: MONARCH_CANDIDACY_WIRE_SIZE,
                actual: data.len(),
            });
        }
        let mut offset = 0;
        Ok(Self {
            pid: read_u32_at(data, &mut offset, "candidacy_pid")?,
            name: read_array_at::<32>(data, &mut offset, "candidacy_name")?,
            date: read_array_at::<32>(data, &mut offset, "candidacy_date")?,
        })
    }
}

/// The monarch-candidacy section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootMonarchCandidacySection {
    /// Declared record size (always 68).
    pub record_size: u16,
    /// Number of candidates.
    pub count: u16,
    /// Parsed candidates in wire order.
    pub candidates: Vec<BootMonarchCandidacy>,
}

/// A fully validated legacy v6 boot payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbBootPayload {
    /// Declared payload size, including this four-byte prefix.
    pub packet_size: u32,
    /// Boot version (always six for this parser).
    pub version: u8,
    /// Ordered generic table sections.
    pub sections: Vec<BootSection>,
    /// Four-byte x86 `time_t` value.
    pub global_time: i32,
    /// Active and spare item-ID ranges.
    pub item_id_ranges: BootItemIdRanges,
    /// GM host section.
    pub gm_hosts: BootGmHostSection,
    /// Administrator section.
    pub admins: BootAdminSection,
    /// Monarch information.
    pub monarch: BootMonarchInfo,
    /// Monarch candidacy section.
    pub monarch_candidacy: BootMonarchCandidacySection,
    /// End marker (always `0xffff`).
    pub end_marker: u16,
}

impl DbBootPayload {
    /// Return a table section by its position-derived kind.
    #[must_use]
    pub fn section(&self, kind: BootSectionKind) -> Option<&BootSection> {
        self.sections.iter().find(|section| section.kind == kind)
    }

    /// Return the four-byte global time as an `i32`.
    #[must_use]
    pub const fn time(&self) -> i32 {
        self.global_time
    }

    /// Decode the unconditional typed active-x86 `TMobTable` section.
    ///
    /// The active build fixes each packed `TMobTable` record at 255 bytes. This
    /// accessor only decodes the retained section bytes. It does not load,
    /// cache, or apply mob data, and it does not interpret raw name arrays or
    /// infer a feature profile from the payload.
    ///
    /// # Errors
    ///
    /// Returns [`DbBootError::MissingSection`] when a manually assembled
    /// payload has no mob section. It also returns a section validation error
    /// for the wrong record width, inconsistent count/data length, or invalid
    /// record bytes.
    pub fn mob_table_records(&self) -> Result<Vec<MobTableRecord>, DbBootError> {
        let section = self
            .section(BootSectionKind::Mob)
            .ok_or(DbBootError::MissingSection {
                section: BootSectionKind::Mob,
            })?;
        decode_mob_table_section(section)
    }

    /// Decode the unconditional typed `TSkillTable` section.
    ///
    /// The active x86 build fixes each packed record at 1,475 bytes. This
    /// accessor only decodes the retained section bytes; it does not load,
    /// cache, or apply skill data.
    ///
    /// # Errors
    ///
    /// Returns [`DbBootError::MissingSection`] when a manually assembled
    /// payload has no skill section, or a section/record validation error for
    /// a wrong width, inconsistent length, or invalid record bytes.
    pub fn skill_table_records(&self) -> Result<Vec<SkillTableRecord>, DbBootError> {
        let section = self
            .section(BootSectionKind::Skill)
            .ok_or(DbBootError::MissingSection {
                section: BootSectionKind::Skill,
            })?;
        decode_skill_table_section(section)
    }

    /// Decode the unconditional typed active-x86 `TItemTable` section.
    ///
    /// The current build enables both six item sockets and the 64-bit gold
    /// fields, fixing each packed record at 204 bytes. This accessor only
    /// decodes the retained section bytes; it does not load, cache, or apply
    /// item data and does not interpret the raw name arrays.
    ///
    /// # Errors
    ///
    /// Returns [`DbBootError::MissingSection`] when a manually assembled
    /// payload has no item section, or a section/record validation error for
    /// a wrong width, inconsistent length, or invalid record bytes.
    pub fn item_table_records(&self) -> Result<Vec<ItemTableRecord>, DbBootError> {
        let section = self
            .section(BootSectionKind::Item)
            .ok_or(DbBootError::MissingSection {
                section: BootSectionKind::Item,
            })?;
        decode_item_table_section(section)
    }

    /// Decode the unconditional typed base `TShopTable` section.
    ///
    /// Only [`BootSectionKind::Shop`] is accepted. Use
    /// [`DbBootPayload::renewal_shop_table_records`] for the optional
    /// [`BootSectionKind::RenewalShop`] section.
    ///
    /// # Errors
    ///
    /// Returns [`DbBootError::MissingSection`] when a manually assembled
    /// payload has no base shop section, or a section/record validation error
    /// for a wrong width, inconsistent length, or invalid record bytes.
    pub fn shop_table_records(&self) -> Result<Vec<ShopTableRecord>, DbBootError> {
        let section = self
            .section(BootSectionKind::Shop)
            .ok_or(DbBootError::MissingSection {
                section: BootSectionKind::Shop,
            })?;
        decode_shop_table_section(section)
    }

    /// Alias for [`DbBootPayload::shop_table_records`].
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`DbBootPayload::shop_table_records`].
    pub fn shop_records(&self) -> Result<Vec<ShopTableRecord>, DbBootError> {
        self.shop_table_records()
    }

    /// Decode the optional typed renewal `TShopTable` section.
    ///
    /// Only [`BootSectionKind::RenewalShop`] is accepted. The raw codec
    /// remains structural; the typed boundary checks the same 40-item capacity
    /// as the base-shop section.
    ///
    /// # Errors
    ///
    /// Returns [`DbBootError::MissingSection`] when the optional section is
    /// absent, or a section/record validation error for a wrong width,
    /// inconsistent length, invalid record bytes, or an item count above the
    /// 40-slot source capacity. The latter is reported as
    /// [`DbBootError::InvalidRecordCount`] with field
    /// `renewal_shop_item_count`.
    pub fn renewal_shop_table_records(&self) -> Result<Vec<ShopTableRecord>, DbBootError> {
        let section =
            self.section(BootSectionKind::RenewalShop)
                .ok_or(DbBootError::MissingSection {
                    section: BootSectionKind::RenewalShop,
                })?;
        decode_renewal_shop_table_section(section)
    }

    /// Alias for [`DbBootPayload::renewal_shop_table_records`].
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`DbBootPayload::renewal_shop_table_records`].
    pub fn renewal_shop_records(&self) -> Result<Vec<ShopTableRecord>, DbBootError> {
        self.renewal_shop_table_records()
    }

    /// Decode the unconditional typed normal `TItemAttrTable` section.
    ///
    /// The active glove-enabled profile fixes each record at 71 bytes. This
    /// accessor only decodes the retained section bytes; it does not load or
    /// apply table data.
    ///
    /// # Errors
    ///
    /// Returns [`DbBootError::MissingSection`] when a manually assembled
    /// payload has no item-attribute section, or a section/record validation
    /// error for malformed metadata or rows.
    pub fn item_attr_records(&self) -> Result<Vec<ItemAttrRecord>, DbBootError> {
        let section =
            self.section(BootSectionKind::ItemAttr)
                .ok_or(DbBootError::MissingSection {
                    section: BootSectionKind::ItemAttr,
                })?;
        decode_item_attr_section(section)
    }

    /// Decode the unconditional typed rare `TItemAttrTable` section.
    ///
    /// The rare section uses the same active 71-byte `TItemAttrTable` record as
    /// the normal section. This accessor only decodes retained section bytes.
    ///
    /// # Errors
    ///
    /// Returns [`DbBootError::MissingSection`] when a manually assembled
    /// payload has no rare item-attribute section, or a section/record
    /// validation error for malformed metadata or rows.
    pub fn item_rare_records(&self) -> Result<Vec<ItemAttrRecord>, DbBootError> {
        let section =
            self.section(BootSectionKind::ItemRare)
                .ok_or(DbBootError::MissingSection {
                    section: BootSectionKind::ItemRare,
                })?;
        decode_item_rare_section(section)
    }

    /// Decode the optional typed `TMarketItemPrice` section.
    ///
    /// The profile switch is checked explicitly. No section is inferred from
    /// payload bytes, and no SQL/table-loading operation is performed.
    ///
    /// # Errors
    ///
    /// Returns [`DbBootError::ProfileFeatureDisabled`] when premium market
    /// pricing is not enabled, [`DbBootError::MissingSection`] when an
    /// enabled payload has no market section, or a section/record validation
    /// error for malformed metadata or rows.
    pub fn market_item_prices(
        &self,
        profile: BootFeatureProfile,
    ) -> Result<Vec<MarketItemPriceRecord>, DbBootError> {
        if !profile.premium_market_price {
            return Err(DbBootError::ProfileFeatureDisabled {
                feature: "premium_market_price",
            });
        }
        let section = self.section(BootSectionKind::PremiumMarketPrice).ok_or(
            DbBootError::MissingSection {
                section: BootSectionKind::PremiumMarketPrice,
            },
        )?;
        decode_market_item_price_section(section, profile)
    }

    /// Decode the optional typed `TEventTable` section.
    ///
    /// The event-manager feature switch is checked explicitly. No section is
    /// inferred from payload bytes, and no SQL/table-loading operation is
    /// performed.
    ///
    /// # Errors
    ///
    /// Returns [`DbBootError::ProfileFeatureDisabled`] when event-manager
    /// support is not enabled, [`DbBootError::MissingSection`] when an enabled
    /// payload has no event section, or a section/record validation error for
    /// malformed metadata or rows.
    pub fn event_table_records(
        &self,
        profile: BootFeatureProfile,
    ) -> Result<Vec<EventTableRecord>, DbBootError> {
        if !profile.event_manager {
            return Err(DbBootError::ProfileFeatureDisabled {
                feature: "event_manager",
            });
        }
        let section = self
            .section(BootSectionKind::Event)
            .ok_or(DbBootError::MissingSection {
                section: BootSectionKind::Event,
            })?;
        decode_event_table_section(section, profile)
    }

    /// Decode the unconditional typed `TRefineTable` section.
    ///
    /// The refine section is present in every supported feature profile. The
    /// boot parser still receives an explicit profile when it builds the
    /// payload; this accessor does not infer a profile from the section bytes.
    /// No SQL/table-loading operation is performed.
    ///
    /// # Errors
    ///
    /// Returns [`DbBootError::MissingSection`] when a manually assembled
    /// payload has no refine section, or a section/record validation error for
    /// malformed metadata or rows.
    pub fn refine_table_records(&self) -> Result<Vec<RefineTableRecord>, DbBootError> {
        let section = self
            .section(BootSectionKind::Refine)
            .ok_or(DbBootError::MissingSection {
                section: BootSectionKind::Refine,
            })?;
        decode_refine_table_section(section)
    }

    /// Decode the unconditional typed `building::TLand` section.
    ///
    /// The land section is present in every supported feature profile. The
    /// explicit profile still controls boot parsing and is never inferred from
    /// section bytes. No SQL/table-loading operation is performed here.
    ///
    /// # Errors
    ///
    /// Returns [`DbBootError::MissingSection`] when a manually assembled
    /// payload has no land section, or a section/record validation error for
    /// malformed metadata or rows.
    pub fn land_records(&self) -> Result<Vec<LandRecord>, DbBootError> {
        let section = self
            .section(BootSectionKind::Land)
            .ok_or(DbBootError::MissingSection {
                section: BootSectionKind::Land,
            })?;
        decode_land_table_section(section)
    }

    /// Decode the unconditional typed `building::TObjectProto` section.
    ///
    /// The object-prototype section is present in every supported feature
    /// profile. The explicit profile still controls boot parsing and is never
    /// inferred from section bytes. No SQL/table-loading operation is performed
    /// here.
    ///
    /// # Errors
    ///
    /// Returns [`DbBootError::MissingSection`] when a manually assembled
    /// payload has no object-prototype section, or a section/record validation
    /// error for malformed metadata or rows.
    pub fn object_proto_records(&self) -> Result<Vec<ObjectProtoRecord>, DbBootError> {
        let section =
            self.section(BootSectionKind::ObjectProto)
                .ok_or(DbBootError::MissingSection {
                    section: BootSectionKind::ObjectProto,
                })?;
        decode_object_proto_section(section)
    }

    /// Alias for [`DbBootPayload::object_proto_records`].
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`DbBootPayload::object_proto_records`].
    pub fn object_proto_table_records(&self) -> Result<Vec<ObjectProtoRecord>, DbBootError> {
        self.object_proto_records()
    }

    /// Decode the unconditional typed `building::TObject` section.
    ///
    /// The object section is present in every supported feature profile. The
    /// explicit profile still controls boot parsing and is never inferred from
    /// section bytes. No SQL/table-loading operation is performed here.
    ///
    /// # Errors
    ///
    /// Returns [`DbBootError::MissingSection`] when a manually assembled
    /// payload has no object section, or a section/record validation error for
    /// malformed metadata or rows.
    pub fn object_records(&self) -> Result<Vec<ObjectRecord>, DbBootError> {
        let section = self
            .section(BootSectionKind::Object)
            .ok_or(DbBootError::MissingSection {
                section: BootSectionKind::Object,
            })?;
        decode_object_table_section(section)
    }

    /// Encode this value as a complete version-6 boot payload.
    ///
    /// The selected profile must match the section order exactly. The stored
    /// packet size, version, and end marker are validated against the encoded
    /// layout rather than trusted as arbitrary metadata.
    ///
    /// # Errors
    ///
    /// Returns [`DbBootError`] for an invalid profile section list, malformed
    /// section or fixed-tail metadata, a size that cannot fit `u32`, or a
    /// payload above [`DEFAULT_MAX_DB_BOOT_PAYLOAD_SIZE`].
    pub fn encode(&self, profile: BootFeatureProfile) -> Result<Vec<u8>, DbBootError> {
        self.encode_with_limit(profile, DEFAULT_MAX_DB_BOOT_PAYLOAD_SIZE)
    }

    /// Encode this value with an explicit complete-payload allocation limit.
    ///
    /// # Errors
    ///
    /// See [`DbBootPayload::encode`]. A computed length above `max_payload_size`
    /// is rejected before the output allocation.
    pub fn encode_with_limit(
        &self,
        profile: BootFeatureProfile,
        max_payload_size: usize,
    ) -> Result<Vec<u8>, DbBootError> {
        encode_db_boot_payload_with_limit(self, profile, max_payload_size)
    }

    /// Encode this value and wrap it in the legacy boot response peer frame.
    ///
    /// The returned frame uses [`HEADER_DG_BOOT`] and the legacy zero peer
    /// handle. It contains only the `QUERY_BOOT` payload; the caller still
    /// owns database loading, response policy, and the TCP write.
    ///
    /// # Errors
    ///
    /// Returns [`DbBootError`] when payload validation or encoding fails.
    pub fn encode_frame(&self, profile: BootFeatureProfile) -> Result<DbFrame, DbBootError> {
        self.encode_frame_with_limit(profile, DEFAULT_MAX_DB_BOOT_PAYLOAD_SIZE)
    }

    /// Encode this value into a boot response frame with an allocation limit.
    ///
    /// The limit applies to the complete boot payload before the peer-frame
    /// payload is copied into the returned [`DbFrame`].
    ///
    /// # Errors
    ///
    /// See [`DbBootPayload::encode_frame`]. A payload above the limit returns
    /// [`DbBootError::EncodePayloadTooLarge`].
    pub fn encode_frame_with_limit(
        &self,
        profile: BootFeatureProfile,
        max_payload_size: usize,
    ) -> Result<DbFrame, DbBootError> {
        let payload = self.encode_with_limit(profile, max_payload_size)?;
        Ok(DbFrame::new(
            HEADER_DG_BOOT,
            DB_BOOT_RESPONSE_HANDLE,
            payload,
        ))
    }
}

/// Decode the typed active-x86 `TMobTable` records in a boot section.
///
/// The section kind, declared 255-byte record width, count, and exact data
/// length are checked before any row is returned. The section is unconditional
/// in every supported feature profile. Raw character arrays remain bytes, and
/// no SQL, string, gameplay, or profile inference is performed here.
///
/// # Errors
///
/// Returns [`DbBootError`] for a wrong section kind, malformed section
/// metadata, or a record that is not exactly 255 bytes. A record codec failure
/// is retained as [`DbBootError::TableRecordDecode`].
pub fn decode_mob_table_section(section: &BootSection) -> Result<Vec<MobTableRecord>, DbBootError> {
    if section.kind != BootSectionKind::Mob {
        return Err(DbBootError::UnexpectedDecoderSection {
            expected: BootSectionKind::Mob,
            actual: section.kind,
        });
    }
    if usize::from(section.record_size) != MOB_TABLE_RECORD_WIRE_SIZE {
        return Err(DbBootError::InvalidFixedRecordSize {
            field: "mob_table_section",
            expected: MOB_TABLE_RECORD_WIRE_SIZE,
            actual: usize::from(section.record_size),
        });
    }
    section
        .try_records()?
        .map(|record| {
            MobTableRecord::decode(record).map_err(|source| DbBootError::TableRecordDecode {
                section: BootSectionKind::Mob,
                source,
            })
        })
        .collect()
}

/// Decode the unconditional typed `TSkillTable` section.
///
/// The section kind, declared 1,475-byte record width, count, and exact data
/// length are checked before any row is returned. The section is present in
/// every supported feature profile; no profile is inferred from its bytes.
/// Character arrays remain raw bytes and are not interpreted as strings.
///
/// # Errors
///
/// Returns [`DbBootError`] for a wrong section kind, malformed section
/// metadata, or a record that is not exactly 1,475 bytes. A record codec
/// failure is retained as [`DbBootError::TableRecordDecode`].
pub fn decode_skill_table_section(
    section: &BootSection,
) -> Result<Vec<SkillTableRecord>, DbBootError> {
    if section.kind != BootSectionKind::Skill {
        return Err(DbBootError::UnexpectedDecoderSection {
            expected: BootSectionKind::Skill,
            actual: section.kind,
        });
    }
    if usize::from(section.record_size) != SKILL_TABLE_RECORD_WIRE_SIZE {
        return Err(DbBootError::InvalidFixedRecordSize {
            field: "skill_table_section",
            expected: SKILL_TABLE_RECORD_WIRE_SIZE,
            actual: usize::from(section.record_size),
        });
    }
    section
        .try_records()?
        .map(|record| {
            SkillTableRecord::decode(record).map_err(|source| DbBootError::TableRecordDecode {
                section: BootSectionKind::Skill,
                source,
            })
        })
        .collect()
}

/// Decode the typed active-x86 `TItemTable` records in a boot section.
///
/// The active build enables `ENABLE_EXTENDED_SOCKETS` and
/// `ENABLE_REMOVE_LIMIT_GOLD`, so every packed record is exactly 204 bytes.
/// The section kind, declared width, count, and exact data length are checked
/// before any record is returned. Raw character arrays are preserved as bytes;
/// no SQL, string, or profile inference is performed here.
///
/// # Errors
///
/// Returns [`DbBootError`] for a wrong section kind, malformed section
/// metadata, or a record that is not exactly 204 bytes. A record codec failure
/// is retained as [`DbBootError::TableRecordDecode`].
pub fn decode_item_table_section(
    section: &BootSection,
) -> Result<Vec<ItemTableRecord>, DbBootError> {
    if section.kind != BootSectionKind::Item {
        return Err(DbBootError::UnexpectedDecoderSection {
            expected: BootSectionKind::Item,
            actual: section.kind,
        });
    }
    if usize::from(section.record_size) != ITEM_TABLE_RECORD_WIRE_SIZE {
        return Err(DbBootError::InvalidFixedRecordSize {
            field: "item_table_section",
            expected: ITEM_TABLE_RECORD_WIRE_SIZE,
            actual: usize::from(section.record_size),
        });
    }
    section
        .try_records()?
        .map(|record| {
            ItemTableRecord::decode(record).map_err(|source| DbBootError::TableRecordDecode {
                section: BootSectionKind::Item,
                source,
            })
        })
        .collect()
}

/// Decode the typed base `TShopTable` records in a boot section.
///
/// The section kind, declared 2,762-byte record width, count, exact data
/// length, and the 40-item per-record capacity are checked before any row is
/// returned. The raw [`ShopTableRecord::decode`] codec remains structural;
/// this typed boot boundary rejects an out-of-range `byItemCount`. This decoder
/// intentionally rejects [`BootSectionKind::RenewalShop`]; callers must use
/// [`decode_renewal_shop_table_section`] for that optional feature section,
/// even though both sections share the same packed record width.
///
/// # Errors
///
/// Returns [`DbBootError`] for a wrong section kind, malformed section
/// metadata, a record that is not exactly 2,762 bytes, or a decoded
/// `byItemCount` above 40. An over-capacity count is reported as
/// [`DbBootError::InvalidRecordCount`] with field `shop_item_count`; a record
/// codec failure is retained as [`DbBootError::TableRecordDecode`].
pub fn decode_shop_table_section(
    section: &BootSection,
) -> Result<Vec<ShopTableRecord>, DbBootError> {
    decode_shop_table_section_for_kind(section, BootSectionKind::Shop)
}

/// Decode the optional typed renewal `TShopTable` section.
///
/// The renewal loader uses the same source-fixed packed record width as the
/// base section, but has different SQL-to-memory initialization semantics.
/// Those semantics belong to the SQL-free loader; this function validates
/// the resulting wire shape and the 40-item per-record capacity.
///
/// # Errors
///
/// Returns [`DbBootError`] for a wrong section kind, malformed metadata, a
/// record that is not exactly 2,762 bytes, or a decoded `byItemCount` above 40.
/// An over-capacity count is reported as [`DbBootError::InvalidRecordCount`]
/// with field `renewal_shop_item_count`.
pub fn decode_renewal_shop_table_section(
    section: &BootSection,
) -> Result<Vec<ShopTableRecord>, DbBootError> {
    decode_shop_table_section_for_kind(section, BootSectionKind::RenewalShop)
}

fn decode_shop_table_section_for_kind(
    section: &BootSection,
    expected_kind: BootSectionKind,
) -> Result<Vec<ShopTableRecord>, DbBootError> {
    if section.kind != expected_kind {
        return Err(DbBootError::UnexpectedDecoderSection {
            expected: expected_kind,
            actual: section.kind,
        });
    }
    if usize::from(section.record_size) != SHOP_TABLE_RECORD_WIRE_SIZE {
        return Err(DbBootError::InvalidFixedRecordSize {
            field: if expected_kind == BootSectionKind::RenewalShop {
                "renewal_shop_table_section"
            } else {
                "shop_table_section"
            },
            expected: SHOP_TABLE_RECORD_WIRE_SIZE,
            actual: usize::from(section.record_size),
        });
    }
    section
        .try_records()?
        .map(|record| {
            let record = ShopTableRecord::decode(record).map_err(|source| {
                DbBootError::TableRecordDecode {
                    section: expected_kind,
                    source,
                }
            })?;
            if usize::from(record.item_count) > SHOP_HOST_ITEM_MAX_NUM {
                return Err(DbBootError::InvalidRecordCount {
                    field: if expected_kind == BootSectionKind::RenewalShop {
                        "renewal_shop_item_count"
                    } else {
                        "shop_item_count"
                    },
                    count: u16::from(record.item_count),
                    expected: u16::try_from(SHOP_HOST_ITEM_MAX_NUM).unwrap_or(u16::MAX),
                });
            }
            Ok(record)
        })
        .collect()
}

/// Table-oriented alias for [`decode_shop_table_section`].
///
/// # Errors
///
/// Returns the same errors as [`decode_shop_table_section`].
pub fn decode_shop_section(section: &BootSection) -> Result<Vec<ShopTableRecord>, DbBootError> {
    decode_shop_table_section(section)
}

/// Renewal-shop-oriented alias for [`decode_renewal_shop_table_section`].
///
/// # Errors
///
/// Returns the same errors as [`decode_renewal_shop_table_section`].
pub fn decode_renewal_shop_section(
    section: &BootSection,
) -> Result<Vec<ShopTableRecord>, DbBootError> {
    decode_renewal_shop_table_section(section)
}

fn decode_item_attr_records_for_kind(
    section: &BootSection,
    expected_kind: BootSectionKind,
) -> Result<Vec<ItemAttrRecord>, DbBootError> {
    if section.kind != expected_kind {
        return Err(DbBootError::UnexpectedDecoderSection {
            expected: expected_kind,
            actual: section.kind,
        });
    }
    if usize::from(section.record_size) != ITEM_ATTR_RECORD_WIRE_SIZE {
        let field = match expected_kind {
            BootSectionKind::ItemAttr => "item_attr_section",
            BootSectionKind::ItemRare => "item_rare_section",
            _ => unreachable!("item-attribute decoder received a non-item-attribute kind"),
        };
        return Err(DbBootError::InvalidFixedRecordSize {
            field,
            expected: ITEM_ATTR_RECORD_WIRE_SIZE,
            actual: usize::from(section.record_size),
        });
    }
    section
        .try_records()?
        .map(|record| {
            ItemAttrRecord::decode(record).map_err(|source| DbBootError::TableRecordDecode {
                section: expected_kind,
                source,
            })
        })
        .collect()
}

/// Decode the unconditional typed normal `TItemAttrTable` section.
///
/// The section kind, declared 71-byte width, count, and exact data length are
/// checked before any row is returned. Both normal and rare sections use the
/// active glove-enabled x86 record layout. No profile is inferred from bytes.
///
/// # Errors
///
/// Returns [`DbBootError`] for a wrong section kind, malformed section
/// metadata, or a record that is not exactly 71 bytes.
pub fn decode_item_attr_section(section: &BootSection) -> Result<Vec<ItemAttrRecord>, DbBootError> {
    decode_item_attr_records_for_kind(section, BootSectionKind::ItemAttr)
}

/// Decode the unconditional typed rare `TItemAttrTable` section.
///
/// The section kind, declared 71-byte width, count, and exact data length are
/// checked before any row is returned. The rare section uses the same active
/// x86 record layout as the normal section. No profile is inferred from bytes.
///
/// # Errors
///
/// Returns [`DbBootError`] for a wrong section kind, malformed section
/// metadata, or a record that is not exactly 71 bytes.
pub fn decode_item_rare_section(section: &BootSection) -> Result<Vec<ItemAttrRecord>, DbBootError> {
    decode_item_attr_records_for_kind(section, BootSectionKind::ItemRare)
}

/// Decode the unconditional typed `TRefineTable` section.
///
/// The section kind, declared record width, count, and exact data length are
/// checked before any row is returned. The section is part of every supported
/// boot feature profile; no profile is inferred from its bytes.
///
/// # Errors
///
/// Returns [`DbBootError`] for a wrong section kind, malformed section
/// metadata, or a record that is not exactly 53 bytes.
pub fn decode_refine_table_section(
    section: &BootSection,
) -> Result<Vec<RefineTableRecord>, DbBootError> {
    if section.kind != BootSectionKind::Refine {
        return Err(DbBootError::UnexpectedDecoderSection {
            expected: BootSectionKind::Refine,
            actual: section.kind,
        });
    }
    if usize::from(section.record_size) != REFINE_TABLE_WIRE_SIZE {
        return Err(DbBootError::InvalidFixedRecordSize {
            field: "refine_table_section",
            expected: REFINE_TABLE_WIRE_SIZE,
            actual: usize::from(section.record_size),
        });
    }
    section
        .try_records()?
        .map(|record| {
            RefineTableRecord::decode(record).map_err(|source| DbBootError::TableRecordDecode {
                section: BootSectionKind::Refine,
                source,
            })
        })
        .collect()
}

/// Decode the unconditional typed `building::TLand` section.
///
/// The section kind, declared 36-byte width, count, and exact data length are
/// checked before any row is returned. The section is part of every supported
/// boot feature profile; no profile is inferred from its bytes.
///
/// # Errors
///
/// Returns [`DbBootError`] for a wrong section kind, malformed section
/// metadata, or a record that is not exactly 36 bytes.
pub fn decode_land_table_section(section: &BootSection) -> Result<Vec<LandRecord>, DbBootError> {
    if section.kind != BootSectionKind::Land {
        return Err(DbBootError::UnexpectedDecoderSection {
            expected: BootSectionKind::Land,
            actual: section.kind,
        });
    }
    if usize::from(section.record_size) != LAND_RECORD_WIRE_SIZE {
        return Err(DbBootError::InvalidFixedRecordSize {
            field: "land_table_section",
            expected: LAND_RECORD_WIRE_SIZE,
            actual: usize::from(section.record_size),
        });
    }
    section
        .try_records()?
        .map(|record| {
            LandRecord::decode(record).map_err(|source| DbBootError::TableRecordDecode {
                section: BootSectionKind::Land,
                source,
            })
        })
        .collect()
}

/// Decode the unconditional typed `building::TObjectProto` section.
///
/// The section kind, declared 96-byte width, count, and exact data length are
/// checked before any row is returned. The section is part of every supported
/// boot feature profile; no profile is inferred from its bytes.
///
/// # Errors
///
/// Returns [`DbBootError`] for a wrong section kind, malformed section
/// metadata, or a record that is not exactly 96 bytes.
pub fn decode_object_proto_section(
    section: &BootSection,
) -> Result<Vec<ObjectProtoRecord>, DbBootError> {
    if section.kind != BootSectionKind::ObjectProto {
        return Err(DbBootError::UnexpectedDecoderSection {
            expected: BootSectionKind::ObjectProto,
            actual: section.kind,
        });
    }
    if usize::from(section.record_size) != OBJECT_PROTO_RECORD_WIRE_SIZE {
        return Err(DbBootError::InvalidFixedRecordSize {
            field: "object_proto_section",
            expected: OBJECT_PROTO_RECORD_WIRE_SIZE,
            actual: usize::from(section.record_size),
        });
    }
    section
        .try_records()?
        .map(|record| {
            ObjectProtoRecord::decode(record).map_err(|source| DbBootError::TableRecordDecode {
                section: BootSectionKind::ObjectProto,
                source,
            })
        })
        .collect()
}

/// Table-oriented alias for [`decode_object_proto_section`].
///
/// # Errors
///
/// Returns the same errors as [`decode_object_proto_section`].
pub fn decode_object_proto_table_section(
    section: &BootSection,
) -> Result<Vec<ObjectProtoRecord>, DbBootError> {
    decode_object_proto_section(section)
}

/// Decode the unconditional typed `building::TObject` section.
///
/// The section kind, declared 40-byte width, count, and exact data length are
/// checked before any row is returned. The section is part of every supported
/// boot feature profile; no profile is inferred from its bytes.
///
/// # Errors
///
/// Returns [`DbBootError`] for a wrong section kind, malformed section
/// metadata, or a record that is not exactly 40 bytes.
pub fn decode_object_table_section(
    section: &BootSection,
) -> Result<Vec<ObjectRecord>, DbBootError> {
    if section.kind != BootSectionKind::Object {
        return Err(DbBootError::UnexpectedDecoderSection {
            expected: BootSectionKind::Object,
            actual: section.kind,
        });
    }
    if usize::from(section.record_size) != OBJECT_RECORD_WIRE_SIZE {
        return Err(DbBootError::InvalidFixedRecordSize {
            field: "object_table_section",
            expected: OBJECT_RECORD_WIRE_SIZE,
            actual: usize::from(section.record_size),
        });
    }
    section
        .try_records()?
        .map(|record| {
            ObjectRecord::decode(record).map_err(|source| DbBootError::TableRecordDecode {
                section: BootSectionKind::Object,
                source,
            })
        })
        .collect()
}

/// Decode the optional typed `TEventTable` section.
///
/// The feature profile remains an explicit caller choice; the decoder does
/// not infer optional sections from payload bytes. The section kind, declared
/// record width, count, and exact data length are checked before rows are
/// returned.
///
/// # Errors
///
/// Returns [`DbBootError`] when event-manager support is disabled, the section
/// kind is wrong, or its metadata and records are malformed.
pub fn decode_event_table_section(
    section: &BootSection,
    profile: BootFeatureProfile,
) -> Result<Vec<EventTableRecord>, DbBootError> {
    if !profile.event_manager {
        return Err(DbBootError::ProfileFeatureDisabled {
            feature: "event_manager",
        });
    }
    if section.kind != BootSectionKind::Event {
        return Err(DbBootError::UnexpectedDecoderSection {
            expected: BootSectionKind::Event,
            actual: section.kind,
        });
    }
    if usize::from(section.record_size) != EVENT_TABLE_WIRE_SIZE {
        return Err(DbBootError::InvalidFixedRecordSize {
            field: "event_table_section",
            expected: EVENT_TABLE_WIRE_SIZE,
            actual: usize::from(section.record_size),
        });
    }
    section
        .try_records()?
        .map(|record| {
            EventTableRecord::decode(record).map_err(|source| DbBootError::TableRecordDecode {
                section: BootSectionKind::Event,
                source,
            })
        })
        .collect()
}

/// Decode the optional typed `TMarketItemPrice` section.
///
/// The feature profile remains an explicit caller choice; the decoder does
/// not infer optional sections from payload bytes. The section kind, declared
/// record width, count, and exact data length are checked before rows are
/// returned.
///
/// # Errors
///
/// Returns [`DbBootError`] when premium market pricing is disabled, the
/// section kind is wrong, or its metadata and records are malformed.
pub fn decode_market_item_price_section(
    section: &BootSection,
    profile: BootFeatureProfile,
) -> Result<Vec<MarketItemPriceRecord>, DbBootError> {
    if !profile.premium_market_price {
        return Err(DbBootError::ProfileFeatureDisabled {
            feature: "premium_market_price",
        });
    }
    if section.kind != BootSectionKind::PremiumMarketPrice {
        return Err(DbBootError::UnexpectedDecoderSection {
            expected: BootSectionKind::PremiumMarketPrice,
            actual: section.kind,
        });
    }
    if usize::from(section.record_size) != MARKET_ITEM_PRICE_WIRE_SIZE {
        return Err(DbBootError::InvalidFixedRecordSize {
            field: "market_item_price_section",
            expected: MARKET_ITEM_PRICE_WIRE_SIZE,
            actual: usize::from(section.record_size),
        });
    }
    section
        .try_records()?
        .map(|record| {
            MarketItemPriceRecord::decode(record).map_err(|source| DbBootError::TableRecordDecode {
                section: BootSectionKind::PremiumMarketPrice,
                source,
            })
        })
        .collect()
}

/// An error raised while parsing or encoding a legacy boot payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DbBootError {
    /// The input ended before a required field or byte range.
    Truncated {
        /// Logical field being read.
        field: &'static str,
        /// Required absolute end offset.
        needed: usize,
        /// Number of bytes available in the input.
        available: usize,
    },
    /// The length prefix did not equal the supplied payload length.
    InvalidPacketSize {
        /// Length declared by the payload.
        declared: u32,
        /// Actual supplied payload length.
        actual: usize,
    },
    /// The declared length cannot be represented by this host's `usize`.
    PacketSizeOverflow {
        /// Length declared by the payload.
        declared: u32,
    },
    /// The complete payload exceeds the configured safety limit.
    PayloadTooLarge {
        /// Supplied payload length.
        length: usize,
        /// Configured maximum.
        maximum: usize,
    },
    /// The version byte is not version six.
    InvalidVersion {
        /// Version supplied by the peer.
        version: u8,
    },
    /// A table section declared a zero record size.
    InvalidRecordSize {
        /// Section whose record size is invalid.
        section: BootSectionKind,
        /// The zero size supplied by the section header.
        record_size: u16,
    },
    /// A typed table decoder was requested for a disabled feature profile.
    ProfileFeatureDisabled {
        /// Stable feature name.
        feature: &'static str,
    },
    /// An enabled profile omitted a required section from a payload value.
    MissingSection {
        /// Section that should be present.
        section: BootSectionKind,
    },
    /// A typed table record could not be decoded.
    TableRecordDecode {
        /// Section containing the invalid record.
        section: BootSectionKind,
        /// Record decoder error.
        source: DbRecordError,
    },
    /// A fixed request or tail record declared an unexpected size.
    InvalidFixedRecordSize {
        /// Logical tail field.
        field: &'static str,
        /// Required size.
        expected: usize,
        /// Supplied size.
        actual: usize,
    },
    /// A fixed-count field declared an unexpected number of records.
    InvalidRecordCount {
        /// Logical tail field.
        field: &'static str,
        /// Supplied count.
        count: u16,
        /// Required count.
        expected: u16,
    },
    /// A checked multiplication or offset addition overflowed `usize`.
    SizeOverflow,
    /// The encoded payload cannot be represented by the legacy `u32` size.
    PayloadSizeOverflow {
        /// Required complete payload length.
        length: usize,
    },
    /// The encoded payload does not fit the configured allocation limit.
    EncodePayloadTooLarge {
        /// Required complete payload length.
        length: usize,
        /// Configured maximum.
        maximum: usize,
    },
    /// The section list length does not match the selected feature profile.
    SectionCountMismatch {
        /// Profile-selected section count.
        expected: usize,
        /// Supplied section count.
        actual: usize,
    },
    /// A section kind is not the one selected for its wire position.
    UnexpectedSectionKind {
        /// Zero-based section position.
        index: usize,
        /// Kind required by the selected profile.
        expected: BootSectionKind,
        /// Kind supplied at that position.
        actual: BootSectionKind,
    },
    /// A typed section decoder was called with a different section kind.
    UnexpectedDecoderSection {
        /// Kind required by the decoder.
        expected: BootSectionKind,
        /// Kind supplied by the caller.
        actual: BootSectionKind,
    },
    /// A table section's data length does not equal record size times count.
    SectionLengthMismatch {
        /// Section whose metadata and data disagree.
        section: BootSectionKind,
        /// Required record-data length.
        expected: usize,
        /// Supplied record-data length.
        actual: usize,
    },
    /// A fixed-tail vector length cannot be represented by the legacy `u16` count.
    TailCountOverflow {
        /// Logical fixed-tail field.
        field: &'static str,
        /// Vector length.
        count: usize,
    },
    /// The end marker was not `0xffff`.
    InvalidEndMarker {
        /// Value found in the end-marker field.
        marker: u16,
    },
    /// Bytes remained after the end marker.
    TrailingBytes {
        /// Number of unconsumed bytes.
        count: usize,
    },
    /// A DB frame carried a header other than `HEADER_DG_BOOT`.
    UnexpectedHeader {
        /// Expected header.
        expected: u8,
        /// Supplied header.
        actual: u8,
    },
    /// A boot response carried a peer handle other than the legacy zero.
    UnexpectedHandle {
        /// Expected peer handle.
        expected: u32,
        /// Supplied peer handle.
        actual: u32,
    },
    /// The backing output allocation failed.
    AllocationFailed,
}

impl DbBootError {
    fn fmt_input_errors(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated {
                field,
                needed,
                available,
            } => write!(
                f,
                "boot {field} is truncated: need {needed} bytes, have {available}"
            ),
            Self::InvalidPacketSize { declared, actual } => {
                write!(
                    f,
                    "boot packet size {declared} does not match payload length {actual}"
                )
            }
            Self::PacketSizeOverflow { declared } => {
                write!(f, "boot packet size {declared} does not fit in usize")
            }
            Self::PayloadTooLarge { length, maximum } => {
                write!(f, "boot payload length {length} exceeds limit {maximum}")
            }
            Self::InvalidVersion { version } => write!(f, "unsupported DB boot version {version}"),
            Self::InvalidRecordSize {
                section,
                record_size,
            } => write!(f, "{section} section has invalid record size {record_size}"),
            Self::ProfileFeatureDisabled { feature } => {
                write!(f, "DB boot feature profile disables {feature}")
            }
            Self::MissingSection { section } => {
                write!(f, "DB boot payload is missing the {section} section")
            }
            Self::TableRecordDecode { section, source } => {
                write!(f, "DB boot {section} record could not be decoded: {source}")
            }
            Self::InvalidFixedRecordSize {
                field,
                expected,
                actual,
            } => write!(
                f,
                "boot {field} record size is {actual}; expected {expected}"
            ),
            _ => self.fmt_structure_errors(f),
        }
    }

    fn fmt_structure_errors(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRecordCount {
                field,
                count,
                expected,
            } => write!(f, "boot {field} count is {count}; expected {expected}"),
            Self::SizeOverflow => f.write_str("boot size arithmetic overflow"),
            Self::PayloadSizeOverflow { length } => write!(
                f,
                "encoded DB boot payload length {length} does not fit u32"
            ),
            Self::EncodePayloadTooLarge { length, maximum } => write!(
                f,
                "encoded DB boot payload length {length} exceeds limit {maximum}"
            ),
            Self::SectionCountMismatch { expected, actual } => write!(
                f,
                "DB boot section count is {actual}; profile requires {expected}"
            ),
            Self::UnexpectedSectionKind {
                index,
                expected,
                actual,
            } => write!(
                f,
                "DB boot section {index} is {actual}; profile requires {expected}"
            ),
            Self::UnexpectedDecoderSection { expected, actual } => {
                write!(
                    f,
                    "DB boot typed decoder expected {expected}, received {actual}"
                )
            }
            Self::SectionLengthMismatch {
                section,
                expected,
                actual,
            } => write!(
                f,
                "DB boot {section} data length is {actual}; expected {expected}"
            ),
            Self::TailCountOverflow { field, count } => {
                write!(f, "DB boot {field} count {count} does not fit u16")
            }
            Self::InvalidEndMarker { marker } => {
                write!(f, "invalid DB boot end marker 0x{marker:04x}")
            }
            Self::TrailingBytes { count } => {
                write!(f, "{count} bytes remain after the DB boot end marker")
            }
            Self::UnexpectedHeader { expected, actual } => {
                write!(
                    f,
                    "unexpected DB frame header {actual}; expected {expected}"
                )
            }
            Self::UnexpectedHandle { expected, actual } => {
                write!(f, "unexpected DB boot handle {actual}; expected {expected}")
            }
            Self::AllocationFailed => f.write_str("DB boot output allocation failed"),
            _ => unreachable!("all DB boot error variants must be formatted"),
        }
    }
}

impl fmt::Display for DbBootError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.fmt_input_errors(f)
    }
}

impl Error for DbBootError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::TableRecordDecode { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// A reusable parser with an explicit feature profile and size limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DbBootParser {
    profile: BootFeatureProfile,
    max_payload_size: usize,
}

impl DbBootParser {
    /// Create a parser for an explicit feature profile.
    #[must_use]
    pub const fn new(profile: BootFeatureProfile) -> Self {
        Self {
            profile,
            max_payload_size: DEFAULT_MAX_DB_BOOT_PAYLOAD_SIZE,
        }
    }

    /// Set the maximum accepted complete payload size.
    #[must_use]
    pub const fn with_max_payload_size(mut self, max_payload_size: usize) -> Self {
        self.max_payload_size = max_payload_size;
        self
    }

    /// Return the configured feature profile.
    #[must_use]
    pub const fn profile(&self) -> BootFeatureProfile {
        self.profile
    }

    /// Return the configured payload-size limit.
    #[must_use]
    pub const fn max_payload_size(&self) -> usize {
        self.max_payload_size
    }

    /// Parse one DB boot payload.
    ///
    /// # Errors
    ///
    /// Returns a typed [`DbBootError`] for any malformed field, section, or
    /// tail.  No table or network operation is performed.
    pub fn parse(&self, payload: &[u8]) -> Result<DbBootPayload, DbBootError> {
        parse_db_boot_payload_with_limit(payload, self.profile, self.max_payload_size)
    }
}

impl Default for DbBootParser {
    fn default() -> Self {
        Self::new(BootFeatureProfile::active())
    }
}

/// Encode a v6 boot payload using the active legacy feature profile.
///
/// # Errors
///
/// See [`encode_db_boot_payload`].
pub fn encode_active_db_boot_payload(payload: &DbBootPayload) -> Result<Vec<u8>, DbBootError> {
    encode_db_boot_payload(payload, BootFeatureProfile::active())
}

/// Encode a v6 boot payload with an explicit feature profile.
///
/// The output starts with its own little-endian `u32` length. It contains only
/// the `QUERY_BOOT` payload. In particular, it does not include a DB peer
/// header or the separate `ENABLE_ITEMSHOP` frame.
///
/// # Errors
///
/// Returns [`DbBootError`] when the profile, section metadata, fixed-tail
/// records, version, end marker, or stored packet size are inconsistent, when
/// checked size arithmetic overflows, or when the payload cannot fit `u32`.
pub fn encode_db_boot_payload(
    payload: &DbBootPayload,
    profile: BootFeatureProfile,
) -> Result<Vec<u8>, DbBootError> {
    encode_db_boot_payload_with_limit(payload, profile, DEFAULT_MAX_DB_BOOT_PAYLOAD_SIZE)
}

/// Encode a v6 boot payload with an explicit profile and allocation limit.
///
/// The limit is checked after exact length calculation and before allocation.
/// The output allocation itself uses `try_reserve_exact`.
///
/// # Errors
///
/// See [`encode_db_boot_payload`]. A computed length above `max_payload_size`
/// returns [`DbBootError::EncodePayloadTooLarge`].
pub fn encode_db_boot_payload_with_limit(
    payload: &DbBootPayload,
    profile: BootFeatureProfile,
    max_payload_size: usize,
) -> Result<Vec<u8>, DbBootError> {
    let length = validate_db_boot_encoding(payload, profile)?;
    if length > max_payload_size {
        return Err(DbBootError::EncodePayloadTooLarge {
            length,
            maximum: max_payload_size,
        });
    }
    let packet_size =
        u32::try_from(length).map_err(|_| DbBootError::PayloadSizeOverflow { length })?;
    if payload.packet_size != packet_size {
        return Err(DbBootError::InvalidPacketSize {
            declared: payload.packet_size,
            actual: length,
        });
    }

    let mut output = Vec::new();
    output
        .try_reserve_exact(length)
        .map_err(|_| DbBootError::AllocationFailed)?;
    output.extend_from_slice(&packet_size.to_le_bytes());
    output.push(DB_BOOT_VERSION);

    for section in &payload.sections {
        output.extend_from_slice(&section.record_size.to_le_bytes());
        output.extend_from_slice(&section.count.to_le_bytes());
        output.extend_from_slice(&section.data);
    }

    output.extend_from_slice(&payload.global_time.to_le_bytes());
    output.extend_from_slice(&ITEM_ID_RANGE_WIRE_SIZE.to_le_bytes()[..2]);
    output.extend_from_slice(&1_u16.to_le_bytes());
    write_item_id_range(&mut output, &payload.item_id_ranges.active);
    write_item_id_range(&mut output, &payload.item_id_ranges.spare);

    output.extend_from_slice(&GM_HOST_WIRE_SIZE.to_le_bytes()[..2]);
    output.extend_from_slice(
        &u16::try_from(payload.gm_hosts.hosts.len())
            .map_err(|_| DbBootError::TailCountOverflow {
                field: "gm_hosts",
                count: payload.gm_hosts.hosts.len(),
            })?
            .to_le_bytes(),
    );
    for host in &payload.gm_hosts.hosts {
        output.extend_from_slice(&host.bytes);
    }

    output.extend_from_slice(&ADMIN_INFO_WIRE_SIZE.to_le_bytes()[..2]);
    output.extend_from_slice(
        &u16::try_from(payload.admins.admins.len())
            .map_err(|_| DbBootError::TailCountOverflow {
                field: "admins",
                count: payload.admins.admins.len(),
            })?
            .to_le_bytes(),
    );
    for admin in &payload.admins.admins {
        output.extend_from_slice(&admin.id.to_le_bytes());
        output.extend_from_slice(&admin.account);
        output.extend_from_slice(&admin.name);
        output.extend_from_slice(&admin.contact_ip);
        output.extend_from_slice(&admin.server_ip);
        output.extend_from_slice(&admin.authority.to_le_bytes());
    }

    output.extend_from_slice(&MONARCH_INFO_WIRE_SIZE.to_le_bytes()[..2]);
    output.extend_from_slice(&1_u16.to_le_bytes());
    write_monarch_info(&mut output, &payload.monarch);

    output.extend_from_slice(&MONARCH_CANDIDACY_WIRE_SIZE.to_le_bytes()[..2]);
    output.extend_from_slice(
        &u16::try_from(payload.monarch_candidacy.candidates.len())
            .map_err(|_| DbBootError::TailCountOverflow {
                field: "monarch_candidacy",
                count: payload.monarch_candidacy.candidates.len(),
            })?
            .to_le_bytes(),
    );
    for candidate in &payload.monarch_candidacy.candidates {
        output.extend_from_slice(&candidate.pid.to_le_bytes());
        output.extend_from_slice(&candidate.name);
        output.extend_from_slice(&candidate.date);
    }

    output.extend_from_slice(&DB_BOOT_END_MARKER.to_le_bytes());
    debug_assert_eq!(output.len(), length);
    Ok(output)
}

/// Parse a v6 boot payload using the active legacy feature profile.
///
/// # Errors
///
/// See [`parse_db_boot_payload`].
pub fn parse_active_db_boot_payload(payload: &[u8]) -> Result<DbBootPayload, DbBootError> {
    parse_db_boot_payload(payload, BootFeatureProfile::active())
}

/// Parse a v6 boot payload with an explicit feature profile.
///
/// The supplied slice is the payload after the nine-byte legacy DB peer
/// header.  Its first four bytes are the payload's own `u32` length prefix.
///
/// # Errors
///
/// Returns [`DbBootError`] for truncation, a length mismatch, an unsupported
/// version, zero/invalid record sizes, count mismatch, checked-size overflow,
/// an invalid tail, or trailing bytes.
pub fn parse_db_boot_payload(
    payload: &[u8],
    profile: BootFeatureProfile,
) -> Result<DbBootPayload, DbBootError> {
    parse_db_boot_payload_with_limit(payload, profile, DEFAULT_MAX_DB_BOOT_PAYLOAD_SIZE)
}

/// Parse a v6 boot payload with an explicit feature profile and size limit.
///
/// # Errors
///
/// See [`parse_db_boot_payload`].  In addition, a payload larger than
/// `max_payload_size` is rejected before any output allocation.
pub fn parse_db_boot_payload_with_limit(
    payload: &[u8],
    profile: BootFeatureProfile,
    max_payload_size: usize,
) -> Result<DbBootPayload, DbBootError> {
    if payload.len() > max_payload_size {
        return Err(DbBootError::PayloadTooLarge {
            length: payload.len(),
            maximum: max_payload_size,
        });
    }

    let mut cursor = Cursor::new(payload);
    let declared = cursor.u32("packet_size")?;
    let declared_len =
        usize::try_from(declared).map_err(|_| DbBootError::PacketSizeOverflow { declared })?;
    if declared_len != payload.len() {
        return Err(DbBootError::InvalidPacketSize {
            declared,
            actual: payload.len(),
        });
    }

    let version = cursor.u8("version")?;
    if version != DB_BOOT_VERSION {
        return Err(DbBootError::InvalidVersion { version });
    }

    let mut sections = Vec::new();
    sections
        .try_reserve_exact(profile.section_kinds().len())
        .map_err(|_| DbBootError::AllocationFailed)?;
    for &kind in profile.section_kinds() {
        sections.push(read_table_section(&mut cursor, kind)?);
    }

    let time_bytes = cursor.take(X86_TIME_T_WIRE_SIZE, "time_t")?;
    let mut time_offset = 0;
    let global_time = read_i32_at(time_bytes, &mut time_offset, "time_t")?;
    let item_id_ranges = read_item_id_ranges(&mut cursor)?;
    let gm_hosts = read_gm_hosts(&mut cursor)?;
    let admins = read_admins(&mut cursor)?;
    let monarch = read_monarch_info(&mut cursor)?;
    let monarch_candidacy = read_monarch_candidacy(&mut cursor)?;
    let end_marker = cursor.u16("end_marker")?;
    if end_marker != DB_BOOT_END_MARKER {
        return Err(DbBootError::InvalidEndMarker { marker: end_marker });
    }
    let remaining = cursor.remaining();
    if remaining != 0 {
        return Err(DbBootError::TrailingBytes { count: remaining });
    }

    Ok(DbBootPayload {
        packet_size: declared,
        version,
        sections,
        global_time,
        item_id_ranges,
        gm_hosts,
        admins,
        monarch,
        monarch_candidacy,
        end_marker,
    })
}

/// Parse a boot payload from an already-decoded legacy DB frame.
///
/// # Errors
///
/// Returns [`DbBootError::UnexpectedHeader`] or [`DbBootError::UnexpectedHandle`]
/// if `frame` is not a zero-handle boot frame; otherwise it returns the same
/// errors as [`parse_db_boot_payload`].
pub fn parse_db_boot_frame(
    frame: &DbFrame,
    profile: BootFeatureProfile,
) -> Result<DbBootPayload, DbBootError> {
    if frame.header != HEADER_DG_BOOT {
        return Err(DbBootError::UnexpectedHeader {
            expected: HEADER_DG_BOOT,
            actual: frame.header,
        });
    }
    if frame.handle != DB_BOOT_RESPONSE_HANDLE {
        return Err(DbBootError::UnexpectedHandle {
            expected: DB_BOOT_RESPONSE_HANDLE,
            actual: frame.handle,
        });
    }
    parse_db_boot_payload(&frame.payload, profile)
}

/// Parse a raw nine-byte-header DB peer frame containing a boot payload.
///
/// # Errors
///
/// Returns [`DbBootError`] for header, handle, length, and other framing
/// errors, and for all payload errors from [`parse_db_boot_payload`].
pub fn parse_db_boot_frame_bytes(
    frame: &[u8],
    profile: BootFeatureProfile,
) -> Result<DbBootPayload, DbBootError> {
    const OUTER_HEADER_SIZE: usize = crate::db_wire::DB_PEER_HEADER_SIZE;
    if frame.len() < OUTER_HEADER_SIZE {
        return Err(DbBootError::Truncated {
            field: "db_peer_header",
            needed: OUTER_HEADER_SIZE,
            available: frame.len(),
        });
    }
    if frame[0] != HEADER_DG_BOOT {
        return Err(DbBootError::UnexpectedHeader {
            expected: HEADER_DG_BOOT,
            actual: frame[0],
        });
    }
    let handle = u32::from_le_bytes([frame[1], frame[2], frame[3], frame[4]]);
    if handle != DB_BOOT_RESPONSE_HANDLE {
        return Err(DbBootError::UnexpectedHandle {
            expected: DB_BOOT_RESPONSE_HANDLE,
            actual: handle,
        });
    }
    let declared = u32::from_le_bytes([frame[5], frame[6], frame[7], frame[8]]);
    let actual = frame.len() - OUTER_HEADER_SIZE;
    let declared_len =
        usize::try_from(declared).map_err(|_| DbBootError::PacketSizeOverflow { declared })?;
    if declared_len != actual {
        return Err(DbBootError::InvalidPacketSize { declared, actual });
    }
    parse_db_boot_payload(&frame[OUTER_HEADER_SIZE..], profile)
}

/// Compatibility alias for [`parse_db_boot_payload`].
pub type ParseDbBootPayload = fn(&[u8], BootFeatureProfile) -> Result<DbBootPayload, DbBootError>;

fn validate_db_boot_encoding(
    payload: &DbBootPayload,
    profile: BootFeatureProfile,
) -> Result<usize, DbBootError> {
    if payload.version != DB_BOOT_VERSION {
        return Err(DbBootError::InvalidVersion {
            version: payload.version,
        });
    }
    if payload.end_marker != DB_BOOT_END_MARKER {
        return Err(DbBootError::InvalidEndMarker {
            marker: payload.end_marker,
        });
    }

    validate_db_boot_sections(payload, profile)?;
    validate_db_boot_fixed_tail(payload)?;

    let mut length = checked_add(4, 1)?;
    for section in &payload.sections {
        length = checked_add(length, 4)?;
        length = checked_add(length, section.data.len())?;
    }
    length = checked_add(length, X86_TIME_T_WIRE_SIZE)?;
    length = checked_add(
        length,
        checked_add(4, checked_product(ITEM_ID_RANGE_WIRE_SIZE, 2)?)?,
    )?;
    length = checked_add(length, 4)?;
    length = checked_add(
        length,
        checked_product(GM_HOST_WIRE_SIZE, payload.gm_hosts.hosts.len())?,
    )?;
    length = checked_add(length, 4)?;
    length = checked_add(
        length,
        checked_product(ADMIN_INFO_WIRE_SIZE, payload.admins.admins.len())?,
    )?;
    length = checked_add(length, 4 + MONARCH_INFO_WIRE_SIZE)?;
    length = checked_add(length, 4)?;
    length = checked_add(
        length,
        checked_product(
            MONARCH_CANDIDACY_WIRE_SIZE,
            payload.monarch_candidacy.candidates.len(),
        )?,
    )?;
    length = checked_add(length, 2)?;

    let packet_size =
        u32::try_from(length).map_err(|_| DbBootError::PayloadSizeOverflow { length })?;
    if payload.packet_size != packet_size {
        return Err(DbBootError::InvalidPacketSize {
            declared: payload.packet_size,
            actual: length,
        });
    }
    Ok(length)
}

fn validate_db_boot_sections(
    payload: &DbBootPayload,
    profile: BootFeatureProfile,
) -> Result<(), DbBootError> {
    let kinds = profile.section_kinds();
    if payload.sections.len() != kinds.len() {
        return Err(DbBootError::SectionCountMismatch {
            expected: kinds.len(),
            actual: payload.sections.len(),
        });
    }
    for (index, (section, expected)) in payload.sections.iter().zip(kinds).enumerate() {
        if section.kind != *expected {
            return Err(DbBootError::UnexpectedSectionKind {
                index,
                expected: *expected,
                actual: section.kind,
            });
        }
        if section.record_size == 0 {
            return Err(DbBootError::InvalidRecordSize {
                section: section.kind,
                record_size: section.record_size,
            });
        }
        let expected_len =
            checked_product(usize::from(section.record_size), usize::from(section.count))?;
        if section.data.len() != expected_len {
            return Err(DbBootError::SectionLengthMismatch {
                section: section.kind,
                expected: expected_len,
                actual: section.data.len(),
            });
        }
    }
    Ok(())
}

fn validate_db_boot_fixed_tail(payload: &DbBootPayload) -> Result<(), DbBootError> {
    if usize::from(payload.item_id_ranges.record_size) != ITEM_ID_RANGE_WIRE_SIZE {
        return Err(DbBootError::InvalidFixedRecordSize {
            field: "item_id_range",
            expected: ITEM_ID_RANGE_WIRE_SIZE,
            actual: usize::from(payload.item_id_ranges.record_size),
        });
    }
    if payload.item_id_ranges.declared_count != 1 {
        return Err(DbBootError::InvalidRecordCount {
            field: "item_id_range",
            count: payload.item_id_ranges.declared_count,
            expected: 1,
        });
    }
    validate_counted_tail(
        "gm_hosts",
        payload.gm_hosts.record_size,
        GM_HOST_WIRE_SIZE,
        payload.gm_hosts.count,
        payload.gm_hosts.hosts.len(),
    )?;
    validate_counted_tail(
        "admins",
        payload.admins.record_size,
        ADMIN_INFO_WIRE_SIZE,
        payload.admins.count,
        payload.admins.admins.len(),
    )?;
    validate_counted_tail(
        "monarch_candidacy",
        payload.monarch_candidacy.record_size,
        MONARCH_CANDIDACY_WIRE_SIZE,
        payload.monarch_candidacy.count,
        payload.monarch_candidacy.candidates.len(),
    )?;
    Ok(())
}

fn validate_counted_tail(
    field: &'static str,
    record_size: u16,
    expected_record_size: usize,
    declared_count: u16,
    actual_count: usize,
) -> Result<(), DbBootError> {
    if usize::from(record_size) != expected_record_size {
        return Err(DbBootError::InvalidFixedRecordSize {
            field,
            expected: expected_record_size,
            actual: usize::from(record_size),
        });
    }
    let actual_count_u16 =
        u16::try_from(actual_count).map_err(|_| DbBootError::TailCountOverflow {
            field,
            count: actual_count,
        })?;
    if declared_count != actual_count_u16 {
        return Err(DbBootError::InvalidRecordCount {
            field,
            count: declared_count,
            expected: actual_count_u16,
        });
    }
    Ok(())
}

fn write_item_id_range(output: &mut Vec<u8>, range: &BootItemIdRange) {
    output.extend_from_slice(&range.min.to_le_bytes());
    output.extend_from_slice(&range.max.to_le_bytes());
    output.extend_from_slice(&range.usable_item_id_min.to_le_bytes());
}

fn write_monarch_info(output: &mut Vec<u8>, monarch: &BootMonarchInfo) {
    for pid in monarch.pid {
        output.extend_from_slice(&pid.to_le_bytes());
    }
    for money in monarch.money {
        output.extend_from_slice(&money.to_le_bytes());
    }
    for name in monarch.name {
        output.extend_from_slice(&name);
    }
    for date in monarch.date {
        output.extend_from_slice(&date);
    }
}

fn read_table_section(
    cursor: &mut Cursor<'_>,
    kind: BootSectionKind,
) -> Result<BootSection, DbBootError> {
    let record_size = cursor.u16(kind.as_str())?;
    let count = cursor.u16(kind.as_str())?;
    if record_size == 0 {
        return Err(DbBootError::InvalidRecordSize {
            section: kind,
            record_size,
        });
    }
    let byte_len = checked_product(usize::from(record_size), usize::from(count))?;
    let data = copy_bytes(cursor.take(byte_len, kind.as_str())?)?;
    Ok(BootSection {
        kind,
        record_size,
        count,
        data,
    })
}

fn read_item_id_ranges(cursor: &mut Cursor<'_>) -> Result<BootItemIdRanges, DbBootError> {
    let record_size = cursor.u16("item_id_range_size")?;
    let declared_count = cursor.u16("item_id_range_count")?;
    if usize::from(record_size) != ITEM_ID_RANGE_WIRE_SIZE {
        return Err(DbBootError::InvalidFixedRecordSize {
            field: "item_id_range",
            expected: ITEM_ID_RANGE_WIRE_SIZE,
            actual: usize::from(record_size),
        });
    }
    if declared_count != 1 {
        return Err(DbBootError::InvalidRecordCount {
            field: "item_id_range",
            count: declared_count,
            expected: 1,
        });
    }
    // QUERY_BOOT writes count=1 but encodes both active and spare ranges.
    let bytes_len = checked_product(ITEM_ID_RANGE_WIRE_SIZE, 2)?;
    let bytes = cursor.take(bytes_len, "item_id_ranges")?;
    let active = BootItemIdRange::decode(&bytes[..ITEM_ID_RANGE_WIRE_SIZE])?;
    let spare = BootItemIdRange::decode(&bytes[ITEM_ID_RANGE_WIRE_SIZE..])?;
    Ok(BootItemIdRanges {
        record_size,
        declared_count,
        active,
        spare,
    })
}

fn read_gm_hosts(cursor: &mut Cursor<'_>) -> Result<BootGmHostSection, DbBootError> {
    let record_size = cursor.u16("gm_host_size")?;
    let count = cursor.u16("gm_host_count")?;
    if usize::from(record_size) != GM_HOST_WIRE_SIZE {
        return Err(DbBootError::InvalidFixedRecordSize {
            field: "gm_host",
            expected: GM_HOST_WIRE_SIZE,
            actual: usize::from(record_size),
        });
    }
    let byte_len = checked_product(GM_HOST_WIRE_SIZE, usize::from(count))?;
    let bytes = cursor.take(byte_len, "gm_hosts")?;
    let mut hosts = Vec::new();
    hosts
        .try_reserve_exact(usize::from(count))
        .map_err(|_| DbBootError::AllocationFailed)?;
    for record in bytes.chunks_exact(GM_HOST_WIRE_SIZE) {
        hosts.push(BootGmHost::decode(record)?);
    }
    Ok(BootGmHostSection {
        record_size,
        count,
        hosts,
    })
}

fn read_admins(cursor: &mut Cursor<'_>) -> Result<BootAdminSection, DbBootError> {
    let record_size = cursor.u16("admin_size")?;
    let count = cursor.u16("admin_count")?;
    if usize::from(record_size) != ADMIN_INFO_WIRE_SIZE {
        return Err(DbBootError::InvalidFixedRecordSize {
            field: "admin_info",
            expected: ADMIN_INFO_WIRE_SIZE,
            actual: usize::from(record_size),
        });
    }
    let byte_len = checked_product(ADMIN_INFO_WIRE_SIZE, usize::from(count))?;
    let bytes = cursor.take(byte_len, "admins")?;
    let mut admins = Vec::new();
    admins
        .try_reserve_exact(usize::from(count))
        .map_err(|_| DbBootError::AllocationFailed)?;
    for record in bytes.chunks_exact(ADMIN_INFO_WIRE_SIZE) {
        admins.push(BootAdminInfo::decode(record)?);
    }
    Ok(BootAdminSection {
        record_size,
        count,
        admins,
    })
}

fn read_monarch_info(cursor: &mut Cursor<'_>) -> Result<BootMonarchInfo, DbBootError> {
    let record_size = cursor.u16("monarch_size")?;
    let count = cursor.u16("monarch_count")?;
    if usize::from(record_size) != MONARCH_INFO_WIRE_SIZE {
        return Err(DbBootError::InvalidFixedRecordSize {
            field: "monarch_info",
            expected: MONARCH_INFO_WIRE_SIZE,
            actual: usize::from(record_size),
        });
    }
    if count != 1 {
        return Err(DbBootError::InvalidRecordCount {
            field: "monarch_info",
            count,
            expected: 1,
        });
    }
    let bytes = cursor.take(MONARCH_INFO_WIRE_SIZE, "monarch_info")?;
    BootMonarchInfo::decode(bytes)
}

fn read_monarch_candidacy(
    cursor: &mut Cursor<'_>,
) -> Result<BootMonarchCandidacySection, DbBootError> {
    let record_size = cursor.u16("monarch_candidacy_size")?;
    let count = cursor.u16("monarch_candidacy_count")?;
    if usize::from(record_size) != MONARCH_CANDIDACY_WIRE_SIZE {
        return Err(DbBootError::InvalidFixedRecordSize {
            field: "monarch_candidacy",
            expected: MONARCH_CANDIDACY_WIRE_SIZE,
            actual: usize::from(record_size),
        });
    }
    let byte_len = checked_product(MONARCH_CANDIDACY_WIRE_SIZE, usize::from(count))?;
    let bytes = cursor.take(byte_len, "monarch_candidacy")?;
    let mut candidates = Vec::new();
    candidates
        .try_reserve_exact(usize::from(count))
        .map_err(|_| DbBootError::AllocationFailed)?;
    for record in bytes.chunks_exact(MONARCH_CANDIDACY_WIRE_SIZE) {
        candidates.push(BootMonarchCandidacy::decode(record)?);
    }
    Ok(BootMonarchCandidacySection {
        record_size,
        count,
        candidates,
    })
}

fn checked_product(left: usize, right: usize) -> Result<usize, DbBootError> {
    left.checked_mul(right).ok_or(DbBootError::SizeOverflow)
}

fn checked_add(left: usize, right: usize) -> Result<usize, DbBootError> {
    left.checked_add(right).ok_or(DbBootError::SizeOverflow)
}

fn copy_bytes(data: &[u8]) -> Result<Vec<u8>, DbBootError> {
    let mut result = Vec::new();
    result
        .try_reserve_exact(data.len())
        .map_err(|_| DbBootError::AllocationFailed)?;
    result.extend_from_slice(data);
    Ok(result)
}

fn read_i32_at(data: &[u8], offset: &mut usize, field: &'static str) -> Result<i32, DbBootError> {
    let end = offset.checked_add(4).ok_or(DbBootError::SizeOverflow)?;
    if end > data.len() {
        return Err(DbBootError::Truncated {
            field,
            needed: end,
            available: data.len(),
        });
    }
    let value = i32::from_le_bytes([
        data[*offset],
        data[*offset + 1],
        data[*offset + 2],
        data[*offset + 3],
    ]);
    *offset = end;
    Ok(value)
}

fn read_u32_at(data: &[u8], offset: &mut usize, field: &'static str) -> Result<u32, DbBootError> {
    let end = offset.checked_add(4).ok_or(DbBootError::SizeOverflow)?;
    if end > data.len() {
        return Err(DbBootError::Truncated {
            field,
            needed: end,
            available: data.len(),
        });
    }
    let value = u32::from_le_bytes([
        data[*offset],
        data[*offset + 1],
        data[*offset + 2],
        data[*offset + 3],
    ]);
    *offset = end;
    Ok(value)
}

fn read_i64_at(data: &[u8], offset: &mut usize, field: &'static str) -> Result<i64, DbBootError> {
    let end = offset.checked_add(8).ok_or(DbBootError::SizeOverflow)?;
    if end > data.len() {
        return Err(DbBootError::Truncated {
            field,
            needed: end,
            available: data.len(),
        });
    }
    let value = i64::from_le_bytes([
        data[*offset],
        data[*offset + 1],
        data[*offset + 2],
        data[*offset + 3],
        data[*offset + 4],
        data[*offset + 5],
        data[*offset + 6],
        data[*offset + 7],
    ]);
    *offset = end;
    Ok(value)
}

fn read_array_at<const N: usize>(
    data: &[u8],
    offset: &mut usize,
    field: &'static str,
) -> Result<[u8; N], DbBootError> {
    let bytes = read_bytes_at(data, offset, N, field)?;
    let mut result = [0_u8; N];
    result.copy_from_slice(bytes);
    Ok(result)
}

fn read_bytes_at<'a>(
    data: &'a [u8],
    offset: &mut usize,
    length: usize,
    field: &'static str,
) -> Result<&'a [u8], DbBootError> {
    let end = offset
        .checked_add(length)
        .ok_or(DbBootError::SizeOverflow)?;
    if end > data.len() {
        return Err(DbBootError::Truncated {
            field,
            needed: end,
            available: data.len(),
        });
    }
    let result = &data[*offset..end];
    *offset = end;
    Ok(result)
}

struct Cursor<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    const fn new(data: &'a [u8]) -> Self {
        Self { data, offset: 0 }
    }

    fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.offset)
    }

    fn take(&mut self, length: usize, field: &'static str) -> Result<&'a [u8], DbBootError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or(DbBootError::SizeOverflow)?;
        if end > self.data.len() {
            return Err(DbBootError::Truncated {
                field,
                needed: end,
                available: self.data.len(),
            });
        }
        let result = &self.data[self.offset..end];
        self.offset = end;
        Ok(result)
    }

    fn u8(&mut self, field: &'static str) -> Result<u8, DbBootError> {
        Ok(self.take(1, field)?[0])
    }

    fn u16(&mut self, field: &'static str) -> Result<u16, DbBootError> {
        let bytes = self.take(2, field)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn u32(&mut self, field: &'static str) -> Result<u32, DbBootError> {
        let bytes = self.take(4, field)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db_records::{
        ItemAttrRecord, ItemTableRecord, LandRecord, MobTableRecord, ObjectMaterial,
        RefineMaterialRecord, RefineTableRecord, EVENT_TYPE_BYTES, ITEM_TABLE_RECORD_WIRE_SIZE,
        MOB_TABLE_RECORD_WIRE_SIZE, OBJECT_MATERIAL_MAX_NUM,
    };

    type DecodeItemAttrSection = fn(&BootSection) -> Result<Vec<ItemAttrRecord>, DbBootError>;

    fn push_section(out: &mut Vec<u8>, record_size: u16, count: u16, data: &[u8]) {
        out.extend_from_slice(&record_size.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(data);
    }

    fn put_u32(out: &mut Vec<u8>, value: u32) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    fn put_i32(out: &mut Vec<u8>, value: i32) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    fn payload(profile: BootFeatureProfile) -> Vec<u8> {
        let mut body = Vec::new();
        body.push(DB_BOOT_VERSION);
        for index in 0..profile.section_kinds().len() {
            let record_size = 1;
            let count = 1;
            let data = [u8::try_from(index).unwrap() + 1];
            push_section(&mut body, record_size, count, &data);
        }
        put_i32(&mut body, 0x1234_5678);
        let item_range_size = u16::try_from(ITEM_ID_RANGE_WIRE_SIZE).unwrap();
        push_section(&mut body, item_range_size, 1, &{
            let mut ranges = Vec::new();
            for value in 1_u32..=6 {
                put_u32(&mut ranges, value);
            }
            ranges
        });
        // The preceding helper receives the two 12-byte ranges as one blob.
        push_section(&mut body, u16::try_from(GM_HOST_WIRE_SIZE).unwrap(), 0, &[]);
        push_section(
            &mut body,
            u16::try_from(ADMIN_INFO_WIRE_SIZE).unwrap(),
            0,
            &[],
        );
        push_section(
            &mut body,
            u16::try_from(MONARCH_INFO_WIRE_SIZE).unwrap(),
            1,
            &vec![0_u8; MONARCH_INFO_WIRE_SIZE],
        );
        push_section(
            &mut body,
            u16::try_from(MONARCH_CANDIDACY_WIRE_SIZE).unwrap(),
            0,
            &[],
        );
        body.extend_from_slice(&DB_BOOT_END_MARKER.to_le_bytes());

        let mut payload = Vec::with_capacity(body.len() + 4);
        put_u32(&mut payload, u32::try_from(body.len() + 4).unwrap());
        payload.extend_from_slice(&body);
        payload
    }

    fn rich_payload() -> Vec<u8> {
        let profile = BootFeatureProfile::minimal();
        let mut body = Vec::new();
        body.push(DB_BOOT_VERSION);
        for _ in profile.section_kinds() {
            // A valid zero-count table still carries its declared record size.
            push_section(&mut body, 1, 0, &[]);
        }

        body.extend_from_slice(&0x0102_0304_i32.to_le_bytes());
        body.extend_from_slice(
            &u16::try_from(ITEM_ID_RANGE_WIRE_SIZE)
                .unwrap()
                .to_le_bytes(),
        );
        body.extend_from_slice(&1_u16.to_le_bytes());
        for value in 10_u32..=15 {
            body.extend_from_slice(&value.to_le_bytes());
        }

        body.extend_from_slice(&u16::try_from(GM_HOST_WIRE_SIZE).unwrap().to_le_bytes());
        body.extend_from_slice(&1_u16.to_le_bytes());
        body.extend_from_slice(b"0123456789abcdef");

        let mut admin = vec![0_u8; ADMIN_INFO_WIRE_SIZE];
        admin[0..4].copy_from_slice(&(-7_i32).to_le_bytes());
        admin[4..9].copy_from_slice(b"admin");
        admin[36..38].copy_from_slice(b"gm");
        admin[68..75].copy_from_slice(b"1.2.3.4");
        admin[84..91].copy_from_slice(b"5.6.7.8");
        admin[100..104].copy_from_slice(&99_i32.to_le_bytes());
        body.extend_from_slice(&u16::try_from(ADMIN_INFO_WIRE_SIZE).unwrap().to_le_bytes());
        body.extend_from_slice(&1_u16.to_le_bytes());
        body.extend_from_slice(&admin);

        let mut monarch = vec![0_u8; MONARCH_INFO_WIRE_SIZE];
        for index in 0..4 {
            let pid_offset = index * 4;
            let money_offset = 16 + index * 8;
            let name_offset = 48 + index * 32;
            let date_offset = 176 + index * 32;
            monarch[pid_offset..pid_offset + 4]
                .copy_from_slice(&(100_u32 + u32::try_from(index).unwrap()).to_le_bytes());
            monarch[money_offset..money_offset + 8]
                .copy_from_slice(&(-1_000_i64 - i64::try_from(index).unwrap()).to_le_bytes());
            monarch[name_offset..name_offset + 4].copy_from_slice(&[b'n', b'a', b'm', b'e'][..]);
            monarch[date_offset..date_offset + 4].copy_from_slice(&[b'd', b'a', b't', b'e'][..]);
        }
        body.extend_from_slice(&u16::try_from(MONARCH_INFO_WIRE_SIZE).unwrap().to_le_bytes());
        body.extend_from_slice(&1_u16.to_le_bytes());
        body.extend_from_slice(&monarch);

        let mut candidate = vec![0_u8; MONARCH_CANDIDACY_WIRE_SIZE];
        candidate[0..4].copy_from_slice(&4242_u32.to_le_bytes());
        candidate[4..9].copy_from_slice(b"candr");
        candidate[36..41].copy_from_slice(b"today");
        body.extend_from_slice(
            &u16::try_from(MONARCH_CANDIDACY_WIRE_SIZE)
                .unwrap()
                .to_le_bytes(),
        );
        body.extend_from_slice(&1_u16.to_le_bytes());
        body.extend_from_slice(&candidate);
        body.extend_from_slice(&DB_BOOT_END_MARKER.to_le_bytes());

        let mut payload = Vec::with_capacity(body.len() + 4);
        put_u32(&mut payload, u32::try_from(body.len() + 4).unwrap());
        payload.extend_from_slice(&body);
        payload
    }

    #[test]
    fn parses_minimal_profile_and_preserves_x86_time() {
        let bytes = payload(BootFeatureProfile::minimal());
        let parsed = parse_db_boot_payload(&bytes, BootFeatureProfile::minimal()).unwrap();
        assert_eq!(parsed.packet_size, u32::try_from(bytes.len()).unwrap());
        assert_eq!(parsed.version, 6);
        assert_eq!(parsed.global_time, 0x1234_5678);
        assert_eq!(parsed.sections.len(), 11);
        assert_eq!(parsed.item_id_ranges.active.min, 1);
        assert_eq!(parsed.item_id_ranges.active.max, 2);
        assert_eq!(parsed.item_id_ranges.active.usable_item_id_min, 3);
        assert_eq!(parsed.item_id_ranges.spare.min, 4);
        assert_eq!(parsed.end_marker, 0xffff);
    }

    #[test]
    fn boot_request_uses_the_exact_x86_layout() {
        let mut ip = [0_u8; 16];
        ip[..9].copy_from_slice(b"127.0.0.1");
        let request = DbBootRequest::new([0x1122_3344, 0x5566_7788], ip);
        let encoded = request.encode();
        assert_eq!(
            encoded,
            [
                0x44, 0x33, 0x22, 0x11, 0x88, 0x77, 0x66, 0x55, b'1', b'2', b'7', b'.', b'0', b'.',
                b'0', b'.', b'1', 0, 0, 0, 0, 0, 0, 0,
            ]
        );
        assert_eq!(DbBootRequest::decode(&encoded), Ok(request));
        assert!(matches!(
            DbBootRequest::decode(&encoded[..23]),
            Err(DbBootError::InvalidFixedRecordSize {
                field: "boot_request",
                ..
            })
        ));
        let mut overlong = encoded.to_vec();
        overlong.push(0);
        assert!(matches!(
            DbBootRequest::decode(&overlong),
            Err(DbBootError::InvalidFixedRecordSize {
                field: "boot_request",
                ..
            })
        ));
    }

    #[test]
    fn encoder_round_trips_every_profile_and_rich_fixed_tail() {
        for mask in 0_u8..8 {
            let profile = BootFeatureProfile::new(mask & 1 != 0, mask & 2 != 0, mask & 4 != 0);
            let bytes = payload(profile);
            let parsed = parse_db_boot_payload(&bytes, profile).unwrap();
            assert_eq!(parsed.encode(profile).unwrap(), bytes);
            assert_eq!(
                parse_db_boot_payload(&parsed.encode(profile).unwrap(), profile).unwrap(),
                parsed
            );
        }

        let profile = BootFeatureProfile::minimal();
        let bytes = rich_payload();
        let parsed = parse_db_boot_payload(&bytes, profile).unwrap();
        assert_eq!(parsed.encode(profile).unwrap(), bytes);
    }

    #[test]
    fn encodes_boot_response_frame_with_legacy_header_and_zero_handle() {
        let profile = BootFeatureProfile::minimal();
        let parsed = parse_db_boot_payload(&payload(profile), profile).unwrap();
        let frame = parsed.encode_frame(profile).unwrap();

        assert_eq!(frame.header, HEADER_DG_BOOT);
        assert_eq!(frame.handle, DB_BOOT_RESPONSE_HANDLE);
        let round_trip = parse_db_boot_frame(&frame, profile).unwrap();
        assert_eq!(round_trip.encode(profile).unwrap(), frame.payload);
    }

    #[test]
    fn encoder_matches_source_transcribed_minimal_golden_bytes() {
        let golden = [
            0x93, 0x01, 0x00, 0x00, 0x06, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
            0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00,
            0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
            0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x04, 0x03, 0x02, 0x01, 0x0c, 0x00, 0x01,
            0x00, 0x0a, 0x00, 0x00, 0x00, 0x0b, 0x00, 0x00, 0x00, 0x0c, 0x00, 0x00, 0x00, 0x14,
            0x00, 0x00, 0x00, 0x15, 0x00, 0x00, 0x00, 0x16, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00,
            0x00, 0x68, 0x00, 0x00, 0x00, 0x30, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x44, 0x00, 0x00, 0x00, 0xff, 0xff,
        ];
        let profile = BootFeatureProfile::minimal();
        let parsed = parse_db_boot_payload(&golden, profile).unwrap();
        assert_eq!(parsed.encode(profile).unwrap(), golden);
    }

    #[test]
    fn encoder_rejects_mutated_profile_section_and_fixed_tail_metadata() {
        let profile = BootFeatureProfile::minimal();
        let valid = parse_db_boot_payload(&rich_payload(), profile).unwrap();

        let mut wrong_count = valid.clone();
        wrong_count.sections.pop();
        assert!(matches!(
            wrong_count.encode(profile),
            Err(DbBootError::SectionCountMismatch { .. })
        ));

        let mut wrong_order = valid.clone();
        wrong_order.sections.swap(0, 1);
        assert!(matches!(
            wrong_order.encode(profile),
            Err(DbBootError::UnexpectedSectionKind { index: 0, .. })
        ));

        let mut zero_size = valid.clone();
        zero_size.sections[0].record_size = 0;
        assert!(matches!(
            zero_size.encode(profile),
            Err(DbBootError::InvalidRecordSize { .. })
        ));

        let mut wrong_section_length = valid.clone();
        wrong_section_length.sections[0].data.push(0);
        assert!(matches!(
            wrong_section_length.encode(profile),
            Err(DbBootError::SectionLengthMismatch { .. })
        ));

        let mut wrong_item_size = valid.clone();
        wrong_item_size.item_id_ranges.record_size = 11;
        assert!(matches!(
            wrong_item_size.encode(profile),
            Err(DbBootError::InvalidFixedRecordSize {
                field: "item_id_range",
                ..
            })
        ));

        let mut wrong_item_count = valid.clone();
        wrong_item_count.item_id_ranges.declared_count = 2;
        assert!(matches!(
            wrong_item_count.encode(profile),
            Err(DbBootError::InvalidRecordCount {
                field: "item_id_range",
                ..
            })
        ));

        let mut wrong_host_size = valid.clone();
        wrong_host_size.gm_hosts.record_size = 15;
        assert!(matches!(
            wrong_host_size.encode(profile),
            Err(DbBootError::InvalidFixedRecordSize {
                field: "gm_hosts",
                ..
            })
        ));

        let mut wrong_host_count = valid.clone();
        wrong_host_count.gm_hosts.count = 0;
        assert!(matches!(
            wrong_host_count.encode(profile),
            Err(DbBootError::InvalidRecordCount {
                field: "gm_hosts",
                ..
            })
        ));

        let mut wrong_admin_count = valid.clone();
        wrong_admin_count.admins.count = 0;
        assert!(matches!(
            wrong_admin_count.encode(profile),
            Err(DbBootError::InvalidRecordCount {
                field: "admins",
                ..
            })
        ));

        let mut wrong_candidacy_size = valid.clone();
        wrong_candidacy_size.monarch_candidacy.record_size = 67;
        assert!(matches!(
            wrong_candidacy_size.encode(profile),
            Err(DbBootError::InvalidFixedRecordSize {
                field: "monarch_candidacy",
                ..
            })
        ));
    }

    #[test]
    fn encoder_enforces_packet_metadata_and_allocation_limit() {
        let profile = BootFeatureProfile::minimal();
        let bytes = rich_payload();
        let payload = parse_db_boot_payload(&bytes, profile).unwrap();

        assert!(matches!(
            payload.encode_with_limit(profile, bytes.len() - 1),
            Err(DbBootError::EncodePayloadTooLarge {
                length,
                maximum,
            }) if length == bytes.len() && maximum == bytes.len() - 1
        ));
        assert_eq!(
            payload.encode_with_limit(profile, bytes.len()).unwrap(),
            bytes
        );

        let mut wrong_packet_size = payload.clone();
        wrong_packet_size.packet_size = wrong_packet_size.packet_size.wrapping_add(1);
        assert!(matches!(
            wrong_packet_size.encode(profile),
            Err(DbBootError::InvalidPacketSize { .. })
        ));

        let mut wrong_version = payload.clone();
        wrong_version.version = 5;
        assert!(matches!(
            wrong_version.encode(profile),
            Err(DbBootError::InvalidVersion { version: 5 })
        ));

        let mut wrong_end = payload;
        wrong_end.end_marker = 0;
        assert!(matches!(
            wrong_end.encode(profile),
            Err(DbBootError::InvalidEndMarker { marker: 0 })
        ));
    }

    #[test]
    fn active_profile_has_all_optional_sections_in_wire_order() {
        let profile = BootFeatureProfile::active();
        let bytes = payload(profile);
        let parsed = parse_db_boot_payload(&bytes, profile).unwrap();
        assert_eq!(parsed.sections.len(), 14);
        assert_eq!(parsed.sections[3].kind, BootSectionKind::RenewalShop);
        assert_eq!(parsed.sections[12].kind, BootSectionKind::Event);
        assert_eq!(
            parsed.sections[13].kind,
            BootSectionKind::PremiumMarketPrice
        );
    }

    #[test]
    fn rejects_profile_mismatch() {
        let bytes = payload(BootFeatureProfile::minimal());
        assert!(parse_db_boot_payload(&bytes, BootFeatureProfile::active()).is_err());
    }

    #[test]
    fn rejects_bad_version_and_packet_size() {
        let mut bytes = payload(BootFeatureProfile::minimal());
        bytes[4] = 5;
        assert!(matches!(
            parse_db_boot_payload(&bytes, BootFeatureProfile::minimal()),
            Err(DbBootError::InvalidVersion { version: 5 })
        ));

        let mut bytes = payload(BootFeatureProfile::minimal());
        bytes[0] = bytes[0].wrapping_add(1);
        assert!(matches!(
            parse_db_boot_payload(&bytes, BootFeatureProfile::minimal()),
            Err(DbBootError::InvalidPacketSize { .. })
        ));
    }

    #[test]
    fn rejects_zero_section_size_and_truncation() {
        let mut bytes = payload(BootFeatureProfile::minimal());
        // The first section starts after packet_size and version.
        bytes[5] = 0;
        bytes[6] = 0;
        assert!(matches!(
            parse_db_boot_payload(&bytes, BootFeatureProfile::minimal()),
            Err(DbBootError::InvalidRecordSize { .. })
        ));

        let mut bytes = payload(BootFeatureProfile::minimal());
        bytes[5] = 0xff;
        bytes[6] = 0xff;
        assert!(parse_db_boot_payload(&bytes, BootFeatureProfile::minimal()).is_err());
    }

    #[test]
    fn rejects_bad_end_marker_and_trailing_bytes() {
        let mut bytes = payload(BootFeatureProfile::minimal());
        let end = bytes.len() - 2;
        bytes[end] = 0;
        bytes[end + 1] = 0;
        assert!(matches!(
            parse_db_boot_payload(&bytes, BootFeatureProfile::minimal()),
            Err(DbBootError::InvalidEndMarker { .. })
        ));

        let mut bytes = payload(BootFeatureProfile::minimal());
        bytes.push(0xaa);
        let new_length = u32::try_from(bytes.len()).unwrap();
        bytes[0..4].copy_from_slice(&new_length.to_le_bytes());
        assert!(matches!(
            parse_db_boot_payload(&bytes, BootFeatureProfile::minimal()),
            Err(DbBootError::TrailingBytes { count: 1 })
        ));
    }

    #[test]
    fn frame_helpers_check_the_db_header() {
        let bytes = payload(BootFeatureProfile::minimal());
        let mut frame = vec![0_u8; 9];
        frame[0] = HEADER_DG_BOOT;
        frame[5..9].copy_from_slice(&u32::try_from(bytes.len()).unwrap().to_le_bytes());
        frame.extend_from_slice(&bytes);
        assert!(parse_db_boot_frame_bytes(&frame, BootFeatureProfile::minimal()).is_ok());
        let mut wrong_handle = frame.clone();
        wrong_handle[1] = 1;
        assert!(matches!(
            parse_db_boot_frame_bytes(&wrong_handle, BootFeatureProfile::minimal()),
            Err(DbBootError::UnexpectedHandle { actual: 1, .. })
        ));
        frame[0] = 0;
        assert!(matches!(
            parse_db_boot_frame_bytes(&frame, BootFeatureProfile::minimal()),
            Err(DbBootError::UnexpectedHeader { .. })
        ));
    }

    #[test]
    fn decodes_fixed_tail_fields_in_exact_wire_order() {
        let bytes = rich_payload();
        let parsed = parse_db_boot_payload(&bytes, BootFeatureProfile::minimal()).unwrap();
        assert_eq!(parsed.global_time, 0x0102_0304);
        assert_eq!(parsed.gm_hosts.record_size, 16);
        assert_eq!(parsed.gm_hosts.count, 1);
        assert_eq!(parsed.gm_hosts.hosts[0].bytes, *b"0123456789abcdef");
        assert_eq!(parsed.gm_hosts.hosts[0].as_str(), "0123456789abcdef");
        assert_eq!(parsed.admins.record_size, 104);
        assert_eq!(parsed.admins.admins[0].id, -7);
        assert_eq!(&parsed.admins.admins[0].account[..5], b"admin");
        assert_eq!(&parsed.admins.admins[0].name[..2], b"gm");
        assert_eq!(&parsed.admins.admins[0].contact_ip[..7], b"1.2.3.4");
        assert_eq!(&parsed.admins.admins[0].server_ip[..7], b"5.6.7.8");
        assert_eq!(parsed.admins.admins[0].authority, 99);
        assert_eq!(parsed.monarch.pid, [100, 101, 102, 103]);
        assert_eq!(parsed.monarch.money, [-1_000, -1_001, -1_002, -1_003]);
        assert_eq!(&parsed.monarch.name[0][..4], b"name");
        assert_eq!(&parsed.monarch.date[0][..4], b"date");
        assert_eq!(parsed.monarch_candidacy.candidates[0].pid, 4242);
        assert_eq!(&parsed.monarch_candidacy.candidates[0].name[..5], b"candr");
        assert_eq!(&parsed.monarch_candidacy.candidates[0].date[..5], b"today");
        assert_eq!(parsed.end_marker, DB_BOOT_END_MARKER);
    }

    #[test]
    fn every_feature_profile_has_the_expected_ordered_sections() {
        for mask in 0_u8..8 {
            let profile = BootFeatureProfile::new(mask & 1 != 0, mask & 2 != 0, mask & 4 != 0);
            let bytes = payload(profile);
            let parsed = parse_db_boot_payload(&bytes, profile).unwrap();
            let expected_len = 11
                + usize::from(mask & 1 != 0)
                + usize::from(mask & 2 != 0)
                + usize::from(mask & 4 != 0);
            assert_eq!(parsed.sections.len(), expected_len);
            assert_eq!(parsed.sections.len(), profile.section_kinds().len());
            for (section, kind) in parsed.sections.iter().zip(profile.section_kinds()) {
                assert_eq!(section.kind, *kind);
            }
        }
    }

    #[test]
    fn decodes_market_prices_only_for_the_enabled_profile() {
        let minimal_profile = BootFeatureProfile::minimal();
        let market_profile = BootFeatureProfile::new(false, false, true);
        let mut parsed = parse_db_boot_payload(&payload(minimal_profile), minimal_profile).unwrap();
        let first = MarketItemPriceRecord {
            vnum: 0x1122_3344,
            gold: 0x0102_0304_0506_0708,
            cheque: 0xa1b2_c3d4,
        };
        let second = MarketItemPriceRecord {
            vnum: 7,
            gold: -9,
            cheque: 11,
        };
        let mut data = first.encode();
        data.extend_from_slice(&second.encode());
        parsed.sections.push(BootSection {
            kind: BootSectionKind::PremiumMarketPrice,
            record_size: u16::try_from(MARKET_ITEM_PRICE_WIRE_SIZE).unwrap(),
            count: 2,
            data,
        });

        assert_eq!(
            parsed.market_item_prices(market_profile).unwrap(),
            vec![first, second]
        );
        assert_eq!(
            decode_market_item_price_section(parsed.sections.last().unwrap(), market_profile)
                .unwrap(),
            vec![first, second]
        );
        assert_eq!(
            parsed.market_item_prices(minimal_profile),
            Err(DbBootError::ProfileFeatureDisabled {
                feature: "premium_market_price",
            })
        );

        let missing = parse_db_boot_payload(&payload(minimal_profile), minimal_profile).unwrap();
        assert_eq!(
            missing.market_item_prices(market_profile),
            Err(DbBootError::MissingSection {
                section: BootSectionKind::PremiumMarketPrice,
            })
        );

        let mut wrong_kind = parsed.sections.last().unwrap().clone();
        wrong_kind.kind = BootSectionKind::Item;
        assert_eq!(
            decode_market_item_price_section(&wrong_kind, market_profile),
            Err(DbBootError::UnexpectedDecoderSection {
                expected: BootSectionKind::PremiumMarketPrice,
                actual: BootSectionKind::Item,
            })
        );

        let mut wrong_size = parsed.sections.last().unwrap().clone();
        wrong_size.record_size = 15;
        assert_eq!(
            decode_market_item_price_section(&wrong_size, market_profile),
            Err(DbBootError::InvalidFixedRecordSize {
                field: "market_item_price_section",
                expected: MARKET_ITEM_PRICE_WIRE_SIZE,
                actual: 15,
            })
        );

        let mut wrong_count = parsed.sections.last().unwrap().clone();
        wrong_count.count = 3;
        assert_eq!(
            decode_market_item_price_section(&wrong_count, market_profile),
            Err(DbBootError::SectionLengthMismatch {
                section: BootSectionKind::PremiumMarketPrice,
                expected: MARKET_ITEM_PRICE_WIRE_SIZE * 3,
                actual: MARKET_ITEM_PRICE_WIRE_SIZE * 2,
            })
        );
    }

    #[test]
    fn decodes_skill_table_section_with_strict_metadata() {
        let profile = BootFeatureProfile::minimal();
        let mut parsed = parse_db_boot_payload(&payload(profile), profile).unwrap();
        let mut first = SkillTableRecord {
            vnum: 101,
            ..SkillTableRecord::default()
        };
        first.name[..6].copy_from_slice(b"SKILL\0");
        first.skill_type = 1;
        first.max_hit = -42;
        let second = SkillTableRecord {
            vnum: 102,
            max_hit: i32::MAX,
            ..SkillTableRecord::default()
        };
        let mut data = first.encode();
        data.extend_from_slice(&second.encode());
        let section = parsed
            .sections
            .iter_mut()
            .find(|section| section.kind == BootSectionKind::Skill)
            .unwrap();
        section.record_size = u16::try_from(SKILL_TABLE_RECORD_WIRE_SIZE).unwrap();
        section.count = 2;
        section.data = data;

        assert_eq!(parsed.skill_table_records().unwrap(), vec![first, second]);
        let section_ref = parsed
            .section(BootSectionKind::Skill)
            .expect("skill section");
        assert_eq!(
            decode_skill_table_section(section_ref).unwrap(),
            vec![first, second]
        );

        let mut missing = parsed.clone();
        missing
            .sections
            .retain(|section| section.kind != BootSectionKind::Skill);
        assert_eq!(
            missing.skill_table_records(),
            Err(DbBootError::MissingSection {
                section: BootSectionKind::Skill,
            })
        );

        let mut wrong_kind = section_ref.clone();
        wrong_kind.kind = BootSectionKind::Item;
        assert_eq!(
            decode_skill_table_section(&wrong_kind),
            Err(DbBootError::UnexpectedDecoderSection {
                expected: BootSectionKind::Skill,
                actual: BootSectionKind::Item,
            })
        );

        for wrong_width in [SKILL_TABLE_RECORD_WIRE_SIZE - 1, 1_480] {
            let mut wrong_size = section_ref.clone();
            wrong_size.record_size = u16::try_from(wrong_width).unwrap();
            assert_eq!(
                decode_skill_table_section(&wrong_size),
                Err(DbBootError::InvalidFixedRecordSize {
                    field: "skill_table_section",
                    expected: SKILL_TABLE_RECORD_WIRE_SIZE,
                    actual: wrong_width,
                })
            );
        }

        let mut wrong_count = section_ref.clone();
        wrong_count.count = 3;
        assert_eq!(
            decode_skill_table_section(&wrong_count),
            Err(DbBootError::SectionLengthMismatch {
                section: BootSectionKind::Skill,
                expected: SKILL_TABLE_RECORD_WIRE_SIZE * 3,
                actual: SKILL_TABLE_RECORD_WIRE_SIZE * 2,
            })
        );
    }

    #[test]
    fn decodes_mob_table_section_with_strict_metadata() {
        let profile = BootFeatureProfile::minimal();
        let mut parsed = parse_db_boot_payload(&payload(profile), profile).unwrap();
        let mut first = MobTableRecord {
            vnum: 0x1122_3344,
            mob_type: 1,
            level: 42,
            max_hp: 10_000,
            ai_flag: 0xa1b2_c3d4,
            damage_range: [7, 9],
            attack_speed: -10,
            moving_speed: 250,
            dam_multiply: 1.25,
            summon_vnum: 77,
            ..MobTableRecord::default()
        };
        first.name[..5].copy_from_slice(b"wolf\0");
        first.locale_name[..3].copy_from_slice(&[0xff, 0x00, 0xfe]);
        first.folder[..11].copy_from_slice(b"field/wolf\0");
        let second = MobTableRecord {
            vnum: 0xaabb_ccdd,
            level: 9,
            max_hp: u32::MAX,
            immune_flag: u32::MAX,
            attack_speed: i16::MIN,
            moving_speed: i16::MAX,
            ..MobTableRecord::default()
        };
        let mut data = first.encode();
        data.extend_from_slice(&second.encode());
        assert_eq!(first.encode().len(), MOB_TABLE_RECORD_WIRE_SIZE);

        let section = parsed
            .sections
            .iter_mut()
            .find(|section| section.kind == BootSectionKind::Mob)
            .unwrap();
        section.record_size = u16::try_from(MOB_TABLE_RECORD_WIRE_SIZE).unwrap();
        section.count = 2;
        section.data = data;

        assert_eq!(parsed.mob_table_records().unwrap(), vec![first, second]);
        let section_ref = parsed.section(BootSectionKind::Mob).expect("mob section");
        assert_eq!(
            decode_mob_table_section(section_ref).unwrap(),
            vec![first, second]
        );

        let mut empty = section_ref.clone();
        empty.count = 0;
        empty.data.clear();
        assert!(decode_mob_table_section(&empty).unwrap().is_empty());

        let mut missing = parsed.clone();
        missing
            .sections
            .retain(|section| section.kind != BootSectionKind::Mob);
        assert_eq!(
            missing.mob_table_records(),
            Err(DbBootError::MissingSection {
                section: BootSectionKind::Mob,
            })
        );

        let mut wrong_kind = section_ref.clone();
        wrong_kind.kind = BootSectionKind::Item;
        assert_eq!(
            decode_mob_table_section(&wrong_kind),
            Err(DbBootError::UnexpectedDecoderSection {
                expected: BootSectionKind::Mob,
                actual: BootSectionKind::Item,
            })
        );

        for wrong_width in [MOB_TABLE_RECORD_WIRE_SIZE - 1, 1] {
            let mut wrong_size = section_ref.clone();
            wrong_size.record_size = u16::try_from(wrong_width).unwrap();
            assert_eq!(
                decode_mob_table_section(&wrong_size),
                Err(DbBootError::InvalidFixedRecordSize {
                    field: "mob_table_section",
                    expected: MOB_TABLE_RECORD_WIRE_SIZE,
                    actual: wrong_width,
                })
            );
        }

        let mut wrong_count = section_ref.clone();
        wrong_count.count = 3;
        assert_eq!(
            decode_mob_table_section(&wrong_count),
            Err(DbBootError::SectionLengthMismatch {
                section: BootSectionKind::Mob,
                expected: MOB_TABLE_RECORD_WIRE_SIZE * 3,
                actual: MOB_TABLE_RECORD_WIRE_SIZE * 2,
            })
        );

        let mut short_data = section_ref.clone();
        short_data.data.pop();
        assert_eq!(
            decode_mob_table_section(&short_data),
            Err(DbBootError::SectionLengthMismatch {
                section: BootSectionKind::Mob,
                expected: MOB_TABLE_RECORD_WIRE_SIZE * 2,
                actual: MOB_TABLE_RECORD_WIRE_SIZE * 2 - 1,
            })
        );
    }

    #[test]
    fn decodes_item_table_section_with_strict_metadata() {
        let profile = BootFeatureProfile::minimal();
        let mut parsed = parse_db_boot_payload(&payload(profile), profile).unwrap();
        let mut first = ItemTableRecord {
            vnum: 0x1122_3344,
            vnum_range: 99,
            item_type: 1,
            sub_type: 2,
            weight: 3,
            size: 4,
            anti_flags: 5,
            flags: 6,
            wear_flags: 7,
            immune_flag: 8,
            gold: u64::MAX,
            shop_buy_price: 42,
            refined_vnum: 9,
            refine_set: 10,
            alter_to_magic_item_pct: 11,
            specular: 12,
            gain_socket_pct: 13,
            addon_type: -14,
            limit_real_time_first_use_index: -1,
            limit_timer_based_on_wear_index: -2,
            ..ItemTableRecord::default()
        };
        first.name[..6].copy_from_slice(b"item\0x");
        first.locale_name[..3].copy_from_slice(&[0xff, 0x00, 0xfe]);
        let second = ItemTableRecord {
            vnum: 0xaabb_ccdd,
            gold: 0,
            shop_buy_price: u64::MAX,
            ..ItemTableRecord::default()
        };
        let mut data = first.encode();
        data.extend_from_slice(&second.encode());
        assert_eq!(first.encode().len(), ITEM_TABLE_RECORD_WIRE_SIZE);
        let section = parsed
            .sections
            .iter_mut()
            .find(|section| section.kind == BootSectionKind::Item)
            .unwrap();
        section.record_size = u16::try_from(ITEM_TABLE_RECORD_WIRE_SIZE).unwrap();
        section.count = 2;
        section.data = data;

        assert_eq!(parsed.item_table_records().unwrap(), vec![first, second]);
        let section_ref = parsed.section(BootSectionKind::Item).expect("item section");
        assert_eq!(
            decode_item_table_section(section_ref).unwrap(),
            vec![first, second]
        );

        let mut empty = section_ref.clone();
        empty.count = 0;
        empty.data.clear();
        assert!(decode_item_table_section(&empty).unwrap().is_empty());

        let mut missing = parsed.clone();
        missing
            .sections
            .retain(|section| section.kind != BootSectionKind::Item);
        assert_eq!(
            missing.item_table_records(),
            Err(DbBootError::MissingSection {
                section: BootSectionKind::Item,
            })
        );

        let mut wrong_kind = section_ref.clone();
        wrong_kind.kind = BootSectionKind::Mob;
        assert_eq!(
            decode_item_table_section(&wrong_kind),
            Err(DbBootError::UnexpectedDecoderSection {
                expected: BootSectionKind::Item,
                actual: BootSectionKind::Mob,
            })
        );

        for wrong_width in [ITEM_TABLE_RECORD_WIRE_SIZE - 1, 1] {
            let mut wrong_size = section_ref.clone();
            wrong_size.record_size = u16::try_from(wrong_width).unwrap();
            assert_eq!(
                decode_item_table_section(&wrong_size),
                Err(DbBootError::InvalidFixedRecordSize {
                    field: "item_table_section",
                    expected: ITEM_TABLE_RECORD_WIRE_SIZE,
                    actual: wrong_width,
                })
            );
        }

        let mut wrong_count = section_ref.clone();
        wrong_count.count = 3;
        assert_eq!(
            decode_item_table_section(&wrong_count),
            Err(DbBootError::SectionLengthMismatch {
                section: BootSectionKind::Item,
                expected: ITEM_TABLE_RECORD_WIRE_SIZE * 3,
                actual: ITEM_TABLE_RECORD_WIRE_SIZE * 2,
            })
        );
    }

    #[test]
    fn decodes_only_the_base_shop_section_with_strict_metadata() {
        let profile = BootFeatureProfile::minimal();
        let mut parsed = parse_db_boot_payload(&payload(profile), profile).unwrap();
        let mut first = ShopTableRecord::zeroed_base_shop();
        first.vnum = 0x1122_3344;
        first.npc_vnum = 0xa1b2_c3d4;
        first.item_count = 1;
        first.items[0].vnum = 42;
        first.items[0].count = 3;
        first.items[0].price = 99;
        first.shop_name[..5].copy_from_slice(b"first");
        let mut second = ShopTableRecord::zeroed_base_shop();
        second.vnum = 7;
        second.npc_vnum = 8;
        second.shop_name[..6].copy_from_slice(b"second");
        let mut data = first.encode();
        data.extend_from_slice(&second.encode());

        let section = parsed
            .sections
            .iter_mut()
            .find(|section| section.kind == BootSectionKind::Shop)
            .unwrap();
        section.record_size = u16::try_from(SHOP_TABLE_RECORD_WIRE_SIZE).unwrap();
        section.count = 2;
        section.data = data;

        assert_eq!(parsed.shop_table_records().unwrap(), vec![first, second]);
        assert_eq!(parsed.shop_records().unwrap(), vec![first, second]);
        let section_ref = parsed.section(BootSectionKind::Shop).unwrap();
        assert_eq!(
            decode_shop_table_section(section_ref).unwrap(),
            vec![first, second]
        );
        assert_eq!(
            decode_shop_section(section_ref).unwrap(),
            vec![first, second]
        );

        let mut missing = parsed.clone();
        missing
            .sections
            .retain(|section| section.kind != BootSectionKind::Shop);
        assert_eq!(
            missing.shop_table_records(),
            Err(DbBootError::MissingSection {
                section: BootSectionKind::Shop,
            })
        );

        let mut renewal = section_ref.clone();
        renewal.kind = BootSectionKind::RenewalShop;
        assert_eq!(
            decode_shop_table_section(&renewal),
            Err(DbBootError::UnexpectedDecoderSection {
                expected: BootSectionKind::Shop,
                actual: BootSectionKind::RenewalShop,
            })
        );

        let mut malformed_count = section_ref.clone();
        let mut malformed_record = first;
        malformed_record.item_count = 41;
        malformed_count.count = 1;
        malformed_count.data = malformed_record.encode();
        assert_eq!(
            decode_shop_table_section(&malformed_count),
            Err(DbBootError::InvalidRecordCount {
                field: "shop_item_count",
                count: 41,
                expected: 40,
            })
        );

        let mut wrong_size = section_ref.clone();
        wrong_size.record_size = u16::try_from(SHOP_TABLE_RECORD_WIRE_SIZE - 1).unwrap();
        assert_eq!(
            decode_shop_table_section(&wrong_size),
            Err(DbBootError::InvalidFixedRecordSize {
                field: "shop_table_section",
                expected: SHOP_TABLE_RECORD_WIRE_SIZE,
                actual: SHOP_TABLE_RECORD_WIRE_SIZE - 1,
            })
        );

        let mut wrong_count = section_ref.clone();
        wrong_count.count = 3;
        assert_eq!(
            decode_shop_table_section(&wrong_count),
            Err(DbBootError::SectionLengthMismatch {
                section: BootSectionKind::Shop,
                expected: SHOP_TABLE_RECORD_WIRE_SIZE * 3,
                actual: SHOP_TABLE_RECORD_WIRE_SIZE * 2,
            })
        );
    }

    #[test]
    fn decodes_optional_renewal_shop_section_with_strict_metadata() {
        let profile = BootFeatureProfile::minimal().with_renewal_shop_ex(true);
        let mut parsed = parse_db_boot_payload(&payload(profile), profile).unwrap();
        let mut record = ShopTableRecord::zeroed_renewal_shop();
        record.vnum = 17;
        record.npc_vnum = 23;
        record.item_count = 1;
        record.items[0].vnum = 99;
        record.shop_name[..3].copy_from_slice(b"RNY");
        let encoded = record.encode();
        let section = parsed
            .sections
            .iter_mut()
            .find(|section| section.kind == BootSectionKind::RenewalShop)
            .unwrap();
        section.record_size = u16::try_from(SHOP_TABLE_RECORD_WIRE_SIZE).unwrap();
        section.count = 1;
        section.data = encoded;
        let section_ref = parsed.section(BootSectionKind::RenewalShop).unwrap();
        assert_eq!(parsed.renewal_shop_table_records().unwrap(), vec![record]);
        assert_eq!(parsed.renewal_shop_records().unwrap(), vec![record]);
        assert_eq!(
            decode_renewal_shop_table_section(section_ref).unwrap(),
            vec![record]
        );
        assert_eq!(
            decode_renewal_shop_section(section_ref).unwrap(),
            vec![record]
        );

        let mut wrong_kind = section_ref.clone();
        wrong_kind.kind = BootSectionKind::Shop;
        assert_eq!(
            decode_renewal_shop_table_section(&wrong_kind),
            Err(DbBootError::UnexpectedDecoderSection {
                expected: BootSectionKind::RenewalShop,
                actual: BootSectionKind::Shop,
            })
        );
        let mut malformed = record;
        malformed.item_count = 41;
        let mut malformed_section = section_ref.clone();
        malformed_section.count = 1;
        malformed_section.data = malformed.encode();
        assert_eq!(
            decode_renewal_shop_table_section(&malformed_section),
            Err(DbBootError::InvalidRecordCount {
                field: "renewal_shop_item_count",
                count: 41,
                expected: 40,
            })
        );
    }

    #[test]
    fn decodes_normal_and_rare_item_attr_sections_with_strict_metadata() {
        let profile = BootFeatureProfile::minimal();
        let mut parsed = parse_db_boot_payload(&payload(profile), profile).unwrap();
        let mut first = ItemAttrRecord::default();
        first.apply[..5].copy_from_slice(b"ATK+1");
        first.apply_index = 0x1122_3344;
        first.prob = 7;
        first.values = [1, -2, 3, -4, 5];
        first.max_level_by_set = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
        let second = ItemAttrRecord {
            apply: *b"DEF_BONUS\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
            apply_index: u32::MAX,
            prob: u32::MAX,
            values: [i32::MIN, -1, 0, 1, i32::MAX],
            max_level_by_set: [10, 9, 8, 7, 6, 5, 4, 3, 2, 1],
        };
        let mut data = first.encode();
        data.extend_from_slice(&second.encode());
        for kind in [BootSectionKind::ItemAttr, BootSectionKind::ItemRare] {
            let section = parsed
                .sections
                .iter_mut()
                .find(|section| section.kind == kind)
                .unwrap();
            section.record_size = u16::try_from(ITEM_ATTR_RECORD_WIRE_SIZE).unwrap();
            section.count = 2;
            section.data.clone_from(&data);
        }
        assert_eq!(parsed.item_attr_records().unwrap(), vec![first, second]);
        assert_eq!(parsed.item_rare_records().unwrap(), vec![first, second]);

        let decoders: [DecodeItemAttrSection; 2] =
            [decode_item_attr_section, decode_item_rare_section];
        let kinds = [BootSectionKind::ItemAttr, BootSectionKind::ItemRare];
        let field_names = ["item_attr_section", "item_rare_section"];
        for ((decode, kind), field) in decoders.into_iter().zip(kinds).zip(field_names) {
            let section = parsed.section(kind).unwrap();
            assert_eq!(decode(section).unwrap(), vec![first, second]);

            let mut wrong_kind = section.clone();
            wrong_kind.kind = BootSectionKind::Item;
            assert_eq!(
                decode(&wrong_kind),
                Err(DbBootError::UnexpectedDecoderSection {
                    expected: kind,
                    actual: BootSectionKind::Item,
                })
            );

            let mut wrong_size = section.clone();
            wrong_size.record_size = 70;
            assert_eq!(
                decode(&wrong_size),
                Err(DbBootError::InvalidFixedRecordSize {
                    field,
                    expected: ITEM_ATTR_RECORD_WIRE_SIZE,
                    actual: 70,
                })
            );

            let mut wrong_count = section.clone();
            wrong_count.count = 3;
            assert_eq!(
                decode(&wrong_count),
                Err(DbBootError::SectionLengthMismatch {
                    section: kind,
                    expected: ITEM_ATTR_RECORD_WIRE_SIZE * 3,
                    actual: ITEM_ATTR_RECORD_WIRE_SIZE * 2,
                })
            );
        }

        let mut missing_attr = parsed.clone();
        missing_attr
            .sections
            .retain(|section| section.kind != BootSectionKind::ItemAttr);
        assert_eq!(
            missing_attr.item_attr_records(),
            Err(DbBootError::MissingSection {
                section: BootSectionKind::ItemAttr,
            })
        );
        let mut missing_rare = parsed;
        missing_rare
            .sections
            .retain(|section| section.kind != BootSectionKind::ItemRare);
        assert_eq!(
            missing_rare.item_rare_records(),
            Err(DbBootError::MissingSection {
                section: BootSectionKind::ItemRare,
            })
        );
    }

    #[test]
    fn decodes_refine_table_section_with_strict_metadata() {
        let profile = BootFeatureProfile::minimal();
        let mut parsed = parse_db_boot_payload(&payload(profile), profile).unwrap();
        let first = RefineTableRecord {
            id: 101,
            material_count: 2,
            cost: -100,
            prob: 25,
            materials: [
                RefineMaterialRecord { vnum: 11, count: 2 },
                RefineMaterialRecord { vnum: 12, count: 3 },
                RefineMaterialRecord::default(),
                RefineMaterialRecord::default(),
                RefineMaterialRecord::default(),
            ],
        };
        let second = RefineTableRecord {
            id: 102,
            material_count: 0,
            cost: i32::MIN,
            prob: i32::MAX,
            materials: [RefineMaterialRecord {
                vnum: u32::MAX,
                count: -1,
            }; 5],
        };
        let mut data = first.encode();
        data.extend_from_slice(&second.encode());
        let section = parsed
            .sections
            .iter_mut()
            .find(|section| section.kind == BootSectionKind::Refine)
            .unwrap();
        section.record_size = u16::try_from(REFINE_TABLE_WIRE_SIZE).unwrap();
        section.count = 2;
        section.data = data;

        assert_eq!(parsed.refine_table_records().unwrap(), vec![first, second]);
        let section_ref = parsed
            .section(BootSectionKind::Refine)
            .expect("refine section");
        assert_eq!(
            decode_refine_table_section(section_ref).unwrap(),
            vec![first, second]
        );

        let mut missing = parsed.clone();
        missing
            .sections
            .retain(|section| section.kind != BootSectionKind::Refine);
        assert_eq!(
            missing.refine_table_records(),
            Err(DbBootError::MissingSection {
                section: BootSectionKind::Refine,
            })
        );

        let mut wrong_kind = section_ref.clone();
        wrong_kind.kind = BootSectionKind::Item;
        assert_eq!(
            decode_refine_table_section(&wrong_kind),
            Err(DbBootError::UnexpectedDecoderSection {
                expected: BootSectionKind::Refine,
                actual: BootSectionKind::Item,
            })
        );

        let mut wrong_size = section_ref.clone();
        wrong_size.record_size = 52;
        assert_eq!(
            decode_refine_table_section(&wrong_size),
            Err(DbBootError::InvalidFixedRecordSize {
                field: "refine_table_section",
                expected: REFINE_TABLE_WIRE_SIZE,
                actual: 52,
            })
        );

        let mut wrong_count = section_ref.clone();
        wrong_count.count = 3;
        assert_eq!(
            decode_refine_table_section(&wrong_count),
            Err(DbBootError::SectionLengthMismatch {
                section: BootSectionKind::Refine,
                expected: REFINE_TABLE_WIRE_SIZE * 3,
                actual: REFINE_TABLE_WIRE_SIZE * 2,
            })
        );
    }

    #[test]
    fn decodes_land_table_section_with_strict_metadata() {
        let profile = BootFeatureProfile::minimal();
        let mut parsed = parse_db_boot_payload(&payload(profile), profile).unwrap();
        let first = LandRecord {
            id: 101,
            map_index: 7,
            x: -10,
            y: 20,
            width: 30,
            height: 40,
            guild_id: 9,
            guild_level_limit: 3,
            price: 500,
        };
        let second = LandRecord {
            id: 102,
            map_index: i32::MIN,
            x: i32::MAX,
            y: 0,
            width: -1,
            height: 1,
            guild_id: u32::MAX,
            guild_level_limit: u8::MAX,
            price: 0,
        };
        let mut data = first.encode();
        data.extend_from_slice(&second.encode());
        let section = parsed
            .sections
            .iter_mut()
            .find(|section| section.kind == BootSectionKind::Land)
            .unwrap();
        section.record_size = u16::try_from(LAND_RECORD_WIRE_SIZE).unwrap();
        section.count = 2;
        section.data = data;
        assert_eq!(parsed.land_records().unwrap(), vec![first, second]);

        let section_ref = parsed.section(BootSectionKind::Land).unwrap();
        assert_eq!(
            decode_land_table_section(section_ref).unwrap(),
            vec![first, second]
        );
        let mut missing = parsed.clone();
        missing
            .sections
            .retain(|section| section.kind != BootSectionKind::Land);
        assert_eq!(
            missing.land_records(),
            Err(DbBootError::MissingSection {
                section: BootSectionKind::Land,
            })
        );
        let mut wrong_kind = section_ref.clone();
        wrong_kind.kind = BootSectionKind::Item;
        assert_eq!(
            decode_land_table_section(&wrong_kind),
            Err(DbBootError::UnexpectedDecoderSection {
                expected: BootSectionKind::Land,
                actual: BootSectionKind::Item,
            })
        );
        let mut wrong_size = section_ref.clone();
        wrong_size.record_size = 35;
        assert_eq!(
            decode_land_table_section(&wrong_size),
            Err(DbBootError::InvalidFixedRecordSize {
                field: "land_table_section",
                expected: LAND_RECORD_WIRE_SIZE,
                actual: 35,
            })
        );
        let mut wrong_count = section_ref.clone();
        wrong_count.count = 3;
        assert_eq!(
            decode_land_table_section(&wrong_count),
            Err(DbBootError::SectionLengthMismatch {
                section: BootSectionKind::Land,
                expected: LAND_RECORD_WIRE_SIZE * 3,
                actual: LAND_RECORD_WIRE_SIZE * 2,
            })
        );
    }

    fn object_proto_section_fixture() -> (ObjectProtoRecord, ObjectProtoRecord) {
        let first = ObjectProtoRecord {
            vnum: 10_001,
            price: 250_000,
            materials: [
                ObjectMaterial {
                    item_vnum: 101,
                    count: 2,
                },
                ObjectMaterial {
                    item_vnum: 102,
                    count: 3,
                },
                ObjectMaterial::default(),
                ObjectMaterial::default(),
                ObjectMaterial::default(),
            ],
            upgrade_vnum: 10_002,
            upgrade_limit_time: 86_400,
            life: 600,
            regions: [10, 20, 30, 40],
            npc_vnum: 20_001,
            npc_x: 0,
            npc_y: 340,
            group_vnum: 30_001,
            dependent_group_vnum: 30_002,
        };
        let second = ObjectProtoRecord {
            vnum: u32::MAX,
            price: 0,
            materials: [ObjectMaterial {
                item_vnum: u32::MAX,
                count: u32::MAX,
            }; OBJECT_MATERIAL_MAX_NUM],
            upgrade_vnum: 0,
            upgrade_limit_time: u32::MAX,
            life: i32::MIN,
            regions: [i32::MAX, i32::MIN, -1, 0],
            npc_vnum: u32::MAX,
            npc_x: i32::MIN,
            npc_y: i32::MAX,
            group_vnum: 0,
            dependent_group_vnum: u32::MAX,
        };
        (first, second)
    }

    #[test]
    fn decodes_object_proto_section_with_strict_metadata() {
        let profile = BootFeatureProfile::minimal();
        let mut parsed = parse_db_boot_payload(&payload(profile), profile).unwrap();
        let (first, second) = object_proto_section_fixture();
        let mut data = first.encode();
        data.extend_from_slice(&second.encode());
        let section = parsed
            .sections
            .iter_mut()
            .find(|section| section.kind == BootSectionKind::ObjectProto)
            .unwrap();
        section.record_size = u16::try_from(OBJECT_PROTO_RECORD_WIRE_SIZE).unwrap();
        section.count = 2;
        section.data = data;

        assert_eq!(parsed.object_proto_records().unwrap(), vec![first, second]);
        assert_eq!(
            parsed.object_proto_table_records().unwrap(),
            vec![first, second]
        );
        let section_ref = parsed.section(BootSectionKind::ObjectProto).unwrap();
        assert_eq!(
            decode_object_proto_section(section_ref).unwrap(),
            vec![first, second]
        );
        assert_eq!(
            decode_object_proto_table_section(section_ref).unwrap(),
            vec![first, second]
        );

        let mut missing = parsed.clone();
        missing
            .sections
            .retain(|section| section.kind != BootSectionKind::ObjectProto);
        assert_eq!(
            missing.object_proto_records(),
            Err(DbBootError::MissingSection {
                section: BootSectionKind::ObjectProto,
            })
        );

        let mut wrong_kind = section_ref.clone();
        wrong_kind.kind = BootSectionKind::Item;
        assert_eq!(
            decode_object_proto_section(&wrong_kind),
            Err(DbBootError::UnexpectedDecoderSection {
                expected: BootSectionKind::ObjectProto,
                actual: BootSectionKind::Item,
            })
        );

        let mut wrong_size = section_ref.clone();
        wrong_size.record_size = 95;
        assert_eq!(
            decode_object_proto_section(&wrong_size),
            Err(DbBootError::InvalidFixedRecordSize {
                field: "object_proto_section",
                expected: OBJECT_PROTO_RECORD_WIRE_SIZE,
                actual: 95,
            })
        );

        let mut wrong_count = section_ref.clone();
        wrong_count.count = 3;
        assert_eq!(
            decode_object_proto_section(&wrong_count),
            Err(DbBootError::SectionLengthMismatch {
                section: BootSectionKind::ObjectProto,
                expected: OBJECT_PROTO_RECORD_WIRE_SIZE * 3,
                actual: OBJECT_PROTO_RECORD_WIRE_SIZE * 2,
            })
        );
    }

    #[test]
    fn decodes_object_table_section_with_strict_metadata() {
        let profile = BootFeatureProfile::minimal();
        let mut parsed = parse_db_boot_payload(&payload(profile), profile).unwrap();
        let first = ObjectRecord {
            id: 101,
            land_id: 202,
            vnum: 303,
            map_index: -1,
            x: -10,
            y: 20,
            x_rot: 1.5,
            y_rot: -2.25,
            z_rot: f32::from_bits(0x8000_0000),
            life: i32::MIN,
        };
        let second = ObjectRecord {
            id: 102,
            land_id: 0,
            vnum: u32::MAX,
            map_index: i32::MAX,
            x: 0,
            y: -1,
            x_rot: f32::from_bits(0x7fc0_0001),
            y_rot: 0.0,
            z_rot: -0.0,
            life: i32::MAX,
        };
        let mut data = first.encode();
        data.extend_from_slice(&second.encode());
        let section = parsed
            .sections
            .iter_mut()
            .find(|section| section.kind == BootSectionKind::Object)
            .unwrap();
        section.record_size = u16::try_from(OBJECT_RECORD_WIRE_SIZE).unwrap();
        section.count = 2;
        section.data = data;

        assert_eq!(parsed.object_records().unwrap(), vec![first, second]);
        let section_ref = parsed.section(BootSectionKind::Object).unwrap();
        assert_eq!(
            decode_object_table_section(section_ref).unwrap(),
            vec![first, second]
        );

        let mut missing = parsed.clone();
        missing
            .sections
            .retain(|section| section.kind != BootSectionKind::Object);
        assert_eq!(
            missing.object_records(),
            Err(DbBootError::MissingSection {
                section: BootSectionKind::Object,
            })
        );

        let mut wrong_kind = section_ref.clone();
        wrong_kind.kind = BootSectionKind::Item;
        assert_eq!(
            decode_object_table_section(&wrong_kind),
            Err(DbBootError::UnexpectedDecoderSection {
                expected: BootSectionKind::Object,
                actual: BootSectionKind::Item,
            })
        );

        let mut wrong_size = section_ref.clone();
        wrong_size.record_size = 39;
        assert_eq!(
            decode_object_table_section(&wrong_size),
            Err(DbBootError::InvalidFixedRecordSize {
                field: "object_table_section",
                expected: OBJECT_RECORD_WIRE_SIZE,
                actual: 39,
            })
        );

        let mut wrong_count = section_ref.clone();
        wrong_count.count = 3;
        assert_eq!(
            decode_object_table_section(&wrong_count),
            Err(DbBootError::SectionLengthMismatch {
                section: BootSectionKind::Object,
                expected: OBJECT_RECORD_WIRE_SIZE * 3,
                actual: OBJECT_RECORD_WIRE_SIZE * 2,
            })
        );
    }

    #[test]
    fn decodes_event_table_only_for_the_enabled_profile() {
        let event_profile = BootFeatureProfile::new(false, true, false);
        let mut parsed = parse_db_boot_payload(&payload(event_profile), event_profile).unwrap();
        let mut event_type = [0_u8; EVENT_TYPE_BYTES];
        event_type[..5].copy_from_slice(b"event");
        let first = EventTableRecord {
            id: 17,
            event_type,
            start_time: -100,
            end_time: 200,
            value0: -3,
            value1: 4,
            completed: 1,
        };
        let second = EventTableRecord {
            id: 18,
            event_type: [b'x'; EVENT_TYPE_BYTES],
            start_time: 300,
            end_time: -400,
            value0: 5,
            value1: -6,
            completed: 0,
        };
        let mut data = first.encode();
        data.extend_from_slice(&second.encode());
        let section = parsed
            .sections
            .iter_mut()
            .find(|section| section.kind == BootSectionKind::Event)
            .unwrap();
        section.record_size = u16::try_from(EVENT_TABLE_WIRE_SIZE).unwrap();
        section.count = 2;
        section.data = data;

        assert_eq!(
            parsed.event_table_records(event_profile).unwrap(),
            vec![first, second]
        );
        let section_ref = parsed
            .section(BootSectionKind::Event)
            .expect("event section");
        assert_eq!(
            decode_event_table_section(section_ref, event_profile).unwrap(),
            vec![first, second]
        );
        assert_eq!(
            parsed.event_table_records(BootFeatureProfile::minimal()),
            Err(DbBootError::ProfileFeatureDisabled {
                feature: "event_manager",
            })
        );

        let missing = parse_db_boot_payload(
            &payload(BootFeatureProfile::minimal()),
            BootFeatureProfile::minimal(),
        )
        .unwrap();
        assert_eq!(
            missing.event_table_records(event_profile),
            Err(DbBootError::MissingSection {
                section: BootSectionKind::Event,
            })
        );

        let mut wrong_kind = section_ref.clone();
        wrong_kind.kind = BootSectionKind::Item;
        assert_eq!(
            decode_event_table_section(&wrong_kind, event_profile),
            Err(DbBootError::UnexpectedDecoderSection {
                expected: BootSectionKind::Event,
                actual: BootSectionKind::Item,
            })
        );
        let mut wrong_size = section_ref.clone();
        wrong_size.record_size = 84;
        assert_eq!(
            decode_event_table_section(&wrong_size, event_profile),
            Err(DbBootError::InvalidFixedRecordSize {
                field: "event_table_section",
                expected: EVENT_TABLE_WIRE_SIZE,
                actual: 84,
            })
        );
        let mut wrong_count = section_ref.clone();
        wrong_count.count = 3;
        assert_eq!(
            decode_event_table_section(&wrong_count, event_profile),
            Err(DbBootError::SectionLengthMismatch {
                section: BootSectionKind::Event,
                expected: EVENT_TABLE_WIRE_SIZE * 3,
                actual: EVENT_TABLE_WIRE_SIZE * 2,
            })
        );
    }

    #[test]
    fn decodes_the_source_fixed_banword_table_boundary() {
        let mut first = [0_u8; BANWORD_WIRE_SIZE];
        first[..4].copy_from_slice(b"spam");
        let second = [b'x'; BANWORD_WIRE_SIZE];
        let mut data = first.to_vec();
        data.extend_from_slice(&second);
        let section = BootSection {
            kind: BootSectionKind::Banword,
            record_size: u16::try_from(BANWORD_WIRE_SIZE).unwrap(),
            count: 2,
            data,
        };

        let words = decode_banword_section(&section).unwrap();
        assert_eq!(words.len(), 2);
        assert_eq!(words[0].as_str(), "spam");
        assert_eq!(words[1].as_str(), &"x".repeat(BANWORD_WIRE_SIZE));
        assert_eq!(words[0].bytes, first);
        assert_eq!(words[1].bytes, second);
    }

    #[test]
    fn rejects_wrong_banword_section_metadata() {
        let mut section = BootSection {
            kind: BootSectionKind::Item,
            record_size: 1,
            count: 0,
            data: Vec::new(),
        };
        assert_eq!(
            decode_banword_section(&section).err().unwrap(),
            DbBootError::UnexpectedDecoderSection {
                expected: BootSectionKind::Banword,
                actual: BootSectionKind::Item,
            }
        );

        section.kind = BootSectionKind::Banword;
        section.record_size = 24;
        assert_eq!(
            decode_banword_section(&section).err().unwrap(),
            DbBootError::InvalidFixedRecordSize {
                field: "banword_section",
                expected: BANWORD_WIRE_SIZE,
                actual: 24,
            }
        );
    }

    #[test]
    fn validates_section_records_before_typed_table_loading() {
        let section = BootSection {
            kind: BootSectionKind::Item,
            record_size: 3,
            count: 2,
            data: vec![1, 2, 3, 4, 5, 6],
        };
        let records = section.try_records().unwrap().collect::<Vec<_>>();
        assert_eq!(records, vec![&[1, 2, 3][..], &[4, 5, 6][..]]);

        let mut short = section.clone();
        short.data.pop();
        assert_eq!(
            short.try_records().err().unwrap(),
            DbBootError::SectionLengthMismatch {
                section: BootSectionKind::Item,
                expected: 6,
                actual: 5,
            }
        );

        let mut zero = section;
        zero.record_size = 0;
        assert_eq!(
            zero.try_records().err().unwrap(),
            DbBootError::InvalidRecordSize {
                section: BootSectionKind::Item,
                record_size: 0,
            }
        );
    }

    #[test]
    fn rejects_item_range_header_that_loses_the_legacy_double_record_layout() {
        let profile = BootFeatureProfile::minimal();
        let item_size_offset = 4 + 1 + profile.section_kinds().len() * 5 + 4;
        let item_count_offset = item_size_offset + 2;

        let mut wrong_size = payload(profile);
        wrong_size[item_size_offset..item_size_offset + 2].copy_from_slice(&11_u16.to_le_bytes());
        assert!(matches!(
            parse_db_boot_payload(&wrong_size, profile),
            Err(DbBootError::InvalidFixedRecordSize {
                field: "item_id_range",
                ..
            })
        ));

        let mut wrong_count = payload(profile);
        wrong_count[item_count_offset..item_count_offset + 2].copy_from_slice(&2_u16.to_le_bytes());
        assert!(matches!(
            parse_db_boot_payload(&wrong_count, profile),
            Err(DbBootError::InvalidRecordCount {
                field: "item_id_range",
                count: 2,
                expected: 1,
            })
        ));
    }

    #[test]
    fn rejects_fixed_monarch_count_and_all_short_prefixes() {
        let profile = BootFeatureProfile::minimal();
        let mut wrong_count = rich_payload();
        // Four-byte packet prefix + version + 11 zero-count sections, then
        // time, two item ranges, one host, and one admin record.
        let monarch_count_offset = 4 + 1 + 11 * 4 + 4 + 4 + 24 + 4 + 16 + 4 + 104 + 2;
        wrong_count[monarch_count_offset..monarch_count_offset + 2]
            .copy_from_slice(&0_u16.to_le_bytes());
        assert!(matches!(
            parse_db_boot_payload(&wrong_count, profile),
            Err(DbBootError::InvalidRecordCount {
                field: "monarch_info",
                ..
            })
        ));

        let full = rich_payload();
        for end in 0..full.len() {
            let mut prefix = full[..end].to_vec();
            if prefix.len() >= 4 {
                let prefix_len = u32::try_from(prefix.len()).unwrap();
                prefix[..4].copy_from_slice(&prefix_len.to_le_bytes());
            }
            assert!(parse_db_boot_payload(&prefix, profile).is_err());
        }
    }

    #[test]
    fn checks_size_limit_and_outer_length_independently() {
        let profile = BootFeatureProfile::minimal();
        let bytes = rich_payload();
        let parser = DbBootParser::new(profile).with_max_payload_size(bytes.len() - 1);
        assert!(matches!(
            parser.parse(&bytes),
            Err(DbBootError::PayloadTooLarge { .. })
        ));

        let mut frame = vec![0_u8; 9];
        frame[0] = HEADER_DG_BOOT;
        frame[1..5].copy_from_slice(&DB_BOOT_RESPONSE_HANDLE.to_le_bytes());
        frame[5..9].copy_from_slice(&u32::try_from(bytes.len()).unwrap().to_le_bytes());
        frame.extend_from_slice(&bytes);
        assert!(parse_db_boot_frame_bytes(&frame, profile).is_ok());

        let mut wrong_inner = frame.clone();
        wrong_inner[9] = wrong_inner[9].wrapping_add(1);
        assert!(matches!(
            parse_db_boot_frame_bytes(&wrong_inner, profile),
            Err(DbBootError::InvalidPacketSize { .. })
        ));

        let mut wrong_outer = frame;
        wrong_outer[5] = wrong_outer[5].wrapping_add(1);
        assert!(matches!(
            parse_db_boot_frame_bytes(&wrong_outer, profile),
            Err(DbBootError::InvalidPacketSize { .. })
        ));
    }

    #[test]
    fn fixed_tail_decoders_reject_short_and_overlong_values() {
        macro_rules! assert_exact {
            ($decode:path, $size:expr) => {{
                assert!(matches!(
                    $decode(&[0_u8; $size - 1]),
                    Err(DbBootError::InvalidFixedRecordSize { .. })
                ));
                assert!(matches!(
                    $decode(&[0_u8; $size + 1]),
                    Err(DbBootError::InvalidFixedRecordSize { .. })
                ));
            }};
        }
        assert_exact!(BootGmHost::decode, GM_HOST_WIRE_SIZE);
        assert_exact!(BootAdminInfo::decode, ADMIN_INFO_WIRE_SIZE);
        assert_exact!(BootMonarchInfo::decode, MONARCH_INFO_WIRE_SIZE);
        assert_exact!(BootMonarchCandidacy::decode, MONARCH_CANDIDACY_WIRE_SIZE);
    }

    #[test]
    fn checked_products_reject_usize_overflow() {
        assert_eq!(
            checked_product(usize::MAX, 2),
            Err(DbBootError::SizeOverflow)
        );
        assert_eq!(
            checked_product(2, usize::MAX),
            Err(DbBootError::SizeOverflow)
        );
    }

    // --- the request address ----------------------------------------------

    /// A NUL-terminated `szIP` decodes to its text.
    #[test]
    fn a_terminated_request_address_decodes() {
        let mut ip = [0_u8; 16];
        ip[..8].copy_from_slice(b"10.0.0.1");
        let request = DbBootRequest::new([1, 2], ip);
        assert_eq!(request.ip_text().as_deref(), Some("10.0.0.1"));
    }

    /// An unterminated `szIP` is not an address.
    ///
    /// The field reaches an unescaped `'%s'`, so sixteen bytes with no NUL
    /// would be read as a sixteen-character address. Reporting that as "no
    /// address" keeps the malformed field out of the statement and lets the
    /// composer use the `ALL` literal.
    #[test]
    fn an_unterminated_request_address_is_none() {
        let request = DbBootRequest::new([1, 2], [b'x'; 16]);
        assert_eq!(request.ip_text(), None);
    }

    /// An empty or non-ASCII `szIP` is not an address either.
    ///
    /// An empty field is a legitimate "no address" case, and a non-ASCII field
    /// cannot be interpolated without a lossy conversion, so both must fail
    /// closed rather than produce a mangled literal.
    #[test]
    fn an_empty_or_non_ascii_request_address_is_none() {
        assert_eq!(DbBootRequest::new([1, 2], [0_u8; 16]).ip_text(), None);
        let mut ip = [0_u8; 16];
        ip[..4].copy_from_slice(&[0xff, 0xfe, 0xfd, 0x41]);
        assert_eq!(DbBootRequest::new([1, 2], ip).ip_text(), None);
    }

    /// Only the bytes before the first NUL are used.
    ///
    /// Legacy stops reading at the first NUL, so trailing bytes after it are
    /// padding and must not appear in the decoded text.
    #[test]
    fn bytes_after_the_first_nul_are_ignored() {
        let mut ip = [b'A'; 16];
        ip[3] = 0;
        let request = DbBootRequest::new([1, 2], ip);
        assert_eq!(request.ip_text().as_deref(), Some("AAA"));
    }
}
