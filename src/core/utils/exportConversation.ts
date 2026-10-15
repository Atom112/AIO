/**
 * Conversation export pure functions - Markdown / JSON generation.
 * Takes a Topic + options, returns a formatted string; no UI dependencies.
 */
import type { Topic, Message } from '../store/store';
import { formatDateTime, locale, t } from '../i18n';

// -------- Option types --------

export interface ExportOptions {
  /** Whether to include system-role messages (default false) */
  includeSystem?: boolean;
  /** Whether to append tool call descriptions after assistant messages (default false) */
  includeToolCalls?: boolean;
  /** Whether to include reasoning content (default true) */
  includeReasoning?: boolean;
  /** Export only messages with these IDs; export all when unset */
  selectedMessageIds?: Set<string>;
}

// -------- Helpers --------

/** Format the current timestamp as YYYY-MM-DD HH:mm */
function formatTime(): string {
  return formatDateTime(new Date(), { dateStyle: 'medium', timeStyle: 'short' });
}

/** Extract plain text from a message. Prefers displayText, then content (if an array, joins each item.text). */
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

/** Decide whether a message should be exported (role filtering + message ID selection) */
function shouldInclude(msg: Message, opts: ExportOptions): boolean {
  if (msg.role === 'tool') return false;
  if (msg.role === 'system' && !opts.includeSystem) return false;
  if (opts.selectedMessageIds && msg.id && !opts.selectedMessageIds.has(msg.id)) return false;
  return true;
}

// -------- Markdown --------

/**
 * Export a topic as a Markdown string.
 * @param topic full topic object
 * @param options filter / formatting options
 */
export function exportAsMarkdown(topic: Topic, options?: ExportOptions): string {
  const opts: ExportOptions = {
    includeSystem: false,
    includeToolCalls: false,
    includeReasoning: true,
    ...options,
  };

  const lines: string[] = [];

  // Title
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

      // reasoning (blockquote)
      if (opts.includeReasoning && msg.reasoning) {
        lines.push('');
        for (const line of msg.reasoning.split('\n')) {
          lines.push(`> ${line}`);
        }
        lines.push('');
      }

      // Body text
      const text = extractText(msg);
      lines.push(text || t('export.empty'));

      // tool call descriptions (optional)
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
  /** 'full': full Topic + metadata; 'simple': role+content array only */
  mode?: 'full' | 'simple';
  /** Export only messages with these IDs; export all when unset */
  selectedMessageIds?: Set<string>;
}

/** Simple mode: each message extracts role + content (content keeps its original type) */
interface SimpleMessage {
  role: string;
  content: unknown;
}

/** Full mode: keeps all key message fields */
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

/** Message-level filter: skip the tool role and filter by selectedMessageIds */
function filterMessages(msgs: Message[], ids?: Set<string>): Message[] {
  return msgs.filter((m) => {
    if (m.role === 'tool') return false;
    if (ids && m.id && !ids.has(m.id)) return false;
    return true;
  });
}

/**
 * Export a topic as a pretty-printed JSON string.
 * @param topic full topic object
 * @param options mode selection + message filtering
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

/** Escape HTML entities to prevent XSS */
function escapeHtml(s: string): string {
  return s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

/**
 * Convert Markdown text to simple HTML (only common inline styles + code blocks + lists; no full markdown parser).
 * Used for print-friendly HTML generation.
 *
 * CHAT-1: the whole input is escaped BEFORE markup replacement — the historic
 * implementation only escaped the captured code/bold fragments, so plain
 * paragraph text passed through raw into the sandboxless srcdoc iframe
 * (form-action / meta-refresh navigation is not constrained by the page CSP,
 * enabling in-app phishing / session-content exfiltration links).
 */
function mdToHtml(md: string): string {
  let html = escapeHtml(md);
  // Code block ```...``` (content is already escaped above)
  html = html.replace(
    /```(\w*)\n([\s\S]*?)```/g,
    (_m, _lang, code: string) => `<pre><code>${code.trimEnd()}</code></pre>`,
  );
  // Inline code `...`
  html = html.replace(/`([^`\n]+)`/g, (_m, code: string) => `<code>${code}</code>`);
  // Bold **...**
  html = html.replace(/\*\*([^*\n]+)\*\*/g, (_m, s: string) => `<strong>${s}</strong>`);
  // Italic *...*
  html = html.replace(/\*([^*\n]+)\*/g, (_m, s: string) => `<em>${s}</em>`);
  // List item - ...
  html = html.replace(/^- (.+)$/gm, (_m, s: string) => `<li>${s}</li>`);
  // wrap consecutive <li> in <ul>
  html = html.replace(/((?:<li>.*<\/li>\n?)+)/g, '<ul>$1</ul>');
  // Double newline -> paragraph
  html = html.replace(/\n\n+/g, '</p><p>');
  // Single newline -> <br>
  html = html.replace(/\n/g, '<br>');
  return `<p>${html}</p>`;
}

/**
 * Generate a complete HTML document suited for print/PDF export.
 * Includes inline styles; no external CSS dependency.
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
      : msg.modelId
        ? `${t('export.role.assistant')} (${msg.modelId})`
        : t('export.role.assistant');
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
