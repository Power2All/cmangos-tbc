// Database module - SQLx-based database abstraction
// Rust equivalent of Database.h/cpp, DatabaseMysql.h/cpp, etc.
//
// Uses SQLx for compile-time checked queries with support for
// MySQL, PostgreSQL, and SQLite (matching the C++ multi-database support).

use sqlx::any::AnyRow;
use sqlx::{AnyPool, Row};
use anyhow::Result;

/// Database connection pool wrapper
/// Equivalent to the C++ Database class with connection pooling
#[derive(Clone)]
pub struct Database {
    pool: Option<AnyPool>,
    name: String,
}

impl Database {
    /// Create a new uninitialized database handle
    pub fn new(name: &str) -> Self {
        Database {
            pool: None,
            name: name.to_string(),
        }
    }

    /// Initialize the database connection
    /// connection_string format depends on the database type:
    /// - MySQL: "mysql://user:password@host:port/database"
    /// - PostgreSQL: "postgres://user:password@host:port/database"
    /// - SQLite: "sqlite://path/to/db.sqlite"
    ///
    /// The C++ code uses a format like "host;port;user;password;database"
    /// which we convert to a URL format internally.
    pub async fn initialize(&mut self, connection_info: &str) -> Result<()> {
        // Support both URL format and legacy CMaNGOS format
        let url = if connection_info.contains("://") {
            connection_info.to_string()
        } else {
            self.convert_legacy_connection_string(connection_info)?
        };

        // Install all drivers
        sqlx::any::install_default_drivers();

        let pool = sqlx::pool::PoolOptions::<sqlx::Any>::new()
            .max_connections(5)
            .min_connections(1)
            .connect(&url)
            .await?;

        self.pool = Some(pool);
        tracing::info!("Connected to {} database", self.name);
        Ok(())
    }

    /// Convert legacy CMaNGOS connection string format
    /// Format: "host;port;user;password;database"
    fn convert_legacy_connection_string(&self, conn: &str) -> Result<String> {
        let parts: Vec<&str> = conn.split(';').collect();
        if parts.len() < 5 {
            anyhow::bail!(
                "Invalid connection string format. Expected: host;port;user;password;database"
            );
        }

        let host = parts[0];
        let port = parts[1];
        let user = parts[2];
        let password = parts[3];
        let database = parts[4];

        // Default to MySQL (matching C++ default)
        Ok(format!(
            "mysql://{}:{}@{}:{}/{}",
            user, password, host, port, database
        ))
    }

    /// Execute a query and return rows
    pub async fn query(&self, sql: &str) -> Result<Vec<AnyRow>> {
        let pool = self.pool.as_ref().ok_or_else(|| {
            anyhow::anyhow!("Database {} not initialized", self.name)
        })?;

        let rows = sqlx::query(sql).fetch_all(pool).await?;
        Ok(rows)
    }

    /// Execute a query that returns a single optional row
    pub async fn query_one(&self, sql: &str) -> Result<Option<AnyRow>> {
        let pool = self.pool.as_ref().ok_or_else(|| {
            anyhow::anyhow!("Database {} not initialized", self.name)
        })?;

        let row = sqlx::query(sql).fetch_optional(pool).await?;
        Ok(row)
    }

    /// Execute a statement (INSERT, UPDATE, DELETE)
    pub async fn execute(&self, sql: &str) -> Result<u64> {
        let pool = self.pool.as_ref().ok_or_else(|| {
            anyhow::anyhow!("Database {} not initialized", self.name)
        })?;

        let result: sqlx::any::AnyQueryResult = sqlx::query(sql).execute(pool).await?;
        Ok(result.rows_affected())
    }

    /// Execute directly (synchronous-style, runs in current task)
    pub async fn direct_execute(&self, sql: &str) -> Result<u64> {
        self.execute(sql).await
    }

    /// Ping the database to keep the connection alive
    pub async fn ping(&self) -> Result<()> {
        let pool = self.pool.as_ref().ok_or_else(|| {
            anyhow::anyhow!("Database {} not initialized", self.name)
        })?;

        // Execute a simple query to keep connection alive
        sqlx::query("SELECT 1").fetch_one(pool).await?;
        Ok(())
    }

    /// Begin a transaction
    pub async fn begin_transaction(&self) -> Result<sqlx::Transaction<'_, sqlx::Any>> {
        let pool = self.pool.as_ref().ok_or_else(|| {
            anyhow::anyhow!("Database {} not initialized", self.name)
        })?;

        let tx = pool.begin().await?;
        Ok(tx)
    }

    /// Escape a string for safe SQL insertion
    /// Note: With SQLx parameterized queries, this is less necessary,
    /// but provided for compatibility with the C++ codebase patterns.
    pub fn escape_string(input: &str) -> String {
        input
            .replace('\\', "\\\\")
            .replace('\'', "\\'")
            .replace('"', "\\\"")
            .replace('\0', "\\0")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\x1a', "\\Z")
    }

    /// Check if a required database field/version exists
    pub async fn check_required_field(&self, table: &str, required: &str) -> Result<bool> {
        let sql = format!("SHOW COLUMNS FROM `{}` LIKE '{}'", table, required);
        match self.query_one(&sql).await {
            Ok(Some(_)) => Ok(true),
            Ok(None) => {
                tracing::error!(
                    "Database {} table '{}' missing required field '{}'",
                    self.name,
                    table,
                    required
                );
                Ok(false)
            }
            Err(e) => {
                tracing::error!("Error checking required field: {}", e);
                Ok(false)
            }
        }
    }

    /// Get the database name
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Check if the database is initialized
    pub fn is_connected(&self) -> bool {
        self.pool.is_some()
    }

    /// Get the underlying pool (for advanced usage)
    pub fn pool(&self) -> Option<&AnyPool> {
        self.pool.as_ref()
    }
}

/// Helper trait to extract values from AnyRow
/// This provides the same interface as the C++ Field class
pub trait FieldExt {
    fn get_string(&self, index: usize) -> String;
    fn get_u8(&self, index: usize) -> u8;
    fn get_u16(&self, index: usize) -> u16;
    fn get_u32(&self, index: usize) -> u32;
    fn get_u64(&self, index: usize) -> u64;
    fn get_i32(&self, index: usize) -> i32;
    fn get_i64(&self, index: usize) -> i64;
    fn get_f32(&self, index: usize) -> f32;
    fn get_f64(&self, index: usize) -> f64;
    fn get_bool(&self, index: usize) -> bool;
}

impl FieldExt for AnyRow {
    fn get_string(&self, index: usize) -> String {
        // The SQLx Any driver maps MySQL TEXT/LONGTEXT/BLOB columns inconsistently.
        // Try multiple Rust types in order of likelihood:
        //   1. String - works for VARCHAR, CHAR
        //   2. &str   - works for some text types
        //   3. Vec<u8> - works for BLOB/LONGTEXT that the Any driver misidentifies
        self.try_get::<String, _>(index)
            .or_else(|_| self.try_get::<&str, _>(index).map(|s| s.to_string()))
            .or_else(|_| {
                self.try_get::<Vec<u8>, _>(index)
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            })
            .unwrap_or_else(|e| {
                tracing::trace!("get_string({}): all decode attempts failed: {}", index, e);
                String::new()
            })
    }

    fn get_u8(&self, index: usize) -> u8 {
        // AnyRow may return i32 for small integers depending on driver
        self.try_get::<i32, _>(index)
            .map(|v| v as u8)
            .unwrap_or(0)
    }

    fn get_u16(&self, index: usize) -> u16 {
        self.try_get::<i32, _>(index)
            .map(|v| v as u16)
            .unwrap_or(0)
    }

    fn get_u32(&self, index: usize) -> u32 {
        self.try_get::<i64, _>(index)
            .map(|v| v as u32)
            .or_else(|_| self.try_get::<i32, _>(index).map(|v| v as u32))
            .unwrap_or(0)
    }

    fn get_u64(&self, index: usize) -> u64 {
        self.try_get::<i64, _>(index)
            .map(|v| v as u64)
            .unwrap_or(0)
    }

    fn get_i32(&self, index: usize) -> i32 {
        self.try_get::<i32, _>(index).unwrap_or(0)
    }

    fn get_i64(&self, index: usize) -> i64 {
        self.try_get::<i64, _>(index).unwrap_or(0)
    }

    fn get_f32(&self, index: usize) -> f32 {
        self.try_get::<f32, _>(index)
            .or_else(|_| self.try_get::<f64, _>(index).map(|v| v as f32))
            .unwrap_or(0.0)
    }

    fn get_f64(&self, index: usize) -> f64 {
        self.try_get::<f64, _>(index).unwrap_or(0.0)
    }

    fn get_bool(&self, index: usize) -> bool {
        self.try_get::<bool, _>(index)
            .or_else(|_| self.try_get::<i32, _>(index).map(|v| v != 0))
            .unwrap_or(false)
    }
}
