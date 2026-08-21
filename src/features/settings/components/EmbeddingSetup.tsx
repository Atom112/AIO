import { Component, createEffect, createSignal, For, onCleanup, Show } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { t } from '../../../core/i18n';
import {
  embeddingDeleteOllamaModel,
  embeddingListOllamaModels,
  embeddingPullOllamaModel,
  embeddingSaveApiKey,
  embeddingTest,
} from '../../../core/utils/memory';
import type { OllamaModelInfo } from '../../../core/types/memory';

/**
 * 嵌入与记忆设置区块：provider/模型选择、连接测试、Ollama 模型下载管理。
 * 挂在 AppSettings「嵌入与记忆」分区；每项目开关在项目设置弹窗。
 */
const EmbeddingSetup: Component = () => {
  const [provider, setProvider] = createSignal<'ollama' | 'openai_compat'>('ollama');
  const [model, setModel] = createSignal('bge-m3:latest');
  const [apiUrl, setApiUrl] = createSignal('');
  const [apiKey, setApiKey] = createSignal('');
  const [dims, setDims] = createSignal(1024);
  const [testing, setTesting] = createSignal(false);
  const [testResult, setTestResult] = createSignal<{ ok: boolean; text: string } | null>(null);
  const [ollamaModels, setOllamaModels] = createSignal<OllamaModelInfo[]>([]);
  const [downloading, setDownloading] = createSignal<Record<string, number>>({});
  const [saveState, setSaveState] = createSignal<'idle' | 'saving' | 'saved' | 'error'>('idle');
  const [pullName, setPullName] = createSignal('');

  let unlisten: UnlistenFn | null = null;

  // 载入当前配置
  createEffect(() => {
    void (async () => {
      try {
        const cfg: any = await invoke('load_app_config');
        const ec = cfg?.memoryEmbedding;
        if (ec) {
          setProvider(ec.provider === 'openai_compat' ? 'openai_compat' : 'ollama');
          setModel(ec.model || 'bge-m3:latest');
          setPullName(ec.model || 'bge-m3:latest');
          setApiUrl(ec.apiUrl || '');
          if (typeof ec.dimensions === 'number' && ec.dimensions > 0) setDims(ec.dimensions);
        }
      } catch {
        // 配置读取失败时保持默认
      }
    })();
    void listen<{
      model: string;
      status: string;
      completed?: number;
      total?: number;
      error?: string;
    }>('embedding-download-progress', (e) => {
      const p = e.payload;
      const pct =
        p.total && p.total > 0
          ? Math.min(99, Math.round(((p.completed ?? 0) / p.total) * 100))
          : 99;
      setDownloading((prev) => ({
        ...prev,
        [p.model]: p.status === 'success' ? 100 : p.status === 'error' ? -1 : pct,
      }));
    }).then((f) => {
      unlisten = f;
    });
  });

  onCleanup(() => {
    unlisten?.();
  });

  // Ollama 已安装模型列表
  const refreshOllamaModels = async () => {
    try {
      setOllamaModels(await embeddingListOllamaModels(apiUrl() || undefined));
    } catch {
      setOllamaModels([]);
    }
  };

  createEffect(() => {
    if (provider() === 'ollama') void refreshOllamaModels();
  });

  const save = async () => {
    setSaveState('saving');
    try {
      const cfg: any = await invoke('load_app_config');
      await invoke('save_app_config', {
        config: {
          ...cfg,
          memoryEmbedding: {
            provider: provider(),
            model: model(),
            apiUrl: apiUrl(),
            dimensions: dims(),
            enabled: true,
          },
        },
      });
      if (provider() === 'openai_compat') {
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
        provider: provider(),
        model: model(),
        apiUrl: apiUrl(),
        dimensions: dims(),
        enabled: true,
        apiKey: provider() === 'openai_compat' ? apiKey() : '',
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

  const download = async (m: string) => {
    setDownloading((prev) => ({ ...prev, [m]: 0 }));
    try {
      await embeddingPullOllamaModel(m, apiUrl() || undefined);
    } catch (err) {
      console.warn('发起下载失败:', err);
      setDownloading((prev) => ({ ...prev, [m]: -1 }));
    }
  };

  // 按名称拉取尚未安装的模型（独立于已安装列表）。
  const pullByName = () => {
    const name = pullName().trim();
    if (name) void download(name);
  };

  const removeModel = async (m: string) => {
    try {
      await embeddingDeleteOllamaModel(m, apiUrl() || undefined);
      await refreshOllamaModels();
    } catch (err) {
      console.warn('删除模型失败:', err);
    }
  };

  const formatBytes = (n: number) => {
    if (n >= 1024 * 1024 * 1024) return (n / (1024 * 1024 * 1024)).toFixed(1) + ' GB';
    if (n >= 1024 * 1024) return (n / (1024 * 1024)).toFixed(0) + ' MB';
    return Math.max(1, Math.round(n / 1024)) + ' KB';
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
              provider() === 'ollama'
                ? 'px-3 py-1.5 rounded-lg text-xs font-semibold bg-pri-20 text-pri border border-pri/30'
                : 'px-3 py-1.5 rounded-lg text-xs bg-dark-300 border border-dark-100 text-white/50'
            }
            onClick={() => setProvider('ollama')}
          >
            {t('app.memory.providerOllama')}
          </button>
          <button
            class={
              provider() === 'openai_compat'
                ? 'px-3 py-1.5 rounded-lg text-xs font-semibold bg-pri-20 text-pri border border-pri/30'
                : 'px-3 py-1.5 rounded-lg text-xs bg-dark-300 border border-dark-100 text-white/50'
            }
            onClick={() => setProvider('openai_compat')}
          >
            {t('app.memory.providerOpenai')}
          </button>
        </div>
      </div>

      <div class="flex flex-col gap-1.5">
        <label class={labelCls}>{t('app.memory.model')}</label>
        <input
          class={inputCls}
          value={model()}
          onInput={(e) => setModel(e.currentTarget.value)}
          placeholder="bge-m3:latest"
        />
      </div>

      <div class="flex flex-col gap-1.5">
        <label class={labelCls}>{t('app.memory.apiUrl')}</label>
        <input
          class={inputCls}
          value={apiUrl()}
          onInput={(e) => setApiUrl(e.currentTarget.value)}
          placeholder="http://127.0.0.1:11434"
        />
      </div>

      <Show when={provider() === 'openai_compat'}>
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

      <Show when={provider() === 'ollama'}>
        <div class="flex flex-col gap-1">
          <label class={labelCls}>{t('app.memory.pullModelLabel')}</label>
          <div class="flex gap-2">
            <input
              class={inputCls}
              value={pullName()}
              onInput={(e) => setPullName(e.currentTarget.value)}
              placeholder={t('app.memory.pullPlaceholder')}
            />
            <button
              class="px-3 py-1.5 rounded-lg text-xs font-semibold bg-pri-20 text-pri border border-pri/30 shrink-0 cursor-pointer hover:bg-pri-30 transition-colors"
              onClick={pullByName}
            >
              {t('app.memory.pullBy')}
            </button>
          </div>
          <label class={labelCls}>{t('app.memory.ollamaModels')}</label>
          <For each={ollamaModels()}>
            {(m) => {
              const pct = downloading()[m.name];
              return (
                <div class="flex items-center gap-2 text-xs py-1 border-b border-white/5 last:border-0">
                  <span class="flex-1 text-white/80 truncate">{m.name}</span>
                  <span class="text-white/35 shrink-0">{formatBytes(m.sizeBytes)}</span>
                  <Show when={pct === undefined}>
                    <button
                      class="px-2 py-0.5 rounded bg-dark-300 border border-dark-100 text-white/60 cursor-pointer hover:bg-dark-200"
                      onClick={() => void download(m.name)}
                    >
                      {t('app.memory.download')}
                    </button>
                    <button
                      class="px-2 py-0.5 rounded bg-dark-300 border border-dark-100 text-white/40 cursor-pointer hover:bg-dark-200"
                      onClick={() => void removeModel(m.name)}
                    >
                      {t('app.memory.deleteModel')}
                    </button>
                  </Show>
                  <Show when={pct !== undefined && pct >= 0 && pct < 100}>
                    <span class="text-pri/80 shrink-0">
                      {t('app.memory.downloading', { pct: String(pct) })}
                    </span>
                  </Show>
                  <Show when={pct === 100}>
                    <span class="text-green-400/90 shrink-0">✓</span>
                  </Show>
                  <Show when={pct === -1}>
                    <span class="text-red-400/90 shrink-0">✗</span>
                  </Show>
                </div>
              );
            }}
          </For>
          <Show when={ollamaModels().length === 0}>
            <div class="text-xs text-white/30">{t('app.memory.noModels')}</div>
          </Show>
        </div>
      </Show>
    </div>
  );
};

export default EmbeddingSetup;
