// ByteBuffer - Binary packet serialization/deserialization
// Rust equivalent of ByteBuffer.h/cpp

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::io::Cursor;

/// A byte buffer for reading/writing binary packet data.
/// Matches the C++ ByteBuffer class used for network packet construction.
#[derive(Debug, Clone)]
pub struct ByteBuffer {
    data: Vec<u8>,
    read_pos: usize,
}

impl Default for ByteBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl ByteBuffer {
    /// Create a new empty ByteBuffer
    pub fn new() -> Self {
        ByteBuffer {
            data: Vec::new(),
            read_pos: 0,
        }
    }

    /// Create with a pre-allocated capacity
    pub fn with_capacity(capacity: usize) -> Self {
        ByteBuffer {
            data: Vec::with_capacity(capacity),
            read_pos: 0,
        }
    }

    /// Get the current size of the buffer
    pub fn size(&self) -> usize {
        self.data.len()
    }

    /// Check if the buffer is empty
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Get the current read position
    pub fn read_pos(&self) -> usize {
        self.read_pos
    }

    /// Get a pointer to the raw contents
    pub fn contents(&self) -> &[u8] {
        &self.data
    }

    /// Get mutable access to the raw data
    pub fn data_mut(&mut self) -> &mut Vec<u8> {
        &mut self.data
    }

    /// Clear the buffer
    pub fn clear(&mut self) {
        self.data.clear();
        self.read_pos = 0;
    }

    // ---- Write operations (append) ----

    /// Append raw bytes
    pub fn append(&mut self, data: &[u8]) {
        self.data.extend_from_slice(data);
    }

    /// Write a u8
    pub fn write_u8(&mut self, val: u8) {
        self.data.push(val);
    }

    /// Write a u16 (little-endian)
    pub fn write_u16(&mut self, val: u16) {
        self.data.write_u16::<LittleEndian>(val).unwrap();
    }

    /// Write a u32 (little-endian)
    pub fn write_u32(&mut self, val: u32) {
        self.data.write_u32::<LittleEndian>(val).unwrap();
    }

    /// Write a u64 (little-endian)
    pub fn write_u64(&mut self, val: u64) {
        self.data.write_u64::<LittleEndian>(val).unwrap();
    }

    /// Write an f32 (little-endian)
    pub fn write_f32(&mut self, val: f32) {
        self.data.write_f32::<LittleEndian>(val).unwrap();
    }

    /// Write a null-terminated string
    pub fn write_string(&mut self, val: &str) {
        self.data.extend_from_slice(val.as_bytes());
        self.data.push(0); // null terminator
    }

    // ---- Read operations ----

    /// Read a u8
    pub fn read_u8(&mut self) -> Result<u8, std::io::Error> {
        if self.read_pos >= self.data.len() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "ByteBuffer read past end",
            ));
        }
        let val = self.data[self.read_pos];
        self.read_pos += 1;
        Ok(val)
    }

    /// Read a u16 (little-endian)
    pub fn read_u16(&mut self) -> Result<u16, std::io::Error> {
        if self.read_pos + 2 > self.data.len() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "ByteBuffer read past end",
            ));
        }
        let mut cursor = Cursor::new(&self.data[self.read_pos..]);
        let val = cursor.read_u16::<LittleEndian>()?;
        self.read_pos += 2;
        Ok(val)
    }

    /// Read a u32 (little-endian)
    pub fn read_u32(&mut self) -> Result<u32, std::io::Error> {
        if self.read_pos + 4 > self.data.len() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "ByteBuffer read past end",
            ));
        }
        let mut cursor = Cursor::new(&self.data[self.read_pos..]);
        let val = cursor.read_u32::<LittleEndian>()?;
        self.read_pos += 4;
        Ok(val)
    }

    /// Read a u64 (little-endian)
    pub fn read_u64(&mut self) -> Result<u64, std::io::Error> {
        if self.read_pos + 8 > self.data.len() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "ByteBuffer read past end",
            ));
        }
        let mut cursor = Cursor::new(&self.data[self.read_pos..]);
        let val = cursor.read_u64::<LittleEndian>()?;
        self.read_pos += 8;
        Ok(val)
    }

    /// Read an f32 (little-endian)
    pub fn read_f32(&mut self) -> Result<f32, std::io::Error> {
        if self.read_pos + 4 > self.data.len() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "ByteBuffer read past end",
            ));
        }
        let mut cursor = Cursor::new(&self.data[self.read_pos..]);
        let val = cursor.read_f32::<LittleEndian>()?;
        self.read_pos += 4;
        Ok(val)
    }

    /// Read a null-terminated string
    pub fn read_string(&mut self) -> Result<String, std::io::Error> {
        let start = self.read_pos;
        while self.read_pos < self.data.len() && self.data[self.read_pos] != 0 {
            self.read_pos += 1;
        }
        let s = String::from_utf8_lossy(&self.data[start..self.read_pos]).to_string();
        if self.read_pos < self.data.len() {
            self.read_pos += 1; // skip null terminator
        }
        Ok(s)
    }

    /// Read N bytes into a slice
    pub fn read_bytes(&mut self, count: usize) -> Result<Vec<u8>, std::io::Error> {
        if self.read_pos + count > self.data.len() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "ByteBuffer read past end",
            ));
        }
        let bytes = self.data[self.read_pos..self.read_pos + count].to_vec();
        self.read_pos += count;
        Ok(bytes)
    }

    /// Skip N bytes in the read position
    pub fn read_skip(&mut self, count: usize) {
        self.read_pos = (self.read_pos + count).min(self.data.len());
    }
}

/// Implement the C++ << operator style for building packets
/// Usage: buf << 0u8 << 0u16 << "hello"
impl std::fmt::Display for ByteBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ByteBuffer(size={}, rpos={})", self.size(), self.read_pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_read_u8() {
        let mut buf = ByteBuffer::new();
        buf.write_u8(42);
        assert_eq!(buf.read_u8().unwrap(), 42);
    }

    #[test]
    fn test_write_read_u32() {
        let mut buf = ByteBuffer::new();
        buf.write_u32(0xDEADBEEF);
        assert_eq!(buf.read_u32().unwrap(), 0xDEADBEEF);
    }

    #[test]
    fn test_write_read_string() {
        let mut buf = ByteBuffer::new();
        buf.write_string("hello");
        assert_eq!(buf.read_string().unwrap(), "hello");
    }

    #[test]
    fn test_append_bytes() {
        let mut buf = ByteBuffer::new();
        buf.append(&[1, 2, 3, 4]);
        assert_eq!(buf.size(), 4);
        assert_eq!(buf.contents(), &[1, 2, 3, 4]);
    }
}
