// RealmList - Server realm management
// Rust equivalent of RealmList.h/cpp

use mangos_shared::database::{Database, FieldExt};
use mangos_shared::{AccountTypes, SEC_ADMINISTRATOR, MAX_REALM_ZONES, RealmFlags};
use parking_lot::RwLock;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Build information for supported client versions
#[derive(Debug, Clone)]
pub struct RealmBuildInfo {
    pub build: u16,
    pub major_version: u8,
    pub minor_version: u8,
    pub bugfix_version: u8,
    pub hotfix_version: char,
    pub windows_hash: [u8; 20],
    pub mac_hash: [u8; 20],
}

/// Supported client builds (matching ExpectedRealmdClientBuilds in RealmList.cpp)
pub static EXPECTED_BUILDS: once_cell::sync::Lazy<Vec<RealmBuildInfo>> =
    once_cell::sync::Lazy::new(|| {
        vec![
            RealmBuildInfo {
                build: 13930, major_version: 3, minor_version: 3, bugfix_version: 5,
                hotfix_version: 'a', windows_hash: [0; 20], mac_hash: [0; 20],
            },
            RealmBuildInfo {
                build: 12340, major_version: 3, minor_version: 3, bugfix_version: 5,
                hotfix_version: 'a',
                windows_hash: [
                    0xCD, 0xCB, 0xBD, 0x51, 0x88, 0x31, 0x5E, 0x6B, 0x4D, 0x19,
                    0x44, 0x9D, 0x49, 0x2D, 0xBC, 0xFA, 0xF1, 0x56, 0xA3, 0x47,
                ],
                mac_hash: [
                    0xB7, 0x06, 0xD1, 0x3F, 0xF2, 0xF4, 0x01, 0x88, 0x39, 0x72,
                    0x94, 0x61, 0xE3, 0xF8, 0xA0, 0xE2, 0xB5, 0xFD, 0xC0, 0x34,
                ],
            },
            RealmBuildInfo {
                build: 11723, major_version: 3, minor_version: 3, bugfix_version: 3,
                hotfix_version: 'a', windows_hash: [0; 20], mac_hash: [0; 20],
            },
            RealmBuildInfo {
                build: 11403, major_version: 3, minor_version: 3, bugfix_version: 2,
                hotfix_version: ' ', windows_hash: [0; 20], mac_hash: [0; 20],
            },
            RealmBuildInfo {
                build: 11159, major_version: 3, minor_version: 3, bugfix_version: 0,
                hotfix_version: 'a', windows_hash: [0; 20], mac_hash: [0; 20],
            },
            RealmBuildInfo {
                build: 10505, major_version: 3, minor_version: 2, bugfix_version: 2,
                hotfix_version: 'a', windows_hash: [0; 20], mac_hash: [0; 20],
            },
            RealmBuildInfo {
                build: 9947, major_version: 3, minor_version: 1, bugfix_version: 3,
                hotfix_version: ' ', windows_hash: [0; 20], mac_hash: [0; 20],
            },
            RealmBuildInfo {
                build: 8606, major_version: 2, minor_version: 4, bugfix_version: 3,
                hotfix_version: ' ',
                windows_hash: [
                    0x31, 0x9A, 0xFA, 0xA3, 0xF2, 0x55, 0x96, 0x82, 0xF9, 0xFF,
                    0x65, 0x8B, 0xE0, 0x14, 0x56, 0x25, 0x5F, 0x45, 0x6F, 0xB1,
                ],
                mac_hash: [
                    0xD8, 0xB0, 0xEC, 0xFE, 0x53, 0x4B, 0xC1, 0x13, 0x1E, 0x19,
                    0xBA, 0xD1, 0xD4, 0xC0, 0xE8, 0x13, 0xEE, 0xE4, 0x99, 0x4F,
                ],
            },
            RealmBuildInfo {
                build: 6141, major_version: 1, minor_version: 12, bugfix_version: 3,
                hotfix_version: ' ',
                windows_hash: [
                    0xEB, 0x88, 0x24, 0x3E, 0x94, 0x26, 0xC9, 0xD6, 0x8C, 0x81,
                    0x87, 0xF7, 0xDA, 0xE2, 0x25, 0xEA, 0xF3, 0x88, 0xD8, 0xAF,
                ],
                mac_hash: [0; 20],
            },
            RealmBuildInfo {
                build: 6005, major_version: 1, minor_version: 12, bugfix_version: 2,
                hotfix_version: ' ',
                windows_hash: [
                    0x06, 0x97, 0x32, 0x38, 0x76, 0x56, 0x96, 0x41, 0x48, 0x79,
                    0x28, 0xFD, 0xC7, 0xC9, 0xE3, 0x3B, 0x44, 0x70, 0xC8, 0x80,
                ],
                mac_hash: [0; 20],
            },
            RealmBuildInfo {
                build: 5875, major_version: 1, minor_version: 12, bugfix_version: 1,
                hotfix_version: ' ',
                windows_hash: [
                    0x95, 0xED, 0xB2, 0x7C, 0x78, 0x23, 0xB3, 0x63, 0xCB, 0xDD,
                    0xAB, 0x56, 0xA3, 0x92, 0xE7, 0xCB, 0x73, 0xFC, 0xCA, 0x20,
                ],
                mac_hash: [
                    0x8D, 0x17, 0x3C, 0xC3, 0x81, 0x96, 0x1E, 0xEB, 0xAB, 0xF3,
                    0x36, 0xF5, 0xE6, 0x67, 0x5B, 0x10, 0x1B, 0xB5, 0x13, 0xE5,
                ],
            },
        ]
    });

/// Find build info for a given client build number
pub fn find_build_info(build: u16) -> Option<&'static RealmBuildInfo> {
    // First build is low bound of always accepted range
    if build >= EXPECTED_BUILDS[0].build {
        return Some(&EXPECTED_BUILDS[0]);
    }

    // Continue from 1 with explicit equal check
    EXPECTED_BUILDS.iter().skip(1).find(|b| b.build == build)
}

/// Realm category ID mapping tables by version and zone
static REALM_CATEGORY_IDS: [[u8; MAX_REALM_ZONES]; 4] = [
    // 0 - Alpha
    [0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
    // 1 - Classic
    [0, 1, 1, 5, 1, 1, 1, 1, 1, 2, 3, 5, 1, 1, 1, 1, 1, 1, 1, 2, 1, 1, 1, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
    // 2 - TBC
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 0, 0, 0, 0, 0, 0, 0],
    // 3 - WotLK
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37],
];

/// Get the realm category ID for a given build and timezone
pub fn get_realm_category_id(build: u16, timezone: u8) -> u8 {
    let zone = if (timezone as usize) >= MAX_REALM_ZONES {
        1 // REALM_ZONE_DEVELOPMENT
    } else {
        timezone as usize
    };

    match find_build_info(build) {
        Some(info) => REALM_CATEGORY_IDS[info.major_version as usize][zone],
        None => zone as u8,
    }
}

/// A single realm server entry
#[derive(Debug, Clone)]
pub struct Realm {
    pub id: u32,
    pub address: String,  // "host:port"
    pub icon: u8,
    pub realm_flags: u8,
    pub timezone: u8,
    pub allowed_security_level: AccountTypes,
    pub population_level: f32,
    pub realm_builds: BTreeSet<u32>,
    pub realm_build_info: RealmBuildInfo,
}

/// The realm list manager
/// Thread-safe singleton managing the collection of available realms
pub struct RealmList {
    realms: Arc<RwLock<BTreeMap<String, Realm>>>,
    update_interval: u32,
    next_update_time: i64,
}

impl RealmList {
    pub fn new() -> Self {
        RealmList {
            realms: Arc::new(RwLock::new(BTreeMap::new())),
            update_interval: 0,
            next_update_time: 0,
        }
    }

    /// Initialize the realm list with periodic update interval
    pub async fn initialize(&mut self, update_interval: u32, db: &Database) {
        tracing::debug!("Initializing realm list (update interval: {}s)", update_interval);
        self.update_interval = update_interval;
        self.update_realms(db, true).await;
    }

    /// Update the realm list if the update interval has passed
    pub async fn update_if_needed(&mut self, db: &Database) {
        if self.update_interval == 0 {
            return;
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        if self.next_update_time > now {
            return;
        }

        tracing::debug!("Realm list update interval expired, refreshing from database");
        self.next_update_time = now + self.update_interval as i64;
        self.realms.write().clear();
        self.update_realms(db, false).await;
    }

    /// Load realms from the database
    async fn update_realms(&mut self, db: &Database, init: bool) {
        tracing::debug!("Updating Realm List...");

        let sql = "SELECT id, name, address, port, \
                   CAST(icon AS SIGNED) AS icon, \
                   CAST(realmflags AS SIGNED) AS realmflags, \
                   CAST(timezone AS SIGNED) AS timezone, \
                   CAST(allowedSecurityLevel AS SIGNED) AS allowedSecurityLevel, \
                   population, realmbuilds \
                   FROM realmlist WHERE (realmflags & 1) = 0 ORDER BY name";

        match db.query(sql).await {
            Ok(rows) => {
                tracing::debug!("Realm query returned {} row(s)", rows.len());
                for row in &rows {
                    let id: u32 = row.get_u32(0);
                    let name: String = row.get_string(1);
                    let address: String = row.get_string(2);
                    let port: u32 = row.get_u32(3);
                    let icon: u8 = row.get_u8(4);
                    let mut realm_flags: u8 = row.get_u8(5);
                    let timezone: u8 = row.get_u8(6);
                    let allowed_security_level: u8 = row.get_u8(7);
                    let population: f32 = row.get_f32(8);
                    let builds_str: String = row.get_string(9);

                    if id == 0 {
                        tracing::error!("Realm ID must be > 0 for {}", name);
                        continue;
                    }

                    // Validate flags
                    let valid_flags = RealmFlags::REALM_FLAG_OFFLINE
                        | RealmFlags::REALM_FLAG_NEW_PLAYERS
                        | RealmFlags::REALM_FLAG_RECOMMENDED
                        | RealmFlags::REALM_FLAG_SPECIFYBUILD;

                    if realm_flags & !valid_flags != 0 {
                        tracing::error!(
                            "Realm (id {}, name '{}') has invalid flags, masking",
                            id,
                            name
                        );
                        realm_flags &= valid_flags;
                    }

                    let security_level = if allowed_security_level <= SEC_ADMINISTRATOR {
                        allowed_security_level
                    } else {
                        SEC_ADMINISTRATOR
                    };

                    // Parse build list
                    let mut realm_builds = BTreeSet::new();
                    for token in builds_str.split_whitespace() {
                        if let Ok(build) = token.parse::<u32>() {
                            realm_builds.insert(build);
                        }
                    }

                    // Get build info for the first supported build
                    let first_build = realm_builds.iter().next().copied().unwrap_or(0);
                    let build_info = if first_build > 0 {
                        find_build_info(first_build as u16)
                            .filter(|b| b.build == first_build as u16)
                            .cloned()
                            .unwrap_or_else(|| RealmBuildInfo {
                                build: first_build as u16,
                                major_version: 0,
                                minor_version: 0,
                                bugfix_version: 0,
                                hotfix_version: ' ',
                                windows_hash: [0; 20],
                                mac_hash: [0; 20],
                            })
                    } else {
                        RealmBuildInfo {
                            build: 0,
                            major_version: 0,
                            minor_version: 0,
                            bugfix_version: 0,
                            hotfix_version: ' ',
                            windows_hash: [0; 20],
                            mac_hash: [0; 20],
                        }
                    };

                    let full_address = format!("{}:{}", address, port);

                    tracing::debug!(
                        "Realm '{}': id={} address='{}' icon={} flags=0x{:02X} timezone={} \
                         security={} population={:.1} builds='{}'",
                        name, id, full_address, icon, realm_flags, timezone,
                        security_level, population, builds_str
                    );

                    let realm = Realm {
                        id,
                        address: full_address,
                        icon,
                        realm_flags,
                        timezone,
                        allowed_security_level: security_level,
                        population_level: population,
                        realm_builds,
                        realm_build_info: build_info,
                    };

                    if init {
                        tracing::info!("Added realm id {}, name '{}'", id, name);
                    }

                    self.realms.write().insert(name, realm);
                }
            }
            Err(e) => {
                tracing::error!("Failed to query realm list: {}", e);
            }
        }
    }

    /// Get a read lock on the realms
    pub fn realms(&self) -> parking_lot::RwLockReadGuard<'_, BTreeMap<String, Realm>> {
        self.realms.read()
    }

    /// Get the number of realms
    pub fn size(&self) -> usize {
        self.realms.read().len()
    }
}

impl Default for RealmList {
    fn default() -> Self {
        Self::new()
    }
}
