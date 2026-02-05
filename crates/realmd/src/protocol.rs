// Protocol - Wire protocol structures for the auth server
// Rust equivalents of the packed C++ structs in AuthSocket.cpp
//
// These represent the binary packet formats exchanged between
// the WoW client and the authentication server.

use mangos_shared::util::ByteBuffer;

/// Logon Challenge header (received from client)
/// Packed struct: cmd (1) + error (1) + size (2)
#[derive(Debug, Clone)]
pub struct AuthLogonChallengeHeader {
    pub error: u8,
    pub size: u16,
}

impl AuthLogonChallengeHeader {
    pub const SIZE: usize = 3; // error (1) + size (2), cmd already read

    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < Self::SIZE {
            return None;
        }
        Some(AuthLogonChallengeHeader {
            error: data[0],
            size: u16::from_le_bytes([data[1], data[2]]),
        })
    }
}

/// Logon Challenge body (received from client)
/// Contains client information: game version, platform, OS, locale, IP, username
#[derive(Debug, Clone)]
pub struct AuthLogonChallengeBody {
    pub gamename: [u8; 4],
    pub version1: u8,
    pub version2: u8,
    pub version3: u8,
    pub build: u16,
    pub platform: [u8; 4],
    pub os: [u8; 4],
    pub country: [u8; 4],
    pub timezone_bias: u32,
    pub ip: u32,
    pub username_len: u8,
    pub username: Vec<u8>,
}

impl AuthLogonChallengeBody {
    /// Minimum size without the variable-length username
    pub const MIN_SIZE: usize = 4 + 1 + 1 + 1 + 2 + 4 + 4 + 4 + 4 + 4 + 1; // = 30

    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < Self::MIN_SIZE {
            return None;
        }

        let mut gamename = [0u8; 4];
        gamename.copy_from_slice(&data[0..4]);

        let version1 = data[4];
        let version2 = data[5];
        let version3 = data[6];
        let build = u16::from_le_bytes([data[7], data[8]]);

        let mut platform = [0u8; 4];
        platform.copy_from_slice(&data[9..13]);

        let mut os = [0u8; 4];
        os.copy_from_slice(&data[13..17]);

        let mut country = [0u8; 4];
        country.copy_from_slice(&data[17..21]);

        let timezone_bias = u32::from_le_bytes([data[21], data[22], data[23], data[24]]);
        let ip = u32::from_le_bytes([data[25], data[26], data[27], data[28]]);
        let username_len = data[29];

        let username_end = 30 + username_len as usize;
        if data.len() < username_end {
            return None;
        }

        let username = data[30..username_end].to_vec();

        Some(AuthLogonChallengeBody {
            gamename,
            version1,
            version2,
            version3,
            build,
            platform,
            os,
            country,
            timezone_bias,
            ip,
            username_len,
            username,
        })
    }

    /// Get the OS as a string (reversed byte order)
    pub fn os_string(&self) -> String {
        let mut os = self.os;
        os[3] = 0;
        let s: String = os.iter()
            .take_while(|&&b| b != 0)
            .map(|&b| b as char)
            .collect();
        s.chars().rev().collect()
    }

    /// Get the platform as a string (reversed byte order)
    pub fn platform_string(&self) -> String {
        let mut platform = self.platform;
        platform[3] = 0;
        let s: String = platform.iter()
            .take_while(|&&b| b != 0)
            .map(|&b| b as char)
            .collect();
        s.chars().rev().collect()
    }

    /// Get the locale as a string (reversed byte order)
    pub fn locale_string(&self) -> String {
        let s: String = self.country.iter()
            .map(|&b| b as char)
            .collect();
        s.chars().rev().collect()
    }

    /// Get the username as a string
    pub fn username_string(&self) -> String {
        String::from_utf8_lossy(&self.username).to_string()
    }
}

/// Logon Proof received from client (CMD_AUTH_LOGON_PROOF)
#[derive(Debug, Clone)]
pub struct AuthLogonProofClient {
    pub a: [u8; 32],         // Client public ephemeral
    pub m1: [u8; 20],        // Client proof
    pub crc_hash: [u8; 20],  // Client version proof
    pub number_of_keys: u8,
    pub security_flags: u8,
}

impl AuthLogonProofClient {
    pub const SIZE_WITHOUT_PIN: usize = 32 + 20 + 20 + 1 + 1; // = 74
    pub const PIN_DATA_SIZE: usize = 16 + 20; // salt(16) + hash(20) = 36
    pub const SIZE_WITH_PIN: usize = Self::SIZE_WITHOUT_PIN + Self::PIN_DATA_SIZE;

    pub fn from_bytes(data: &[u8], with_pin: bool) -> Option<Self> {
        let expected_size = if with_pin {
            Self::SIZE_WITH_PIN
        } else {
            Self::SIZE_WITHOUT_PIN
        };

        if data.len() < expected_size {
            return None;
        }

        let mut a = [0u8; 32];
        a.copy_from_slice(&data[0..32]);

        let mut m1 = [0u8; 20];
        m1.copy_from_slice(&data[32..52]);

        let mut crc_hash = [0u8; 20];
        crc_hash.copy_from_slice(&data[52..72]);

        let number_of_keys = data[72];
        let security_flags = data[73];

        Some(AuthLogonProofClient {
            a,
            m1,
            crc_hash,
            number_of_keys,
            security_flags,
        })
    }
}

/// PIN data from client (classic only)
#[derive(Debug, Clone)]
pub struct AuthLogonPinData {
    pub salt: [u8; 16],
    pub hash: [u8; 20],
}

impl AuthLogonPinData {
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < 36 {
            return None;
        }
        let mut salt = [0u8; 16];
        salt.copy_from_slice(&data[0..16]);
        let mut hash = [0u8; 20];
        hash.copy_from_slice(&data[16..36]);
        Some(AuthLogonPinData { salt, hash })
    }
}

/// Logon Proof sent to client (post-2.x builds)
#[derive(Debug, Clone)]
pub struct AuthLogonProofServer {
    pub cmd: u8,
    pub error: u8,
    pub m2: [u8; 20],
    pub account_flags: u32,
    pub survey_id: u32,
    pub unk_flags: u16,
}

impl AuthLogonProofServer {
    pub const SIZE: usize = 1 + 1 + 20 + 4 + 4 + 2; // = 32

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = ByteBuffer::with_capacity(Self::SIZE);
        buf.write_u8(self.cmd);
        buf.write_u8(self.error);
        buf.append(&self.m2);
        buf.write_u32(self.account_flags);
        buf.write_u32(self.survey_id);
        buf.write_u16(self.unk_flags);
        buf.contents().to_vec()
    }
}

/// Logon Proof sent to client (1.x builds, build <= 6005)
#[derive(Debug, Clone)]
pub struct AuthLogonProofServerLegacy {
    pub cmd: u8,
    pub error: u8,
    pub m2: [u8; 20],
    pub login_flags: u32,
}

impl AuthLogonProofServerLegacy {
    pub const SIZE: usize = 1 + 1 + 20 + 4; // = 26

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = ByteBuffer::with_capacity(Self::SIZE);
        buf.write_u8(self.cmd);
        buf.write_u8(self.error);
        buf.append(&self.m2);
        buf.write_u32(self.login_flags);
        buf.contents().to_vec()
    }
}

/// Reconnect Proof received from client
#[derive(Debug, Clone)]
pub struct AuthReconnectProofClient {
    pub r1: [u8; 16],
    pub r2: [u8; 20],
    pub r3: [u8; 20],
    pub number_of_keys: u8,
}

impl AuthReconnectProofClient {
    pub const SIZE: usize = 16 + 20 + 20 + 1; // = 57 (cmd already read)

    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < Self::SIZE {
            return None;
        }

        let mut r1 = [0u8; 16];
        r1.copy_from_slice(&data[0..16]);

        let mut r2 = [0u8; 20];
        r2.copy_from_slice(&data[16..36]);

        let mut r3 = [0u8; 20];
        r3.copy_from_slice(&data[36..56]);

        let number_of_keys = data[56];

        Some(AuthReconnectProofClient {
            r1,
            r2,
            r3,
            number_of_keys,
        })
    }
}
