//! Shared compaction-summary directive for both summarizers (frontend-triggered
//! `summarize_history` and the in-run `compress` path). Structure follows pi's
//! compaction summary format (Goal / Constraints / Progress / Key Decisions /
//! Next Steps / Critical Context + read/modified file lists); the verbatim-quoting
//! rules follow DSH's compaction instruction (paths, commands, error strings,
//! identifiers and numbers are not paraphraseable). Previous checkpoint summaries
//! are merged iteratively, never copied forward verbatim.

/// The locale-dependent compaction directive appended as the final message of the
/// summarization request (`is_en`: en-US gets English, otherwise Chinese).
pub(crate) fn compaction_instruction(is_en: bool) -> &'static str {
    if is_en {
        "You are now acting as a compaction engine for this AI coding assistant. Condense the conversation above into a structured checkpoint that lets the assistant resume later with no loss of essential context.\n\nOutput EXACTLY the Markdown structure below. Keep every section, in order. Use terse bullets, not prose. Write \"(none)\" for an empty section — never drop a section.\n\n## Goal\n- [the user's original and evolving goals]\n\n## Constraints & Preferences\n- [requirements, corrections and preferences the user stated]\n\n## Progress\n### Done\n- [x] [completed tasks]\n### In Progress\n- [ ] [current work]\n### Blocked\n- [issues, if any]\n\n## Key Decisions\n- **[Decision]**: [rationale]\n\n## Next Steps\n1. [what should happen next]\n\n## Critical Context\n- [data needed to continue: state, numbers, snippets]\n\n<read-files>\n[paths actually read and still relevant]\n</read-files>\n\n<modified-files>\n[files changed so far]\n</modified-files>\n\nRules:\n- Preserve exact file paths, commands, error strings, identifiers, numeric values and function signatures VERBATIM — they are not paraphraseable.\n- Capture user feedback and explicit instructions faithfully, especially corrections.\n- Do NOT mention this summarization request or that the context was compacted.\n- If the conversation contains a previous checkpoint summary, do NOT copy it forward verbatim: keep still-true facts, drop stale ones, and merge newer information into one consolidated checkpoint.\n- Output only the checkpoint text."
    } else {
        "你现在担任这个 AI 编码助手的压缩引擎。把上方对话浓缩为一份结构化检查点，让助手日后能无损恢复关键上下文。\n\n严格按以下 Markdown 结构输出。保留全部小节及顺序，用简洁的要点而非段落，空小节写 \"（无）\"——不得省略小节。\n\n## 目标\n- [用户的原始目标与演进目标]\n\n## 约束与偏好\n- [用户提出的要求、纠正与偏好]\n\n## 进展\n### 已完成\n- [x] [已完成的任务]\n### 进行中\n- [ ] [当前工作]\n### 受阻\n- [存在的问题（如有）]\n\n## 关键决策\n- **[决策]**: [理由]\n\n## 下一步\n1. [接下来应该做什么]\n\n## 关键上下文\n- [继续工作所需的数据：状态、数值、代码片段]\n\n<read-files>\n[已读取且仍然相关的文件路径]\n</read-files>\n\n<modified-files>\n[目前已改动的文件]\n</modified-files>\n\n规则：\n- 文件路径、命令、错误信息、标识符、数值、函数签名必须逐字保留——它们不可转述。\n- 忠实记录用户的反馈与明确指示，尤其是纠正。\n- 不得提及本次摘要请求或上下文已被压缩。\n- 若对话中包含先前的检查点摘要，不要原样照抄：保留仍然成立的事实，丢弃过时内容，把新信息合并为一份统一的检查点。\n- 只输出检查点文本。"
    }
}
