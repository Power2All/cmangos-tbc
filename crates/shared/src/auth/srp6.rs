// SRP6 - Secure Remote Password Protocol v6
// Rust equivalent of SRP6.h/cpp
//
// This implements the WoW-specific SRP6 authentication protocol.
// The protocol constants (N, g) are specific to the WoW client.

use super::big_number::BigNumber;
use super::crypto_hash::Sha1Hash;

/// SRP6 protocol state
/// Implements the server side of the SRP6 authentication handshake.
pub struct SRP6 {
    // Protocol parameters
    /// Safe prime (N) - the large prime modulus
    n: BigNumber,
    /// Generator modulo (g)
    g: BigNumber,
    /// Salt (s) - random per-account
    s: BigNumber,
    /// Password verifier (v) - stored in database
    v: BigNumber,

    // Ephemeral values
    /// Host private ephemeral (b) - random per session
    b: BigNumber,
    /// Host public ephemeral (B) - sent to client
    big_b: BigNumber,
    /// Client public ephemeral (A) - received from client
    big_a: BigNumber,
    /// Scrambler (u) = SHA1(A || B)
    u: BigNumber,
    /// Session key (S)
    big_s: BigNumber,

    // Derived keys
    /// Strong session key (K) - interleaved hash of S
    k: BigNumber,
    /// Proof (M) - proof of session key knowledge
    m: BigNumber,
}

impl Default for SRP6 {
    fn default() -> Self {
        Self::new()
    }
}

impl SRP6 {
    /// Salt byte size
    pub const S_BYTE_SIZE: usize = 32;

    /// Create a new SRP6 instance with the WoW-specific prime and generator
    pub fn new() -> Self {
        let mut n = BigNumber::new();
        n.set_hex_str("894B645E89E1535BBDAD5B8B290650530801B18EBFBF5E8FAB3C82872A3E9BB7");

        let g = BigNumber::from_u32(7);

        SRP6 {
            n,
            g,
            s: BigNumber::new(),
            v: BigNumber::new(),
            b: BigNumber::new(),
            big_b: BigNumber::new(),
            big_a: BigNumber::new(),
            u: BigNumber::new(),
            big_s: BigNumber::new(),
            k: BigNumber::new(),
            m: BigNumber::new(),
        }
    }

    /// Calculate the host public ephemeral (B)
    /// Also generates a random host private ephemeral (b)
    /// B = (v * 3 + g^b mod N) mod N
    pub fn calculate_host_public_ephemeral(&mut self) {
        self.b.set_rand(19 * 8);
        let g_mod = self.g.mod_exp(&self.b, &self.n);
        let v_times_3 = &self.v * 3u32;
        let sum = &v_times_3 + &g_mod;
        self.big_b = &sum % &self.n;

        assert!(g_mod.get_num_bytes() <= 32);
    }

    /// Calculate proof (M) of the strong session key (K)
    /// M = SHA1(H(N) XOR H(g) || H(username) || s || A || B || K)
    pub fn calculate_proof(&mut self, username: &str) {
        // H(N)
        let mut sha = Sha1Hash::new();
        sha.update_big_numbers(&[&self.n]);
        sha.finalize();
        let mut hash = *sha.get_digest();

        // H(g)
        sha.initialize();
        sha.update_big_numbers(&[&self.g]);
        sha.finalize();

        // H(N) XOR H(g)
        for (i, byte) in hash.iter_mut().enumerate().take(20) {
            *byte ^= sha.get_digest()[i];
        }

        // H(username)
        sha.initialize();
        sha.update_data(username);
        sha.finalize();
        let t4 = *sha.get_digest();

        // M = SHA1(H(N) XOR H(g) || H(username) || s || A || B || K)
        sha.initialize();
        sha.update_data_bytes(&hash);
        sha.update_data_bytes(&t4);
        sha.update_big_numbers(&[&self.s, &self.big_a, &self.big_b, &self.k]);
        sha.finalize();

        self.m.set_binary(sha.get_digest());
    }

    /// Calculate the session key (S) based on client public ephemeral (A)
    /// S = (A * v^u)^b mod N
    ///
    /// Returns false if A is invalid (A == 0 or A % N == 0)
    pub fn calculate_session_key(&mut self, client_a: &[u8]) -> bool {
        self.big_a.set_binary(client_a);

        // SRP safeguard: abort if A == 0
        if self.big_a.is_zero() {
            return false;
        }

        // SRP safeguard: abort if A % N == 0
        let a_mod_n = &self.big_a % &self.n;
        if a_mod_n.is_zero() {
            return false;
        }

        // u = SHA1(A || B)
        let mut sha = Sha1Hash::new();
        sha.update_big_numbers(&[&self.big_a, &self.big_b]);
        sha.finalize();
        self.u.set_binary(sha.get_digest());

        // S = (A * v^u mod N)^b mod N
        let v_mod = self.v.mod_exp(&self.u, &self.n);
        let a_times_v = &self.big_a * &v_mod;
        self.big_s = a_times_v.mod_exp(&self.b, &self.n);

        true
    }

    /// Calculate the password verifier (v) with a random salt
    /// v = g^x mod N where x = SHA1(s || H(USERNAME:PASSWORD))
    pub fn calculate_verifier_random(&mut self, ri: &str) -> bool {
        let mut salt = BigNumber::new();
        salt.set_rand(Self::S_BYTE_SIZE as u64 * 8);
        let salt_hex = salt.as_hex_str();
        self.calculate_verifier(ri, &salt_hex)
    }

    /// Calculate the password verifier (v) with a predefined salt
    pub fn calculate_verifier(&mut self, ri: &str, salt: &str) -> bool {
        if self.s.set_hex_str(salt) == 0 || self.s.is_zero() {
            return false;
        }

        let mut i = BigNumber::new();
        i.set_hex_str(ri);

        // In case of leading zeros in the rI hash, restore them
        let mut m_digest = [0u8; Sha1Hash::DIGEST_LENGTH];
        if i.get_num_bytes() <= Sha1Hash::DIGEST_LENGTH {
            let vect_i = i.as_byte_array(0);
            let copy_len = vect_i.len().min(Sha1Hash::DIGEST_LENGTH);
            m_digest[..copy_len].copy_from_slice(&vect_i[..copy_len]);
        }

        m_digest.reverse();

        // x = SHA1(s || rI)
        let mut sha = Sha1Hash::new();
        sha.update_data_bytes(&self.s.as_byte_array(0));
        sha.update_data_bytes(&m_digest);
        sha.finalize();

        let mut x = BigNumber::new();
        x.set_binary(sha.get_digest());

        // v = g^x mod N
        self.v = self.g.mod_exp(&x, &self.n);

        true
    }

    /// Generate the strong session key (K) from session key (S)
    /// K is derived by interleaving SHA1 hashes of even/odd bytes of S
    pub fn hash_session_key(&mut self) {
        let t = self.big_s.as_byte_array(32);
        let mut t1 = [0u8; 16];
        let mut vk = [0u8; 40];

        // Hash even bytes
        for i in 0..16 {
            t1[i] = t[i * 2];
        }
        let mut sha = Sha1Hash::new();
        sha.initialize();
        sha.update_data_bytes(&t1);
        sha.finalize();
        for i in 0..20 {
            vk[i * 2] = sha.get_digest()[i];
        }

        // Hash odd bytes
        for i in 0..16 {
            t1[i] = t[i * 2 + 1];
        }
        sha.initialize();
        sha.update_data_bytes(&t1);
        sha.finalize();
        for i in 0..20 {
            vk[i * 2 + 1] = sha.get_digest()[i];
        }

        self.k.set_binary(&vk);
    }

    /// Verify client proof (M)
    /// Returns true if the client's M matches our computed M (password correct)
    /// Returns false if they don't match (wrong password)
    ///
    /// Note: The C++ Proof() returns memcmp() result which is 0 (falsy) on match,
    /// so C++ callers use `if (!srp.Proof(...))` for the success branch.
    /// Our Rust version uses idiomatic `true` = match, so callers use
    /// `if !srp.proof(...)` for the failure branch.
    pub fn proof(&self, client_m: &[u8]) -> bool {
        let our_m = self.m.as_byte_array(client_m.len());
        our_m[..client_m.len()] == client_m[..client_m.len()]
    }

    /// Verify password verifier (v) against a database value
    pub fn proof_verifier(&self, vc: &str) -> bool {
        let v_hex = self.v.as_hex_str();
        vc == v_hex
    }

    /// Generate server proof hash for client verification
    /// sha = SHA1(A || M || K)
    pub fn finalize(&self, sha: &mut Sha1Hash) {
        sha.initialize();
        sha.update_big_numbers(&[&self.big_a, &self.m, &self.k]);
        sha.finalize();
    }

    // Getters
    pub fn get_host_public_ephemeral(&self) -> &BigNumber {
        &self.big_b
    }

    pub fn get_generator_modulo(&self) -> &BigNumber {
        &self.g
    }

    pub fn get_prime(&self) -> &BigNumber {
        &self.n
    }

    pub fn get_proof(&self) -> &BigNumber {
        &self.m
    }

    pub fn get_salt(&self) -> &BigNumber {
        &self.s
    }

    pub fn get_strong_session_key(&self) -> &BigNumber {
        &self.k
    }

    pub fn get_verifier(&self) -> &BigNumber {
        &self.v
    }

    // Setters
    pub fn set_salt(&mut self, new_s: &str) -> bool {
        if self.s.set_hex_str(new_s) == 0 || self.s.is_zero() {
            return false;
        }
        true
    }

    pub fn set_strong_session_key(&mut self, new_k: &str) {
        self.k.set_hex_str(new_k);
    }

    pub fn set_verifier(&mut self, new_v: &str) -> bool {
        if self.v.set_hex_str(new_v) == 0 || self.v.is_zero() {
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_srp6_init() {
        let srp = SRP6::new();
        assert!(!srp.n.is_zero());
        assert_eq!(srp.g.as_dword(), 7);
    }

    #[test]
    fn test_set_verifier() {
        let mut srp = SRP6::new();
        assert!(srp.set_verifier("312B99EEF1C0196BB73B79D114CE161C5D089319E6EF54FAA6117DAB8B672C14"));
        assert!(!srp.get_verifier().is_zero());
    }
}
