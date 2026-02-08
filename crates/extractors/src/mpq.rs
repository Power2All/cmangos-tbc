use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use mpq::Archive;

pub struct MpqManager {
    archives: Vec<Archive>,
}

impl MpqManager {
    pub fn new() -> Self {
        Self { archives: Vec::new() }
    }

    pub fn open_archive(&mut self, path: &Path) -> anyhow::Result<bool> {
        if !path.exists() {
            return Ok(false);
        }
        let archive = Archive::open(path)?;
        self.archives.insert(0, archive);
        Ok(true)
    }

    pub fn open_file(&mut self, filename: &str) -> Option<Vec<u8>> {
        for archive in &mut self.archives {
            // Handle empty files gracefully - the mpq crate throws an error for 0-byte files
            let file_result = archive.open_file(filename);
            let file = match file_result {
                Ok(f) => f,
                Err(_) => continue, // Skip files that can't be opened (including 0-byte files)
            };

            let size = file.size() as usize;
            if size == 0 {
                continue; // Skip empty files
            }

            let mut buf = vec![0u8; size];
            if file.read(archive, &mut buf).is_ok() {
                return Some(buf);
            }
        }
        None
    }

    pub fn list_files(&mut self) -> BTreeSet<String> {
        let mut entries = BTreeSet::new();
        for archive in &mut self.archives {
            if let Ok(listfile) = archive.open_file("(listfile)") {
                let mut buf = vec![0u8; listfile.size() as usize];
                if listfile.read(archive, &mut buf).is_ok() {
                    let content = String::from_utf8_lossy(&buf);
                    for line in content.lines() {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        entries.insert(trimmed.to_string());
                    }
                }
            }
        }
        entries
    }
}

pub fn build_path(base: &Path, parts: &[&str]) -> PathBuf {
    let mut path = base.to_path_buf();
    for part in parts {
        if !part.is_empty() {
            path.push(part);
        }
    }
    path
}
