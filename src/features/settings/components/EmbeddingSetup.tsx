import { Component, createEffect, createSignal, Show } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import { t } from '../../../core/i18n';
import { embeddingSaveApiKey, embeddingTest } from '../../../core/utils/memory';
import type { MemoryEmbeddingConfig } from '../../../core/types/memory';

/**
 * 嵌入与记忆设置区块：provider/模型、在线连接测试。
 * - 本地内置（默认，开箱即用）：随应用打包的小型模型，无需下载与配置，离线可用。
 * - 在线 API：OpenAI 兼容 /v1/embeddings 端点，供更强的语义检索。
 * 挂在 AppSettings「嵌入与记忆」分区；每项目开关在项目设置弹窗。
 */
const EmbeddingSetup: Component = () => {
  const [provider, setProvider] = createSignal<'local' | 'online'>('local');
  const [model, setModel] = createSignal('all-MiniLM-L6-v2');
  const [apiUrl, setApiUrl] = createSignal('');
  const [apiKey, setApiKey] = createSignal('');
  const [dims, setDims] = createSignal(384);
  const [testing, setTesting] = createSignal(false);
  const [testResult, setTestResult] = createSignal<{ ok: boolean; text: string } | null>(null);
  const [saveState, setSaveState] = createSignal<'idle' | 'saving' | 'saved' | 'error'>('idle');

  // 载入当前配置（兼容旧 provider 值：openai_compat -> online，ollama -> local）
  createEffect(() => {
    void (async () => {
      try {
        const cfg: any = await invoke('load_app_config');
        const ec = cfg?.memoryEmbedding;
        if (ec) {
          setProvider(
            ec.provider === 'online' || ec.provider === 'openai_compat' ? 'online' : 'local',
          );
          setModel(ec.model || 'all-MiniLM-L6-v2');
          setApiUrl(ec.apiUrl || '');
          if (typeof ec.dimensions === 'number' && ec.dimensions > 0) setDims(ec.dimensions);
        }
      } catch {
        // 配置读取失败时保持默认
      }
    })();
  });

  const baseCfg = (): MemoryEmbeddingConfig =>
    provider() === 'local'
      ? { provider: 'local', model: 'all-MiniLM-L6-v2', apiUrl: '', dimensions: 384, enabled: true }
      : {
          provider: 'online',
          model: model(),
          apiUrl: apiUrl(),
          dimensions: dims(),
          enabled: true,
        };

  const save = async () => {
    setSaveState('saving');
    try {
      const cfg: any = await invoke('load_app_config');
      await invoke('save_app_config', {
        config: { ...cfg, memoryEmbedding: baseCfg() },
      });
      if (provider() === 'online') {
        await embeddingSaveApiKey(apiKey());
      }
      setSaveState('saved');
    } catch (err) {
      console.warn('保存嵌入配置失败:', err);
      setSaveState('error');
    }
  };

  const test = async () => {
    setTesting(true);
    setTestResult(null);
    try {
      const res = await embeddingTest({
        ...baseCfg(),
        apiKey: provider() === 'online' ? apiKey() : '',
      });
      if (res.ok) {
        setTestResult({
          ok: true,
          text: t('app.memory.testOk', { dims: String(res.dimensions), ms: String(res.latencyMs) }),
        });
      } else {
        setTestResult({
          ok: false,
          text: t('app.memory.testFail', { error: res.error ?? 'unknown' }),
        });
      }
    } catch (err) {
      setTestResult({ ok: false, text: t('app.memory.testFail', { error: String(err) }) });
    } finally {
      setTesting(false);
    }
  };

  const inputCls =
    'w-full p-2 bg-dark-300 border border-dark-100 rounded-lg text-sm text-[#e0e0e0] focus:outline-none focus:border-white/20';
  const labelCls = 'text-[10px] text-white/45 uppercase tracking-[1.5px] font-semibold mb-1 block';

  return (
    <div class="flex flex-col gap-3 py-3 border-b border-white/5">
      <div>
        <span class="block text-[#eee] text-[14px]">{t('app.memory.title')}</span>
        <p class="text-xs text-white/35 mt-1">{t('app.memory.description')}</p>
      </div>

      <div class="flex flex-col gap-2">
        <label class={labelCls}>{t('app.memory.provider')}</label>
        <div class="flex gap-2">
          <button
            class={
              provider() === 'local'
                ? 'px-3 py-1.5 rounded-lg text-xs font-semibold bg-pri-20 text-pri border border-pri/30'
                : 'px-3 py-1.5 rounded-lg text-xs bg-dark-300 border border-dark-100 text-white/50'
            }
            onClick={() => setProvider('local')}
          >
            {t('app.memory.providerLocal')}
          </button>
          <button
            class={
              provider() === 'online'
                ? 'px-3 py-1.5 rounded-lg text-xs font-semibold bg-pri-20 text-pri border border-pri/30'
                : 'px-3 py-1.5 rounded-lg text-xs bg-dark-300 border border-dark-100 text-white/50'
            }
            onClick={() => setProvider('online')}
          >
            {t('app.memory.providerOnline')}
          </button>
        </div>
      </div>

      <Show when={provider() === 'local'}>
        <div class="flex flex-col gap-1.5">
          <label class={labelCls}>{t('app.memory.localModel')}</label>
          <p class="text-xs text-white/60">{t('app.memory.localModelDesc')}</p>
        </div>
      </Show>

      <Show when={provider() === 'online'}>
        <div class="flex flex-col gap-1.5">
          <label class={labelCls}>{t('app.memory.model')}</label>
          <input
            class={inputCls}
            value={model()}
            onInput={(e) => setModel(e.currentTarget.value)}
            placeholder="text-embedding-3-small"
          />
        </div>
        <div class="flex flex-col gap-1.5">
          <label class={labelCls}>{t('app.memory.apiUrl')}</label>
          <input
            class={inputCls}
            value={apiUrl()}
            onInput={(e) => setApiUrl(e.currentTarget.value)}
            placeholder="https://api.openai.com/v1"
          />
        </div>
        <div class="flex flex-col gap-1.5">
          <label class={labelCls}>{t('app.memory.apiKey')}</label>
          <input
            class={inputCls}
            type="password"
            value={apiKey()}
            onInput={(e) => setApiKey(e.currentTarget.value)}
            placeholder="sk-..."
          />
        </div>
      </Show>

      <div class="flex items-center gap-2">
        <button
          class="px-3 py-1.5 rounded-lg text-xs font-semibold bg-pri-20 text-pri border border-pri/30 cursor-pointer hover:bg-pri-30 transition-colors"
          onClick={save}
        >
          {saveState() === 'saving'
            ? '...'
            : saveState() === 'saved'
              ? '✓'
              : saveState() === 'error'
                ? '✗'
                : t('app.memory.save')}
        </button>
        <button
          class="px-3 py-1.5 rounded-lg text-xs bg-dark-300 border border-dark-100 text-white/60 cursor-pointer hover:bg-dark-200 transition-colors"
          disabled={testing()}
          onClick={test}
        >
          {testing() ? t('app.memory.testing') : t('app.memory.test')}
        </button>
      </div>

      <Show when={testResult()}>
        <div
          class="text-xs"
          style={{ color: testResult()?.ok ? 'rgba(74,222,128,0.9)' : 'rgba(248,113,113,0.9)' }}
        >
          {testResult()?.text}
        </div>
      </Show>
    </div>
  );
};

export default EmbeddingSetup;
