//! The validating JSON reader the Game data JSON files share.
//!
//! Legacy reads these files with rapidjson, so the number rules below are its
//! `IsUint`/`GetUint` rules: a value counts as a number only when it is ASCII
//! digits with no sign, fraction, or exponent and fits in a `u32`. Strings
//! decode rapidjson's escapes, including `\u` pairs, and a raw control byte
//! or a bad escape refuses the file, as rapidjson's validating reader does.
//!
//! Trailing bytes past the root value and duplicate object keys refuse the
//! file; legacy ignores trailing bytes and debug asserts on duplicates.

use std::error::Error;
use std::fmt;

/// Why the JSON bytes would not parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonError {
    /// The input is not valid UTF-8.
    NotUtf8,
    /// A value ends where the shape needs more.
    Truncated,
    /// A value opens with a byte no value opens with.
    UnexpectedByte(u8),
    /// An object member is not a string, or misses its colon.
    BadMember,
    /// A string never closes, or holds a bad escape or a raw control byte.
    BadString,
    /// A `\u` escape is malformed or a lone surrogate.
    BadUnicode,
    /// A duplicate object key.
    DuplicateKey,
    /// Bytes past trailing whitespace follow the root value.
    TrailingBytes,
}

impl fmt::Display for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotUtf8 => write!(f, "the file is not UTF-8"),
            Self::Truncated => write!(f, "a value ends early"),
            Self::UnexpectedByte(byte) => write!(f, "unexpected byte 0x{byte:02x}"),
            Self::BadMember => write!(f, "an object member is not a string with a colon"),
            Self::BadString => write!(f, "a string never closes or holds a bad escape"),
            Self::BadUnicode => write!(f, "a \\u escape is malformed"),
            Self::DuplicateKey => write!(f, "an object repeats a key"),
            Self::TrailingBytes => write!(f, "bytes follow the root value"),
        }
    }
}

impl Error for JsonError {}

/// A parsed JSON value: only what the battle-pass shape needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Json {
    /// An object, in file order.
    Object(Vec<(Vec<u8>, Json)>),
    /// An array, in file order.
    Array(Vec<Json>),
    /// A string, UTF-8 with escapes decoded.
    Text(Vec<u8>),
    /// A bare number: its sign, its magnitude (saturating), and whether it
    /// carried a fraction or an exponent.
    Number {
        /// A leading `-` was present.
        negative: bool,
        /// The digit run's value, saturating at `u64::MAX`.
        magnitude: u64,
        /// A `.` fraction or an `e`/`E` exponent was present.
        fuzzy: bool,
    },
    /// `true`, `false`, or `null`: parsed and otherwise unused.
    Atom,
}

impl Json {
    /// rapidjson's `IsUint`/`GetUint`: an unsigned 32-bit integer spelled as
    /// plain digits.
    pub(crate) fn as_uint(&self) -> Option<u32> {
        match self {
            Self::Number {
                negative,
                magnitude,
                fuzzy,
            } => {
                if *negative || *fuzzy || *magnitude > u64::from(u32::MAX) {
                    return None;
                }
                u32::try_from(*magnitude).ok()
            }
            Self::Object(_) | Self::Array(_) | Self::Text(_) | Self::Atom => None,
        }
    }

    /// Look an object member up by name.
    pub(crate) fn member(&self, name: &[u8]) -> Option<&Json> {
        match self {
            Self::Object(members) => members
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value),
            Self::Array(_) | Self::Text(_) | Self::Number { .. } | Self::Atom => None,
        }
    }
}

/// A byte cursor over the file.
struct Cursor<'a> {
    /// The file's bytes.
    bytes: &'a [u8],
    /// The next unread offset.
    pos: usize,
}

impl Cursor<'_> {
    /// Peek the next byte.
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    /// Take the next byte.
    fn take(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.pos += 1;
        Some(byte)
    }

    /// Skip JSON whitespace.
    fn space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    /// Take an exact byte after whitespace.
    fn punct(&mut self, want: u8) -> Result<(), JsonError> {
        self.space();
        match self.take() {
            Some(got) if got == want => Ok(()),
            Some(got) => Err(JsonError::UnexpectedByte(got)),
            None => Err(JsonError::Truncated),
        }
    }

    /// Parse one value after whitespace.
    fn value(&mut self) -> Result<Json, JsonError> {
        self.space();
        match self.peek() {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => self.string().map(Json::Text),
            Some(byte) if byte == b'-' || byte.is_ascii_digit() => self.number(),
            Some(b't') => self.word("true"),
            Some(b'f') => self.word("false"),
            Some(b'n') => self.word("null"),
            Some(byte) => Err(JsonError::UnexpectedByte(byte)),
            None => Err(JsonError::Truncated),
        }
    }

    /// Parse an object.
    fn object(&mut self) -> Result<Json, JsonError> {
        self.pos += 1;
        let mut members = Vec::new();
        self.space();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Json::Object(members));
        }
        loop {
            self.space();
            if self.peek() != Some(b'"') {
                return Err(JsonError::BadMember);
            }
            let key = self.string()?;
            if members
                .iter()
                .any(|(seen, _): &(Vec<u8>, Json)| *seen == key)
            {
                return Err(JsonError::DuplicateKey);
            }
            self.punct(b':')?;
            let value = self.value()?;
            members.push((key, value));
            self.space();
            match self.take() {
                Some(b',') => {}
                Some(b'}') => return Ok(Json::Object(members)),
                Some(byte) => return Err(JsonError::UnexpectedByte(byte)),
                None => return Err(JsonError::Truncated),
            }
        }
    }

    /// Parse an array.
    fn array(&mut self) -> Result<Json, JsonError> {
        self.pos += 1;
        let mut items = Vec::new();
        self.space();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Json::Array(items));
        }
        loop {
            items.push(self.value()?);
            self.space();
            match self.take() {
                Some(b',') => {}
                Some(b']') => return Ok(Json::Array(items)),
                Some(byte) => return Err(JsonError::UnexpectedByte(byte)),
                None => return Err(JsonError::Truncated),
            }
        }
    }

    /// Parse a `true`/`false`/`null` word.
    fn word(&mut self, want: &str) -> Result<Json, JsonError> {
        for byte in want.bytes() {
            match self.take() {
                Some(got) if got == byte => {}
                Some(got) => return Err(JsonError::UnexpectedByte(got)),
                None => return Err(JsonError::Truncated),
            }
        }
        Ok(Json::Atom)
    }

    /// Parse a number: an optional `-`, digits, and an optional fraction or
    /// exponent whose presence only marks the value fuzzy.
    fn number(&mut self) -> Result<Json, JsonError> {
        let negative = self.peek() == Some(b'-');
        if negative {
            self.pos += 1;
        }
        let mut magnitude: u64 = 0;
        let mut digits = 0_u32;
        while let Some(byte) = self.peek() {
            if !byte.is_ascii_digit() {
                break;
            }
            digits += 1;
            magnitude = magnitude
                .saturating_mul(10)
                .saturating_add(u64::from(byte - b'0'));
            self.pos += 1;
        }
        if digits == 0 {
            return match self.peek() {
                Some(byte) => Err(JsonError::UnexpectedByte(byte)),
                None => Err(JsonError::Truncated),
            };
        }
        let mut fuzzy = false;
        if self.peek() == Some(b'.') {
            fuzzy = true;
            self.pos += 1;
            while matches!(self.peek(), Some(byte) if byte.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            fuzzy = true;
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            while matches!(self.peek(), Some(byte) if byte.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        Ok(Json::Number {
            negative,
            magnitude,
            fuzzy,
        })
    }

    /// Parse a string with rapidjson's escapes decoded to UTF-8.
    fn string(&mut self) -> Result<Vec<u8>, JsonError> {
        self.pos += 1;
        let mut out = Vec::new();
        loop {
            let byte = self.take().ok_or(JsonError::Truncated)?;
            match byte {
                b'"' => return Ok(out),
                b'\\' => self.escape(&mut out)?,
                0x00..=0x1F => return Err(JsonError::BadString),
                _ => out.push(byte),
            }
        }
    }

    /// Decode one escape into `out`.
    fn escape(&mut self, out: &mut Vec<u8>) -> Result<(), JsonError> {
        match self.take().ok_or(JsonError::Truncated)? {
            b'"' => out.push(b'"'),
            b'\\' => out.push(b'\\'),
            b'/' => out.push(b'/'),
            b'b' => out.push(0x08),
            b'f' => out.push(0x0C),
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'u' => {
                let unit = self.hex4()?;
                let scalar = if (0xD800..0xDC00).contains(&unit) {
                    if self.take() != Some(b'\\') || self.take() != Some(b'u') {
                        return Err(JsonError::BadUnicode);
                    }
                    let low = self.hex4()?;
                    if !(0xDC00..0xE000).contains(&low) {
                        return Err(JsonError::BadUnicode);
                    }
                    0x1_0000 + (u32::from(unit - 0xD800) << 10) + u32::from(low - 0xDC00)
                } else if (0xDC00..0xE000).contains(&unit) {
                    return Err(JsonError::BadUnicode);
                } else {
                    u32::from(unit)
                };
                let symbol = char::from_u32(scalar).ok_or(JsonError::BadUnicode)?;
                let mut encoded = [0_u8; 4];
                out.extend_from_slice(symbol.encode_utf8(&mut encoded).as_bytes());
            }
            _ => return Err(JsonError::BadString),
        }
        Ok(())
    }

    /// Parse four hex digits.
    fn hex4(&mut self) -> Result<u16, JsonError> {
        let mut unit: u16 = 0;
        for _ in 0..4 {
            let byte = self.take().ok_or(JsonError::Truncated)?;
            let digit = match byte {
                b'0'..=b'9' => u16::from(byte - b'0'),
                b'a'..=b'f' => u16::from(byte - b'a') + 10,
                b'A'..=b'F' => u16::from(byte - b'A') + 10,
                _ => return Err(JsonError::BadUnicode),
            };
            unit = unit.saturating_mul(16).saturating_add(digit);
        }
        Ok(unit)
    }
}

/// Parse the file's bytes into the JSON root value.
pub(crate) fn parse(data: &[u8]) -> Result<Json, JsonError> {
    let text = std::str::from_utf8(data).map_err(|_| JsonError::NotUtf8)?;
    let bytes = text
        .strip_prefix('\u{FEFF}')
        .map_or(data, |_| data.get(3..).unwrap_or_default());
    let mut cursor = Cursor { bytes, pos: 0 };
    let root = cursor.value()?;
    cursor.space();
    if cursor.peek().is_some() {
        return Err(JsonError::TrailingBytes);
    }
    Ok(root)
}
