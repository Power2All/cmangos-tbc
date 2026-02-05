// AuthCodes - Authentication opcodes and result codes
// Rust equivalent of AuthCodes.h

/// Authentication command opcodes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AuthCmd {
    LogonChallenge = 0x00,
    LogonProof = 0x01,
    ReconnectChallenge = 0x02,
    ReconnectProof = 0x03,
    RealmList = 0x10,
    XferInitiate = 0x30,
    XferData = 0x31,
    XferAccept = 0x32,
    XferResume = 0x33,
    XferCancel = 0x34,
}

impl AuthCmd {
    pub fn from_u8(val: u8) -> Option<Self> {
        match val {
            0x00 => Some(AuthCmd::LogonChallenge),
            0x01 => Some(AuthCmd::LogonProof),
            0x02 => Some(AuthCmd::ReconnectChallenge),
            0x03 => Some(AuthCmd::ReconnectProof),
            0x10 => Some(AuthCmd::RealmList),
            0x30 => Some(AuthCmd::XferInitiate),
            0x31 => Some(AuthCmd::XferData),
            0x32 => Some(AuthCmd::XferAccept),
            0x33 => Some(AuthCmd::XferResume),
            0x34 => Some(AuthCmd::XferCancel),
            _ => None,
        }
    }
}

/// Authentication result codes sent to the client
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
#[allow(dead_code)]
pub enum AuthLogonResult {
    Success = 0x00,
    FailedUnknown0 = 0x01,
    FailedUnknown1 = 0x02,
    FailedBanned = 0x03,
    FailedUnknownAccount = 0x04,
    FailedIncorrectPassword = 0x05,
    FailedAlreadyOnline = 0x06,
    FailedNoTime = 0x07,
    FailedDbBusy = 0x08,
    FailedVersionInvalid = 0x09,
    FailedVersionUpdate = 0x0A,
    FailedInvalidServer = 0x0B,
    FailedSuspended = 0x0C,
    FailedFailNoaccess = 0x0D,
    SuccessSurvey = 0x0E,
    FailedParentcontrol = 0x0F,
    FailedLockedEnforced = 0x10,
    FailedTrialEnded = 0x11,
    FailedUseBnet = 0x12,
}

/// Account flags
#[derive(Debug, Clone, Copy)]
#[repr(u32)]
#[allow(dead_code)]
pub enum AccountFlags {
    Gm = 0x00000001,
    Trial = 0x00000008,
    ProPass = 0x00800000,
}

/// Security flags for authenticator/PIN support
#[derive(Debug, Clone, Copy)]
#[repr(u8)]
#[allow(dead_code)]
pub enum SecurityFlags {
    None = 0x00,
    Pin = 0x01,
    Unk = 0x02,
    Authenticator = 0x04,
}

/// Version challenge bytes sent to the client
pub const VERSION_CHALLENGE: [u8; 16] = [
    0xBA, 0xA3, 0x1E, 0x99, 0xA0, 0x0B, 0x21, 0x57,
    0xFC, 0x37, 0x3F, 0xB3, 0x69, 0xCD, 0xD2, 0xF1,
];

/// Maximum username length in the logon challenge
pub const AUTH_LOGON_MAX_NAME: usize = 16;
