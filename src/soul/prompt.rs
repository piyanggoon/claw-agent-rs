use chrono::Utc;
use tracing::debug;

use super::SoulManager;
use crate::memory::MemoryManager;

/// Build the dynamic system prompt by assembling AGENTS.md, all soul files,
/// recent memory logs, and contextual information.
///
/// Each file section is wrapped with `<!-- filename -->` comments.
/// Missing files are silently skipped.
pub async fn build_system_prompt(
    soul: &SoulManager,
    memory: &MemoryManager,
    timezone: &str,
    agents_md_path: &std::path::Path,
) -> String {
    let mut parts: Vec<String> = Vec::new();

    // AGENTS.md — primary agent instructions (group-level)
    match tokio::fs::read_to_string(agents_md_path).await {
        Ok(content) => {
            parts.push(format!(
                "<!-- AGENTS.md -->\n{}\n<!-- /AGENTS.md -->",
                content.trim()
            ));
        }
        Err(e) => {
            debug!("AGENTS.md not found at {}: {}", agents_md_path.display(), e);
        }
    }

    // Core soul files to include (in order)
    let soul_files = [
        "SOUL.md",
        "IDENTITY.md",
        "USER.md",
        "MEMORY.md",
        "HEARTBEAT.md",
        "TOOLS.md",
    ];

    for filename in &soul_files {
        if let Some(section) = read_soul_section(soul, filename).await {
            parts.push(section);
        }
    }

    // BOOTSTRAP.md — only if it exists (first-run scenario)
    if soul.exists("BOOTSTRAP.md") {
        if let Some(section) = read_soul_section(soul, "BOOTSTRAP.md").await {
            parts.push(section);
        }
    }

    // Recent daily logs from memory manager
    match memory.get_recent_daily_logs(3).await {
        Ok(logs) if !logs.is_empty() => {
            let mut log_section = String::from("<!-- recent-daily-logs -->\n");
            log_section.push_str("## Recent Daily Logs\n\n");
            for (date_file, content) in &logs {
                log_section.push_str(&format!("### {}\n{}\n\n", date_file, content));
            }
            log_section.push_str("<!-- /recent-daily-logs -->");
            parts.push(log_section);
        }
        Ok(_) => {}
        Err(e) => {
            debug!("failed to load daily logs for prompt: {}", e);
        }
    }

    // Current date/time with timezone
    let now_utc = Utc::now();
    let datetime_str = format!(
        "Current date/time: {} (timezone: {})",
        now_utc.format("%Y-%m-%d %H:%M:%S UTC"),
        timezone
    );

    parts.push(format!("<!-- context -->\n{}\n<!-- /context -->", datetime_str));

    parts.join("\n\n")
}

/// Read a single soul file and wrap it in HTML comment markers.
/// Returns `None` if the file doesn't exist or can't be read.
async fn read_soul_section(soul: &SoulManager, filename: &str) -> Option<String> {
    match soul.read(filename).await {
        Ok(content) => {
            let section = format!(
                "<!-- {} -->\n{}\n<!-- /{} -->",
                filename,
                content.trim(),
                filename
            );
            Some(section)
        }
        Err(e) => {
            debug!("skipping {} for prompt (not found or unreadable: {})", filename, e);
            None
        }
    }
}
