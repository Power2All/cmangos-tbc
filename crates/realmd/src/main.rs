// realmd - CMaNGOS TBC Realm/Authentication Server
// Rust rewrite of src/realmd/Main.cpp
//
// This is the authentication server that handles:
// - Client login via SRP6 protocol
// - Realm list distribution
// - Account banning/locking
// - Session key management

mod auth_codes;
mod auth_socket;
mod protocol;
mod realm_list;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use clap::Parser;
use tokio::net::TcpListener;

use mangos_shared::config::get_config;
use mangos_shared::database::Database;
use mangos_shared::log::initialize_logging;
use mangos_shared::MINUTE;

use realm_list::RealmList;

/// Default realm server port
const DEFAULT_REALMSERVER_PORT: i32 = 3724;

/// Default config file name
const DEFAULT_CONFIG: &str = "realmd.conf";

/// CLI arguments
#[derive(Parser, Debug)]
#[command(name = "realmd")]
#[command(about = "CMaNGOS TBC Authentication Server (Rust)")]
#[command(version)]
struct Args {
    /// Configuration file path
    #[arg(short, long, default_value = DEFAULT_CONFIG)]
    config: String,
}

/// Global stop signal
static STOP_EVENT: AtomicBool = AtomicBool::new(false);
static RESTART: AtomicBool = AtomicBool::new(false);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // Load configuration
    {
        let mut config = get_config().lock();
        if !config.set_source(&args.config, "Realmd_") {
            eprintln!("Could not find configuration file {}.", args.config);
            return Err(anyhow::anyhow!("Configuration file not found"));
        }
    }

    // Initialize logging
    let log_dir = {
        let config = get_config().lock();
        let dir = config.get_string_default("LogsDir", "");
        if dir.is_empty() { None } else { Some(dir) }
    };
    initialize_logging(log_dir.as_deref(), "info");

    // Print banner
    tracing::info!("CMaNGOS TBC Auth Server (Rust) v{}", env!("CARGO_PKG_VERSION"));
    tracing::info!("");
    tracing::info!("       _____     __  __       _   _  _____  ____   _____ ");
    tracing::info!("      / ____|   |  \\/  |     | \\ | |/ ____|/ __ \\ / ____|");
    tracing::info!("     | |        | \\  / |     |  \\| | |  __  |  | | (___  ");
    tracing::info!("     | |ontinued| |\\/| | __ _| . ` | | |_ | |  | |\\___ \\ ");
    tracing::info!("     | |____    | |  | |/ _` | |\\  | |__| | |__| |____) |");
    tracing::info!("      \\_____|   |_|  |_| (_| |_| \\_|\\_____|\\____/ \\____/ ");
    tracing::info!("      http://cmangos.net\\__,_|     Doing emulation right!");
    tracing::info!("");
    tracing::info!("Rewritten in Rust for memory safety and performance");
    tracing::info!("");
    tracing::info!("Using configuration file: {}", args.config);
    tracing::info!("<Ctrl-C> to stop.");

    // Initialize database
    let mut login_db = Database::new("Login");
    let db_string = {
        let config = get_config().lock();
        config.get_string("LoginDatabaseInfo")
    };

    if db_string.is_empty() {
        tracing::error!("Database not specified in configuration");
        return Err(anyhow::anyhow!("Database not specified"));
    }

    tracing::info!("Login Database total connections: 2");

    if let Err(e) = login_db.initialize(&db_string).await {
        tracing::error!("Cannot connect to database: {}", e);
        return Err(anyhow::anyhow!("Database connection failed"));
    }

    let db = Arc::new(login_db);

    // Initialize realm list
    let update_interval = {
        let config = get_config().lock();
        config.get_int_default("RealmsStateUpdateDelay", 20) as u32
    };

    let mut realm_list = RealmList::new();
    realm_list.initialize(update_interval, &db).await;

    if realm_list.size() == 0 {
        tracing::error!("No valid realms specified.");
        return Err(anyhow::anyhow!("No realms configured"));
    }

    let realm_list = Arc::new(tokio::sync::RwLock::new(realm_list));

    // Cleanup expired bans
    let _ = db
        .execute("UPDATE account_banned SET active = 0 WHERE expires_at <= UNIX_TIMESTAMP() AND expires_at <> banned_at")
        .await;
    let _ = db
        .execute("DELETE FROM ip_banned WHERE expires_at <= UNIX_TIMESTAMP() AND expires_at <> banned_at")
        .await;

    // Start the TCP listener
    let bind_ip = {
        let config = get_config().lock();
        config.get_string_default("BindIP", "0.0.0.0")
    };
    let port = {
        let config = get_config().lock();
        config.get_int_default("RealmServerPort", DEFAULT_REALMSERVER_PORT)
    };

    let bind_addr = format!("{}:{}", bind_ip, port);
    let listener = TcpListener::bind(&bind_addr).await?;
    tracing::info!("Listening on {}", bind_addr);

    // Setup Ctrl-C handler
    let stop_event = Arc::new(AtomicBool::new(false));
    let stop_clone = stop_event.clone();

    ctrlc::set_handler(move || {
        tracing::info!("Received shutdown signal");
        stop_clone.store(true, Ordering::SeqCst);
        STOP_EVENT.store(true, Ordering::SeqCst);
    })?;

    // Database ping interval
    let ping_interval = {
        let config = get_config().lock();
        config.get_int_default("MaxPingTime", 30) as u64
    };
    let ping_interval_secs = ping_interval * MINUTE as u64;

    // Spawn database ping task
    let db_ping = db.clone();
    let stop_ping = stop_event.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(ping_interval_secs));
        loop {
            interval.tick().await;
            if stop_ping.load(Ordering::SeqCst) {
                break;
            }
            tracing::debug!("Ping database to keep connection alive");
            if let Err(e) = db_ping.ping().await {
                tracing::error!("Database ping failed: {}", e);
            }
        }
    });

    // Main accept loop
    loop {
        tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok((stream, addr)) => {
                        let db = db.clone();
                        let realm_list = realm_list.clone();
                        tokio::spawn(async move {
                            auth_socket::handle_client(stream, addr, db, realm_list).await;
                        });
                    }
                    Err(e) => {
                        tracing::error!("Failed to accept connection: {}", e);
                    }
                }
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("Shutting down...");
                break;
            }
        }
    }

    tracing::info!("Halting process...");
    Ok(())
}
