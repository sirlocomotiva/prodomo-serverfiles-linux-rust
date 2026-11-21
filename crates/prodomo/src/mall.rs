//! The in-game mall line: `do_in_game_mall` (`G/cmd_general.cpp:3382-3456`).
//!
//! Any character may run it. Past the Europe gate (which this build's `europe` Locale
//! passes: `LC_EUROPE` under `__MULTI_LANGUAGE_SYSTEM__`, `G/locale_service.cpp:1515`), the
//! caller hears one `CHAT_TYPE_COMMAND` line naming the web shop:
//! `mall https://{mall_url}/ishop?pid={pid}&c={cc}&sid={sid}&sas={sas}`. The country code
//! is the local type's two letters, `eu` here; the signature is the lowercase hex MD5 of
//! `pid`, `aid` and the fixed key `GF9001`, printed as `%u%u%s` prints them. Nothing is
//! stored, and the world is not involved: legacy reads only the descriptor's IDs and the
//! two config keys.
//!
//! [`crate::mall::mall_line`] builds the line's bytes. The descriptor sends them.

use md5::{Digest, Md5};

/// The mall signature's fixed key (`cmd_general.cpp:3430`).
const SAS_KEY: &str = "GF9001";

/// The country code of the `europe` Locale (`LC_EUROPE`, `cmd_general.cpp:3441`).
const COUNTRY_CODE: &str = "eu";

/// The lowercase hex MD5 of `"{pid}{aid}GF9001"`, as `%u%u%s` prints the two IDs.
#[must_use]
pub fn mall_signature(pid: u32, aid: u32) -> String {
    let mut hash = Md5::new();
    hash.update(format!("{pid}{aid}{SAS_KEY}").as_bytes());
    format!("{:x}", hash.finalize())
}

/// The `mall https://...` line for the character `pid` of account `aid`.
///
/// `mall_url` is `[game] mall_url` and `server_id` `[game] server_id`, legacy's `mall_url`
/// and `server_id` (`G/config.cpp:1337-1344`).
#[must_use]
pub fn mall_line(pid: u32, aid: u32, mall_url: &str, server_id: i32) -> Vec<u8> {
    format!(
        "mall https://{mall_url}/ishop?pid={pid}&c={COUNTRY_CODE}&sid={server_id}&sas={}",
        mall_signature(pid, aid)
    )
    .into_bytes()
}

#[cfg(test)]
mod tests {
    use super::{mall_line, mall_signature};

    /// The signature of a known pair: MD5 of `"12GF9001"`, pinned against the system's
    /// `md5sum`, not against the crate itself.
    #[test]
    fn the_signature_is_the_md5_of_the_ids_and_the_key() {
        assert_eq!(mall_signature(1, 2), "2728eddf0cf131f8517d6614862eea2f");
    }

    /// The line names the shop, the IDs, the country and the signature in legacy's order.
    #[test]
    fn the_line_names_the_shop_and_signs_it() {
        assert_eq!(
            mall_line(1, 2, "www.example.com", 7),
            b"mall https://www.example.com/ishop?pid=1&c=eu&sid=7&sas=2728eddf0cf131f8517d6614862eea2f"
                .to_vec()
        );
    }
}
