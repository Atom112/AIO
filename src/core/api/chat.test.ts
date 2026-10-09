import { describe, expect, it, vi } from 'vitest';

// llmCreds is a pure helper, but chat.ts depends on @tauri-apps and the store, so mock them in Node
// and test only the llmCreds field-mapping logic.
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));
vi.mock('../store/profiles', () => ({}));
vi.mock('../store/types', () => ({}));
vi.mock('../types/skill', () => ({}));

const { llmCreds } = await import('./chat');

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
