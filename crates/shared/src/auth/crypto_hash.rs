// CryptoHash - SHA1 and MD5 hash wrappers
// Rust equivalent of CryptoHash.h using the sha1 and md-5 crates

use digest::Digest;
use super::big_number::BigNumber;

/// SHA1 hash wrapper matching the C++ Sha1Hash class
#[derive(Clone)]
pub struct Sha1Hash {
    hasher: sha1::Sha1,
    digest: [u8; 20],
}

impl Default for Sha1Hash {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha1Hash {
    pub const DIGEST_LENGTH: usize = 20;

    pub fn new() -> Self {
        Sha1Hash {
            hasher: sha1::Sha1::new(),
            digest: [0u8; 20],
        }
    }

    /// Re-initialize the hasher
    pub fn initialize(&mut self) {
        self.hasher = sha1::Sha1::new();
    }

    /// Update with raw bytes
    pub fn update_data_bytes(&mut self, data: &[u8]) {
        self.hasher.update(data);
    }

    /// Update with a string
    pub fn update_data(&mut self, data: &str) {
        self.hasher.update(data.as_bytes());
    }

    /// Update with BigNumber values (variadic in C++, slice here)
    /// Each BigNumber is converted to its byte array representation
    pub fn update_big_numbers(&mut self, numbers: &[&BigNumber]) {
        for bn in numbers {
            let bytes = bn.as_byte_array(0);
            self.update_data_bytes(&bytes);
        }
    }

    /// Finalize the hash computation
    pub fn finalize(&mut self) {
        let result = self.hasher.clone().finalize();
        self.digest.copy_from_slice(&result);
    }

    /// Get the computed digest
    pub fn get_digest(&self) -> &[u8; 20] {
        &self.digest
    }

    /// Get the length of the digest
    pub const fn get_length() -> usize {
        Self::DIGEST_LENGTH
    }
}

/// MD5 hash wrapper matching the C++ MD5Hash class
#[derive(Clone)]
pub struct Md5Hash {
    hasher: md5::Md5,
    digest: [u8; 16],
}

impl Default for Md5Hash {
    fn default() -> Self {
        Self::new()
    }
}

impl Md5Hash {
    pub const DIGEST_LENGTH: usize = 16;

    pub fn new() -> Self {
        Md5Hash {
            hasher: md5::Md5::new(),
            digest: [0u8; 16],
        }
    }

    pub fn initialize(&mut self) {
        self.hasher = md5::Md5::new();
    }

    pub fn update_data_bytes(&mut self, data: &[u8]) {
        self.hasher.update(data);
    }

    pub fn update_data(&mut self, data: &str) {
        self.hasher.update(data.as_bytes());
    }

    pub fn finalize(&mut self) {
        let result = self.hasher.clone().finalize();
        self.digest.copy_from_slice(&result);
    }

    pub fn get_digest(&self) -> &[u8; 16] {
        &self.digest
    }

    pub const fn get_length() -> usize {
        Self::DIGEST_LENGTH
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha1_basic() {
        let mut sha = Sha1Hash::new();
        sha.update_data("test");
        sha.finalize();
        // SHA1("test") = a94a8fe5ccb19ba61c4c0873d391e987982fbbd3
        assert_eq!(sha.get_digest()[0], 0xa9);
        assert_eq!(sha.get_digest()[1], 0x4a);
    }

    #[test]
    fn test_md5_basic() {
        let mut md5 = Md5Hash::new();
        md5.update_data("test");
        md5.finalize();
        // MD5("test") = 098f6bcd4621d373cade4e832627b4f6
        assert_eq!(md5.get_digest()[0], 0x09);
    }
}
