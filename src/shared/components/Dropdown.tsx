/**
 * 亚克力风格自定义下拉组件
 *
 * 用法:
 *   <Dropdown
 *       value={sortKey()}
 *       onChange={setSortKey}
 *       options={[
 *           { value: 'releaseDesc', label: '发布日期 ↓' },
 *           { value: 'nameAsc', label: '名称 A-Z' },
 *       ]}
 *   />
 */
import { Component, For, Show, createSignal, onCleanup, onMount, JSX } from 'solid-js';
import Icon from './Icon';

export interface DropdownOption<V extends string = string> {
    value: V;
    label: string;
    icon?: string;
    disabled?: boolean;
}

export interface DropdownProps<V extends string = string> {
    value: V;
    onChange: (value: V) => void;
    options: DropdownOption<V>[];
    /** 触发器占位符 (无选中时显示) */
    placeholder?: string;
    /** 自定义 class 应用到触发器 */
    class?: string;
    /** 触发器对齐方式 */
    align?: 'left' | 'right';
    /** 自定义触发器渲染 (用于插入图标等) */
    trigger?: (current: DropdownOption<V> | undefined) => JSX.Element;
    /** 禁用整个下拉 */
    disabled?: boolean;
    /** 唯一 ID (用于 aria) */
    id?: string;
}

export const Dropdown = <V extends string = string>(props: DropdownProps<V>) => {
    const [open, setOpen] = createSignal(false);
    let containerRef: HTMLDivElement | undefined;
    let triggerRef: HTMLButtonElement | undefined;

    const current = () => props.options.find(o => o.value === props.value);

    /** 点击外部关闭 */
    const onDocClick = (e: MouseEvent) => {
        if (!containerRef) return;
        if (!containerRef.contains(e.target as Node)) {
            setOpen(false);
        }
    };

    /** Escape 关闭, 方向键导航 */
    const onKeyDown = (e: KeyboardEvent) => {
        if (!open()) return;
        if (e.key === 'Escape') {
            setOpen(false);
            triggerRef?.focus();
            e.preventDefault();
        } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
            e.preventDefault();
            const dir = e.key === 'ArrowDown' ? 1 : -1;
            const enabledOptions = props.options.filter(o => !o.disabled);
            if (enabledOptions.length === 0) return;
            const currentIdx = enabledOptions.findIndex(o => o.value === props.value);
            const nextIdx = currentIdx < 0
                ? 0
                : (currentIdx + dir + enabledOptions.length) % enabledOptions.length;
            props.onChange(enabledOptions[nextIdx].value);
        } else if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            setOpen(false);
        }
    };

    onMount(() => {
        document.addEventListener('click', onDocClick);
        document.addEventListener('keydown', onKeyDown);
    });
    onCleanup(() => {
        document.removeEventListener('click', onDocClick);
        document.removeEventListener('keydown', onKeyDown);
    });

    return (
        <div ref={containerRef} class={`relative inline-block ${props.class ?? ''}`}>
            <button
                ref={triggerRef}
                type="button"
                id={props.id}
                disabled={props.disabled}
                class="select-glass flex items-center gap-1.5 w-full text-left py-1 text-xs"
                style={{ 'padding-left': '10px' }}
                onClick={(e) => { e.stopPropagation(); setOpen(!open()); }}
                aria-haspopup="listbox"
                aria-expanded={open()}
            >
                <Show
                    when={current() || props.trigger}
                    fallback={<span class="text-[#666]">{props.placeholder ?? '请选择'}</span>}
                >
                    <Show
                        when={!props.trigger}
                        fallback={props.trigger!(current())}
                    >
                        <span class="truncate">{current()!.label}</span>
                    </Show>
                </Show>
            </button>

            <Show when={open()}>
                <div
                    class={`dropdown-panel ${props.align === 'right' ? 'right-0' : 'left-0'} top-full mt-1`}
                    role="listbox"
                    onClick={(e) => e.stopPropagation()}
                >
                    <For each={props.options}>
                        {(opt) => (
                            <div
                                class={`dropdown-item ${opt.value === props.value ? 'selected' : ''}`}
                                role="option"
                                aria-selected={opt.value === props.value}
                                classList={{ 'opacity-40 pointer-events-none': opt.disabled }}
                                onClick={() => {
                                    if (opt.disabled) return;
                                    props.onChange(opt.value);
                                    setOpen(false);
                                    triggerRef?.focus();
                                }}
                            >
                                <Show when={opt.icon}>
                                    <Icon name={opt.icon as any} size={13} class="opacity-70" />
                                </Show>
                                <span class="grow truncate">{opt.label}</span>
                                <Show when={opt.value === props.value}>
                                    <Icon name="check" size={12} class="text-pri" />
                                </Show>
                            </div>
                        )}
                    </For>
                </div>
            </Show>
        </div>
    );
};

export default Dropdown;
