// Configuration module
// Rust equivalent of Config.h/cpp
// Reads INI-style configuration files with environment variable overrides

use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::Path;

/// Global configuration singleton
static CONFIG: once_cell::sync::Lazy<Mutex<Config>> =
    once_cell::sync::Lazy::new(|| Mutex::new(Config::new()));

/// Get a reference to the global config instance (equivalent to sConfig macro)
pub fn get_config() -> &'static Mutex<Config> {
    &CONFIG
}

/// Configuration file parser
/// Supports INI-style files with environment variable override
pub struct Config {
    values: HashMap<String, String>,
    filename: String,
    env_prefix: String,
}

impl Config {
    pub fn new() -> Self {
        Config {
            values: HashMap::new(),
            filename: String::new(),
            env_prefix: String::new(),
        }
    }

    /// Load configuration from a file
    /// env_prefix is used to check environment variables (e.g., "Realmd_")
    pub fn set_source(&mut self, filename: &str, env_prefix: &str) -> bool {
        self.filename = filename.to_string();
        self.env_prefix = env_prefix.to_string();
        self.reload()
    }

    /// Reload the configuration file
    pub fn reload(&mut self) -> bool {
        self.values.clear();

        let path = Path::new(&self.filename);
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return false,
        };

        for line in content.lines() {
            let trimmed = line.trim();

            // Skip empty lines and comments
            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
                continue;
            }

            // Skip section headers [Section]
            if trimmed.starts_with('[') {
                continue;
            }

            // Parse key = value
            if let Some(eq_pos) = trimmed.find('=') {
                let key = trimmed[..eq_pos].trim().to_string();
                let mut value = trimmed[eq_pos + 1..].trim().to_string();

                // Strip quotes
                if value.starts_with('"') && value.ends_with('"') {
                    value = value[1..value.len() - 1].to_string();
                }

                self.values.insert(key, value);
            }
        }

        true
    }

    /// Check if a key is set
    pub fn is_set(&self, key: &str) -> bool {
        self.get_env_or_config(key).is_some()
    }

    /// Get a string value with a default
    pub fn get_string_default(&self, key: &str, default: &str) -> String {
        self.get_env_or_config(key)
            .unwrap_or_else(|| default.to_string())
    }

    /// Get a string value (empty string default)
    pub fn get_string(&self, key: &str) -> String {
        self.get_string_default(key, "")
    }

    /// Get a boolean value with a default
    pub fn get_bool_default(&self, key: &str, default: bool) -> bool {
        match self.get_env_or_config(key) {
            Some(val) => {
                let lower = val.to_lowercase();
                matches!(lower.as_str(), "1" | "true" | "yes")
            }
            None => default,
        }
    }

    /// Get an integer value with a default
    pub fn get_int_default(&self, key: &str, default: i32) -> i32 {
        match self.get_env_or_config(key) {
            Some(val) => val.parse().unwrap_or(default),
            None => default,
        }
    }

    /// Get a float value with a default
    pub fn get_float_default(&self, key: &str, default: f32) -> f32 {
        match self.get_env_or_config(key) {
            Some(val) => val.parse().unwrap_or(default),
            None => default,
        }
    }

    /// Try environment variable first, then config file
    fn get_env_or_config(&self, key: &str) -> Option<String> {
        // Convert key to env var name: replace '.' with '_', add prefix
        if !self.env_prefix.is_empty() {
            let env_key = format!("{}{}", self.env_prefix, key.replace('.', "_"));
            if let Ok(val) = std::env::var(&env_key) {
                return Some(val);
            }
        }

        self.values.get(key).cloned()
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_defaults() {
        let config = Config::new();
        assert_eq!(config.get_int_default("nonexistent", 42), 42);
        assert_eq!(config.get_string_default("nonexistent", "hello"), "hello");
        assert!(config.get_bool_default("nonexistent", true));
    }
}
