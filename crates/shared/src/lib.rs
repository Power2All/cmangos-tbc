// CMaNGOS TBC - Shared Library
// Rust rewrite of the mangos-tbc shared components

pub mod auth;
pub mod config;
pub mod database;
pub mod log;
pub mod network;
pub mod util;

/// Common type aliases matching the C++ codebase
pub type AccountTypes = u8;

/// Account security levels
pub const SEC_PLAYER: AccountTypes = 0;
pub const SEC_MODERATOR: AccountTypes = 1;
pub const SEC_GAMEMASTER: AccountTypes = 2;
pub const SEC_ADMINISTRATOR: AccountTypes = 3;

/// Login source types
pub const LOGIN_TYPE_REALMD: u32 = 0;
pub const LOGIN_TYPE_MANGOSD: u32 = 1;

/// Realm flags
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RealmFlags {
    None = 0x00,
    Invalid = 0x01,
    Offline = 0x02,
    SpecifyBuild = 0x04,
    // 0x08 unused
    // 0x10 unused
    NewPlayers = 0x20,
    Recommended = 0x40,
}

impl RealmFlags {
    pub const REALM_FLAG_OFFLINE: u8 = 0x02;
    pub const REALM_FLAG_SPECIFYBUILD: u8 = 0x04;
    pub const REALM_FLAG_NEW_PLAYERS: u8 = 0x20;
    pub const REALM_FLAG_RECOMMENDED: u8 = 0x40;
}

/// Realm timezone/zone identifiers
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RealmZone {
    Unknown = 0,
    Development = 1,
    UnitedStates = 2,
    Oceanic = 3,
    LatinAmerica = 4,
    Tournament = 5,
    Korea = 6,
    Tournament2 = 7,
    English = 8,
    German = 9,
    French = 10,
    Spanish = 11,
    Russian = 12,
    Tournament3 = 13,
    Taiwan = 14,
    Tournament4 = 15,
    China = 16,
    Cn1 = 17,
    Cn2 = 18,
    Cn3 = 19,
    Cn4 = 20,
    Cn5 = 21,
    Cn6 = 22,
    Cn7 = 23,
    Cn8 = 24,
    Tournament5 = 25,
    TestServer = 26,
    Tournament6 = 27,
    QaServer = 28,
    Cn9 = 29,
    TestServer2 = 30,
    Cn10 = 31,
    Ctc = 32,
    Cnr = 33,
    Cnr2 = 34,
    Br1 = 35,
    Br2 = 36,
    Br3 = 37,
}

pub const MAX_REALM_ZONES: usize = 38;

/// Minute in seconds
pub const MINUTE: u32 = 60;
