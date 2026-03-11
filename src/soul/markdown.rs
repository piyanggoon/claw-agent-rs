/// Update a specific `## heading` section within markdown content.
///
/// If the section exists, its body is replaced with `new_content`.
/// If the section does not exist, it is appended at the end of the document.
///
/// A section starts at `## heading` and ends at the next `## ` line or end of file.
pub fn update_section(content: &str, heading: &str, new_content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let target = format!("## {}", heading);

    // Find the line index where the target heading starts
    let mut start_idx = None;
    for (i, line) in lines.iter().enumerate() {
        if line.trim_end() == target || line.starts_with(&format!("{} ", target)) {
            // Exact match for "## Heading" (optionally with trailing whitespace)
            start_idx = Some(i);
            break;
        }
    }

    let Some(heading_idx) = start_idx else {
        // Section not found — append at the end
        let separator = if content.is_empty() || content.ends_with('\n') {
            ""
        } else {
            "\n"
        };
        return format!(
            "{}{separator}\n## {heading}\n{new_content}\n",
            content,
            separator = separator,
            heading = heading,
            new_content = new_content.trim_end()
        );
    };

    // Find the end of this section (next `## ` heading or end of file)
    let mut end_idx = lines.len();
    for i in (heading_idx + 1)..lines.len() {
        if lines[i].starts_with("## ") {
            end_idx = i;
            break;
        }
    }

    // Rebuild: before heading + heading line + new content + rest
    let mut result = String::new();

    // Lines before the heading
    for line in &lines[..heading_idx] {
        result.push_str(line);
        result.push('\n');
    }

    // The heading line + new content
    result.push_str(&target);
    result.push('\n');
    result.push_str(new_content.trim_end());
    result.push('\n');

    // Lines after the section (next heading and beyond)
    if end_idx < lines.len() {
        // Add a blank line separator before next section if not already present
        if !new_content.trim_end().is_empty() {
            result.push('\n');
        }
        for line in &lines[end_idx..] {
            result.push_str(line);
            result.push('\n');
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_update_existing_section() {
        let content = "# Title\n\n## Facts\n- old fact\n\n## Preferences\n- dark mode\n";
        let result = update_section(content, "Facts", "- new fact 1\n- new fact 2");
        assert!(result.contains("## Facts\n- new fact 1\n- new fact 2\n"));
        assert!(result.contains("## Preferences\n- dark mode"));
    }

    #[test]
    fn test_append_new_section() {
        let content = "# Title\n\n## Facts\n- some fact\n";
        let result = update_section(content, "Insights", "- first insight");
        assert!(result.contains("## Insights\n- first insight\n"));
        assert!(result.contains("## Facts\n- some fact"));
    }

    #[test]
    fn test_update_last_section() {
        let content = "## Only\nold content\n";
        let result = update_section(content, "Only", "new content");
        assert_eq!(result, "## Only\nnew content\n");
    }

    #[test]
    fn test_empty_content() {
        let content = "";
        let result = update_section(content, "Fresh", "hello");
        assert!(result.contains("## Fresh\nhello\n"));
    }
}
