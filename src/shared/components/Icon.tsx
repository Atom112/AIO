/**
 * 统一 SVG 图标库 (亚克力主题)
 *
 * 两种用法:
 * 1. <Icon name="search" class="w-4 h-4" />            命名图标 (内联 SVG, 跟随 currentColor)
 * 2. <Icon src="/icons/app-logo/chat.svg" />            文件图标 (CSS mask-image, 历史用法)
 *
 * 风格: 1.5px 描边, 圆头线帽, currentColor 颜色, 24x24 viewBox
 */
import { Component, JSX, splitProps, Show } from 'solid-js';

export type IconName =
    | 'folder' | 'plus' | 'cpu' | 'refresh' | 'chart-bar' | 'search'
    | 'check' | 'alert-triangle' | 'beaker' | 'download' | 'trash'
    | 'book' | 'arrow-left' | 'clock' | 'stop' | 'play' | 'bolt'
    | 'eye' | 'wrench' | 'brain' | 'x' | 'code' | 'lightbulb'
    | 'document' | 'check-circle' | 'image' | 'globe' | 'logo' | 'sparkles'
    | 'gear' | 'chat' | 'send' | 'clip' | 'copy' | 'model' | 'user' | 'spinner' | 'file' | 'menu';

/**
 * 图标路径工厂表。
 * 重要: 必须用工厂函数 (() => JSX.Element) 而非静态 JSX.Element。
 * 若用静态元素 (模块作用域求值, 只创建一次), 同一图标在多处同时渲染时
 * 会共享同一个 DOM 节点, SolidJS 会把节点「移动」到后渲染的位置, 导致
 * 先渲染处图标消失 (例如点开推理按钮弹窗后触发按钮图标消失)。
 * 工厂函数让 Solid 每次调用都模板克隆出独立节点, 杜绝共享。
 */
const PATHS: Record<IconName, () => JSX.Element> = {
    folder: () => <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7Z" />,
    plus: () => <><path d="M12 5v14M5 12h14" /></>,
    cpu: () => <><rect x="5" y="5" width="14" height="14" rx="2" /><rect x="9" y="9" width="6" height="6" /><path d="M9 2v3M15 2v3M9 19v3M15 19v3M2 9h3M2 15h3M19 9h3M19 15h3" /></>,
    refresh: () => <path d="M21 12a9 9 0 0 0-15-6.7L3 8M3 3v5h5M3 12a9 9 0 0 0 15 6.7L21 16M21 21v-5h-5" />,
    'chart-bar': () => <path d="M4 20h16M6 16v-4M11 16V8M16 16v-6M21 16V4" />,
    search: () => <><circle cx="11" cy="11" r="7" /><path d="m20 20-3.5-3.5" /></>,
    check: () => <path d="m5 12 5 5L20 7" />,
    'check-circle': () => <><circle cx="12" cy="12" r="9" /><path d="m8 12 3 3 5-6" /></>,
    'alert-triangle': () => <><path d="M12 3 2 21h20L12 3Z" /><path d="M12 10v5M12 18v.5" /></>,
    beaker: () => <><path d="M9 3h6M10 3v6L4.5 18a2 2 0 0 0 1.7 3h11.6a2 2 0 0 0 1.7-3L14 9V3" /><path d="M7 14h10" /></>,
    download: () => <><path d="M12 4v12M7 11l5 5 5-5" /><path d="M4 20h16" /></>,
    trash: () => <><path d="M4 7h16M9 7V4h6v3M6 7v13a2 2 0 0 0 2 2h8a2 2 0 0 0 2-2V7" /><path d="M10 11v6M14 11v6" /></>,
    book: () => <><path d="M4 4a2 2 0 0 1 2-2h13a2 2 0 0 1 2 2v18l-7-3-7 3V4Z" /><path d="M4 4v15" /></>,
    'arrow-left': () => <path d="M19 12H5M12 19l-7-7 7-7" />,
    clock: () => <><circle cx="12" cy="12" r="9" /><path d="M12 7v5l3 2" /></>,
    stop: () => <rect x="6" y="6" width="12" height="12" rx="1" />,
    play: () => <path d="M7 5v14l12-7-12-7Z" />,
    bolt: () => <path d="M13 2 4 14h7l-1 8 9-12h-7l1-8Z" />,
    eye: () => <><path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7-10-7-10-7Z" /><circle cx="12" cy="12" r="3" /></>,
    wrench: () => <path d="M14.7 6.3a4 4 0 0 0-5.4 5.4L3 18l3 3 6.3-6.3a4 4 0 0 0 5.4-5.4l-2.5 2.5-2.5-2.5 2.5-2.5Z" />,
    brain: () => <><path d="M9 4a3 3 0 0 0-3 3v0a3 3 0 0 0-2 5 3 3 0 0 0 1 4 3 3 0 0 0 3 3 3 3 0 0 0 3-1l1-1V7l-1-1a3 3 0 0 0-3-2Z" /><path d="M15 4a3 3 0 0 1 3 3v0a3 3 0 0 1 2 5 3 3 0 0 1-1 4 3 3 0 0 1-3 3 3 3 0 0 1-3-1l-1-1V7l1-1a3 3 0 0 1 3-2Z" /></>,
    x: () => <path d="M6 6l12 12M18 6 6 18" />,
    code: () => <path d="m9 8-4 4 4 4M15 8l4 4-4 4" />,
    lightbulb: () => <><path d="M9 18h6M10 22h4" /><path d="M12 2a7 7 0 0 0-4 12.7c.7.7 1 1.6 1 2.5V18h6v-.8c0-.9.3-1.8 1-2.5A7 7 0 0 0 12 2Z" /></>,
    document: () => <><path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8l-5-5Z" /><path d="M14 3v5h5M9 13h6M9 17h6" /></>,
    image: () => <><rect x="3" y="3" width="18" height="18" rx="2" /><circle cx="9" cy="9" r="2" /><path d="m21 15-5-5L5 21" /></>,
    globe: () => <><circle cx="12" cy="12" r="9" /><path d="M3 12h18M12 3a14 14 0 0 1 0 18M12 3a14 14 0 0 0 0 18" /></>,
    logo: () => <><path d="M12 2 4 7v10l8 5 8-5V7l-8-5Z" /><path d="M12 22V12M4 7l8 5 8-5" /></>,
    sparkles: () => <path d="M12 3v3M12 18v3M3 12h3M18 12h3M5.6 5.6l2.1 2.1M16.3 16.3l2.1 2.1M5.6 18.4l2.1-2.1M16.3 7.7l2.1-2.1" />,
    gear: () => <><circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 0 1-4 0v-.1a1.7 1.7 0 0 0-1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 0 1 0-4h.1a1.7 1.7 0 0 0 1.5-1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3 1.7 1.7 0 0 0 1-1.5V3a2 2 0 0 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8 1.7 1.7 0 0 0 1.5 1H21a2 2 0 0 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1Z" /></>,
    chat: () => <path d="M21 12c0 4.4-4 8-9 8a9.9 9.9 0 0 1-4-.8L3 21l1.8-4A8 8 0 0 1 3 12c0-4.4 4-8 9-8s9 3.6 9 8Z" />,
    send: () => <path d="m22 2-7 20-4-9-9-4 20-7Z" />,
    clip: () => <path d="M21 11.5v5a4 4 0 0 1-4 4H7a4 4 0 0 1-4-4V6a4 4 0 0 1 4-4h5M16 4h2a2 2 0 0 1 2 2v2M12 11v6M9 14h6" />,
    copy: () => <><rect x="9" y="9" width="11" height="11" rx="2" /><path d="M5 15V5a2 2 0 0 1 2-2h10" /></>,
    model: () => <><circle cx="12" cy="12" r="3" /><path d="M12 3a9 9 0 0 0-9 9h0M12 21a9 9 0 0 0 9-9h0M3 12h18" /><path d="M3.5 7.5h17M3.5 16.5h17" /></>,
    user: () => <><circle cx="12" cy="7" r="4" /><path d="M4 21a8 8 0 0 1 16 0" /></>,
    spinner: () => <path d="M12 3a9 9 0 1 0 9 9" />,
    file: () => <><path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8l-5-5Z" /><path d="M14 3v5h5" /></>,
    menu: () => <><circle cx="5" cy="6" r="1" /><circle cx="12" cy="6" r="1" /><circle cx="19" cy="6" r="1" /><circle cx="5" cy="12" r="1" /><circle cx="12" cy="12" r="1" /><circle cx="19" cy="12" r="1" /><circle cx="5" cy="18" r="1" /><circle cx="12" cy="18" r="1" /><circle cx="19" cy="18" r="1" /></>,
};

export interface IconProps extends Omit<JSX.SvgSVGAttributes<SVGSVGElement>, 'children'> {
    /** 命名图标 (推荐) */
    name?: IconName;
    /** 文件路径图标 (历史用法, 走 CSS mask-image) */
    src?: string;
    /** 像素尺寸 (宽高), 默认 16 */
    size?: number;
    class?: string;
}

/**
 * 通用 SVG 图标组件
 * - 1.5px 描边, currentColor 自动跟随父元素 color
 * - 统一 24x24 viewBox
 */
export const Icon: Component<IconProps> = (props) => {
    const [local, rest] = splitProps(props, ['name', 'src', 'size', 'class']);
    const size = () => local.size ?? 16;
    return (
        <Show
            when={local.name}
            fallback={
                <span
                    class={`block bg-current ${local.class ?? ''}`}
                    style={{
                        '-webkit-mask-image': `url(${local.src})`,
                        'mask-image': `url(${local.src})`,
                        '-webkit-mask-size': 'contain',
                        'mask-size': 'contain',
                        '-webkit-mask-repeat': 'no-repeat',
                        'mask-repeat': 'no-repeat',
                        '-webkit-mask-position': 'center',
                        'mask-position': 'center',
                    }}
                    aria-hidden="true"
                />
            }
        >
            <svg
                xmlns="http://www.w3.org/2000/svg"
                viewBox="0 0 24 24"
                width={size()}
                height={size()}
                fill="none"
                stroke="currentColor"
                stroke-width="1.5"
                stroke-linecap="round"
                stroke-linejoin="round"
                class={local.class}
                aria-hidden="true"
                {...rest}
            >
                {PATHS[local.name!]!()}
            </svg>
        </Show>
    );
};

export default Icon;
