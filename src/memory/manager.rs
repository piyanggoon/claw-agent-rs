use std::sync::Arc;

use anyhow::{Result, Context};
use chrono::Local;
use tracing::debug;

use crate::soul::SoulManager;

/// MemoryManager provides memory-specific operations on top of SoulManager.
///
/// It handles structured memory in MEMORY.md (sections like Key Facts,
/// Decisions & Preferences, Lessons Learned, Open Loops) and raw daily
/// logs in `memory/YYYY-MM-DD.md`.
pub struct MemoryManager {
    soul: Arc<SoulManager>,
}

impl MemoryManager {
    /// Create a new MemoryManager wrapping the given SoulManager.
    pub fn new(soul: Arc<SoulManager>) -> Self {
        Self { soul }
    }

    /// Save content to a section of MEMORY.md.
    ///
    /// - `action = "replace"`: replaces the entire section content.
    /// - `action = "append"` (default): appends content to the existing section.
    pub async fn save(&self, section: &str, content: &str, action: &str) -> Result<()> {
        match action {
            "replace" => {
                self.soul
                    .update_section("MEMORY.md", section, content)
                    .await?;
            }
            "append" | _ => {
                // Read existing content, find the section, and append
                let existing = match self.soul.read("MEMORY.md").await {
                    Ok(text) => text,
                    Err(_) => String::new(),
                };

                let section_content = extract_section(&existing, section);
                let new_section = if section_content.is_empty() {
                    content.to_string()
                } else {
                    format!("{}\n{}", section_content.trim_end(), content)
                };

                self.soul
                    .update_section("MEMORY.md", section, &new_section)
                    .await?;
            }
        }

        debug!("memory saved to section '{}' (action: {})", section, action);
        Ok(())
    }

    /// Append an entry to today's daily log file.
    ///
    /// The entry is formatted as `- **HH:MM** [category] content` and written
    /// to `memory/YYYY-MM-DD.md`. The file is created with a header if it
    /// doesn't exist yet.
    pub async fn daily_log(&self, content: &str, category: Option<&str>) -> Result<()> {
        let now = Local::now();
        let date_str = now.format("%Y-%m-%d").to_string();
        let time_str = now.format("%H:%M").to_string();
        let filename = format!("memory/{}.md", date_str);

        let category_tag = match category {
            Some(cat) => format!("[{}] ", cat),
            None => String::new(),
        };

        let entry = format!("- **{}** {}{}", time_str, category_tag, content);

        // Read existing file or create header
        let existing = match self.soul.read(&filename).await {
            Ok(text) => text,
            Err(_) => format!("# Daily Log \u{2014} {}\n", date_str),
        };

        let updated = format!("{}\n{}\n", existing.trim_end(), entry);
        self.soul.write(&filename, &updated).await?;

        debug!("daily log entry added to {}", filename);
        Ok(())
    }

    /// Search MEMORY.md and recent daily logs for matching content.
    ///
    /// If `query` is `None`, returns all content.
    /// Otherwise, performs case-insensitive substring matching and returns
    /// matching lines with their source file.
    pub async fn recall(&self, query: Option<&str>, days: u32) -> Result<String> {
        let mut results: Vec<String> = Vec::new();

        // Search MEMORY.md
        match self.soul.read("MEMORY.md").await {
            Ok(memory_content) => {
                let matching = filter_lines(&memory_content, query);
                if !matching.is_empty() {
                    results.push(format!("=== MEMORY.md ===\n{}", matching));
                }
            }
            Err(_) => {
                debug!("MEMORY.md not found, skipping");
            }
        }

        // Search daily logs
        let logs = self.get_recent_daily_logs(days).await?;
        for (filename, content) in &logs {
            let matching = filter_lines(content, query);
            if !matching.is_empty() {
                results.push(format!("=== {} ===\n{}", filename, matching));
            }
        }

        if results.is_empty() {
            Ok("No matching memories found.".to_string())
        } else {
            Ok(results.join("\n\n"))
        }
    }

    /// Remove entries matching `entry` (case-insensitive) from a section of MEMORY.md.
    ///
    /// Reads MEMORY.md, finds lines within the specified section that contain
    /// the entry substring, removes them, and writes the file back.
    pub async fn forget(&self, section: &str, entry: &str) -> Result<()> {
        let content = self
            .soul
            .read("MEMORY.md")
            .await
            .context("failed to read MEMORY.md for forget operation")?;

        let entry_lower = entry.to_lowercase();
        let mut lines: Vec<&str> = content.lines().collect();
        let mut in_target_section = false;
        let mut indices_to_remove: Vec<usize> = Vec::new();

        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim();

            // Check if we're entering a ## section
            if trimmed.starts_with("## ") {
                let heading = trimmed.trim_start_matches("## ").trim();
                in_target_section = heading.eq_ignore_ascii_case(section);
                continue;
            }

            // If we're in the target section and the line matches, mark for removal
            if in_target_section && line.to_lowercase().contains(&entry_lower) {
                indices_to_remove.push(i);
            }
        }

        if indices_to_remove.is_empty() {
            debug!(
                "no matching entries found in section '{}' for '{}'",
                section, entry
            );
            return Ok(());
        }

        // Remove marked lines in reverse order to preserve indices
        for &idx in indices_to_remove.iter().rev() {
            lines.remove(idx);
        }

        let updated = lines.join("\n");
        // Ensure trailing newline
        let updated = if updated.ends_with('\n') {
            updated
        } else {
            format!("{}\n", updated)
        };

        self.soul.write("MEMORY.md", &updated).await?;
        debug!(
            "forgot {} entries from section '{}'",
            indices_to_remove.len(),
            section
        );
        Ok(())
    }

    /// Read the last N daily log files, sorted by date descending.
    ///
    /// Returns a vector of `(filename, content)` tuples.
    pub async fn get_recent_daily_logs(&self, days: u32) -> Result<Vec<(String, String)>> {
        self.soul.list_daily_logs(days).await
    }
}

/// Extract the body text of a `## section` from markdown content.
///
/// Returns the lines between `## section` and the next `## ` heading (or EOF).
fn extract_section(content: &str, section: &str) -> String {
    let mut in_section = false;
    let mut lines: Vec<&str> = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("## ") {
            let heading = trimmed.trim_start_matches("## ").trim();
            if heading.eq_ignore_ascii_case(section) {
                in_section = true;
                continue;
            } else if in_section {
                break;
            }
        }

        if in_section {
            lines.push(line);
        }
    }

    lines.join("\n")
}

/// Filter lines from content by case-insensitive substring match.
///
/// If `query` is `None`, returns all non-empty lines.
fn filter_lines(content: &str, query: Option<&str>) -> String {
    match query {
        Some(q) => {
            let q_lower = q.to_lowercase();
            content
                .lines()
                .filter(|line| line.to_lowercase().contains(&q_lower))
                .collect::<Vec<_>>()
                .join("\n")
        }
        None => content.to_string(),
    }
}
