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
                class="border border-white/[0.08] rounded-lg text-white px-[14px] cursor-pointer hover:border-white/[0.14] focus:outline-none focus:border-[rgba(var(--primary-rgb),0.55)] focus:bg-[rgba(0,0,0,0.35)] focus:shadow-[0_0_0_3px_rgba(var(--primary-rgb),0.12)] flex items-center gap-1.5 w-full text-center py-1 text-xs"
                style={{ appearance: 'none', WebkitAppearance: 'none', MozAppearance: 'none', backgroundColor: 'rgba(0, 0, 0, 0.25)', backgroundImage: 'url("data:image/svg+xml;utf8,<svg xmlns=\'http://www.w3.org/2000/svg\' width=\'12\' height=\'12\' viewBox=\'0 0 24 24\' fill=\'none\' stroke=\'rgba(255,255,255,0.5)\' stroke-width=\'1.5\' stroke-linecap=\'round\' stroke-linejoin=\'round\'><path d=\'m6 9 6 6 6-6\'/></svg>")', backgroundRepeat: 'no-repeat', backgroundPosition: 'right 8px center', backgroundSize: '12px', transition: 'border-color 0.2s ease, background-color 0.2s ease, box-shadow 0.2s ease' }}
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

            <div
                class={`absolute z-[100] min-w-full mt-1 rounded-[10px] p-1 transition-all duration-150 ease-out origin-top ${props.align === 'right' ? 'right-0' : 'left-0'} top-full`}
                style="background: rgba(18, 22, 35, 0.88); backdrop-filter: blur(40px) saturate(180%); -webkit-backdrop-filter: blur(40px) saturate(180%); border: 1px solid rgba(255, 255, 255, 0.08); box-shadow: 0 12px 40px rgba(0, 0, 0, 0.45);"
                classList={{
                    'invisible opacity-0 scale-95 translate-y-1 pointer-events-none': !open(),
                    'visible opacity-100 scale-100 translate-y-0 pointer-events-auto': open(),
                }}
                role="listbox"
                onClick={(e) => e.stopPropagation()}
            >
                <For each={props.options}>
                    {(opt) => (
                        <div
                            class={`flex items-center gap-1.5 px-2.5 py-[7px] rounded-md text-white/[0.78] cursor-pointer transition-[background,color] duration-[120ms] select-none ${opt.value === props.value ? 'selected' : ''}`}
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
        </div>
    );
};

export default Dropdown;
