import { describe, expect, it, vi, beforeEach } from 'vitest';
import { invoke } from '@tauri-apps/api/core';

// llmCreds is a pure helper, but chat.ts depends on @tauri-apps and the store, so mock them in Node
// and test only the llmCreds field-mapping logic.
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));
vi.mock('../store/profiles', () => ({}));
vi.mock('../store/types', () => ({}));
vi.mock('../types/skill', () => ({}));

const { llmCreds, runAgentTurn, stopActiveLlmStream, clearStreamKey, reconcileActiveStreamKeys } =
  await import('./chat');

describe('llmCreds', () => {
  it('从 ActivatedModel 字段转换为 LLM 凭证', () => {
    const creds = llmCreds({ api_url: 'https://api.x.com', api_key: 'k1', model_id: 'gpt-x' });
    expect(creds).toEqual({ apiUrl: 'https://api.x.com', apiKey: 'k1', model: 'gpt-x' });
  });
  it('api_key 缺省/null 时回退为空串', () => {
    expect(llmCreds({ api_url: 'u', model_id: 'm' }).apiKey).toBe('');
    expect(llmCreds({ api_url: 'u', api_key: null, model_id: 'm' }).apiKey).toBe('');
  });
});

describe('流归属键（停止按钮目标）', () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset();
  });

  it('停止命中 runAgentTurn 记录的流键', async () => {
    const args = { assistantId: 'a1', topicId: 't1' } as unknown as Parameters<
      typeof runAgentTurn
    >[0];
    void runAgentTurn(args);
    await stopActiveLlmStream();
    expect(invoke).toHaveBeenCalledWith('stop_llm_stream', { assistantId: 'a1', topicId: 't1' });
    clearStreamKey('a1', 't1');
  });

  it('done 后键被清除，停止变为空转', async () => {
    const args = { assistantId: 'a2', topicId: 't2' } as unknown as Parameters<
      typeof runAgentTurn
    >[0];
    void runAgentTurn(args);
    clearStreamKey('a2', 't2');
    await stopActiveLlmStream();
    expect(invoke).not.toHaveBeenCalledWith('stop_llm_stream', {
      assistantId: 'a2',
      topicId: 't2',
    });
  });

  it('多条流时优先停止当前视图的流', async () => {
    void runAgentTurn({ assistantId: 'a3', topicId: 't3' } as unknown as Parameters<
      typeof runAgentTurn
    >[0]);
    void runAgentTurn({ assistantId: 'a4', topicId: 't4' } as unknown as Parameters<
      typeof runAgentTurn
    >[0]);
    await stopActiveLlmStream('a3', 't3');
    expect(invoke).toHaveBeenCalledWith('stop_llm_stream', { assistantId: 'a3', topicId: 't3' });
    clearStreamKey('a3', 't3');
    clearStreamKey('a4', 't4');
  });

  it('启动对账导入后端活动流（webview 重载后停止恢复可用）', async () => {
    vi.mocked(invoke).mockResolvedValueOnce([{ assistantId: 'ra', topicId: 'rt' }]);
    const n = await reconcileActiveStreamKeys();
    expect(n).toBe(1);
    await stopActiveLlmStream();
    expect(invoke).toHaveBeenCalledWith('stop_llm_stream', { assistantId: 'ra', topicId: 'rt' });
    clearStreamKey('ra', 'rt');
  });
});
