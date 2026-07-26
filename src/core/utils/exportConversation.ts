/**
 * 对话导出纯函数 — Markdown / JSON 格式生成。
 * 输入 Topic + 选项，输出格式化字符串；无 UI 依赖。
 */
import type { Topic, Message } from '../store/store';
import { formatDateTime, locale, t } from '../i18n';

// -------- 选项类型 --------

export interface ExportOptions {
  /** 是否包含 system 角色消息（默认 false） */
  includeSystem?: boolean;
  /** 是否在 assistant 消息后附带 tool calls 描述（默认 false） */
  includeToolCalls?: boolean;
  /** 是否包含 reasoning 内容（默认 true） */
  includeReasoning?: boolean;
  /** 仅导出指定 ID 的消息；未指定时导出全部 */
  selectedMessageIds?: Set<string>;
}

// -------- 辅助 --------

/** 格式化时间戳为 YYYY-MM-DD HH:mm */
function formatTime(): string {
  return formatDateTime(new Date(), { dateStyle: 'medium', timeStyle: 'short' });
}

/** 从消息提取纯文本内容。优先 displayText，其次 content（若为数组则取各 item.text 拼接）。 */
function extractText(msg: Message): string {
  if (msg.displayText != null) return String(msg.displayText);
  if (typeof msg.content === 'string') return msg.content;
  if (Array.isArray(msg.content)) {
    return msg.content
      .map((c: any) => c?.text ?? '')
      .filter(Boolean)
      .join('\n');
  }
  return '';
}

/** 判断消息是否应被导出（综合考虑角色过滤 + 消息 ID 筛选） */
function shouldInclude(msg: Message, opts: ExportOptions): boolean {
  if (msg.role === 'tool') return false;
  if (msg.role === 'system' && !opts.includeSystem) return false;
  if (opts.selectedMessageIds && msg.id && !opts.selectedMessageIds.has(msg.id)) return false;
  return true;
}

// -------- Markdown --------

/**
 * 将话题导出为 Markdown 字符串。
 * @param topic 完整话题对象
 * @param options 过滤 / 格式化选项
 */
export function exportAsMarkdown(topic: Topic, options?: ExportOptions): string {
  const opts: ExportOptions = {
    includeSystem: false,
    includeToolCalls: false,
    includeReasoning: true,
    ...options,
  };

  const lines: string[] = [];

  // 标题
  lines.push(`# ${topic.name}`);
  lines.push(`> ${formatTime()}`);
  lines.push('');
  lines.push('---');

  for (const msg of topic.history) {
    if (!shouldInclude(msg, opts)) continue;

    lines.push('');

    if (msg.role === 'user') {
      lines.push(`### ${t('export.role.user')}`);
      const text = extractText(msg);
      lines.push(text || t('export.empty'));
    } else if (msg.role === 'assistant') {
      const label = msg.modelId
        ? `${t('export.role.assistant')} (${msg.modelId})`
        : t('export.role.assistant');
      lines.push(`### ${label}`);

      // reasoning（引用块）
      if (opts.includeReasoning && msg.reasoning) {
        lines.push('');
        for (const line of msg.reasoning.split('\n')) {
          lines.push(`> ${line}`);
        }
        lines.push('');
      }

      // 正文
      const text = extractText(msg);
      lines.push(text || t('export.empty'));

      // tool calls 描述（可选）
      if (opts.includeToolCalls && msg.toolCalls && msg.toolCalls.length > 0) {
        lines.push('');
        lines.push(`**${t('export.toolCalls')}:**`);
        for (const tc of msg.toolCalls) {
          const fn = tc.function;
          lines.push(`- \`${fn.name}\``);
          if (fn.arguments) {
            lines.push('  ```json');
            lines.push(`  ${fn.arguments}`);
            lines.push('  ```');
          }
          if (tc.result != null) {
            lines.push(`  ${t('export.result')}: \`${JSON.stringify(tc.result).slice(0, 200)}\``);
          }
        }
      }
    } else if (msg.role === 'system') {
      lines.push(`### ${t('export.role.system')}`);
      lines.push(extractText(msg));
    }

    lines.push('');
    lines.push('---');
  }

  return lines.join('\n');
}

// -------- JSON --------

export interface JsonExportOptions {
  /** 'full'：完整 Topic + 元数据；'simple'：仅 role+content 数组 */
  mode?: 'full' | 'simple';
  /** 仅导出指定 ID 的消息；未指定时导出全部 */
  selectedMessageIds?: Set<string>;
}

/** 简单模式：每条消息提取 role + content（保持原始 content 类型） */
interface SimpleMessage {
  role: string;
  content: unknown;
}

/** 完整模式：保留消息所有关键字段 */
interface FullMessage {
  role: string;
  content: unknown;
  modelId?: string;
  reasoning?: string;
  toolCalls?: unknown[];
  agentSteps?: unknown[];
  inputTokens?: number;
  outputTokens?: number;
}

interface FullExport {
  id: string;
  name: string;
  exportedAt: string;
  messages: FullMessage[];
}

/** 消息级过滤：跳过 tool 角色，并按 selectedMessageIds 筛选 */
function filterMessages(msgs: Message[], ids?: Set<string>): Message[] {
  return msgs.filter((m) => {
    if (m.role === 'tool') return false;
    if (ids && m.id && !ids.has(m.id)) return false;
    return true;
  });
}

/**
 * 将话题导出为 JSON 字符串（pretty-print）。
 * @param topic 完整话题对象
 * @param options 模式选择 + 消息筛选
 */
export function exportAsJSON(topic: Topic, options?: JsonExportOptions): string {
  const mode = options?.mode ?? 'full';
  const ids = options?.selectedMessageIds;
  const filtered = filterMessages(topic.history, ids);

  if (mode === 'simple') {
    const result: SimpleMessage[] = filtered.map((m) => ({
      role: m.role,
      content: m.content,
    }));
    return JSON.stringify(result, null, 2);
  }

  const full: FullExport = {
    id: topic.id,
    name: topic.name,
    exportedAt: new Date().toISOString(),
    messages: filtered.map((m) => ({
      role: m.role,
      content: m.content,
      modelId: m.modelId,
      reasoning: m.reasoning,
      toolCalls: m.toolCalls,
      agentSteps: m.agentSteps,
      inputTokens: m.inputTokens,
      outputTokens: m.outputTokens,
    })),
  };

  return JSON.stringify(full, null, 2);
}

// -------- HTML (for PDF / print) --------

/** HTML 实体转义，防止 XSS */
function escapeHtml(s: string): string {
  return s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

/**
 * 将 Markdown 文本转为简易 HTML（仅处理常见内联 + 代码块 + 列表，不引入完整 markdown 解析器）。
 * 用于 print-friendly HTML 生成。
 */
function mdToHtml(md: string): string {
  let html = md;
  // 代码块 ```...```
  html = html.replace(/```(\w*)\n([\s\S]*?)```/g, (_m, lang, code: string) =>
    `<pre><code>${escapeHtml(code.trimEnd())}</code></pre>`
  );
  // 行内代码 `...`
  html = html.replace(/`([^`\n]+)`/g, '<code>$1</code>');
  // 粗体 **...**
  html = html.replace(/\*\*([^*\n]+)\*\*/g, '<strong>$1</strong>');
  // 斜体 *...*
  html = html.replace(/\*([^*\n]+)\*/g, '<em>$1</em>');
  // 列表项 - ...
  html = html.replace(/^- (.+)$/gm, '<li>$1</li>');
  // wrap consecutive <li> in <ul>
  html = html.replace(/((?:<li>.*<\/li>\n?)+)/g, '<ul>$1</ul>');
  // 双换行 → 段落
  html = html.replace(/\n\n+/g, '</p><p>');
  // 单换行 → <br>
  html = html.replace(/\n/g, '<br>');
  return `<p>${html}</p>`;
}

/**
 * 生成适合打印/PDF 导出的完整 HTML 文档。
 * 包含内联样式，不依赖外部 CSS。
 */
export function exportAsHtml(topic: Topic, options?: ExportOptions): string {
  const opts: ExportOptions = {
    includeSystem: false,
    includeToolCalls: false,
    includeReasoning: true,
    ...options,
  };

  const messagesHtml: string[] = [];

  for (const msg of topic.history) {
    if (!shouldInclude(msg, opts)) continue;

    const isUser = msg.role === 'user';
    const roleLabel = isUser
      ? t('export.role.user')
      : (msg.modelId ? `${t('export.role.assistant')} (${msg.modelId})` : t('export.role.assistant'));
    const roleClass = isUser ? 'user' : 'assistant';

    let body = '';

    if (opts.includeReasoning && msg.reasoning) {
      body += `<blockquote class="reasoning">${escapeHtml(msg.reasoning).replace(/\n/g, '<br>')}</blockquote>`;
    }

    const text = extractText(msg);
    body += mdToHtml(text || t('export.empty'));

    if (opts.includeToolCalls && msg.toolCalls && msg.toolCalls.length > 0) {
      body += `<div class="tool-calls"><strong>${escapeHtml(t('export.toolCalls'))}:</strong><ul>`;
      for (const tc of msg.toolCalls) {
        const fn = tc.function;
        body += `<li><code>${escapeHtml(fn.name)}</code>`;
        if (fn.arguments) {
          body += `<pre><code>${escapeHtml(fn.arguments)}</code></pre>`;
        }
        body += '</li>';
      }
      body += '</ul></div>';
    }

    messagesHtml.push(`
      <div class="message ${roleClass}">
        <div class="role-badge">${escapeHtml(roleLabel)}</div>
        <div class="content">${body}</div>
      </div>`);
  }

  return `<!DOCTYPE html>
<html lang="${locale()}">
<head>
<meta charset="UTF-8">
<title>${escapeHtml(topic.name)}</title>
<style>
  * { margin: 0; padding: 0; box-sizing: border-box; }
  body {
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 14px;
    line-height: 1.7;
    color: #1a1a2e;
    background: #fff;
    max-width: 800px;
    margin: 0 auto;
    padding: 40px 32px;
  }
  h1 { font-size: 1.6em; margin-bottom: 4px; color: #111; }
  .export-time { color: #888; font-size: 0.85em; margin-bottom: 24px; }
  .message { margin-bottom: 24px; padding-bottom: 20px; border-bottom: 1px solid #eee; }
  .role-badge {
    font-weight: 700; font-size: 0.8em; text-transform: uppercase;
    letter-spacing: 0.5px; margin-bottom: 8px; color: #555;
  }
  .user .role-badge { color: #7c9abf; }
  .assistant .role-badge { color: #4a9; }
  .content p { margin-bottom: 8px; }
  .content ul { padding-left: 20px; margin: 6px 0; }
  .content li { margin-bottom: 2px; }
  .content code {
    background: #f0f0f0; padding: 1px 5px; border-radius: 3px;
    font-family: "SF Mono", "Fira Code", monospace; font-size: 0.9em;
  }
  .content pre {
    background: #f5f5f5; padding: 12px 16px; border-radius: 6px;
    overflow-x: auto; margin: 8px 0; font-size: 0.88em;
  }
  .content pre code { background: none; padding: 0; }
  .reasoning {
    border-left: 3px solid #ccc; padding: 6px 12px; margin: 8px 0;
    color: #666; font-style: italic;
  }
  .tool-calls { margin-top: 10px; font-size: 0.88em; }
  .tool-calls ul { padding-left: 18px; }
  @media print {
    body { padding: 20px 24px; }
    .message { page-break-inside: avoid; }
  }
</style>
</head>
<body>
<h1>${escapeHtml(topic.name)}</h1>
<div class="export-time">${formatTime()}</div>
${messagesHtml.join('\n')}
</body>
</html>`;
}
