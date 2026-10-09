import { Component, createSignal, Show, For } from 'solid-js';
import type { ToolCall, ToolResultContent } from '../../../core/types/mcp';
import Icon from '../../../shared/components/Icon';
import { t } from '../../../core/i18n';

interface Props {
  toolCall: ToolCall;
  /** Current state: 'calling' = tool call in flight; 'success' = result returned; 'error' = failed */
  state: 'calling' | 'success' | 'error';
  /** Tool execution result (when success) */
  result?: ToolResultContent[];
  /** Error message (when error) */
  error?: string;
  /** Tool name (for display) */
  serverName?: string;
}

const ToolCallBubble: Component<Props> = (props) => {
  const [expanded, setExpanded] = createSignal(false);
  const tc = () => props.toolCall;
  const argsObj = () => {
    try {
      return JSON.parse(tc().function.arguments || '{}');
    } catch {
      return tc().function.arguments;
    }
  };

  return (
    <div
      class="mb-1.5 rounded-md overflow-hidden text-xs animate-expand-width"
      style={{ border: '1px solid var(--border-dim)' }}
    >
      {/* More compact header: no base background, interactive only on hover */}
      <div
        class="flex items-center gap-1.5 px-2 py-1 cursor-pointer select-none rounded-md transition-colors duration-150"
        style={{ background: 'rgba(var(--text-base-rgb),0.03)' }}
        classList={{ 'rounded-b-none': expanded() }}
        onClick={() => setExpanded(!expanded())}
      >
        <span
          class="flex items-center justify-center w-4 h-4 rounded-full flex-shrink-0"
          style={
            props.state === 'calling'
              ? 'background: rgba(224,192,96,0.15);'
              : props.state === 'error'
                ? 'background: rgba(255,107,107,0.15);'
                : 'background: rgba(124,217,160,0.15);'
          }
        >
          <Show
            when={props.state === 'calling'}
            fallback={
              <Show
                when={props.state === 'error'}
                fallback={
                  <span style={{ color: 'var(--color-success)', 'font-size': '10px' }}>✓</span>
                }
              >
                <Icon
                  src="/icons/app-logo/close-x.svg"
                  class="w-2.5 h-2.5"
                  style={{ color: 'var(--color-danger)' }}
                />
              </Show>
            }
          >
            <Icon
              src="/icons/app-logo/loading.svg"
              class="w-2.5 h-2.5 animate-spin"
              style={{ color: 'var(--color-warn)' }}
            />
          </Show>
        </span>
        <span
          class="font-medium truncate min-w-0"
          style={{ color: 'rgba(var(--text-base-rgb),0.65)', 'font-size': '11px' }}
        >
          {tc().function.name}
        </span>
        <Show when={props.serverName}>
          <span
            class="text-[10px] px-1 py-[1px] rounded flex-shrink-0"
            style={{
              background: 'rgba(var(--primary-rgb),0.1)',
              color: 'rgba(var(--primary-rgb),0.7)',
            }}
          >
            {props.serverName}
          </span>
        </Show>
        <span
          class="ml-auto flex-shrink-0"
          style={{
            color: 'rgba(var(--text-base-rgb),0.25)',
            'font-size': '9px',
            'letter-spacing': '0.5px',
          }}
        >
          {props.state === 'calling' ? '…' : props.state === 'error' ? '✗' : '✓'}
        </span>
      </div>

      {/* Expanded details */}
      <Show when={expanded()}>
        <div
          class="px-2 py-1.5 flex flex-col gap-1.5 text-[11px]"
          style={{ background: 'rgba(0,0,0,0.15)' }}
        >
          <Show when={tc().function.arguments && tc().function.arguments !== '{}'}>
            <div>
              <div class="mb-0.5" style={{ color: 'rgba(var(--text-base-rgb),0.35)' }}>
                {t('chat.tool.params')}
              </div>
              <pre
                class="px-1.5 py-1 rounded overflow-x-auto whitespace-pre-wrap break-all"
                style={{
                  background: 'rgba(0,0,0,0.2)',
                  color: 'rgba(var(--text-base-rgb),0.55)',
                  'font-size': '10px',
                  'line-height': '1.4',
                }}
              >
                {JSON.stringify(argsObj(), null, 2)}
              </pre>
            </div>
          </Show>
          <Show when={props.state === 'success' && props.result && props.result.length > 0}>
            <div>
              <div class="mb-0.5" style={{ color: 'rgba(var(--text-base-rgb),0.35)' }}>
                {t('chat.tool.result')}
              </div>
              <div class="flex flex-col gap-0.5">
                <For each={props.result}>
                  {(c) => (
                    <Show when={c.type === 'text'}>
                      <pre
                        class="px-1.5 py-1 rounded overflow-x-auto whitespace-pre-wrap break-all"
                        style={{
                          background: 'rgba(0,0,0,0.2)',
                          color: 'rgba(var(--text-base-rgb),0.55)',
                          'font-size': '10px',
                          'line-height': '1.4',
                          'max-height': '120px',
                          'overflow-y': 'auto',
                        }}
                      >
                        {typeof (c as any).text === 'string'
                          ? (c as any).text
                          : JSON.stringify(c, null, 2)}
                      </pre>
                    </Show>
                  )}
                </For>
                <For each={props.result}>
                  {(c) => (
                    <Show when={c.type === 'image'}>
                      <span
                        style={{ color: 'rgba(var(--text-base-rgb),0.4)', 'font-size': '10px' }}
                      >
                        {t('chat.tool.image')}
                      </span>
                    </Show>
                  )}
                </For>
                <For each={props.result}>
                  {(c) => (
                    <Show when={c.type === 'resource'}>
                      <span
                        style={{ color: 'rgba(var(--text-base-rgb),0.4)', 'font-size': '10px' }}
                      >
                        {t('chat.tool.resource')}
                      </span>
                    </Show>
                  )}
                </For>
              </div>
            </div>
          </Show>
          <Show when={props.state === 'error'}>
            <div>
              <div class="mb-0.5" style={{ color: 'rgba(255,107,107,0.7)' }}>
                {t('chat.tool.error')}
              </div>
              <pre
                class="px-1.5 py-1 rounded overflow-x-auto whitespace-pre-wrap break-all"
                style={{
                  background: 'rgba(255,77,77,0.08)',
                  color: 'rgba(255,138,138,0.7)',
                  'font-size': '10px',
                }}
              >
                {props.error}
              </pre>
            </div>
          </Show>
        </div>
      </Show>
    </div>
  );
};

export default ToolCallBubble;
