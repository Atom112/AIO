//! Diff 生成与预览辅助 —— 从 commands/llm/mod.rs 下沉（该文件中的通用工具函数）。
//!
//! 供 Agent 文件修改工具的审批预览使用：对 write_file / replace_in_file / delete_file
//! 预计算 Git 风格 unified diff，展示给用户确认。

/// 为文件修改工具预计算 diff 预览（审批前展示用）。
/// 返回 (preview_diff, file_path)；非文件修改类工具返回 (None, None)。
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
        "write_file" => {
            let new_content = arguments["content"].as_str().unwrap_or("");
            let old_content = full_path()
                .and_then(|p| std::fs::read_to_string(&p).ok())
                .unwrap_or_default();
            let diff = simple_diff(
                &old_content,
                new_content,
                &file_path.clone().unwrap_or_default(),
            );
            (Some(diff), file_path)
        }
        "replace_in_file" => {
            let old_s = arguments["old_string"].as_str().unwrap_or("");
            let new_s = arguments["new_string"].as_str().unwrap_or("");
            let old_content = full_path()
                .and_then(|p| std::fs::read_to_string(&p).ok())
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
        "delete_file" => {
            let old_content = full_path()
                .and_then(|p| std::fs::read_to_string(&p).ok())
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

/// 简易 unified diff 生成器：比较 old 和 new 文本，输出 Git 风格的 diff。
/// 策略：共同前缀 + 共同后缀裁剪，保留前后各 3 行上下文；整体超过 8000 字节截断
/// （截断处自动落在字符边界，避免多字节文本 panic）。
pub fn simple_diff(old: &str, new: &str, file_name: &str) -> String {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    // 找共同前缀
    let mut prefix = 0;
    while prefix < old_lines.len()
        && prefix < new_lines.len()
        && old_lines[prefix] == new_lines[prefix]
    {
        prefix += 1;
    }
    // 找共同后缀（在去除前缀后）
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

    // 上下文（前3行）
    let ctx_start = prefix.saturating_sub(3);
    for line in &old_lines[ctx_start..prefix] {
        diff.push_str(&format!(" {}\n", line));
    }

    // 删除的行
    for line in &old_lines[prefix..old_end] {
        diff.push_str(&format!("-{}\n", line));
    }
    // 新增的行
    for line in &new_lines[prefix..new_end] {
        diff.push_str(&format!("+{}\n", line));
    }

    // 上下文（后3行）
    let ctx_end = (new_end + 3).min(new_lines.len());
    for line in &new_lines[new_end..ctx_end] {
        diff.push_str(&format!(" {}\n", line));
    }

    if diff.len() > 8000 {
        // 不能用 str::truncate：8000 可能落在多字节字符中间导致 panic
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
        // 多字节内容使 8000 恰好落在字符中间时不得 panic
        let big = "中".repeat(3000) + "\n";
        let d = simple_diff("", &big, "c.txt");
        assert!(d.contains("已截断"));
    }
}
