//! The refused addresses (`sys.net.ip_ban`): the auth listener's `BANIP` ranges
//! (`G/ip_ban.cpp`) and the Channel login's blocked-country ranges with their login
//! exceptions (`G/block_country.cpp`, fed by the `iptocountry` and `block_exception`
//! tables through the retired DB peer).
//!
//! Legacy loads both lists once at boot — the `BANIP` file when the auth process parses
//! its config (`G/config.cpp:808`), the country ranges when the DB process loads
//! (`D/BlockCountry.cpp:41-100`) — so the Rewrite reads them from the `[access]` table
//! once at startup. An empty table refuses nothing, as the owner's deployment has no
//! `BANIP` file and no rows in either table.
//!
//! The match semantics are legacy's. A `BANIP` line holds `start` and an optional `end`;
//! each octet's mask is `255 - (start ^ end)`, and a candidate matches when its first
//! octet is the range's (the `mapBanIP` hash, `G/ip_ban.cpp:195-214`) and
//! `(candidate & mask) == (start & mask)`. The match is bitwise, not a span: a
//! `10.0.0.0`-`10.0.0.5` line masks `250` in the last octet, so it bans `.0`, `.1`,
//! `.4` and `.5` but not `.2`, `.3` or `.6`. The hash narrows cross-octet lines: a
//! `10.0.0.0`-`11.0.0.0` line never sees a candidate starting with `11`.
//!
//! A country range holds two dotted addresses; a candidate matches when it falls
//! between them, compared first-octet-high (`htonl`, `G/block_country.cpp:125`).
//! A login in the exception set skips the country check (`G/input_login.cpp:164`);
//! the match is against the lowercased login, as legacy compares after
//! `trim_and_lower`. Legacy fails a candidate its `inet_aton` cannot parse closed;
//! the Rewrite's peers are always well-formed socket addresses, so only the list
//! match can refuse one. An IPv6 peer matches by its mapped IPv4 address when it has
//! one, and matches nothing otherwise — legacy never saw IPv6, so there is no
//! stricter reading to keep.

use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr};

/// One `BANIP` line: the start address and the octet mask, both first-octet-low, as
/// legacy's `IP` constructor builds them (`G/ip_ban.cpp:48-76`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BanRange {
    /// The start address as `u32::from_le_bytes` of its octets.
    start: u32,
    /// The mask as `u32::from_le_bytes` of its octets.
    mask: u32,
}

impl BanRange {
    /// The range of the `BANIP` line `start` to `end`, or of the single address `start`
    /// when `end` is `None`, as legacy builds it when the line has no tab
    /// (`G/ip_ban.cpp:152-167`).
    #[must_use]
    pub fn of(start: Ipv4Addr, end: Option<Ipv4Addr>) -> Self {
        let end = end.unwrap_or(start);
        let (a, b) = (start.octets(), end.octets());
        let mask = [
            255 - (a[0] ^ b[0]),
            255 - (a[1] ^ b[1]),
            255 - (a[2] ^ b[2]),
            255 - (a[3] ^ b[3]),
        ];
        Self {
            start: u32::from_le_bytes(a),
            mask: u32::from_le_bytes(mask),
        }
    }

    /// Whether `candidate` falls in this range: its first octet is the range's (the
    /// `mapBanIP` hash) and the masked addresses agree (`IP::IsChildOf` for a single
    /// address, whose own mask is all ones, `G/ip_ban.cpp:88-97`).
    #[must_use]
    pub fn matches(&self, candidate: Ipv4Addr) -> bool {
        if candidate.octets()[0] != self.start.to_le_bytes()[0] {
            return false;
        }
        let candidate = u32::from_le_bytes(candidate.octets());
        candidate & self.mask == self.start & self.mask
    }
}

/// One blocked-country row: the two ends as first-octet-high numbers, as the game
/// compares them (`ip_number = htonl(st_addr.s_addr)`, `G/block_country.cpp:125`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CountryRange {
    /// The low end as `u32::from_be_bytes` of its octets.
    from: u32,
    /// The high end as `u32::from_be_bytes` of its octets.
    to: u32,
}

impl CountryRange {
    /// The row spanning `from` to `to`.
    #[must_use]
    pub const fn of(from: Ipv4Addr, to: Ipv4Addr) -> Self {
        Self {
            from: u32::from_be_bytes(from.octets()),
            to: u32::from_be_bytes(to.octets()),
        }
    }

    /// Whether `candidate` falls between the ends, inclusive.
    #[must_use]
    pub fn matches(&self, candidate: Ipv4Addr) -> bool {
        let candidate = u32::from_be_bytes(candidate.octets());
        self.from <= candidate && candidate <= self.to
    }
}

/// The `[access]` table's refused addresses: the auth listener's `BANIP` ranges, the
/// Channel login's blocked-country ranges, and the logins excepted from the country
/// check.
#[derive(Debug, Clone, Default)]
pub struct AccessRules {
    /// The `ban_ip` ranges, in configured order.
    ban: Vec<BanRange>,
    /// The `block_country_ip` ranges, in configured order.
    country: Vec<CountryRange>,
    /// The lowercased `block_exception` logins.
    exceptions: HashSet<String>,
}

impl AccessRules {
    /// The rules of the three configured lists.
    #[must_use]
    pub fn new(
        ban: Vec<BanRange>,
        country: Vec<CountryRange>,
        exceptions: HashSet<String>,
    ) -> Self {
        Self {
            ban,
            country,
            exceptions,
        }
    }

    /// Whether the auth listener refuses `peer` (`IsBanIP`, `G/desc_manager.cpp:164`).
    #[must_use]
    pub fn is_ban_ip(&self, peer: IpAddr) -> bool {
        let Some(candidate) = ipv4_of(peer) else {
            return false;
        };
        self.ban.iter().any(|range| range.matches(candidate))
    }

    /// Whether the Channel login refuses `peer` for `login`: a blocked-country range
    /// holds it and no exception names the login (`G/input_login.cpp:164-170`). `login`
    /// is the lowercased login, as legacy compares after `trim_and_lower`.
    #[must_use]
    pub fn is_blocked_country(&self, peer: IpAddr, login: &str) -> bool {
        if self.exceptions.contains(login) {
            return false;
        }
        let Some(candidate) = ipv4_of(peer) else {
            return false;
        };
        self.country.iter().any(|range| range.matches(candidate))
    }
}

/// The IPv4 address `peer` matches by: itself, or the mapped address of an
/// IPv6 peer that carries one. `None` matches no range.
fn ipv4_of(peer: IpAddr) -> Option<Ipv4Addr> {
    match peer {
        IpAddr::V4(candidate) => Some(candidate),
        IpAddr::V6(candidate) => candidate.to_ipv4_mapped(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A dotted address, parsed once.
    fn ip(dotted: &str) -> Ipv4Addr {
        dotted.parse().expect("a dotted address")
    }

    /// The single line bans its address and nothing beside it.
    #[test]
    fn a_single_line_bans_its_address_only() {
        let range = BanRange::of(ip("10.1.2.3"), None);
        assert!(range.matches(ip("10.1.2.3")));
        assert!(!range.matches(ip("10.1.2.4")));
        assert!(!range.matches(ip("11.1.2.3")));
    }

    /// The mask decides, not the end: `10.0.0.0`-`10.0.0.5` masks `250` (`~5`), so
    /// `.0`, `.1`, `.4` and `.5` match while `.2`, `.3` and `.6` do not.
    #[test]
    fn the_mask_decides_not_the_end() {
        let range = BanRange::of(ip("10.0.0.0"), Some(ip("10.0.0.5")));
        for last in [0, 1, 4, 5] {
            assert!(
                range.matches(Ipv4Addr::new(10, 0, 0, last)),
                ".{last} matches"
            );
        }
        for last in [2, 3, 6, 7, 255] {
            assert!(
                !range.matches(Ipv4Addr::new(10, 0, 0, last)),
                ".{last} does not match"
            );
        }
        assert!(!range.matches(ip("10.0.1.0")));
    }

    /// The mask xors, not ors: `10.0.0.4`-`10.0.0.5` masks `254`, so `.4` and `.5`
    /// match, while `.0`, `.1` and `.6` do not — an or-mask of `250` would take `.0`
    /// and `.1`, which `&` to `0` as the start does.
    #[test]
    fn the_mask_xors_not_ors() {
        let range = BanRange::of(ip("10.0.0.4"), Some(ip("10.0.0.5")));
        assert!(range.matches(ip("10.0.0.4")));
        assert!(range.matches(ip("10.0.0.5")));
        assert!(!range.matches(ip("10.0.0.0")));
        assert!(!range.matches(ip("10.0.0.1")));
        assert!(!range.matches(ip("10.0.0.6")));
    }

    /// The first-octet hash narrows a cross-octet line to its start address:
    /// `10.0.0.0`-`11.0.0.0` masks `254` in the first octet and `255` elsewhere, so
    /// `10.0.0.0` matches, while `11.0.0.0` never reaches the mask — the hash only
    /// consults the start's octet.
    #[test]
    fn the_first_octet_hash_narrows_a_cross_octet_line() {
        let range = BanRange::of(ip("10.0.0.0"), Some(ip("11.0.0.0")));
        assert!(range.matches(ip("10.0.0.0")));
        assert!(!range.matches(ip("11.0.0.0")));
        assert!(!range.matches(ip("10.0.0.1")));
    }

    /// The country range holds its ends and what falls between, first-octet-high: the
    /// low octet decides before the high one.
    #[test]
    fn the_country_range_holds_its_span() {
        let range = CountryRange::of(ip("10.0.0.5"), ip("10.0.1.0"));
        assert!(range.matches(ip("10.0.0.5")));
        assert!(range.matches(ip("10.0.0.200")));
        assert!(range.matches(ip("10.0.1.0")));
        assert!(!range.matches(ip("10.0.0.4")));
        assert!(!range.matches(ip("10.0.1.1")));
        assert!(!range.matches(ip("9.255.255.255")));
    }

    /// An excepted login passes a blocked range; any other login from the same peer
    /// is refused, and a peer outside every range passes.
    #[test]
    fn the_exception_skips_the_country_check() {
        let rules = AccessRules::new(
            Vec::new(),
            vec![CountryRange::of(ip("127.0.0.0"), ip("127.0.0.255"))],
            HashSet::from(["alice".to_owned()]),
        );
        let peer = IpAddr::V4(ip("127.0.0.1"));
        assert!(!rules.is_blocked_country(peer, "alice"));
        assert!(rules.is_blocked_country(peer, "bob"));
        assert!(!rules.is_blocked_country(IpAddr::V4(ip("10.0.0.1")), "bob"));
    }

    /// A mapped IPv6 peer matches by its inner address; any other IPv6 peer matches
    /// nothing.
    #[test]
    fn a_mapped_peer_matches_by_its_inner_address() {
        let rules = AccessRules::new(
            vec![BanRange::of(ip("127.0.0.1"), None)],
            Vec::new(),
            HashSet::new(),
        );
        assert!(rules.is_ban_ip(IpAddr::V4(ip("127.0.0.1"))));
        assert!(rules.is_ban_ip("::ffff:127.0.0.1".parse().unwrap()));
        assert!(!rules.is_ban_ip("::1".parse().unwrap()));
    }

    /// Empty lists refuse nothing.
    #[test]
    fn empty_lists_refuse_nothing() {
        let rules = AccessRules::default();
        let peer = IpAddr::V4(ip("127.0.0.1"));
        assert!(!rules.is_ban_ip(peer));
        assert!(!rules.is_blocked_country(peer, "alice"));
    }
}
