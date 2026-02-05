// BigNumber - Large integer arithmetic wrapper
// Rust equivalent of BigNumber.h/cpp using num-bigint

use num_bigint::{BigUint, RandBigInt};
use num_traits::Zero;
use rand::thread_rng;

/// BigNumber wraps num-bigint's BigUint for cryptographic operations.
/// Mirrors the C++ BigNumber class that wraps OpenSSL's BIGNUM.
///
/// Important: The C++ code stores numbers in little-endian byte order
/// for SRP6 wire protocol. Methods like `set_binary` and `as_byte_array`
/// handle the reversal (little-endian <-> big-endian) to match.
#[derive(Debug, Clone)]
pub struct BigNumber {
    bn: BigUint,
}

impl Default for BigNumber {
    fn default() -> Self {
        Self::new()
    }
}

impl BigNumber {
    /// Create a new BigNumber initialized to zero
    pub fn new() -> Self {
        BigNumber { bn: BigUint::zero() }
    }

    /// Create from a u32 value
    pub fn from_u32(val: u32) -> Self {
        BigNumber { bn: BigUint::from(val) }
    }

    /// Create from a u64 value
    pub fn from_u64(val: u64) -> Self {
        BigNumber { bn: BigUint::from(val) }
    }

    /// Set from a u32 (dword)
    pub fn set_dword(&mut self, val: u32) {
        self.bn = BigUint::from(val);
    }

    /// Set from a u64 (qword)
    pub fn set_qword(&mut self, val: u64) {
        self.bn = BigUint::from(val);
    }

    /// Set from binary data in little-endian order
    /// (matches C++ SetBinary which reverses bytes before BN_bin2bn)
    pub fn set_binary(&mut self, bytes: &[u8]) {
        let mut reversed = bytes.to_vec();
        reversed.reverse();
        self.bn = BigUint::from_bytes_be(&reversed);
    }

    /// Set from a hex string (big-endian, as stored in database)
    /// Returns the number of characters processed, 0 on error
    pub fn set_hex_str(&mut self, hex: &str) -> usize {
        let hex = hex.trim();
        if hex.is_empty() {
            return 0;
        }
        match BigUint::parse_bytes(hex.as_bytes(), 16) {
            Some(val) => {
                self.bn = val;
                hex.len()
            }
            None => 0,
        }
    }

    /// Generate a random number with the specified number of bits
    pub fn set_rand(&mut self, num_bits: u64) {
        let mut rng = thread_rng();
        self.bn = rng.gen_biguint(num_bits);
    }

    /// Check if the number is zero
    pub fn is_zero(&self) -> bool {
        self.bn.is_zero()
    }

    /// Modular exponentiation: self^exp mod modulus
    pub fn mod_exp(&self, exp: &BigNumber, modulus: &BigNumber) -> BigNumber {
        BigNumber {
            bn: self.bn.modpow(&exp.bn, &modulus.bn),
        }
    }

    /// Regular exponentiation: self^exp
    pub fn exp(&self, exp: &BigNumber) -> BigNumber {
        BigNumber {
            bn: num_traits::pow::Pow::pow(&self.bn, &exp.bn),
        }
    }

    /// Get the number of bytes needed to represent this number
    pub fn get_num_bytes(&self) -> usize {
        let bits = self.bn.bits() as usize;
        bits.div_ceil(8)
    }

    /// Get as a u32 value
    pub fn as_dword(&self) -> u32 {
        use num_traits::ToPrimitive;
        self.bn.to_u32().unwrap_or(0)
    }

    /// Convert to a byte array in little-endian order (matching C++ AsByteArray with reverse=true)
    /// Pads to min_size if specified
    pub fn as_byte_array(&self, min_size: usize) -> Vec<u8> {
        let be_bytes = self.bn.to_bytes_be();
        let length = if min_size > be_bytes.len() {
            min_size
        } else {
            be_bytes.len()
        };

        let mut result = vec![0u8; length];

        // Copy bytes with padding offset (leading zeros in BE become trailing in LE)
        let padding_offset = length - be_bytes.len();
        result[padding_offset..].copy_from_slice(&be_bytes);

        // Reverse to little-endian (matching C++ default reverse=true)
        result.reverse();
        result
    }

    /// Convert to big-endian byte array (no reversal)
    pub fn as_byte_array_be(&self, min_size: usize) -> Vec<u8> {
        let be_bytes = self.bn.to_bytes_be();
        let length = if min_size > be_bytes.len() {
            min_size
        } else {
            be_bytes.len()
        };

        let mut result = vec![0u8; length];
        let padding_offset = length - be_bytes.len();
        result[padding_offset..].copy_from_slice(&be_bytes);
        result
    }

    /// Convert to hex string (uppercase)
    pub fn as_hex_str(&self) -> String {
        if self.bn.is_zero() {
            return "0".to_string();
        }
        format!("{:X}", self.bn)
    }

    /// Convert to decimal string
    pub fn as_dec_str(&self) -> String {
        self.bn.to_string()
    }

    /// Get a reference to the inner BigUint
    pub fn inner(&self) -> &BigUint {
        &self.bn
    }
}

// Arithmetic operator implementations

impl std::ops::Add for &BigNumber {
    type Output = BigNumber;
    fn add(self, rhs: &BigNumber) -> BigNumber {
        BigNumber {
            bn: &self.bn + &rhs.bn,
        }
    }
}

impl std::ops::Add for BigNumber {
    type Output = BigNumber;
    fn add(self, rhs: BigNumber) -> BigNumber {
        BigNumber {
            bn: self.bn + rhs.bn,
        }
    }
}

impl std::ops::Sub for &BigNumber {
    type Output = BigNumber;
    fn sub(self, rhs: &BigNumber) -> BigNumber {
        BigNumber {
            bn: if self.bn >= rhs.bn {
                &self.bn - &rhs.bn
            } else {
                BigUint::zero()
            },
        }
    }
}

impl std::ops::Mul for &BigNumber {
    type Output = BigNumber;
    fn mul(self, rhs: &BigNumber) -> BigNumber {
        BigNumber {
            bn: &self.bn * &rhs.bn,
        }
    }
}

impl std::ops::Mul<u32> for &BigNumber {
    type Output = BigNumber;
    fn mul(self, rhs: u32) -> BigNumber {
        BigNumber {
            bn: &self.bn * BigUint::from(rhs),
        }
    }
}

impl std::ops::Div for &BigNumber {
    type Output = BigNumber;
    fn div(self, rhs: &BigNumber) -> BigNumber {
        BigNumber {
            bn: &self.bn / &rhs.bn,
        }
    }
}

impl std::ops::Rem for &BigNumber {
    type Output = BigNumber;
    fn rem(self, rhs: &BigNumber) -> BigNumber {
        BigNumber {
            bn: &self.bn % &rhs.bn,
        }
    }
}

impl std::ops::AddAssign<&BigNumber> for BigNumber {
    fn add_assign(&mut self, rhs: &BigNumber) {
        self.bn += &rhs.bn;
    }
}

impl PartialEq for BigNumber {
    fn eq(&self, other: &Self) -> bool {
        self.bn == other.bn
    }
}

impl Eq for BigNumber {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_arithmetic() {
        let a = BigNumber::from_u32(10);
        let b = BigNumber::from_u32(5);
        let sum = &a + &b;
        assert_eq!(sum.as_dword(), 15);
    }

    #[test]
    fn test_hex_roundtrip() {
        let mut bn = BigNumber::new();
        bn.set_hex_str("894B645E89E1535BBDAD5B8B290650530801B18EBFBF5E8FAB3C82872A3E9BB7");
        let hex = bn.as_hex_str();
        assert_eq!(hex, "894B645E89E1535BBDAD5B8B290650530801B18EBFBF5E8FAB3C82872A3E9BB7");
    }

    #[test]
    fn test_byte_array_le() {
        let bn = BigNumber::from_u32(0x01020304);
        let bytes = bn.as_byte_array(4);
        assert_eq!(bytes, vec![0x04, 0x03, 0x02, 0x01]);
    }

    #[test]
    fn test_set_binary_le() {
        let mut bn = BigNumber::new();
        bn.set_binary(&[0x04, 0x03, 0x02, 0x01]);
        assert_eq!(bn.as_dword(), 0x01020304);
    }

    #[test]
    fn test_mod_exp() {
        let base = BigNumber::from_u32(4);
        let exp = BigNumber::from_u32(13);
        let modulus = BigNumber::from_u32(497);
        let result = base.mod_exp(&exp, &modulus);
        assert_eq!(result.as_dword(), 445);
    }
}
