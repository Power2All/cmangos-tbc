// HMAC-SHA1 implementation
// Rust equivalent of HMACSHA1.h/cpp

use hmac::{Hmac, Mac};
use sha1::Sha1;
use super::big_number::BigNumber;

type HmacSha1Inner = Hmac<Sha1>;

/// HMAC-SHA1 wrapper matching the C++ HMACSHA1 class
pub struct HmacSha1 {
    mac: HmacSha1Inner,
    digest: [u8; 20],
}

impl HmacSha1 {
    pub const DIGEST_LENGTH: usize = 20;

    /// Create a new HMAC-SHA1 with the given key
    pub fn new(key: &[u8]) -> Self {
        HmacSha1 {
            mac: HmacSha1Inner::new_from_slice(key)
                .expect("HMAC-SHA1 key can be any length"),
            digest: [0u8; 20],
        }
    }

    /// Update with BigNumber data
    pub fn update_big_number(&mut self, bn: &BigNumber) {
        let data = bn.as_byte_array(0);
        self.update_data(&data);
    }

    /// Update with raw bytes
    pub fn update_data(&mut self, data: &[u8]) {
        self.mac.update(data);
    }

    /// Update with a string
    pub fn update_string(&mut self, data: &str) {
        self.mac.update(data.as_bytes());
    }

    /// Finalize and compute the MAC
    pub fn finalize(&mut self) {
        let result = self.mac.clone().finalize();
        self.digest.copy_from_slice(&result.into_bytes());
    }

    /// Compute hash from a BigNumber (update + finalize)
    pub fn compute_hash(&mut self, bn: &BigNumber) -> [u8; 20] {
        self.update_big_number(bn);
        self.finalize();
        self.digest
    }

    /// Get the computed digest
    pub fn get_digest(&self) -> &[u8; 20] {
        &self.digest
    }

    pub const fn get_length() -> usize {
        Self::DIGEST_LENGTH
    }
}

/// Compute HMAC-SHA1 in one shot (matching the OpenSSL HMAC() function call in generateToken)
pub fn hmac_sha1(key: &[u8], data: &[u8]) -> [u8; 20] {
    let mut mac = HmacSha1Inner::new_from_slice(key)
        .expect("HMAC-SHA1 key can be any length");
    mac.update(data);
    let result = mac.finalize();
    let mut digest = [0u8; 20];
    digest.copy_from_slice(&result.into_bytes());
    digest
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hmac_sha1() {
        let key = b"secret";
        let data = b"message";
        let result = hmac_sha1(key, data);
        assert_eq!(result.len(), 20);
    }
}
