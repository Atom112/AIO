import { describe, expect, it, vi } from 'vitest';

// shortcuts 模块在加载期恢复 localStorage 绑定并取 i18n 词表——node 环境
// 需要先打桩再动态导入（导入必须发生在 stub 生效之后）。i18n 是目录模块，
// 用 shortcuts.ts 的原始说明符 ./i18n（同时也是本文件视角的同一路径）确保
// mock 命中。
vi.mock('./i18n', () => ({ t: (key: string) => key }));
vi.stubGlobal('localStorage', {
  getItem: () => null,
  setItem: () => {},
  removeItem: () => {},
});

const { getRegisteredCommands, findLeadingSlashCommand } = await import('./shortcuts');

describe('findLeadingSlashCommand', () => {
  it('locates only the recognized command token at the start', () => {
    expect(findLeadingSlashCommand('/compact keep this')).toEqual({
      start: 0,
      end: 8,
      token: '/compact',
    });
    expect(findLeadingSlashCommand('  /compact')).toEqual({
      start: 2,
      end: 10,
      token: '/compact',
    });
  });

  it('does not color partial, unknown, or non-leading commands', () => {
    expect(findLeadingSlashCommand('/com')).toBeNull();
    expect(findLeadingSlashCommand('/not-a-command')).toBeNull();
    expect(findLeadingSlashCommand('hello /compact')).toBeNull();
    expect(findLeadingSlashCommand('https://example.com')).toBeNull();
  });
});

describe('default shortcut uniqueness (FE-03)', () => {
  it('factory defaults have no key conflicts', () => {
    // 注册序决定 getActionByKeys 的胜者——出厂默认撞键即"永不可触发"，
    // 且历史冲突检测只在用户录制时运行，对默认表不设防
    const seen = new Map<string, string>();
    for (const cmd of getRegisteredCommands()) {
      if (!cmd.defaultKeys) continue;
      const owner = seen.get(cmd.defaultKeys);
      expect(
        owner,
        `默认键 ${cmd.defaultKeys} 同时绑定 ${owner} 与 ${cmd.id}（${cmd.id} 将永不可触发）`,
      ).toBeUndefined();
      seen.set(cmd.defaultKeys, cmd.id);
    }
  });
});
