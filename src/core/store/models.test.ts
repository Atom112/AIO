import { describe, expect, it, vi } from 'vitest';

// The models domain is a leaf (depends only on types/utils); its two pure helpers modelKey /
// isLocalModel underpin assistant model binding resolution and deserve dedicated regression coverage.
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
// The models domain indirectly depends on i18n (module init reads navigator/localStorage) and the models-data package — mocked out under Node.
vi.mock('../i18n', () => ({ t: (k: string) => k, locale: () => 'zh-CN' }));
vi.mock('@aio/models-data', () => ({}));

const { isLocalModel, modelKey } = await import('./models');

// isLocalModel semantics: local_path or engine_type alone marks a model as local (api_url is not considered).
describe('isLocalModel', () => {
  it('有 local_path 判定为本地模型', () => {
    expect(isLocalModel({ local_path: '/models/x.gguf' } as never)).toBe(true);
  });
  it('有 engine_type（如 ollama 远端引擎）判定为本地/引擎托管', () => {
    expect(isLocalModel({ engine_type: 'ollama' } as never)).toBe(true);
  });
  it('云端模型（仅 api_url）判定为非本地', () => {
    expect(isLocalModel({ api_url: 'https://api.openai.com' } as never)).toBe(false);
    expect(isLocalModel({ api_url: 'http://127.0.0.1:8080' } as never)).toBe(false);
  });
});

describe('modelKey', () => {
  it('云端模型使用 model_id@api_url 复合键', () => {
    expect(modelKey({ model_id: 'gpt-4o', api_url: 'https://api.openai.com' } as never)).toBe(
      'gpt-4o@https://api.openai.com',
    );
  });
  it('本地模型退化为纯 model_id', () => {
    expect(modelKey({ model_id: 'llama3', local_path: '/m/x.gguf' } as never)).toBe('llama3');
  });
  it('无 api_url 时退化为纯 model_id', () => {
    expect(modelKey({ model_id: 'x' } as never)).toBe('x');
  });
});
