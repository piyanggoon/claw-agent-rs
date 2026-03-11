use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use tokio::fs;
use tracing::debug;

use super::markdown;

/// Manages reading and writing soul files on disk.
///
/// Soul files live under `groups/{group}/soul/` and contain the agent's
/// identity, personality, memories, and configuration.
pub struct SoulManager {
    soul_dir: PathBuf,
}

impl SoulManager {
    /// Create a new SoulManager for the given soul directory.
    pub fn new(soul_dir: impl Into<PathBuf>) -> Self {
        Self {
            soul_dir: soul_dir.into(),
        }
    }

    /// Return the base soul directory path.
    pub fn soul_dir(&self) -> &Path {
        &self.soul_dir
    }

    /// Resolve a filename to its full path within the soul directory.
    fn resolve(&self, filename: &str) -> PathBuf {
        self.soul_dir.join(filename)
    }

    /// Read a soul file and return its contents as a string.
    ///
    /// Returns an error if the file does not exist or cannot be read.
    pub async fn read(&self, filename: &str) -> Result<String> {
        let path = self.resolve(filename);
        debug!("reading soul file: {}", path.display());
        fs::read_to_string(&path)
            .await
            .with_context(|| format!("failed to read soul file: {}", path.display()))
    }

    /// Write content to a soul file, creating parent directories as needed.
    pub async fn write(&self, filename: &str, content: &str) -> Result<()> {
        let path = self.resolve(filename);
        debug!("writing soul file: {}", path.display());

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .await
                .with_context(|| format!("failed to create directory: {}", parent.display()))?;
        }

        fs::write(&path, content)
            .await
            .with_context(|| format!("failed to write soul file: {}", path.display()))
    }

    /// Update a specific `## heading` section within a soul file.
    ///
    /// If the file does not exist, a new file is created with just that section.
    /// If the section does not exist in the file, it is appended at the end.
    pub async fn update_section(
        &self,
        filename: &str,
        heading: &str,
        content: &str,
    ) -> Result<()> {
        let existing = match self.read(filename).await {
            Ok(text) => text,
            Err(_) => String::new(),
        };

        let updated = markdown::update_section(&existing, heading, content);
        self.write(filename, &updated).await
    }

    /// Delete a soul file. Only BOOTSTRAP.md is allowed to be deleted.
    pub async fn delete(&self, filename: &str) -> Result<()> {
        if filename != "BOOTSTRAP.md" {
            bail!(
                "only BOOTSTRAP.md can be deleted (attempted: {})",
                filename
            );
        }

        let path = self.resolve(filename);
        debug!("deleting soul file: {}", path.display());

        if path.exists() {
            fs::remove_file(&path)
                .await
                .with_context(|| format!("failed to delete soul file: {}", path.display()))?;
        }

        Ok(())
    }

    /// Check whether a soul file exists on disk.
    pub fn exists(&self, filename: &str) -> bool {
        self.resolve(filename).exists()
    }

    /// List recent daily log files sorted by date descending.
    ///
    /// Returns a vector of `(filename, content)` tuples for the last `days` daily logs.
    /// Daily logs live at `memory/YYYY-MM-DD.md` inside the soul directory.
    pub async fn list_daily_logs(&self, days: u32) -> Result<Vec<(String, String)>> {
        let memory_dir = self.soul_dir.join("memory");

        if !memory_dir.exists() {
            return Ok(Vec::new());
        }

        let mut entries: Vec<String> = Vec::new();

        let mut dir = fs::read_dir(&memory_dir)
            .await
            .with_context(|| format!("failed to read memory dir: {}", memory_dir.display()))?;

        while let Some(entry) = dir.next_entry().await? {
            let name = entry.file_name().to_string_lossy().into_owned();
            // Match files like YYYY-MM-DD.md
            if name.len() == 13 && name.ends_with(".md") && name.chars().nth(4) == Some('-') {
                entries.push(name);
            }
        }

        // Sort descending by filename (date)
        entries.sort_unstable_by(|a, b| b.cmp(a));

        // Take only the last N days
        entries.truncate(days as usize);

        let mut results = Vec::new();
        for name in entries {
            let rel_path = format!("memory/{}", name);
            match self.read(&rel_path).await {
                Ok(content) => results.push((name, content)),
                Err(e) => {
                    debug!("skipping daily log {} (read error: {})", name, e);
                }
            }
        }

        Ok(results)
    }
}
