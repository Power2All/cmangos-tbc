// Base32 decoding
// Rust equivalent of base32.h/cpp
// Uses data-encoding crate for RFC 4648 Base32

/// Decode a base32-encoded string into bytes.
/// Tolerates whitespace and hyphens (matching C++ behavior).
/// Returns the decoded bytes, or an error if the input is invalid.
pub fn base32_decode(input: &str) -> Result<Vec<u8>, String> {
    // Strip whitespace and hyphens as the C++ version does
    let cleaned: String = input
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| {
            // Handle common typos like the C++ version:
            // 0 -> O, 1 -> L, 8 -> B
            match c {
                '0' => 'O',
                '1' => 'L',
                '8' => 'B',
                _ => c.to_ascii_uppercase(),
            }
        })
        .collect();

    // Pad to multiple of 8 with '='
    let padded = if cleaned.len() % 8 != 0 {
        let pad_len = 8 - (cleaned.len() % 8);
        format!("{}{}", cleaned, "=".repeat(pad_len))
    } else {
        cleaned
    };

    data_encoding::BASE32
        .decode(padded.as_bytes())
        .map_err(|e| format!("base32 decode error: {}", e))
}

/// Decode base32 into a pre-allocated buffer (matching C++ API)
/// Returns the number of bytes decoded, or -1 on error
pub fn base32_decode_into(input: &str, output: &mut [u8]) -> i32 {
    match base32_decode(input) {
        Ok(decoded) => {
            let copy_len = decoded.len().min(output.len());
            output[..copy_len].copy_from_slice(&decoded[..copy_len]);
            copy_len as i32
        }
        Err(_) => -1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_base32_decode() {
        // "JBSWY3DPEHPK3PXP" encodes "Hello!"
        let result = base32_decode("JBSWY3DPEHPK3PXP").unwrap();
        assert_eq!(result, b"Hello!");
    }

    #[test]
    fn test_base32_with_whitespace() {
        let result = base32_decode("JBSW Y3DP EHPK 3PXP").unwrap();
        assert_eq!(result, b"Hello!");
    }
}
