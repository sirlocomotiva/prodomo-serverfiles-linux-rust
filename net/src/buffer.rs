//! Generic length-prefixed buffer management for network I/O.
//!
//! This module can be used by private Rust protocols that adopt its framing.
//! It is **not** a decoder for the legacy Metin2 client stream. Legacy client
//! packets begin with a one-byte header whose header-specific size comes from
//! `CPacketInfoCG`; they do not have the two-byte prefix below. The legacy DB
//! peer path uses a different nine-byte header.
//!
//! # Packet Format
//!
//! This module's generic framing is:
//! - 2 bytes: total packet size (little-endian, includes header)
//! - 1 byte: packet header (type identifier)
//! - N bytes: packet payload
//!
//! # Examples
//!
//! ```rust
//! use net::buffer::ReadBuffer;
//!
//! let mut buf = ReadBuffer::new();
//! // Simulate receiving data from TCP
//! buf.put(&[0x05, 0x00, 0x01, 0xAA, 0xBB, 0xCC]);
//! // Extract complete packets
//! while let Some(packet) = buf.try_extract_packet() {
//!     println!("Got packet: header={}, payload={:?}", packet.header, packet.payload);
//! }
//! ```

use bytes::{Buf, BufMut, BytesMut};

/// Minimum packet size: 2 bytes (size) + 1 byte (header) = 3 bytes
pub const MIN_PACKET_SIZE: usize = 3;

/// Maximum encoded packet data size: one header plus payload.
///
/// The two-byte length field cannot represent a larger value.
pub const MAX_PACKET_SIZE: usize = u16::MAX as usize;

/// Default initial capacity for buffers
const DEFAULT_CAPACITY: usize = 8192;

/// A complete packet extracted from the buffer
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packet {
    /// Packet header byte (type identifier)
    pub header: u8,
    /// Packet payload (everything after the header)
    pub payload: Vec<u8>,
}

impl Packet {
    /// Create a new packet with header and payload
    pub fn new(header: u8, payload: Vec<u8>) -> Self {
        Self { header, payload }
    }

    /// Total packet size including header (but not the 2-byte size prefix)
    pub fn data_size(&self) -> usize {
        1 + self.payload.len()
    }

    /// Total wire size including the 2-byte size prefix
    pub fn wire_size(&self) -> usize {
        2 + self.data_size()
    }

    /// Serialize this framing as size + header + payload.
    ///
    /// Returns an error instead of truncating when the encoded length does not
    /// fit in the two-byte size field.
    ///
    /// # Errors
    ///
    /// Returns an error when the packet data cannot be represented by the
    /// generic two-byte length field.
    pub fn try_to_bytes(&self) -> Result<Vec<u8>, String> {
        let total_size = self.data_size();
        if total_size > MAX_PACKET_SIZE {
            return Err(format!(
                "packet data size {total_size} exceeds {MAX_PACKET_SIZE}"
            ));
        }
        let total_size = u16::try_from(total_size)
            .map_err(|_| "packet data size does not fit in u16".to_string())?;
        let mut buf = Vec::with_capacity(self.wire_size());
        buf.extend_from_slice(&total_size.to_le_bytes());
        buf.push(self.header);
        buf.extend_from_slice(&self.payload);
        Ok(buf)
    }

    /// Serialize this framing, panicking if the length cannot be encoded.
    ///
    /// Use [`Packet::try_to_bytes`] when input length is not already trusted.
    ///
    /// # Panics
    ///
    /// Panics when the packet data cannot be represented by the generic
    /// two-byte length field.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.try_to_bytes()
            .expect("packet data size must fit in the u16 length field")
    }
}

/// Read buffer for accumulating partial TCP reads and extracting complete packets.
///
/// This buffer handles the common TCP scenario where:
/// - A single `read()` may return partial packets
/// - A single `read()` may return multiple complete packets
/// - Packet boundaries don't align with TCP segment boundaries
///
/// The buffer automatically compacts itself when data is consumed to prevent
/// unbounded memory growth.
#[derive(Debug)]
pub struct ReadBuffer {
    /// Internal buffer using `BytesMut` for efficient accumulation
    buf: BytesMut,
}

impl ReadBuffer {
    /// Create a new empty read buffer
    pub fn new() -> Self {
        Self {
            buf: BytesMut::with_capacity(DEFAULT_CAPACITY),
        }
    }

    /// Create a new read buffer with specified initial capacity
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            buf: BytesMut::with_capacity(capacity),
        }
    }

    /// Add data from a TCP read into the buffer
    ///
    /// This is called after each successful `TcpStream::read()` to accumulate
    /// incoming data until complete packets can be extracted.
    pub fn put(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    /// Get the number of bytes currently in the buffer
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// Check if the buffer is empty
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Try to extract a complete packet from the buffer
    ///
    /// Returns `Some(Packet)` if a complete packet is available, `None` if
    /// more data is needed. The packet data is consumed from the buffer.
    ///
    /// # Packet Format
    ///
    /// - Bytes 0-1: Total data size (u16 LE), includes header byte but not size prefix
    /// - Byte 2: Packet header (type identifier)
    /// - Bytes 3..N: Packet payload
    pub fn try_extract_packet(&mut self) -> Option<Packet> {
        // Need at least 2 bytes for the size prefix
        if self.buf.len() < 2 {
            return None;
        }

        // Read the packet size (little-endian u16)
        let size = u16::from_le_bytes([self.buf[0], self.buf[1]]) as usize;

        // Validate packet size
        if size < 1 {
            // Invalid packet: size must be at least 1 (the header byte)
            tracing::warn!("Invalid packet size: {} (too small)", size);
            self.buf.advance(2);
            return None;
        }

        if size > MAX_PACKET_SIZE {
            // Invalid packet: size too large
            tracing::warn!("Invalid packet size: {} (exceeds max)", size);
            self.buf.advance(2);
            return None;
        }

        // Check if we have the complete packet (2 bytes size + size bytes data)
        let total_wire_size = 2 + size;
        if self.buf.len() < total_wire_size {
            return None;
        }

        // Consume the size prefix
        self.buf.advance(2);

        // Extract header byte
        let header = self.buf[0];
        self.buf.advance(1);

        // Extract payload (size - 1 bytes, since size includes header)
        let payload_size = size - 1;
        let payload = if payload_size > 0 {
            self.buf[..payload_size].to_vec()
        } else {
            Vec::new()
        };
        self.buf.advance(payload_size);

        Some(Packet::new(header, payload))
    }

    /// Try to extract a packet with a custom size resolver
    ///
    /// This variant allows the caller to provide a function that determines
    /// packet size from the header byte, for protocols where packet size
    /// is not embedded in the packet itself.
    ///
    /// The resolver function takes the header byte and returns `Some(size)`
    /// if the size is known, or `None` if the header is invalid.
    pub fn try_extract_packet_with_resolver<F>(&mut self, resolver: F) -> Option<Packet>
    where
        F: Fn(u8) -> Option<usize>,
    {
        // Need at least 1 byte for the header
        if self.buf.is_empty() {
            return None;
        }

        let header = self.buf[0];

        // Get packet size from resolver
        let Some(size) = resolver(header) else {
            tracing::warn!("Unknown packet header: 0x{:02X}", header);
            self.buf.advance(1);
            return None;
        };

        // Validate size
        if size == 0 {
            tracing::warn!("Invalid packet size for header 0x{:02X}: 0", header);
            self.buf.advance(1);
            return None;
        }

        if size > MAX_PACKET_SIZE {
            tracing::warn!(
                "Invalid packet size for header 0x{:02X}: {} (exceeds max)",
                header,
                size
            );
            self.buf.advance(1);
            return None;
        }

        // Check if we have the complete packet
        if self.buf.len() < size {
            return None;
        }

        // Consume header
        self.buf.advance(1);

        // Extract payload (size - 1 bytes, since size includes header)
        let payload_size = size - 1;
        let payload = if payload_size > 0 {
            self.buf[..payload_size].to_vec()
        } else {
            Vec::new()
        };
        self.buf.advance(payload_size);

        Some(Packet::new(header, payload))
    }

    /// Peek at the next packet header without consuming it
    ///
    /// Returns `Some(header)` if at least one byte is available.
    pub fn peek_header(&self) -> Option<u8> {
        self.buf.first().copied()
    }

    /// Get a slice of the current buffer contents (for inspection)
    pub fn as_slice(&self) -> &[u8] {
        &self.buf
    }

    /// Clear all data from the buffer
    pub fn clear(&mut self) {
        self.buf.clear();
    }

    /// Compact the buffer by removing consumed data
    ///
    /// This is called automatically when needed, but can be triggered
    /// manually to free memory after processing large batches.
    pub fn compact(&mut self) {
        self.buf.reserve(0); // BytesMut::reserve(0) triggers compaction
    }

    /// Reserve additional capacity for incoming data
    pub fn reserve(&mut self, additional: usize) {
        self.buf.reserve(additional);
    }
}

impl Default for ReadBuffer {
    fn default() -> Self {
        Self::new()
    }
}

/// Write buffer for batching outgoing data before flushing to TCP.
///
/// This buffer accumulates packet data and writes it to the underlying
/// stream in efficient batches, reducing the number of system calls.
#[derive(Debug)]
pub struct WriteBuffer {
    /// Internal buffer using `BytesMut` for efficient writes
    buf: BytesMut,
}

impl WriteBuffer {
    /// Create a new empty write buffer
    pub fn new() -> Self {
        Self {
            buf: BytesMut::with_capacity(DEFAULT_CAPACITY),
        }
    }

    /// Create a new write buffer with specified initial capacity
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            buf: BytesMut::with_capacity(capacity),
        }
    }

    /// Queue a packet for writing
    ///
    /// The packet is serialized to wire format and added to the buffer.
    /// Call `flush_to` to actually write the data to the stream.
    ///
    /// # Panics
    ///
    /// Panics if the packet data cannot be represented by the generic two-byte
    /// length field. Use [`Packet::try_to_bytes`] first for untrusted sizes.
    pub fn put_packet(&mut self, packet: &Packet) {
        let total_size = u16::try_from(packet.data_size())
            .expect("packet data size must fit in the u16 length field");
        self.buf.extend_from_slice(&total_size.to_le_bytes());
        self.buf.put_u8(packet.header);
        self.buf.extend_from_slice(&packet.payload);
    }

    /// Queue raw bytes for writing
    ///
    /// Use this for pre-serialized data or non-packet writes.
    pub fn put(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    /// Queue a single byte for writing
    pub fn put_u8(&mut self, val: u8) {
        self.buf.put_u8(val);
    }

    /// Queue a u16 (little-endian) for writing
    pub fn put_u16_le(&mut self, val: u16) {
        self.buf.extend_from_slice(&val.to_le_bytes());
    }

    /// Queue a u32 (little-endian) for writing
    pub fn put_u32_le(&mut self, val: u32) {
        self.buf.extend_from_slice(&val.to_le_bytes());
    }

    /// Get the number of bytes waiting to be flushed
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// Check if the write buffer is empty
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Get a slice of the pending data
    pub fn as_slice(&self) -> &[u8] {
        &self.buf
    }

    /// Mark bytes as written (after a successful write syscall)
    ///
    /// This is used when the caller handles the actual I/O and needs to
    /// notify the buffer how many bytes were successfully written.
    pub fn advance(&mut self, cnt: usize) {
        self.buf.advance(cnt);
    }

    /// Clear all pending data
    pub fn clear(&mut self) {
        self.buf.clear();
    }

    /// Take all pending data as a `Vec<u8>`.
    ///
    /// This consumes the buffer contents and resets it.
    pub fn take(&mut self) -> Vec<u8> {
        let data = self.buf.to_vec();
        self.buf.clear();
        data
    }

    /// Flush pending data to an async writer
    ///
    /// Writes all buffered data to the provided writer and clears the buffer.
    /// Returns the number of bytes written.
    ///
    /// # Errors
    ///
    /// Returns the writer's I/O error unchanged.
    #[cfg(feature = "tokio")]
    pub async fn flush_to<W: tokio::io::AsyncWriteExt + Unpin>(
        &mut self,
        writer: &mut W,
    ) -> std::io::Result<usize> {
        if self.buf.is_empty() {
            return Ok(0);
        }

        let bytes_written = writer.write(&self.buf).await?;
        self.buf.advance(bytes_written);
        Ok(bytes_written)
    }

    /// Flush all pending data to an async writer (retrying as needed)
    ///
    /// Ensures all buffered data is written before returning.
    ///
    /// # Errors
    ///
    /// Returns the writer's I/O error unchanged, or a write-zero error when
    /// the writer stops making progress.
    #[cfg(feature = "tokio")]
    pub async fn flush_all<W: tokio::io::AsyncWriteExt + Unpin>(
        &mut self,
        writer: &mut W,
    ) -> std::io::Result<()> {
        while !self.buf.is_empty() {
            let bytes_written = writer.write(&self.buf).await?;
            if bytes_written == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "failed to write buffered data",
                ));
            }
            self.buf.advance(bytes_written);
        }
        writer.flush().await?;
        Ok(())
    }
}

impl Default for WriteBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // ReadBuffer tests
    // ========================================================================

    #[test]
    fn test_read_buffer_new() {
        let buf = ReadBuffer::new();
        assert!(buf.is_empty());
        assert_eq!(buf.len(), 0);
    }

    #[test]
    fn test_read_buffer_put() {
        let mut buf = ReadBuffer::new();
        buf.put(&[1, 2, 3]);
        assert_eq!(buf.len(), 3);
        assert!(!buf.is_empty());
    }

    #[test]
    fn test_read_buffer_extract_empty() {
        let mut buf = ReadBuffer::new();
        assert!(buf.try_extract_packet().is_none());
    }

    #[test]
    fn test_read_buffer_extract_partial_header() {
        let mut buf = ReadBuffer::new();
        // Only 1 byte - not enough for size prefix
        buf.put(&[0x05]);
        assert!(buf.try_extract_packet().is_none());
        assert_eq!(buf.len(), 1);
    }

    #[test]
    fn test_read_buffer_extract_partial_body() {
        let mut buf = ReadBuffer::new();
        // Size = 4 (header + 3 bytes payload), but only provide 2 bytes of data
        buf.put(&[0x04, 0x00, 0x01, 0xAA]);
        assert!(buf.try_extract_packet().is_none());
        assert_eq!(buf.len(), 4);
    }

    #[test]
    fn test_read_buffer_extract_complete_packet() {
        let mut buf = ReadBuffer::new();
        // Size = 4 (header + 3 bytes payload)
        // Packet: [0x04, 0x00] = size, [0x01] = header, [0xAA, 0xBB, 0xCC] = payload
        buf.put(&[0x04, 0x00, 0x01, 0xAA, 0xBB, 0xCC]);

        let packet = buf.try_extract_packet().unwrap();
        assert_eq!(packet.header, 0x01);
        assert_eq!(packet.payload, vec![0xAA, 0xBB, 0xCC]);
        assert!(buf.is_empty());
    }

    #[test]
    fn test_read_buffer_extract_multiple_packets() {
        let mut buf = ReadBuffer::new();
        // Two packets in one read
        // Packet 1: size=2 (header + 1 byte payload), header=0x01, payload=[0xAA]
        // Packet 2: size=3 (header + 2 bytes payload), header=0x02, payload=[0xBB, 0xCC]
        buf.put(&[0x02, 0x00, 0x01, 0xAA, 0x03, 0x00, 0x02, 0xBB, 0xCC]);

        let packet1 = buf.try_extract_packet().unwrap();
        assert_eq!(packet1.header, 0x01);
        assert_eq!(packet1.payload, vec![0xAA]);

        let packet2 = buf.try_extract_packet().unwrap();
        assert_eq!(packet2.header, 0x02);
        assert_eq!(packet2.payload, vec![0xBB, 0xCC]);

        assert!(buf.is_empty());
    }

    #[test]
    fn test_read_buffer_partial_accumulation() {
        let mut buf = ReadBuffer::new();

        // First read: partial packet (only size + header)
        // size=3 means header + 2 bytes payload
        buf.put(&[0x03, 0x00, 0x01]);
        assert!(buf.try_extract_packet().is_none());

        // Second read: rest of payload
        buf.put(&[0xAA, 0xBB]);
        let packet = buf.try_extract_packet().unwrap();
        assert_eq!(packet.header, 0x01);
        assert_eq!(packet.payload, vec![0xAA, 0xBB]);
        assert!(buf.is_empty());
    }

    #[test]
    fn test_read_buffer_fragmented_reads() {
        let mut buf = ReadBuffer::new();

        // Simulate very fragmented TCP reads
        // Packet: size=6, header=0x10, payload=[0x01, 0x02, 0x03, 0x04, 0x05]
        buf.put(&[0x06]); // size byte 1
        assert!(buf.try_extract_packet().is_none());

        buf.put(&[0x00]); // size byte 2
        assert!(buf.try_extract_packet().is_none());

        buf.put(&[0x10]); // header
        assert!(buf.try_extract_packet().is_none());

        buf.put(&[0x01, 0x02]); // partial payload
        assert!(buf.try_extract_packet().is_none());

        buf.put(&[0x03, 0x04, 0x05]); // rest of payload
        let packet = buf.try_extract_packet().unwrap();
        assert_eq!(packet.header, 0x10);
        assert_eq!(packet.payload, vec![0x01, 0x02, 0x03, 0x04, 0x05]);
    }

    #[test]
    fn test_read_buffer_header_only_packet() {
        let mut buf = ReadBuffer::new();
        // Packet with only header, no payload: size=1, header=0xFF
        buf.put(&[0x01, 0x00, 0xFF]);

        let packet = buf.try_extract_packet().unwrap();
        assert_eq!(packet.header, 0xFF);
        assert!(packet.payload.is_empty());
    }

    #[test]
    fn test_read_buffer_invalid_size_too_small() {
        let mut buf = ReadBuffer::new();
        // Size = 0 (invalid, must be at least 1 for header)
        buf.put(&[0x00, 0x00, 0x01]);

        assert!(buf.try_extract_packet().is_none());
        // The invalid size prefix should be consumed
        assert_eq!(buf.len(), 1); // Only the header byte remains
    }

    #[test]
    fn test_packet_serialization_rejects_oversized_payload() {
        let packet = Packet::new(0x01, vec![0xAA; MAX_PACKET_SIZE]);
        let error = packet.try_to_bytes().unwrap_err();
        assert!(error.contains("exceeds"));
    }

    #[test]
    fn test_resolver_rejects_oversized_packet() {
        let mut buf = ReadBuffer::new();
        buf.put(&[0x01, 0xAA]);

        assert!(buf
            .try_extract_packet_with_resolver(|_| Some(MAX_PACKET_SIZE + 1))
            .is_none());
        // The invalid header should be consumed.
        assert_eq!(buf.len(), 1);
    }

    #[test]
    fn test_read_buffer_clear() {
        let mut buf = ReadBuffer::new();
        buf.put(&[1, 2, 3, 4, 5]);
        assert_eq!(buf.len(), 5);

        buf.clear();
        assert!(buf.is_empty());
    }

    #[test]
    fn test_read_buffer_peek_header() {
        let mut buf = ReadBuffer::new();
        assert!(buf.peek_header().is_none());

        buf.put(&[0x42, 0x00]);
        assert_eq!(buf.peek_header(), Some(0x42));
        // Peek doesn't consume
        assert_eq!(buf.len(), 2);
    }

    #[test]
    fn test_read_buffer_with_resolver() {
        let mut buf = ReadBuffer::new();

        // Custom resolver: header 0x01 = 5 bytes, header 0x02 = 3 bytes
        let resolver = |header: u8| -> Option<usize> {
            match header {
                0x01 => Some(5), // header + 4 bytes payload
                0x02 => Some(3), // header + 2 bytes payload
                _ => None,
            }
        };

        // Packet with header 0x01, payload [0xAA, 0xBB, 0xCC, 0xDD]
        buf.put(&[0x01, 0xAA, 0xBB, 0xCC, 0xDD]);

        let packet = buf.try_extract_packet_with_resolver(resolver).unwrap();
        assert_eq!(packet.header, 0x01);
        assert_eq!(packet.payload, vec![0xAA, 0xBB, 0xCC, 0xDD]);
    }

    #[test]
    fn test_read_buffer_with_resolver_partial() {
        let mut buf = ReadBuffer::new();

        let resolver = |header: u8| -> Option<usize> {
            match header {
                0x01 => Some(5),
                _ => None,
            }
        };

        // Only header + partial payload
        buf.put(&[0x01, 0xAA, 0xBB]);
        assert!(buf.try_extract_packet_with_resolver(resolver).is_none());
        assert_eq!(buf.len(), 3);
    }

    #[test]
    fn test_read_buffer_with_resolver_unknown_header() {
        let mut buf = ReadBuffer::new();

        let resolver = |_header: u8| -> Option<usize> { None };

        buf.put(&[0xFF, 0xAA, 0xBB]);
        assert!(buf.try_extract_packet_with_resolver(resolver).is_none());
        // Unknown header should be consumed
        assert_eq!(buf.len(), 2);
    }

    // ========================================================================
    // WriteBuffer tests
    // ========================================================================

    #[test]
    fn test_write_buffer_new() {
        let buf = WriteBuffer::new();
        assert!(buf.is_empty());
        assert_eq!(buf.len(), 0);
    }

    #[test]
    fn test_write_buffer_put_packet() {
        let mut buf = WriteBuffer::new();
        let packet = Packet::new(0x01, vec![0xAA, 0xBB, 0xCC]);

        buf.put_packet(&packet);
        assert_eq!(buf.len(), 6); // 2 (size) + 1 (header) + 3 (payload)

        // Verify wire format
        let data = buf.as_slice();
        assert_eq!(&data[0..2], &[0x04, 0x00]); // size = 4 (header + 3 payload)
        assert_eq!(data[2], 0x01); // header
        assert_eq!(&data[3..], &[0xAA, 0xBB, 0xCC]); // payload
    }

    #[test]
    fn test_write_buffer_put_raw() {
        let mut buf = WriteBuffer::new();
        buf.put(&[0x01, 0x02, 0x03]);
        assert_eq!(buf.len(), 3);
        assert_eq!(buf.as_slice(), &[0x01, 0x02, 0x03]);
    }

    #[test]
    fn test_write_buffer_put_u8() {
        let mut buf = WriteBuffer::new();
        buf.put_u8(0x42);
        assert_eq!(buf.len(), 1);
        assert_eq!(buf.as_slice(), &[0x42]);
    }

    #[test]
    fn test_write_buffer_put_u16_le() {
        let mut buf = WriteBuffer::new();
        buf.put_u16_le(0x1234);
        assert_eq!(buf.len(), 2);
        assert_eq!(buf.as_slice(), &[0x34, 0x12]);
    }

    #[test]
    fn test_write_buffer_put_u32_le() {
        let mut buf = WriteBuffer::new();
        buf.put_u32_le(0x1234_5678);
        assert_eq!(buf.len(), 4);
        assert_eq!(buf.as_slice(), &[0x78, 0x56, 0x34, 0x12]);
    }

    #[test]
    fn test_write_buffer_advance() {
        let mut buf = WriteBuffer::new();
        buf.put(&[0x01, 0x02, 0x03, 0x04, 0x05]);
        assert_eq!(buf.len(), 5);

        buf.advance(2);
        assert_eq!(buf.len(), 3);
        assert_eq!(buf.as_slice(), &[0x03, 0x04, 0x05]);
    }

    #[test]
    fn test_write_buffer_clear() {
        let mut buf = WriteBuffer::new();
        buf.put(&[0x01, 0x02, 0x03]);
        assert_eq!(buf.len(), 3);

        buf.clear();
        assert!(buf.is_empty());
    }

    #[test]
    fn test_write_buffer_take() {
        let mut buf = WriteBuffer::new();
        buf.put(&[0x01, 0x02, 0x03]);

        let data = buf.take();
        assert_eq!(data, vec![0x01, 0x02, 0x03]);
        assert!(buf.is_empty());
    }

    // ========================================================================
    // Packet tests
    // ========================================================================

    #[test]
    fn test_packet_new() {
        let packet = Packet::new(0x01, vec![0xAA, 0xBB]);
        assert_eq!(packet.header, 0x01);
        assert_eq!(packet.payload, vec![0xAA, 0xBB]);
    }

    #[test]
    fn test_packet_sizes() {
        let packet = Packet::new(0x01, vec![0xAA, 0xBB, 0xCC]);
        assert_eq!(packet.data_size(), 4); // 1 header + 3 payload
        assert_eq!(packet.wire_size(), 6); // 2 size prefix + 4 data
    }

    #[test]
    fn test_packet_to_bytes() {
        let packet = Packet::new(0x10, vec![0x01, 0x02]);
        let bytes = packet.to_bytes();

        assert_eq!(bytes.len(), 5); // 2 + 1 + 2
        assert_eq!(&bytes[0..2], &[0x03, 0x00]); // size = 3
        assert_eq!(bytes[2], 0x10); // header
        assert_eq!(&bytes[3..], &[0x01, 0x02]); // payload
    }

    #[test]
    fn test_packet_roundtrip() {
        let original = Packet::new(0x42, vec![0xDE, 0xAD, 0xBE, 0xEF]);
        let bytes = original.to_bytes();

        let mut buf = ReadBuffer::new();
        buf.put(&bytes);

        let decoded = buf.try_extract_packet().unwrap();
        assert_eq!(decoded, original);
    }

    // ========================================================================
    // Integration tests: partial read scenarios
    // ========================================================================

    #[test]
    fn test_partial_read_scenario_1() {
        // Scenario: Large packet arrives in multiple small TCP reads
        let mut buf = ReadBuffer::new();
        let payload: Vec<u8> = (0..100).collect();
        let packet = Packet::new(0x50, payload.clone());
        let wire_bytes = packet.to_bytes();

        // Simulate receiving in 10-byte chunks
        for chunk in wire_bytes.chunks(10) {
            buf.put(chunk);
        }

        let decoded = buf.try_extract_packet().unwrap();
        assert_eq!(decoded.header, 0x50);
        assert_eq!(decoded.payload, payload);
    }

    #[test]
    fn test_partial_read_scenario_2() {
        // Scenario: Multiple packets arrive with one spanning two reads
        let mut buf = ReadBuffer::new();

        // Packet 1: complete
        // Packet 2: split across two reads
        let p1 = Packet::new(0x01, vec![0xAA]);
        let p2 = Packet::new(0x02, vec![0xBB, 0xCC, 0xDD]);

        let mut combined = p1.to_bytes();
        // Add first part of packet 2 (size + header only)
        combined.extend_from_slice(&p2.to_bytes()[..3]);

        buf.put(&combined);

        // Should extract packet 1
        let decoded1 = buf.try_extract_packet().unwrap();
        assert_eq!(decoded1.header, 0x01);

        // Packet 2 should not be extractable yet
        assert!(buf.try_extract_packet().is_none());

        // Add rest of packet 2
        buf.put(&p2.to_bytes()[3..]);

        let decoded2 = buf.try_extract_packet().unwrap();
        assert_eq!(decoded2.header, 0x02);
        assert_eq!(decoded2.payload, vec![0xBB, 0xCC, 0xDD]);
    }

    #[test]
    fn test_partial_read_scenario_3() {
        // Scenario: Three packets arrive in one big read
        let mut buf = ReadBuffer::new();

        let p1 = Packet::new(0x01, vec![0x11]);
        let p2 = Packet::new(0x02, vec![0x22, 0x22]);
        let p3 = Packet::new(0x03, vec![0x33, 0x33, 0x33]);

        let mut combined = Vec::new();
        combined.extend_from_slice(&p1.to_bytes());
        combined.extend_from_slice(&p2.to_bytes());
        combined.extend_from_slice(&p3.to_bytes());

        buf.put(&combined);

        let d1 = buf.try_extract_packet().unwrap();
        assert_eq!(d1.header, 0x01);

        let d2 = buf.try_extract_packet().unwrap();
        assert_eq!(d2.header, 0x02);

        let d3 = buf.try_extract_packet().unwrap();
        assert_eq!(d3.header, 0x03);

        assert!(buf.is_empty());
    }

    #[test]
    fn test_partial_read_scenario_4() {
        // Scenario: Byte-by-byte arrival (worst case)
        let mut buf = ReadBuffer::new();

        let packet = Packet::new(0xFF, vec![0x01, 0x02, 0x03, 0x04, 0x05]);
        let wire_bytes = packet.to_bytes();

        // Feed one byte at a time
        for (i, byte) in wire_bytes.iter().enumerate() {
            buf.put(&[*byte]);
            if i < wire_bytes.len() - 1 {
                assert!(
                    buf.try_extract_packet().is_none(),
                    "Should not extract at byte {i}"
                );
            }
        }

        let decoded = buf.try_extract_packet().unwrap();
        assert_eq!(decoded.header, 0xFF);
        assert_eq!(decoded.payload, vec![0x01, 0x02, 0x03, 0x04, 0x05]);
    }

    #[test]
    fn test_read_write_buffer_integration() {
        // Test that WriteBuffer output can be fed into ReadBuffer
        let mut write_buf = WriteBuffer::new();

        let p1 = Packet::new(0x10, vec![0xAA, 0xBB]);
        let p2 = Packet::new(0x20, vec![0xCC, 0xDD, 0xEE]);

        write_buf.put_packet(&p1);
        write_buf.put_packet(&p2);

        let data = write_buf.take();

        let mut read_buf = ReadBuffer::new();
        read_buf.put(&data);

        let d1 = read_buf.try_extract_packet().unwrap();
        assert_eq!(d1, p1);

        let d2 = read_buf.try_extract_packet().unwrap();
        assert_eq!(d2, p2);
    }
}
