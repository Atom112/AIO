//! Diff generation and preview helpers.
//!
//! Serves approval previews for the Agent file-modification tools: precomputes a
//! Git-style unified diff for write_file / replace_in_file / delete_file for user confirmation.

use crate::core::tool_names;

/// Preview source cap (SEC-04): only the head of oversized files feeds the preview —
/// a multi-GB text file must not be read whole to render an 8000-byte diff.
const PREVIEW_SOURCE_CAP: u64 = 2 * 1024 * 1024;

/// Reads up to [`PREVIEW_SOURCE_CAP`] bytes of a file (UTF-8 lossy; missing file →
/// empty string). Preview-only reads must stay bounded regardless of file size.
fn read_preview_source(path: &std::path::Path) -> String {
    use std::io::Read;
    let mut buf: Vec<u8> = Vec::new();
    if let Ok(mut f) = std::fs::File::open(path) {
        let _ = (&mut f).take(PREVIEW_SOURCE_CAP).read_to_end(&mut buf);
    }
    String::from_utf8_lossy(&buf).into_owned()
}

/// Precompute the diff preview for file-modification tools (shown before approval).
/// Returns (preview_diff, file_path); non-file-modification tools return (None, None).
pub fn compute_diff_preview(
    tool_name: &str,
    arguments: &serde_json::Value,
    project_root: &str,
) -> (Option<String>, Option<String>) {
    let file_path = arguments["path"].as_str().map(|s| s.to_string());
    let full_path = || {
        let root = std::path::Path::new(project_root);
        file_path.as_ref().map(|p| root.join(p))
    };

    match tool_name {
        tool_names::WRITE_FILE => {
            let new_content = arguments["content"].as_str().unwrap_or("");
            let old_content = full_path()
                .map(|p| read_preview_source(&p))
                .unwrap_or_default();
            let diff = simple_diff(
                &old_content,
                new_content,
                &file_path.clone().unwrap_or_default(),
            );
            (Some(diff), file_path)
        }
        tool_names::REPLACE_IN_FILE => {
            let old_s = arguments["old_string"].as_str().unwrap_or("");
            let new_s = arguments["new_string"].as_str().unwrap_or("");
            let old_content = full_path()
                .map(|p| read_preview_source(&p))
                .unwrap_or_default();
            if old_content.contains(old_s) {
                let new_content = old_content.replacen(old_s, new_s, 1);
                let diff = simple_diff(
                    &old_content,
                    &new_content,
                    &file_path.clone().unwrap_or_default(),
                );
                (Some(diff), file_path)
            } else {
                (None, file_path)
            }
        }
        tool_names::DELETE_FILE => {
            let old_content = full_path()
                .map(|p| read_preview_source(&p))
                .unwrap_or_default();
            if old_content.is_empty() {
                (None, file_path)
            } else {
                let diff = simple_diff(&old_content, "", &file_path.clone().unwrap_or_default());
                (Some(diff), file_path)
            }
        }
        _ => (None, None),
    }
}

/// Simple unified diff generator: compares old and new text, outputting a Git-style diff.
/// Strategy: trim the common prefix and suffix, keep 3 context lines on each side; truncate the
/// whole output past 8000 bytes (truncation lands on a char boundary to avoid panics on multi-byte text).
pub fn simple_diff(old: &str, new: &str, file_name: &str) -> String {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    // Find the common prefix
    let mut prefix = 0;
    while prefix < old_lines.len()
        && prefix < new_lines.len()
        && old_lines[prefix] == new_lines[prefix]
    {
        prefix += 1;
    }
    // Find the common suffix (after removing the prefix)
    let mut suffix = 0;
    while suffix < old_lines.len().saturating_sub(prefix)
        && suffix < new_lines.len().saturating_sub(prefix)
        && old_lines[old_lines.len() - 1 - suffix] == new_lines[new_lines.len() - 1 - suffix]
    {
        suffix += 1;
    }

    let old_start = prefix + 1;
    let old_end = old_lines.len() - suffix;
    let new_start = prefix + 1;
    let new_end = new_lines.len() - suffix;

    let added = (new_end as i64 - new_start as i64 + 1).max(0);
    let removed = (old_end as i64 - old_start as i64 + 1).max(0);

    let mut diff = format!(
        "--- a/{}\n+++ b/{}\n@@ -{},{} +{},{} @@\n",
        file_name,
        file_name,
        old_start,
        removed.max(1),
        new_start,
        added.max(1),
    );

    // Context (3 preceding lines)
    let ctx_start = prefix.saturating_sub(3);
    for line in &old_lines[ctx_start..prefix] {
        diff.push_str(&format!(" {}\n", line));
    }

    // Removed lines
    for line in &old_lines[prefix..old_end] {
        diff.push_str(&format!("-{}\n", line));
    }
    // Added lines
    for line in &new_lines[prefix..new_end] {
        diff.push_str(&format!("+{}\n", line));
    }

    // Context (3 following lines)
    let ctx_end = (new_end + 3).min(new_lines.len());
    for line in &new_lines[new_end..ctx_end] {
        diff.push_str(&format!(" {}\n", line));
    }

    if diff.len() > 8000 {
        // str::truncate is not safe here: 8000 may land mid multi-byte character and panic
        let end = diff.floor_char_boundary(8000);
        diff.truncate(end);
        diff.push_str("\n... [diff 已截断]");
    }
    diff
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_diff_shows_change_hunk() {
        let old = "line1\nline2\nline3\nline4\n";
        let new = "line1\nline2 changed\nline3\nline4\n";
        let d = simple_diff(old, new, "a.txt");
        assert!(d.contains("--- a/a.txt"));
        assert!(d.contains("+++ b/a.txt"));
        assert!(d.contains("-line2"));
        assert!(d.contains("+line2 changed"));
        assert!(d.contains(" line1"), "应保留前置上下文");
    }

    #[test]
    fn simple_diff_delete_to_empty() {
        let d = simple_diff("only\n", "", "b.txt");
        assert!(d.contains("-only"));
    }

    #[test]
    fn simple_diff_truncation_is_char_boundary_safe() {
        // Must not panic when multi-byte content puts 8000 exactly mid-character
        let big = "中".repeat(3000) + "\n";
        let d = simple_diff("", &big, "c.txt");
        assert!(d.contains("已截断"));
    }
}
