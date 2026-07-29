/**
 * 分享弹窗 — 对话导出为 Markdown / JSON / Screenshot / PDF，支持预览、复制、下载。
 * 仅纯对话模式（isChatMode()）可用；外部需通过 open/onClose 控制显隐。
 */
import {
  Component,
  createSignal,
  createMemo,
  createEffect,
  Show,
  Switch,
  Match,
  For,
} from 'solid-js';
import { Portal } from 'solid-js/web';
import { toCanvas, toPng } from 'html-to-image';
import { save } from '@tauri-apps/plugin-dialog';
import { writeFile } from '@tauri-apps/plugin-fs';
import { marked } from 'marked';
import DOMPurify from 'dompurify';
import { jsPDF } from 'jspdf';
import type { Topic, Message } from '../../../core/store/store';
import {
  exportAsMarkdown,
  exportAsJSON,
  exportAsHtml,
  type ExportOptions,
} from '../../../core/utils/exportConversation';
import Icon from '../../../shared/components/Icon';
import { locale, t } from '../../../core/i18n';
type Tab = 'markdown' | 'json' | 'screenshot' | 'pdf';
type JsonMode = 'full' | 'simple';
type ShotFormat = 'png' | 'jpeg';
type ShotWidth = 'narrow' | 'wide';

interface ShareModalProps {
  open: boolean;
  onClose: () => void;
  topic: Topic | null;
  /** 消息级筛选：仅导出指定 ID 的消息；undefined = 全部 */
  selectedMessageIds?: Set<string>;
}

// -------- standalone utils --------
/** 触发下载：优先走 Tauri 原生保存对话框，失败则回退到 Blob + anchor */
async function downloadBlob(content: string | Blob, filename: string, mime: string) {
  const blob = content instanceof Blob ? content : new Blob([content], { type: mime });
  try {
    const buf = new Uint8Array(await blob.arrayBuffer());
    const filePath = await save({ defaultPath: filename });
    if (filePath) {
      await writeFile(filePath, buf);
      return;
    }
    // 用户取消了保存对话框 → 静默结束
  } catch {
    // 非 Tauri 环境或写文件失败 → 回退到浏览器下载
  }
  fallbackDownload(blob, filename);
}

function fallbackDownload(blob: Blob, filename: string) {
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  URL.revokeObjectURL(url);
}

async function copyToClipboard(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}

/** 拷贝图片到剪贴板（PNG blob） */
async function copyImageToClipboard(dataUrl: string): Promise<boolean> {
  try {
    const res = await fetch(dataUrl);
    const blob = await res.blob();
    await navigator.clipboard.write([new ClipboardItem({ [blob.type]: blob })]);
    return true;
  } catch {
    return false;
  }
}

/** 从消息提取纯文本 */
function extractText(msg: Message): string {
  if (msg.displayText != null) return String(msg.displayText);
  if (typeof msg.content === 'string') return msg.content;
  if (Array.isArray(msg.content))
    return msg.content
      .map((c: any) => c?.text ?? '')
      .filter(Boolean)
      .join('\n');
  return '';
}

/** 将消息内容渲染为 HTML：assistant 消息走 marked 解析，user 消息保持纯文本（转义） */
function renderMessageHtml(msg: Message): string {
  const raw = extractText(msg);
  if (!raw) return `<span style="opacity:0.4">${t('export.empty')}</span>`;
  if (msg.role === 'user') {
    return raw
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/\n/g, '<br>');
  }
  // assistant: parse markdown
  return DOMPurify.sanitize(marked.parse(raw, { async: false }) as string);
}

// -------- Screenshot preview: message bubble renderer (used in hidden capture container) --------

const ScreenshotBubble: Component<{ msg: Message; wide: boolean; includeReasoning: boolean }> = (
  p,
) => {
  const isUser = createMemo(() => p.msg.role === 'user');
  const html = () => renderMessageHtml(p.msg);
  return (
    <div
      style={{
        display: 'flex',
        'flex-direction': 'column',
        'align-items': isUser() ? 'flex-end' : 'flex-start',
        margin: '12px 0',
        'max-width': '100%',
      }}
    >
      <div
        style={{
          'font-size': '11px',
          'font-weight': '600',
          'text-transform': 'uppercase',
          'letter-spacing': '0.3px',
          'margin-bottom': '4px',
          color: isUser() ? '#7c9abf' : '#4a9',
          'padding-left': isUser() ? '0' : '4px',
          'padding-right': isUser() ? '4px' : '0',
        }}
      >
        {isUser()
          ? t('export.roleYou')
          : p.msg.modelId
            ? `${t('export.roleAssistant')} (${p.msg.modelId})`
            : t('export.roleAssistant')}
      </div>
      <Show when={p.includeReasoning && p.msg.reasoning && !isUser()}>
        <div
          style={{
            'border-left': '3px solid #555',
            padding: '6px 12px',
            margin: '4px 0 8px',
            color: '#999',
            'font-style': 'italic',
            'font-size': '13px',
            'line-height': '1.5',
            'max-width': p.wide ? '640px' : '420px',
            background: 'rgba(var(--text-base-rgb),0.03)',
            'border-radius': '0 6px 6px 0',
          }}
        >
          {p.msg.reasoning}
        </div>
      </Show>
      <div
        class="shot-bubble-content"
        style={{
          padding: '10px 14px',
          'border-radius': '12px',
          'font-size': '14px',
          'line-height': '1.65',
          'max-width': p.wide ? '640px' : '420px',
          'word-break': 'break-word',
          background: isUser()
            ? 'rgba(var(--primary-rgb),0.18)'
            : 'rgba(var(--text-base-rgb),0.06)',
          border: isUser()
            ? '1px solid rgba(var(--primary-rgb),0.25)'
            : '1px solid rgba(var(--text-base-rgb),0.08)',
          color: '#e0e0e0',
        }}
        // eslint-disable-next-line solid/no-innerhtml -- renderMessageHtml sanitizes content
        innerHTML={html()}
      />
    </div>
  );
};

// -------- component --------

const ShareModal: Component<ShareModalProps> = (props) => {
  const [activeTab, setActiveTab] = createSignal<Tab>('screenshot');
  const [jsonMode, setJsonMode] = createSignal<JsonMode>('full');
  const [includeReasoning, setIncludeReasoning] = createSignal(true);
  const [includeSystem, setIncludeSystem] = createSignal(false);
  const [includeToolCalls, setIncludeToolCalls] = createSignal(false);
  const [isEntering, setIsEntering] = createSignal(true);
  const [isExiting, setIsExiting] = createSignal(false);
  const [copied, setCopied] = createSignal<string | null>(null);

  // screenshot options
  const [shotFormat, setShotFormat] = createSignal<ShotFormat>('png');
  const [shotWidth, setShotWidth] = createSignal<ShotWidth>('wide');
  const [shotCapturing, setShotCapturing] = createSignal(false);

  // pdf state
  const [pdfCapturing, setPdfCapturing] = createSignal(false);

  let captureRef: HTMLDivElement | undefined;
  let pdfIframeRef: HTMLIFrameElement | undefined;

  const topic = () => props.topic;

  const currentOpts = (): ExportOptions => ({
    includeReasoning: includeReasoning(),
    includeSystem: includeSystem(),
    includeToolCalls: includeToolCalls(),
    selectedMessageIds: props.selectedMessageIds,
  });

  const mdContent = createMemo(() => {
    const t = topic();
    return t ? exportAsMarkdown(t, currentOpts()) : '';
  });

  const jsonContent = createMemo(() => {
    const t = topic();
    return t
      ? exportAsJSON(t, { mode: jsonMode(), selectedMessageIds: props.selectedMessageIds })
      : '';
  });

  const handleCopy = async (mode: 'markdown' | 'json') => {
    const text = mode === 'markdown' ? mdContent() : jsonContent();
    const ok = await copyToClipboard(text);
    if (ok) {
      setCopied(mode);
      setTimeout(() => setCopied(null), 2000);
    }
  };

  const handleDownload = async (mode: 'markdown' | 'json') => {
    const t = topic();
    if (!t) return;
    const safeName = t.name.replace(/[<>:"/\\|?*]/g, '_');
    const content = mode === 'markdown' ? mdContent() : jsonContent();
    const ext = mode === 'markdown' ? 'md' : 'json';
    const mime = mode === 'markdown' ? 'text/markdown' : 'application/json';
    await downloadBlob(content, `${safeName}.${ext}`, mime);
  };
  const htmlContent = createMemo(() => {
    const t = topic();
    return t ? exportAsHtml(t, currentOpts()) : '';
  });

  /** 需要过滤的消息列表（跳过 tool 和可选的 system，并按 selectedMessageIds 筛选） */
  const visibleMessages = createMemo(() => {
    const t = topic();
    if (!t) return [];
    const ids = props.selectedMessageIds;
    return t.history.filter((m) => {
      if (m.role === 'tool') return false;
      if (m.role === 'system' && !includeSystem()) return false;
      if (ids && ids.size > 0 && m.id && !ids.has(m.id)) return false;
      return true;
    });
  });

  // 弹窗打开时触发入场动画
  createEffect(() => {
    if (props.open) {
      setIsEntering(true);
      setTimeout(() => setIsEntering(false), 0);
    }
  });

  const handleClose = () => {
    setIsExiting(true);
    setTimeout(() => {
      setIsExiting(false);
      setCopied(null);
      props.onClose();
    }, 300);
  };

  // ---- screenshot ----

  const handleScreenshotCapture = async (action: 'download' | 'copy') => {
    if (!captureRef) return;
    setShotCapturing(true);
    try {
      const dataUrl = await toPng(captureRef, {
        pixelRatio: 2,
        backgroundColor: '#111827',
        quality: 0.95,
      });
      if (action === 'download') {
        const res = await fetch(dataUrl);
        const blob = await res.blob();
        const t = topic()!;
        const safeName = t.name.replace(/[<>:"/\\|?*]/g, '_');
        const ext = shotFormat() === 'jpeg' ? 'jpg' : 'png';
        await downloadBlob(blob, `${safeName}.${ext}`, `image/${shotFormat()}`);
      } else {
        const ok = await copyImageToClipboard(dataUrl);
        if (ok) {
          setCopied('screenshot');
          setTimeout(() => setCopied(null), 2000);
        }
      }
    } catch (e) {
      console.error('Screenshot failed:', e);
    } finally {
      setShotCapturing(false);
    }
  };

  // ---- pdf ----

  const handlePdfDownload = async () => {
    setPdfCapturing(true);
    try {
      const iframeEl = pdfIframeRef;
      if (!iframeEl?.contentDocument?.body) {
        console.error('PDF iframe not ready');
        setPdfCapturing(false);
        return;
      }
      const topicName = topic()?.name || 'conversation';
      const body = iframeEl.contentDocument.body;

      const canvas = await toCanvas(body, {
        backgroundColor: '#ffffff',
        pixelRatio: 2,
        width: body.scrollWidth,
        height: body.scrollHeight,
      });

      // Create PDF with A4 dimensions (595 x 842 pt)
      const doc = new jsPDF('p', 'pt', 'a4');
      const pageWidth = doc.internal.pageSize.getWidth();
      const pageHeight = doc.internal.pageSize.getHeight();
      const margin = 20;
      const contentWidth = pageWidth - margin * 2;
      const contentHeight = pageHeight - margin * 2;

      // Scale canvas to fit page width
      const imgWidth = contentWidth;
      const imgHeight = (canvas.height * contentWidth) / canvas.width;

      // Split into pages if needed
      let remainingHeight = imgHeight;
      let page = 0;

      while (remainingHeight > 0) {
        if (page > 0) doc.addPage();
        const sliceHeight = Math.min(remainingHeight, contentHeight);
        doc.addImage(
          canvas.toDataURL('image/png'),
          'PNG',
          margin,
          margin,
          imgWidth,
          sliceHeight,
          undefined,
          'FAST',
        );

        remainingHeight -= sliceHeight;
        page++;
      }

      const pdfBytes = doc.output('arraybuffer');
      const filePath = await save({
        defaultPath: `${topicName}.pdf`,
        filters: [{ name: 'PDF', extensions: ['pdf'] }],
      });

      if (filePath) {
        await writeFile(filePath, new Uint8Array(pdfBytes));
      }
    } catch (e) {
      console.error('PDF export failed:', e);
    } finally {
      setPdfCapturing(false);
    }
  };

  const tab = () => activeTab();

  // ---- filter options row (shared by markdown / screenshot / pdf) ----

  /** 自定义复选框（内联组件，避免额外文件依赖） */
  const Checkbox = (p: { checked: boolean; onChange: (v: boolean) => void; label: string }) => (
    <label
      class="flex items-center gap-1.5 cursor-pointer select-none hover:text-white/70 transition-colors"
      onClick={() => p.onChange(!p.checked)}
    >
      <div
        class="w-4 h-4 rounded border flex items-center justify-center shrink-0 transition-all duration-150"
        classList={{
          'bg-[rgba(var(--primary-rgb),0.25)] border-[rgba(var(--primary-rgb),0.4)]': p.checked,
          'bg-transparent border-white/[0.15] hover:border-white/[0.3]': !p.checked,
        }}
      >
        <Show when={p.checked}>
          <Icon
            name="check"
            class="w-2.5 h-2.5"
            style={{ color: 'rgba(var(--primary-rgb),0.9)' }}
          />
        </Show>
      </div>
      <span>{p.label}</span>
    </label>
  );

  const FilterOptions = () => (
    <>
      <Checkbox
        checked={includeReasoning()}
        onChange={setIncludeReasoning}
        label={t('export.includeReasoning')}
      />
      <Checkbox
        checked={includeSystem()}
        onChange={setIncludeSystem}
        label={t('export.includeSystem')}
      />
      <Checkbox
        checked={includeToolCalls()}
        onChange={setIncludeToolCalls}
        label={t('export.includeToolCalls')}
      />
    </>
  );

  const tabClass = (t: Tab) =>
    `px-4 py-2.5 bg-transparent border-0 border-b-2 cursor-pointer text-sm transition-all duration-200 ${
      tab() === t
        ? 'border-[rgba(var(--primary-rgb),0.8)] text-[rgba(var(--primary-rgb),0.95)]'
        : 'border-transparent text-white/40 hover:text-white/70'
    }`;

  return (
    <Show when={props.open && topic()}>
      <Portal>
        <div
          classList={{
            'opacity-0 pointer-events-none': isExiting() || isEntering(),
            'opacity-100': !isExiting() && !isEntering(),
          }}
          class="fixed inset-0 z-[1100] flex items-center justify-center bg-black/60 backdrop-blur-[12px] transition-all duration-200 ease-out"
          onClick={(e) => e.target === e.currentTarget && handleClose()}
        >
          <div
            classList={{
              'scale-95 opacity-0': isExiting() || isEntering(),
              'scale-100 opacity-100': !isExiting() && !isEntering(),
            }}
            class="flex flex-col w-[92%] max-w-[760px] h-[88vh] max-h-[94vh] rounded-xl overflow-hidden transition-all duration-500 ease-out transform"
            style={{
              background: 'rgba(34, 38, 54, 0.82)',
              'backdrop-filter': 'blur(var(--acrylic-blur))',
              border: '1px solid var(--acrylic-border)',
            }}
          >
            {/* 标题行 */}
            <div class="flex items-center justify-between px-6 py-4 border-b border-white/[0.08] shrink-0">
              <h2 class="m-0 text-lg font-semibold text-white/90">{t('export.shareTitle')}</h2>
              <button
                onClick={handleClose}
                class="w-8 h-8 rounded-lg bg-transparent border-none text-xl cursor-pointer leading-none p-0 transition-all duration-200 text-white/40 hover:text-white hover:bg-danger/80"
              >
                &times;
              </button>
            </div>

            {/* Tab 栏 */}
            <div class="flex gap-0 px-6 pt-4 border-b border-white/[0.06] shrink-0">
              <button class={tabClass('screenshot')} onClick={() => setActiveTab('screenshot')}>
                {t('export.screenshot')}
              </button>
              <button class={tabClass('markdown')} onClick={() => setActiveTab('markdown')}>
                Markdown
              </button>
              <button class={tabClass('json')} onClick={() => setActiveTab('json')}>
                JSON
              </button>
              <button class={tabClass('pdf')} onClick={() => setActiveTab('pdf')}>
                PDF
              </button>
            </div>

            {/* 内容区 */}
            <div class="flex flex-col flex-1 min-h-0 overflow-hidden">
              <Switch>
                {/* ======== Screenshot Tab ======== */}
                <Match when={tab() === 'screenshot'}>
                  <div class="flex flex-col flex-1 min-h-0">
                    {/* options */}
                    <div class="flex items-center gap-4 px-6 py-3 shrink-0 text-xs text-white/50 flex-wrap">
                      <FilterOptions />
                      <span class="mx-1 text-white/20">|</span>
                      <span>{t('export.format')}</span>
                      <button
                        class={`px-3 py-1 rounded text-xs transition-all ${shotFormat() === 'png' ? 'bg-[rgba(var(--primary-rgb),0.15)] text-[rgba(var(--primary-rgb),0.9)] border border-[rgba(var(--primary-rgb),0.25)]' : 'bg-white/[0.04] text-white/50 border border-white/[0.06] hover:text-white/70'}`}
                        onClick={() => setShotFormat('png')}
                      >
                        PNG
                      </button>
                      <button
                        class={`px-3 py-1 rounded text-xs transition-all ${shotFormat() === 'jpeg' ? 'bg-[rgba(var(--primary-rgb),0.15)] text-[rgba(var(--primary-rgb),0.9)] border border-[rgba(var(--primary-rgb),0.25)]' : 'bg-white/[0.04] text-white/50 border border-white/[0.06] hover:text-white/70'}`}
                        onClick={() => setShotFormat('jpeg')}
                      >
                        JPEG
                      </button>
                      <span>{t('export.width')}</span>
                      <button
                        class={`px-3 py-1 rounded text-xs transition-all ${shotWidth() === 'narrow' ? 'bg-[rgba(var(--primary-rgb),0.15)] text-[rgba(var(--primary-rgb),0.9)] border border-[rgba(var(--primary-rgb),0.25)]' : 'bg-white/[0.04] text-white/50 border border-white/[0.06] hover:text-white/70'}`}
                        onClick={() => setShotWidth('narrow')}
                      >
                        {t('export.narrow')}
                      </button>
                      <button
                        class={`px-3 py-1 rounded text-xs transition-all ${shotWidth() === 'wide' ? 'bg-[rgba(var(--primary-rgb),0.15)] text-[rgba(var(--primary-rgb),0.9)] border border-[rgba(var(--primary-rgb),0.25)]' : 'bg-white/[0.04] text-white/50 border border-white/[0.06] hover:text-white/70'}`}
                        onClick={() => setShotWidth('wide')}
                      >
                        {t('export.wide')}
                      </button>
                    </div>

                    {/* 截图预览区 */}
                    <div class="flex-1 min-h-0 px-6 pb-2 overflow-auto">
                      <div
                        ref={captureRef}
                        style={{
                          padding: '24px 20px',
                          background: '#111827',
                          'min-width': shotWidth() === 'wide' ? '720px' : '480px',
                          display: 'flex',
                          'flex-direction': 'column',
                          'border-radius': '8px',
                        }}
                      >
                        <style>{`
                          .shot-bubble-content p { margin: 0 0 6px; }
                          .shot-bubble-content p:last-child { margin-bottom: 0; }
                          .shot-bubble-content ul, .shot-bubble-content ol { padding-left: 20px; margin: 4px 0; }
                          .shot-bubble-content li { margin-bottom: 2px; }
                          .shot-bubble-content code {
                            background: rgba(var(--text-base-rgb),0.06); padding: 1px 5px; border-radius: 3px;
                            font-family: "SF Mono", "Fira Code", monospace; font-size: 0.88em;
                          }
                          .shot-bubble-content pre {
                            background: rgba(0,0,0,0.35); padding: 12px 14px; border-radius: 8px;
                            overflow-x: auto; margin: 8px 0; font-size: 0.85em; line-height: 1.5;
                          }
                          .shot-bubble-content pre code { background: none; padding: 0; font-size: inherit; }
                          .shot-bubble-content blockquote {
                            border-left: 3px solid rgba(var(--primary-rgb),0.5); padding: 2px 10px; margin: 6px 0;
                            color: rgba(var(--text-base-rgb),0.6); font-style: italic;
                          }
                          .shot-bubble-content table { border-collapse: collapse; margin: 8px 0; width: 100%; }
                          .shot-bubble-content th, .shot-bubble-content td {
                            border: 1px solid var(--border-dim); padding: 6px 10px; text-align: left;
                          }
                          .shot-bubble-content th { background: rgba(var(--text-base-rgb),0.06); }
                          .shot-bubble-content h1, .shot-bubble-content h2, .shot-bubble-content h3,
                          .shot-bubble-content h4, .shot-bubble-content h5, .shot-bubble-content h6 {
                            margin: 10px 0 4px; font-weight: 600; line-height: 1.3;
                          }
                          .shot-bubble-content h1 { font-size: 1.3em; }
                          .shot-bubble-content h2 { font-size: 1.15em; }
                          .shot-bubble-content h3 { font-size: 1.05em; }
                          .shot-bubble-content hr { border: none; border-top: 1px solid var(--border-dim); margin: 10px 0; }
                          .shot-bubble-content a { color: rgba(var(--primary-rgb),0.9); }
                          .shot-bubble-content strong { font-weight: 600; color: rgba(var(--text-base-rgb),0.95); }
                          .shot-bubble-content em { font-style: italic; }
                        `}</style>
                        <div
                          style={{
                            'font-size': '18px',
                            'font-weight': '700',
                            color: '#fff',
                            'margin-bottom': '4px',
                          }}
                        >
                          {topic()?.name ?? ''}
                        </div>
                        <div
                          style={{ 'font-size': '12px', color: '#888', 'margin-bottom': '16px' }}
                        >
                          {new Date().toLocaleString(locale())}
                        </div>
                        <For each={visibleMessages()}>
                          {(msg) => (
                            <ScreenshotBubble
                              msg={msg}
                              wide={shotWidth() === 'wide'}
                              includeReasoning={includeReasoning()}
                            />
                          )}
                        </For>
                      </div>
                    </div>

                    {/* buttons */}
                    <div class="flex items-center justify-end gap-3 px-6 py-4 border-t border-white/[0.06] shrink-0 mt-auto">
                      <button
                        class="inline-flex items-center gap-1.5 px-4 py-2 rounded-lg text-xs font-medium transition-all duration-200"
                        classList={{
                          'bg-[rgba(74,249,8,0.12)] border border-[#4af908] text-[#4af908]':
                            copied() === 'screenshot',
                          'bg-white/[0.04] border border-white/[0.08] text-white/60 hover:bg-white/[0.08] hover:text-white/80':
                            copied() !== 'screenshot',
                        }}
                        disabled={shotCapturing()}
                        onClick={() => handleScreenshotCapture('copy')}
                      >
                        <Icon name="copy" class="w-3.5 h-3.5" />
                        {copied() === 'screenshot'
                          ? t('common.copied')
                          : shotCapturing()
                            ? t('export.capturing')
                            : t('export.copyScreenshot')}
                      </button>
                      <button
                        class="inline-flex items-center gap-1.5 px-4 py-2 rounded-lg bg-[rgba(var(--primary-rgb),0.12)] border border-[rgba(var(--primary-rgb),0.3)] text-[rgba(var(--primary-rgb),0.9)] text-xs font-medium transition-all duration-200 hover:bg-[rgba(var(--primary-rgb),0.2)]"
                        disabled={shotCapturing()}
                        onClick={() => handleScreenshotCapture('download')}
                      >
                        <Icon name="download" class="w-3.5 h-3.5" />
                        {shotCapturing()
                          ? t('export.capturing')
                          : `${t('common.download')} ${shotFormat().toUpperCase()}`}
                      </button>
                    </div>
                  </div>
                </Match>

                {/* ======== Markdown Tab ======== */}
                <Match when={tab() === 'markdown'}>
                  <div class="flex flex-col flex-1 min-h-0">
                    <div class="flex items-center gap-4 px-6 py-3 shrink-0 text-xs text-white/50">
                      <FilterOptions />
                    </div>
                    <div class="flex-1 min-h-0 px-6 pb-2">
                      <textarea
                        readonly
                        value={mdContent()}
                        class="w-full h-full resize-none rounded-lg p-4 text-[13px] leading-relaxed font-mono outline-none"
                        style={{
                          background: 'rgba(0,0,0,0.25)',
                          border: '1px solid var(--border-dim)',
                          color: 'rgba(var(--text-base-rgb),0.80)',
                        }}
                      />
                    </div>
                    <div class="flex items-center justify-end gap-3 px-6 py-4 border-t border-white/[0.06] shrink-0">
                      <button
                        class={`inline-flex items-center gap-1.5 px-4 py-2 rounded-lg text-xs font-medium transition-all duration-200 ${copied() === 'markdown' ? 'bg-[rgba(74,249,8,0.12)] border border-[#4af908] text-[#4af908]' : 'bg-white/[0.04] border border-white/[0.08] text-white/60 hover:bg-white/[0.08] hover:text-white/80'}`}
                        onClick={() => handleCopy('markdown')}
                      >
                        <Icon name="copy" class="w-3.5 h-3.5" />
                        {copied() === 'markdown' ? t('common.copied') : t('export.copyClipboard')}
                      </button>
                      <button
                        class="inline-flex items-center gap-1.5 px-4 py-2 rounded-lg bg-[rgba(var(--primary-rgb),0.12)] border border-[rgba(var(--primary-rgb),0.3)] text-[rgba(var(--primary-rgb),0.9)] text-xs font-medium transition-all duration-200 hover:bg-[rgba(var(--primary-rgb),0.2)]"
                        onClick={() => handleDownload('markdown')}
                      >
                        <Icon name="download" class="w-3.5 h-3.5" />
                        {t('export.downloadMd')}
                      </button>
                    </div>
                  </div>
                </Match>

                {/* ======== JSON Tab ======== */}
                <Match when={tab() === 'json'}>
                  <div class="flex flex-col flex-1 min-h-0">
                    <div class="flex items-center gap-4 px-6 py-3 shrink-0 text-xs text-white/50">
                      <span>{t('export.exportMode')}</span>
                      <button
                        class={`px-3 py-1 rounded text-xs transition-all ${jsonMode() === 'full' ? 'bg-[rgba(var(--primary-rgb),0.15)] text-[rgba(var(--primary-rgb),0.9)] border border-[rgba(var(--primary-rgb),0.25)]' : 'bg-white/[0.04] text-white/50 border border-white/[0.06] hover:text-white/70'}`}
                        onClick={() => setJsonMode('full')}
                      >
                        {t('export.full')}
                      </button>
                      <button
                        class={`px-3 py-1 rounded text-xs transition-all ${jsonMode() === 'simple' ? 'bg-[rgba(var(--primary-rgb),0.15)] text-[rgba(var(--primary-rgb),0.9)] border border-[rgba(var(--primary-rgb),0.25)]' : 'bg-white/[0.04] text-white/50 border border-white/[0.06] hover:text-white/70'}`}
                        onClick={() => setJsonMode('simple')}
                      >
                        {t('export.simple')}
                      </button>
                    </div>
                    <div class="flex-1 min-h-0 px-6 pb-2">
                      <textarea
                        readonly
                        value={jsonContent()}
                        class="w-full h-full resize-none rounded-lg p-4 text-[13px] leading-relaxed font-mono outline-none"
                        style={{
                          background: 'rgba(0,0,0,0.25)',
                          border: '1px solid var(--border-dim)',
                          color: 'rgba(var(--text-base-rgb),0.80)',
                        }}
                      />
                    </div>
                    <div class="flex items-center justify-end gap-3 px-6 py-4 border-t border-white/[0.06] shrink-0">
                      <button
                        class={`inline-flex items-center gap-1.5 px-4 py-2 rounded-lg text-xs font-medium transition-all duration-200 ${copied() === 'json' ? 'bg-[rgba(74,249,8,0.12)] border border-[#4af908] text-[#4af908]' : 'bg-white/[0.04] border border-white/[0.08] text-white/60 hover:bg-white/[0.08] hover:text-white/80'}`}
                        onClick={() => handleCopy('json')}
                      >
                        <Icon name="copy" class="w-3.5 h-3.5" />
                        {copied() === 'json' ? t('common.copied') : t('export.copyClipboard')}
                      </button>
                      <button
                        class="inline-flex items-center gap-1.5 px-4 py-2 rounded-lg bg-[rgba(var(--primary-rgb),0.12)] border border-[rgba(var(--primary-rgb),0.3)] text-[rgba(var(--primary-rgb),0.9)] text-xs font-medium transition-all duration-200 hover:bg-[rgba(var(--primary-rgb),0.2)]"
                        onClick={() => handleDownload('json')}
                      >
                        <Icon name="download" class="w-3.5 h-3.5" />
                        {t('export.downloadJson')}
                      </button>
                    </div>
                  </div>
                </Match>

                {/* ======== PDF Tab ======== */}
                <Match when={tab() === 'pdf'}>
                  <div class="flex flex-col flex-1 min-h-0">
                    <div class="flex items-center gap-4 px-6 py-3 shrink-0 text-xs text-white/50">
                      <FilterOptions />
                    </div>
                    {/* HTML preview */}
                    <div class="flex-1 min-h-0 px-6 pb-2">
                      <iframe
                        ref={pdfIframeRef}
                        srcdoc={htmlContent()}
                        class="w-full h-full rounded-lg border-0"
                        style={{ background: '#fff' }}
                        title="PDF preview"
                      />
                    </div>
                    <div class="flex items-center justify-end gap-3 px-6 py-4 border-t border-white/[0.06] shrink-0">
                      <button
                        class="inline-flex items-center gap-1.5 px-4 py-2 rounded-lg bg-[rgba(var(--primary-rgb),0.12)] border border-[rgba(var(--primary-rgb),0.3)] text-[rgba(var(--primary-rgb),0.9)] text-xs font-medium transition-all duration-200 hover:bg-[rgba(var(--primary-rgb),0.2)]"
                        disabled={pdfCapturing()}
                        onClick={handlePdfDownload}
                      >
                        <Icon name="download" class="w-3.5 h-3.5" />
                        {pdfCapturing() ? t('export.preparing') : t('export.downloadPdf')}
                      </button>
                    </div>
                  </div>
                </Match>
              </Switch>
            </div>
          </div>
        </div>
      </Portal>
    </Show>
  );
};

export default ShareModal;
