// AuthSocket - Authentication socket handler
// Rust equivalent of AuthSocket.h/AuthSocket.cpp
//
// Handles the full WoW authentication flow:
// 1. Client sends LogonChallenge -> Server responds with SRP6 challenge
// 2. Client sends LogonProof -> Server verifies and responds
// 3. Client requests RealmList -> Server sends available realms
// OR for reconnection:
// 1. ReconnectChallenge -> random proof
// 2. ReconnectProof -> verify session

use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{timeout, Duration};

use mangos_shared::auth::{BigNumber, Sha1Hash, SRP6, base32_decode};
use mangos_shared::auth::hmac_sha1::hmac_sha1;
use mangos_shared::config::get_config;
use mangos_shared::database::{Database, FieldExt};
use mangos_shared::util::ByteBuffer;
use mangos_shared::{SEC_ADMINISTRATOR, SEC_PLAYER, AccountTypes, RealmFlags, LOGIN_TYPE_REALMD};

use crate::auth_codes::*;
use crate::protocol::*;
use crate::realm_list::{self, RealmList, find_build_info, get_realm_category_id};

/// Session status state machine
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionStatus {
    Challenge,
    LogonProof,
    ReconProof,
    Patch,
    Authed,
    Closed,
}

/// Handle a single authentication session
pub async fn handle_client(
    mut stream: TcpStream,
    addr: SocketAddr,
    db: Arc<Database>,
    realm_list: Arc<tokio::sync::RwLock<RealmList>>,
) {
    tracing::debug!("New connection from {}", addr);

    let mut status = SessionStatus::Challenge;
    let mut srp = SRP6::new();
    let mut reconnect_proof = BigNumber::new();
    let mut login = String::new();
    let mut safe_login = String::new();
    let mut token = String::new();
    let mut os = String::new();
    let mut platform = String::new();
    let mut locale = String::new();
    let mut safe_locale = String::new();
    let mut build: u16 = 0;
    let mut account_security_level: AccountTypes = SEC_PLAYER;
    let mut server_security_salt = BigNumber::new();
    let mut grid_seed: u32 = 0;
    let mut prompt_pin = false;

    // Connection timeout: 30 seconds for initial data
    let timeout_duration = Duration::from_secs(30);

    loop {
        // Read the command byte
        let cmd_byte = match timeout(timeout_duration, stream.read_u8()).await {
            Ok(Ok(byte)) => byte,
            Ok(Err(e)) => {
                tracing::debug!("Connection closed from {}: {}", addr, e);
                return;
            }
            Err(_) => {
                tracing::debug!("Connection timeout from {}", addr);
                return;
            }
        };

        let cmd = match AuthCmd::from_u8(cmd_byte) {
            Some(cmd) => cmd,
            None => {
                tracing::debug!("Unknown command {:02x} from {}", cmd_byte, addr);
                return;
            }
        };

        tracing::trace!("Got command {:?} from {} (status: {:?})", cmd, addr, status);

        // Check if the command is valid for the current status
        let expected_status = match cmd {
            AuthCmd::LogonChallenge => SessionStatus::Challenge,
            AuthCmd::LogonProof => SessionStatus::LogonProof,
            AuthCmd::ReconnectChallenge => SessionStatus::Challenge,
            AuthCmd::ReconnectProof => SessionStatus::ReconProof,
            AuthCmd::RealmList => SessionStatus::Authed,
            AuthCmd::XferAccept | AuthCmd::XferResume | AuthCmd::XferCancel => SessionStatus::Patch,
            _ => SessionStatus::Closed,
        };

        if expected_status != status {
            tracing::debug!(
                "Unauthorized command {:?} in state {:?} from {}",
                cmd,
                status,
                addr
            );
            return;
        }

        let result = match cmd {
            AuthCmd::LogonChallenge => {
                handle_logon_challenge(
                    &mut stream,
                    &addr,
                    &db,
                    &mut status,
                    &mut srp,
                    &mut login,
                    &mut safe_login,
                    &mut token,
                    &mut os,
                    &mut platform,
                    &mut locale,
                    &mut safe_locale,
                    &mut build,
                    &mut account_security_level,
                    &mut server_security_salt,
                    &mut grid_seed,
                    &mut prompt_pin,
                )
                .await
            }
            AuthCmd::LogonProof => {
                handle_logon_proof(
                    &mut stream,
                    &addr,
                    &db,
                    &mut status,
                    &mut srp,
                    &login,
                    &safe_login,
                    &safe_locale,
                    &token,
                    &os,
                    &platform,
                    build,
                    prompt_pin,
                    &server_security_salt,
                    grid_seed,
                    &mut account_security_level,
                )
                .await
            }
            AuthCmd::ReconnectChallenge => {
                handle_reconnect_challenge(
                    &mut stream,
                    &addr,
                    &db,
                    &mut status,
                    &mut srp,
                    &mut login,
                    &mut safe_login,
                    &mut build,
                    &mut reconnect_proof,
                )
                .await
            }
            AuthCmd::ReconnectProof => {
                handle_reconnect_proof(
                    &mut stream,
                    &addr,
                    &db,
                    &mut status,
                    &srp,
                    &login,
                    &reconnect_proof,
                    build,
                    &os,
                )
                .await
            }
            AuthCmd::RealmList => {
                handle_realm_list(
                    &mut stream,
                    &addr,
                    &db,
                    &realm_list,
                    &safe_login,
                    &login,
                    build,
                    account_security_level,
                )
                .await
            }
            AuthCmd::XferResume => {
                // Skip 8 bytes
                let mut buf = [0u8; 8];
                let _ = stream.read_exact(&mut buf).await;
                Ok(())
            }
            AuthCmd::XferCancel => {
                return;
            }
            AuthCmd::XferAccept => Ok(()),
            _ => {
                tracing::debug!("Unhandled command {:?}", cmd);
                return;
            }
        };

        if result.is_err() {
            tracing::debug!("Handler failed for {:?} from {}", cmd, addr);
            return;
        }

        if status == SessionStatus::Closed {
            return;
        }
    }
}

/// Handle CMD_AUTH_LOGON_CHALLENGE
async fn handle_logon_challenge(
    stream: &mut TcpStream,
    addr: &SocketAddr,
    db: &Database,
    status: &mut SessionStatus,
    srp: &mut SRP6,
    login: &mut String,
    safe_login: &mut String,
    token: &mut String,
    os: &mut String,
    platform: &mut String,
    locale: &mut String,
    safe_locale: &mut String,
    build: &mut u16,
    account_security_level: &mut AccountTypes,
    server_security_salt: &mut BigNumber,
    grid_seed: &mut u32,
    prompt_pin: &mut bool,
) -> Result<(), anyhow::Error> {
    // Read header (3 bytes: error + size)
    let mut header_buf = [0u8; AuthLogonChallengeHeader::SIZE];
    stream.read_exact(&mut header_buf).await?;

    let header = AuthLogonChallengeHeader::from_bytes(&header_buf)
        .ok_or_else(|| anyhow::anyhow!("Invalid logon challenge header"))?;

    let remaining = header.size as usize;
    if remaining < AuthLogonChallengeBody::MIN_SIZE - AUTH_LOGON_MAX_NAME {
        return Err(anyhow::anyhow!("Logon challenge body too small"));
    }

    // Session is closed unless overridden
    *status = SessionStatus::Closed;

    // Read the body
    let mut body_buf = vec![0u8; remaining];
    stream.read_exact(&mut body_buf).await?;

    let body = AuthLogonChallengeBody::from_bytes(&body_buf)
        .ok_or_else(|| anyhow::anyhow!("Invalid logon challenge body"))?;

    if body.username_len as usize > AUTH_LOGON_MAX_NAME {
        return Err(anyhow::anyhow!("Username too long"));
    }

    tracing::trace!("Logon challenge from '{}' build {}", body.username_string(), body.build);

    // Store client info
    *login = body.username_string();
    *build = body.build;
    *os = body.os_string();
    *platform = body.platform_string();
    *locale = body.locale_string();

    // Escape for SQL safety
    *safe_login = Database::escape_string(login);
    *safe_locale = Database::escape_string(locale);
    let escaped_os = Database::escape_string(os);
    *os = escaped_os;

    let mut pkt = ByteBuffer::new();
    pkt.write_u8(AuthCmd::LogonChallenge as u8);
    pkt.write_u8(0x00);

    // Check IP ban
    let ip_str = addr.ip().to_string();
    let ip_ban_sql = format!(
        "SELECT expires_at FROM ip_banned \
         WHERE (expires_at = banned_at OR expires_at > UNIX_TIMESTAMP()) AND ip = '{}'",
        Database::escape_string(&ip_str)
    );

    if let Ok(Some(_)) = db.query_one(&ip_ban_sql).await {
        pkt.write_u8(AuthLogonResult::FailedFailNoaccess as u8);
        tracing::info!("Banned ip {} tries to login!", ip_str);
        stream.write_all(pkt.contents()).await?;
        return Ok(());
    }

    // Get account details
    let account_sql = format!(
        "SELECT id, CAST(locked AS SIGNED) AS locked, lockedIp, \
         CAST(gmlevel AS SIGNED) AS gmlevel, v, s, token \
         FROM account WHERE username = '{}'",
        safe_login
    );

    match db.query_one(&account_sql).await? {
        Some(row) => {
            let locked: u8 = row.get_u8(1);

            // Check IP lock
            if locked == 1 {
                let locked_ip: String = row.get_string(2);
                tracing::debug!("Account '{}' is locked to IP '{}'", login, locked_ip);
                if locked_ip != ip_str {
                    tracing::debug!("Account IP differs, rejecting");
                    pkt.write_u8(AuthLogonResult::FailedSuspended as u8);
                    stream.write_all(pkt.contents()).await?;
                    return Ok(());
                }
            }

            let database_v: String = row.get_string(4);
            let database_s: String = row.get_string(5);

            // Set SRP6 verifier and salt
            if !srp.set_verifier(&database_v) || !srp.set_salt(&database_s) {
                pkt.write_u8(AuthLogonResult::FailedFailNoaccess as u8);
                tracing::debug!("Broken v/s values for account {}!", login);
                stream.write_all(pkt.contents()).await?;
                return Ok(());
            }

            // Check account ban
            let account_id: u32 = row.get_u32(0);
            let ban_sql = format!(
                "SELECT banned_at, expires_at FROM account_banned \
                 WHERE account_id = {} AND CAST(active AS SIGNED) = 1 AND \
                 (expires_at > UNIX_TIMESTAMP() OR expires_at = banned_at)",
                account_id
            );

            if let Ok(Some(ban_row)) = db.query_one(&ban_sql).await {
                let banned_at: u64 = ban_row.get_u64(0);
                let expires_at: u64 = ban_row.get_u64(1);

                if banned_at == expires_at {
                    pkt.write_u8(AuthLogonResult::FailedBanned as u8);
                    tracing::info!("Banned account {} tries to login!", login);
                } else {
                    pkt.write_u8(AuthLogonResult::FailedSuspended as u8);
                    tracing::info!("Temporarily banned account {} tries to login!", login);
                }
                stream.write_all(pkt.contents()).await?;
                return Ok(());
            }

            // Generate SRP6 challenge
            srp.calculate_host_public_ephemeral();

            pkt.write_u8(AuthLogonResult::Success as u8);

            // B (32 bytes)
            pkt.append(&srp.get_host_public_ephemeral().as_byte_array(32));

            // g length (1) + g value
            pkt.write_u8(1);
            pkt.append(&srp.get_generator_modulo().as_byte_array(0));

            // N length (32) + N value (32 bytes)
            pkt.write_u8(32);
            pkt.append(&srp.get_prime().as_byte_array(32));

            // Salt (32 bytes)
            let mut salt_bn = BigNumber::new();
            salt_bn.set_hex_str(&database_s);
            pkt.append(&salt_bn.as_byte_array(0));

            // Version challenge (16 bytes)
            pkt.append(&VERSION_CHALLENGE);

            // Security flags
            *token = row.get_string(6);
            let mut security_flags: u8 = 0;

            if !token.is_empty() && *build >= 8606 {
                // Authenticator was added in 2.4.3
                security_flags = SecurityFlags::Authenticator as u8;
            }

            if !token.is_empty() && *build <= 6141 {
                security_flags = SecurityFlags::Pin as u8;
            }

            pkt.write_u8(security_flags);

            if security_flags & SecurityFlags::Pin as u8 != 0 {
                *grid_seed = 0;
                pkt.write_u32(*grid_seed);
                server_security_salt.set_rand(16 * 8);
                pkt.append(&server_security_salt.as_byte_array(16)[..16]);
                *prompt_pin = true;
            }

            if security_flags & SecurityFlags::Unk as u8 != 0 {
                pkt.write_u8(0);
                pkt.write_u8(0);
                pkt.write_u8(0);
                pkt.write_u8(0);
                pkt.write_u64(0);
            }

            if security_flags & SecurityFlags::Authenticator as u8 != 0 {
                pkt.write_u8(1);
            }

            let sec_level: u8 = row.get_u8(3);
            *account_security_level = if sec_level <= SEC_ADMINISTRATOR {
                sec_level
            } else {
                SEC_ADMINISTRATOR
            };

            *status = SessionStatus::LogonProof;
        }
        None => {
            pkt.write_u8(AuthLogonResult::FailedUnknownAccount as u8);
        }
    }

    stream.write_all(pkt.contents()).await?;
    Ok(())
}

/// Handle CMD_AUTH_LOGON_PROOF
async fn handle_logon_proof(
    stream: &mut TcpStream,
    addr: &SocketAddr,
    db: &Database,
    status: &mut SessionStatus,
    srp: &mut SRP6,
    login: &str,
    safe_login: &str,
    safe_locale: &str,
    token: &str,
    os: &str,
    platform: &str,
    build: u16,
    prompt_pin: bool,
    _server_security_salt: &BigNumber,
    _grid_seed: u32,
    account_security_level: &mut AccountTypes,
) -> Result<(), anyhow::Error> {
    // Read the proof data
    let proof_size = if prompt_pin {
        AuthLogonProofClient::SIZE_WITH_PIN
    } else {
        AuthLogonProofClient::SIZE_WITHOUT_PIN
    };

    let mut proof_buf = vec![0u8; proof_size];
    stream.read_exact(&mut proof_buf).await?;

    let proof = AuthLogonProofClient::from_bytes(&proof_buf, prompt_pin)
        .ok_or_else(|| anyhow::anyhow!("Invalid logon proof"))?;

    *status = SessionStatus::Closed;

    // Check build validity
    if find_build_info(build).is_none() {
        let mut pkt = ByteBuffer::new();
        pkt.write_u8(AuthCmd::LogonChallenge as u8);
        pkt.write_u8(0x00);
        pkt.write_u8(AuthLogonResult::FailedVersionInvalid as u8);
        tracing::info!("Account {} tried to login with invalid client version {}!", login, build);
        stream.write_all(pkt.contents()).await?;
        return Ok(());
    }

    // Calculate session key
    if !srp.calculate_session_key(&proof.a) {
        tracing::info!("Session calculation failed for account {}!", login);
        return Ok(());
    }

    srp.hash_session_key();
    srp.calculate_proof(login);

    // Check if proof matches (password correct)
    if srp.proof(&proof.m1) {
        // Proof matched = password incorrect in the C++ code's logic
        // (C++ Proof() returns false on match)

        // Handle authenticator token for builds > 6141
        if build > 6141 && (proof.security_flags & SecurityFlags::Authenticator as u8 != 0 || !token.is_empty()) {
            // Read authenticator token
            let mut pin_count_buf = [0u8; 1];
            if stream.read_exact(&mut pin_count_buf).await.is_err() {
                send_logon_proof_error(stream, build).await?;
                return Ok(());
            }
            let pin_count = pin_count_buf[0];

            if pin_count > 16 {
                send_logon_proof_error(stream, build).await?;
                return Ok(());
            }

            let mut keys = vec![0u8; pin_count as usize];
            if stream.read_exact(&mut keys).await.is_err() {
                send_logon_proof_error(stream, build).await?;
                return Ok(());
            }

            let client_token: i32 = String::from_utf8_lossy(&keys)
                .parse()
                .unwrap_or(-1);
            let server_token = generate_token(token);

            if server_token != client_token {
                tracing::info!(
                    "Account {} tried to login with wrong pincode! Given {} Expected {}",
                    login,
                    client_token,
                    server_token
                );
                send_logon_proof_error(stream, build).await?;
                return Ok(());
            }

            // Token verified, proceed to finalize
            verify_and_finalize(stream, addr, db, status, srp, login, safe_login, safe_locale, os, platform, build, &proof).await?;
            return Ok(());
        }

        // No authenticator, just wrong password
        send_logon_proof_error(stream, build).await?;
        tracing::info!("Account {} tried to login with wrong password!", login);

        // Handle failed login counting
        handle_failed_login(db, login, safe_login, addr).await;
        return Ok(());
    }

    // Proof did not match = password correct
    verify_and_finalize(stream, addr, db, status, srp, login, safe_login, safe_locale, os, platform, build, &proof).await?;
    Ok(())
}

/// Send an error response for logon proof
async fn send_logon_proof_error(stream: &mut TcpStream, build: u16) -> Result<(), anyhow::Error> {
    if build > 6005 {
        let response: [u8; 4] = [
            AuthCmd::LogonProof as u8,
            AuthLogonResult::FailedUnknownAccount as u8,
            0,
            0,
        ];
        stream.write_all(&response).await?;
    } else {
        let response: [u8; 2] = [
            AuthCmd::LogonProof as u8,
            AuthLogonResult::FailedUnknownAccount as u8,
        ];
        stream.write_all(&response).await?;
    }
    Ok(())
}

/// Handle failed login attempt counting and auto-banning
async fn handle_failed_login(db: &Database, login: &str, safe_login: &str, addr: &SocketAddr) {
    let max_wrong = {
        let config = get_config().lock();
        config.get_int_default("WrongPass.MaxCount", 0) as u32
    };

    if max_wrong == 0 {
        return;
    }

    let _ = db
        .execute(&format!(
            "UPDATE account SET failed_logins = failed_logins + 1 WHERE username = '{}'",
            safe_login
        ))
        .await;

    let sql = format!(
        "SELECT id, CAST(failed_logins AS SIGNED) AS failed_logins FROM account WHERE username = '{}'",
        safe_login
    );

    if let Ok(Some(row)) = db.query_one(&sql).await {
        let failed_logins: u32 = row.get_u32(1);

        if failed_logins >= max_wrong {
            let (ban_time, ban_type) = {
                let config = get_config().lock();
                (
                    config.get_int_default("WrongPass.BanTime", 600) as u32,
                    config.get_bool_default("WrongPass.BanType", false),
                )
            };

            if ban_type {
                let acc_id: u32 = row.get_u32(0);
                let _ = db
                    .execute(&format!(
                        "INSERT INTO account_banned(account_id, banned_at, expires_at, banned_by, reason, active) \
                         VALUES ('{}', UNIX_TIMESTAMP(), UNIX_TIMESTAMP()+'{}', 'MaNGOS realmd', 'Failed login autoban', 1)",
                        acc_id, ban_time
                    ))
                    .await;
                tracing::info!(
                    "Account {} got banned for {} seconds (failed {} times)",
                    login,
                    ban_time,
                    failed_logins
                );
            } else {
                let ip = Database::escape_string(&addr.ip().to_string());
                let _ = db
                    .execute(&format!(
                        "INSERT INTO ip_banned VALUES ('{}', UNIX_TIMESTAMP(), UNIX_TIMESTAMP()+'{}', 'MaNGOS realmd', 'Failed login autoban')",
                        ip, ban_time
                    ))
                    .await;
                tracing::info!(
                    "IP {} got banned for {} seconds (account {} failed {} times)",
                    addr.ip(),
                    ban_time,
                    login,
                    failed_logins
                );
            }
        }
    }
}

/// Verify client version and finalize authentication
async fn verify_and_finalize(
    stream: &mut TcpStream,
    addr: &SocketAddr,
    db: &Database,
    status: &mut SessionStatus,
    srp: &mut SRP6,
    login: &str,
    safe_login: &str,
    safe_locale: &str,
    os: &str,
    platform: &str,
    build: u16,
    proof: &AuthLogonProofClient,
) -> Result<(), anyhow::Error> {
    // Verify version
    if !verify_version(build, os, &proof.a, &proof.crc_hash, false) {
        tracing::info!("Account {} tried to login with modified client!", login);
        let response: [u8; 2] = [
            AuthCmd::LogonProof as u8,
            AuthLogonResult::FailedVersionInvalid as u8,
        ];
        stream.write_all(&response).await?;
        return Ok(());
    }

    tracing::info!("User '{}' successfully authenticated", login);

    // Update session in database
    let k_hex = srp.get_strong_session_key().as_hex_str();
    let _ = db
        .execute(&format!(
            "UPDATE account SET sessionkey = '{}', locale = '{}', failed_logins = 0, os = '{}', platform = '{}' \
             WHERE username = '{}'",
            k_hex, safe_locale, os, platform, safe_login
        ))
        .await;

    // Log the login
    if let Ok(Some(row)) = db
        .query_one(&format!(
            "SELECT id FROM account WHERE username = '{}'",
            safe_login
        ))
        .await
    {
        let account_id: u32 = row.get_u32(0);
        let ip = Database::escape_string(&addr.ip().to_string());
        let _ = db
            .execute(&format!(
                "INSERT INTO account_logons(accountId, ip, loginTime, loginSource) \
                 VALUES('{}', '{}', NOW(), '{}')",
                account_id, ip, LOGIN_TYPE_REALMD
            ))
            .await;
    }

    // Send proof to client
    let mut sha = Sha1Hash::new();
    srp.finalize(&mut sha);
    send_proof(stream, build, &sha).await?;

    *status = SessionStatus::Authed;
    Ok(())
}

/// Send the logon proof response to the client
async fn send_proof(
    stream: &mut TcpStream,
    build: u16,
    sha: &Sha1Hash,
) -> Result<(), anyhow::Error> {
    match build {
        5875 | 6005 | 6141 => {
            // 1.12.x client
            let proof = AuthLogonProofServerLegacy {
                cmd: AuthCmd::LogonProof as u8,
                error: 0,
                m2: *sha.get_digest(),
                login_flags: 0x00,
            };
            stream.write_all(&proof.to_bytes()).await?;
        }
        _ => {
            // 2.x+ client
            let proof = AuthLogonProofServer {
                cmd: AuthCmd::LogonProof as u8,
                error: 0,
                m2: *sha.get_digest(),
                account_flags: AccountFlags::ProPass as u32,
                survey_id: 0,
                unk_flags: 0,
            };
            stream.write_all(&proof.to_bytes()).await?;
        }
    }
    Ok(())
}

/// Handle CMD_AUTH_RECONNECT_CHALLENGE
async fn handle_reconnect_challenge(
    stream: &mut TcpStream,
    addr: &SocketAddr,
    db: &Database,
    status: &mut SessionStatus,
    srp: &mut SRP6,
    login: &mut String,
    safe_login: &mut String,
    build: &mut u16,
    reconnect_proof: &mut BigNumber,
) -> Result<(), anyhow::Error> {
    // Read header
    let mut header_buf = [0u8; AuthLogonChallengeHeader::SIZE];
    stream.read_exact(&mut header_buf).await?;

    let header = AuthLogonChallengeHeader::from_bytes(&header_buf)
        .ok_or_else(|| anyhow::anyhow!("Invalid reconnect challenge header"))?;

    let remaining = header.size as usize;

    *status = SessionStatus::Closed;

    // Read body
    let mut body_buf = vec![0u8; remaining];
    stream.read_exact(&mut body_buf).await?;

    let body = AuthLogonChallengeBody::from_bytes(&body_buf)
        .ok_or_else(|| anyhow::anyhow!("Invalid reconnect challenge body"))?;

    if body.username_len > 10 {
        return Err(anyhow::anyhow!("Username too long for reconnect"));
    }

    *login = body.username_string();
    *safe_login = Database::escape_string(login);
    *build = body.build;

    // Look up session key
    let sql = format!(
        "SELECT sessionkey FROM account WHERE username = '{}'",
        safe_login
    );

    match db.query_one(&sql).await? {
        Some(row) => {
            let session_key: String = row.get_string(0);
            srp.set_strong_session_key(&session_key);
        }
        None => {
            tracing::error!("User {} tried to reconnect but no session key found", login);
            return Err(anyhow::anyhow!("No session key"));
        }
    }

    *status = SessionStatus::ReconProof;

    // Send response
    let mut pkt = ByteBuffer::new();
    pkt.write_u8(AuthCmd::ReconnectChallenge as u8);
    pkt.write_u8(0x00);

    reconnect_proof.set_rand(16 * 8);
    pkt.append(&reconnect_proof.as_byte_array(16)[..16]);
    pkt.append(&VERSION_CHALLENGE);

    stream.write_all(pkt.contents()).await?;
    Ok(())
}

/// Handle CMD_AUTH_RECONNECT_PROOF
async fn handle_reconnect_proof(
    stream: &mut TcpStream,
    _addr: &SocketAddr,
    _db: &Database,
    status: &mut SessionStatus,
    srp: &SRP6,
    login: &str,
    reconnect_proof: &BigNumber,
    build: u16,
    os: &str,
) -> Result<(), anyhow::Error> {
    let mut proof_buf = [0u8; AuthReconnectProofClient::SIZE];
    stream.read_exact(&mut proof_buf).await?;

    let proof = AuthReconnectProofClient::from_bytes(&proof_buf)
        .ok_or_else(|| anyhow::anyhow!("Invalid reconnect proof"))?;

    *status = SessionStatus::Closed;

    let k = srp.get_strong_session_key();
    if login.is_empty() || reconnect_proof.get_num_bytes() == 0 || k.get_num_bytes() == 0 {
        return Ok(());
    }

    let mut t1 = BigNumber::new();
    t1.set_binary(&proof.r1);

    let mut sha = Sha1Hash::new();
    sha.initialize();
    sha.update_data(login);
    sha.update_big_numbers(&[&t1, reconnect_proof, k]);
    sha.finalize();

    if sha.get_digest()[..] == proof.r2[..] {
        // Verify version
        if !verify_version(build, os, &proof.r1, &proof.r3, true) {
            let mut pkt = ByteBuffer::new();
            pkt.write_u8(AuthCmd::ReconnectProof as u8);
            pkt.write_u8(AuthLogonResult::FailedVersionInvalid as u8);
            stream.write_all(pkt.contents()).await?;
            return Ok(());
        }

        let mut pkt = ByteBuffer::new();
        pkt.write_u8(AuthCmd::ReconnectProof as u8);
        pkt.write_u8(AuthLogonResult::Success as u8);
        pkt.write_u16(0x00);
        stream.write_all(pkt.contents()).await?;

        *status = SessionStatus::Authed;
    } else {
        tracing::error!("User {} tried to reconnect, but session invalid", login);
    }

    Ok(())
}

/// Handle CMD_REALM_LIST
async fn handle_realm_list(
    stream: &mut TcpStream,
    _addr: &SocketAddr,
    db: &Database,
    realm_list: &Arc<tokio::sync::RwLock<RealmList>>,
    safe_login: &str,
    login: &str,
    build: u16,
    account_security_level: AccountTypes,
) -> Result<(), anyhow::Error> {
    // Skip 4 bytes of padding from client
    let mut skip_buf = [0u8; 4];
    stream.read_exact(&mut skip_buf).await?;

    // Get account ID and GM level
    let sql = format!(
        "SELECT id, CAST(gmlevel AS SIGNED) AS gmlevel FROM account WHERE username = '{}'",
        safe_login
    );

    let (account_id, security_level) = match db.query_one(&sql).await? {
        Some(row) => (row.get_u32(0), row.get_u8(1)),
        None => {
            tracing::error!("User {} not found in database for realm list", login);
            return Err(anyhow::anyhow!("Account not found"));
        }
    };

    // Update realm list if needed
    {
        let mut rl = realm_list.write().await;
        rl.update_if_needed(db).await;
    }

    // Build realm list packet - clone realm data to avoid holding lock across await
    let realms_snapshot = {
        let rl = realm_list.read().await;
        rl.realms().clone()
    };

    let mut pkt = ByteBuffer::new();
    load_realm_list(&mut pkt, &realms_snapshot, account_id, security_level, build, account_security_level, db).await;

    // Send header + realm list
    let mut hdr = ByteBuffer::new();
    hdr.write_u8(AuthCmd::RealmList as u8);
    hdr.write_u16(pkt.size() as u16);
    hdr.append(pkt.contents());

    stream.write_all(hdr.contents()).await?;
    Ok(())
}

/// Build the realm list packet
async fn load_realm_list(
    pkt: &mut ByteBuffer,
    realms: &std::collections::BTreeMap<String, realm_list::Realm>,
    account_id: u32,
    security_level: u8,
    build: u16,
    account_security_level: AccountTypes,
    db: &Database,
) {
    // Count eligible realms
    let eligible_count = realms
        .values()
        .filter(|r| r.allowed_security_level <= security_level)
        .count();

    match build {
        5875 | 6005 | 6141 => {
            // 1.12.x client format
            pkt.write_u32(0); // unused
            pkt.write_u8(eligible_count as u8);

            for (name, realm) in realms {
                // Skip realms that require higher security
                if security_level == 0 && realm.allowed_security_level > 0 {
                    continue;
                }

                // Get character count
                let char_count = get_char_count(db, realm.id, account_id).await;

                let ok_build = realm.realm_builds.contains(&(build as u32));
                let build_info = if ok_build {
                    find_build_info(build)
                } else {
                    None
                };
                let build_info = build_info.unwrap_or(&realm.realm_build_info);

                let mut realm_flags = realm.realm_flags;

                // Append version to name for SPECIFYBUILD flag (1.x doesn't support it natively)
                let display_name = if realm_flags & RealmFlags::REALM_FLAG_SPECIFYBUILD != 0 {
                    format!(
                        "{} ({},{},{})",
                        name,
                        build_info.major_version,
                        build_info.minor_version,
                        build_info.bugfix_version
                    )
                } else {
                    name.clone()
                };

                if !ok_build || realm.allowed_security_level > account_security_level {
                    realm_flags |= RealmFlags::REALM_FLAG_OFFLINE;
                }

                let category_id = get_realm_category_id(build, realm.timezone);

                pkt.write_u32(realm.icon as u32);
                pkt.write_u8(realm_flags);
                pkt.write_string(&display_name);
                pkt.write_string(&realm.address);
                pkt.write_f32(realm.population_level);
                pkt.write_u8(char_count);
                pkt.write_u8(category_id);
                pkt.write_u8(0x00);
            }

            pkt.write_u16(0x0002);
        }
        _ => {
            // 2.x+ client format
            pkt.write_u32(0); // unused
            pkt.write_u16(eligible_count as u16);

            for (name, realm) in realms {
                if security_level == 0 && realm.allowed_security_level > 0 {
                    continue;
                }

                let char_count = get_char_count(db, realm.id, account_id).await;
                let ok_build = realm.realm_builds.contains(&(build as u32));

                let build_info = if ok_build {
                    find_build_info(build)
                } else {
                    None
                };
                let build_info_ref = build_info.unwrap_or(&realm.realm_build_info);

                let lock: u8 = if realm.allowed_security_level > account_security_level {
                    1
                } else {
                    0
                };

                let mut realm_flags = realm.realm_flags;
                if !ok_build {
                    realm_flags |= RealmFlags::REALM_FLAG_OFFLINE;
                }
                if build_info.is_none() {
                    realm_flags &= !RealmFlags::REALM_FLAG_SPECIFYBUILD;
                }

                let category_id = get_realm_category_id(build, realm.timezone);

                pkt.write_u8(realm.icon);
                pkt.write_u8(lock);
                pkt.write_u8(realm_flags);
                pkt.write_string(name);
                pkt.write_string(&realm.address);
                pkt.write_f32(realm.population_level);
                pkt.write_u8(char_count);
                pkt.write_u8(category_id);
                pkt.write_u8(0x2C);

                if realm_flags & RealmFlags::REALM_FLAG_SPECIFYBUILD != 0 {
                    pkt.write_u8(build_info_ref.major_version);
                    pkt.write_u8(build_info_ref.minor_version);
                    pkt.write_u8(build_info_ref.bugfix_version);
                    pkt.write_u16(build);
                }
            }

            pkt.write_u16(0x0010);
        }
    }
}

/// Get the character count for an account on a realm
async fn get_char_count(db: &Database, realm_id: u32, account_id: u32) -> u8 {
    let sql = format!(
        "SELECT CAST(numchars AS SIGNED) AS numchars FROM realmcharacters WHERE realmid = '{}' AND acctid = '{}'",
        realm_id, account_id
    );

    match db.query_one(&sql).await {
        Ok(Some(row)) => row.get_u8(0),
        _ => 0,
    }
}

/// Verify client version hash
fn verify_version(build: u16, os: &str, a: &[u8], version_proof: &[u8], is_reconnect: bool) -> bool {
    let config = get_config().lock();
    if !config.get_bool_default("StrictVersionCheck", false) {
        return true;
    }
    drop(config);

    let zeros = [0u8; 20];
    let version_hash: &[u8; 20];

    if !is_reconnect {
        let build_info = match find_build_info(build) {
            Some(info) => info,
            None => return false,
        };

        let hash = match os {
            "Win" => &build_info.windows_hash,
            "OSX" => &build_info.mac_hash,
            _ => return false,
        };

        if *hash == zeros {
            return true; // not filled serverside
        }

        version_hash = hash;
    } else {
        version_hash = &zeros;
    }

    let mut sha = Sha1Hash::new();
    sha.update_data_bytes(a);
    sha.update_data_bytes(version_hash);
    sha.finalize();

    sha.get_digest()[..] == version_proof[..20.min(version_proof.len())]
}

/// Generate a TOTP token from a base32 key
/// Matches the C++ generateToken function
pub fn generate_token(b32key: &str) -> i32 {
    let decoded = match base32_decode(b32key) {
        Ok(d) => d,
        Err(_) => return -1,
    };

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        / 30;

    let mut challenge = [0u8; 8];
    let mut ts = timestamp;
    for i in (0..8).rev() {
        challenge[i] = (ts & 0xFF) as u8;
        ts >>= 8;
    }

    let hmac_result = hmac_sha1(&decoded, &challenge);

    let offset = (hmac_result[19] & 0x0F) as usize;
    let trunc_hash = ((hmac_result[offset] as u32) << 24)
        | ((hmac_result[offset + 1] as u32) << 16)
        | ((hmac_result[offset + 2] as u32) << 8)
        | (hmac_result[offset + 3] as u32);

    let trunc_hash = trunc_hash & 0x7FFFFFFF;

    (trunc_hash % 1_000_000) as i32
}
