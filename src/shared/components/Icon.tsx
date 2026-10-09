/**
 * Shared SVG icon library (acrylic theme)
 *
 * Two usage modes:
 * 1. <Icon name="search" class="w-4 h-4" />            named icon (inline SVG, follows currentColor)
 * 2. <Icon src="/icons/app-logo/chat.svg" />            file icon (CSS mask-image, legacy usage)
 *
 */
import { Component, JSX, splitProps, Show } from 'solid-js';

export type IconName =
  | 'folder'
  | 'plus'
  | 'cpu'
  | 'refresh'
  | 'chart-bar'
  | 'search'
  | 'check'
  | 'x-circle'
  | 'alert-triangle'
  | 'beaker'
  | 'download'
  | 'trash'
  | 'book'
  | 'arrow-left'
  | 'pause'
  | 'clock'
  | 'stop'
  | 'play'
  | 'bolt'
  | 'eye'
  | 'wrench'
  | 'brain'
  | 'x'
  | 'code'
  | 'lightbulb'
  | 'document'
  | 'check-circle'
  | 'image'
  | 'globe'
  | 'logo'
  | 'sparkles'
  | 'gear'
  | 'chat'
  | 'send'
  | 'clip'
  | 'copy'
  | 'model'
  | 'user'
  | 'spinner'
  | 'file'
  | 'menu'
  | 'layers'
  | 'info'
  | 'clipboard'
  | 'trending-up'
  | 'zap'
  | 'git-branch'
  | 'chevron-left'
  | 'chevron-right'
  | 'chevron-up'
  | 'chevron-down';

/**
 * Icon path factory table.
 * Important: must use factory functions (() => JSX.Element), not static JSX.Element.
 * A static element is evaluated once at module scope; the same icon rendered in
 * multiple places would share one DOM node, and SolidJS "moves" that node to the
 * latest render position, making the icon vanish where it rendered first (e.g. the
 * trigger button icon disappears after opening the reasoning popover).
 * Factories make Solid clone an independent node per call, preventing sharing.
 */
const PATHS: Record<IconName, () => JSX.Element> = {
  folder: () => (
    <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7Z" />
  ),
  plus: () => (
    <>
      <path d="M12 5v14M5 12h14" />
    </>
  ),
  cpu: () => (
    <>
      <rect x="5" y="5" width="14" height="14" rx="2" />
      <rect x="9" y="9" width="6" height="6" />
      <path d="M9 2v3M15 2v3M9 19v3M15 19v3M2 9h3M2 15h3M19 9h3M19 15h3" />
    </>
  ),
  refresh: () => (
    <path d="M21 12a9 9 0 0 0-15-6.7L3 8M3 3v5h5M3 12a9 9 0 0 0 15 6.7L21 16M21 21v-5h-5" />
  ),
  'chart-bar': () => <path d="M4 20h16M6 16v-4M11 16V8M16 16v-6M21 16V4" />,
  search: () => (
    <>
      <circle cx="11" cy="11" r="7" />
      <path d="m20 20-3.5-3.5" />
    </>
  ),
  check: () => <path d="m5 12 5 5L20 7" />,
  'x-circle': () => (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="m8 8 8 8M16 8l-8 8" />
    </>
  ),
  'check-circle': () => (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="m8 12 3 3 5-6" />
    </>
  ),
  'alert-triangle': () => (
    <>
      <path d="M12 3 2 21h20L12 3Z" />
      <path d="M12 10v5M12 18v.5" />
    </>
  ),
  beaker: () => (
    <>
      <path d="M9 3h6M10 3v6L4.5 18a2 2 0 0 0 1.7 3h11.6a2 2 0 0 0 1.7-3L14 9V3" />
      <path d="M7 14h10" />
    </>
  ),
  download: () => (
    <>
      <path d="M12 4v12M7 11l5 5 5-5" />
      <path d="M4 20h16" />
    </>
  ),
  trash: () => (
    <>
      <path d="M4 7h16M9 7V4h6v3M6 7v13a2 2 0 0 0 2 2h8a2 2 0 0 0 2-2V7" />
      <path d="M10 11v6M14 11v6" />
    </>
  ),
  book: () => (
    <>
      <path d="M4 4a2 2 0 0 1 2-2h13a2 2 0 0 1 2 2v18l-7-3-7 3V4Z" />
      <path d="M4 4v15" />
    </>
  ),
  'arrow-left': () => <path d="M19 12H5M12 19l-7-7 7-7" />,
  pause: () => <path d="M9 5v14M15 5v14" />,
  clock: () => (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M12 7v5l3 2" />
    </>
  ),
  stop: () => <rect x="6" y="6" width="12" height="12" rx="1" />,
  play: () => <path d="M7 5v14l12-7-12-7Z" />,
  bolt: () => <path d="M13 2 4 14h7l-1 8 9-12h-7l1-8Z" />,
  eye: () => (
    <>
      <path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7-10-7-10-7Z" />
      <circle cx="12" cy="12" r="3" />
    </>
  ),
  wrench: () => (
    <path d="M14.7 6.3a4 4 0 0 0-5.4 5.4L3 18l3 3 6.3-6.3a4 4 0 0 0 5.4-5.4l-2.5 2.5-2.5-2.5 2.5-2.5Z" />
  ),
  brain: () => (
    <>
      <path d="M9 4a3 3 0 0 0-3 3v0a3 3 0 0 0-2 5 3 3 0 0 0 1 4 3 3 0 0 0 3 3 3 3 0 0 0 3-1l1-1V7l-1-1a3 3 0 0 0-3-2Z" />
      <path d="M15 4a3 3 0 0 1 3 3v0a3 3 0 0 1 2 5 3 3 0 0 1-1 4 3 3 0 0 1-3 3 3 3 0 0 1-3-1l-1-1V7l1-1a3 3 0 0 1 3-2Z" />
    </>
  ),
  x: () => <path d="M6 6l12 12M18 6 6 18" />,
  code: () => <path d="m9 8-4 4 4 4M15 8l4 4-4 4" />,
  lightbulb: () => (
    <>
      <path d="M9 18h6M10 22h4" />
      <path d="M12 2a7 7 0 0 0-4 12.7c.7.7 1 1.6 1 2.5V18h6v-.8c0-.9.3-1.8 1-2.5A7 7 0 0 0 12 2Z" />
    </>
  ),
  document: () => (
    <>
      <path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8l-5-5Z" />
      <path d="M14 3v5h5M9 13h6M9 17h6" />
    </>
  ),
  image: () => (
    <>
      <rect x="3" y="3" width="18" height="18" rx="2" />
      <circle cx="9" cy="9" r="2" />
      <path d="m21 15-5-5L5 21" />
    </>
  ),
  globe: () => (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M3 12h18M12 3a14 14 0 0 1 0 18M12 3a14 14 0 0 0 0 18" />
    </>
  ),
  logo: () => (
    <>
      <path d="M12 2 4 7v10l8 5 8-5V7l-8-5Z" />
      <path d="M12 22V12M4 7l8 5 8-5" />
    </>
  ),
  sparkles: () => (
    <path d="M12 3v3M12 18v3M3 12h3M18 12h3M5.6 5.6l2.1 2.1M16.3 16.3l2.1 2.1M5.6 18.4l2.1-2.1M16.3 7.7l2.1-2.1" />
  ),
  gear: () => (
    <>
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 0 1-4 0v-.1a1.7 1.7 0 0 0-1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 0 1 0-4h.1a1.7 1.7 0 0 0 1.5-1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3 1.7 1.7 0 0 0 1-1.5V3a2 2 0 0 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8 1.7 1.7 0 0 0 1.5 1H21a2 2 0 0 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1Z" />
    </>
  ),
  chat: () => (
    <path d="M21 12c0 4.4-4 8-9 8a9.9 9.9 0 0 1-4-.8L3 21l1.8-4A8 8 0 0 1 3 12c0-4.4 4-8 9-8s9 3.6 9 8Z" />
  ),
  send: () => <path d="m22 2-7 20-4-9-9-4 20-7Z" />,
  clip: () => (
    <path d="M21 11.5v5a4 4 0 0 1-4 4H7a4 4 0 0 1-4-4V6a4 4 0 0 1 4-4h5M16 4h2a2 2 0 0 1 2 2v2M12 11v6M9 14h6" />
  ),
  copy: () => (
    <>
      <rect x="9" y="9" width="11" height="11" rx="2" />
      <path d="M5 15V5a2 2 0 0 1 2-2h10" />
    </>
  ),
  model: () => (
    <>
      <circle cx="12" cy="12" r="3" />
      <path d="M12 3a9 9 0 0 0-9 9h0M12 21a9 9 0 0 0 9-9h0M3 12h18" />
      <path d="M3.5 7.5h17M3.5 16.5h17" />
    </>
  ),
  user: () => (
    <>
      <circle cx="12" cy="7" r="4" />
      <path d="M4 21a8 8 0 0 1 16 0" />
    </>
  ),
  spinner: () => <path d="M12 3a9 9 0 1 0 9 9" />,
  file: () => (
    <>
      <path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8l-5-5Z" />
      <path d="M14 3v5h5" />
    </>
  ),
  menu: () => (
    <>
      <circle cx="5" cy="6" r="1" />
      <circle cx="12" cy="6" r="1" />
      <circle cx="19" cy="6" r="1" />
      <circle cx="5" cy="12" r="1" />
      <circle cx="12" cy="12" r="1" />
      <circle cx="19" cy="12" r="1" />
      <circle cx="5" cy="18" r="1" />
      <circle cx="12" cy="18" r="1" />
      <circle cx="19" cy="18" r="1" />
    </>
  ),
  layers: () => (
    <>
      <path d="m3 12 9-7 9 7-9 7-9-7Z" />
      <path d="m3 16 9 5 9-5" />
      <path d="m3 8 9 5 9-5" />
    </>
  ),
  info: () => (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M11.25 11.25l.041-.02a.75.75 0 011.063.852l-.708 2.836a.75.75 0 001.063.853l.041-.021M12 8.25h.008v.008H12V8.25Z" />
    </>
  ),
  clipboard: () => (
    <>
      <path d="M8 5H6a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2v-1M8 5a2 2 0 0 0 2 2h2a2 2 0 0 0 2-2M8 5a2 2 0 0 1 2-2h2a2 2 0 0 1 2 2m0 0h2a2 2 0 0 1 2 2v3m2 4H10m0 0l3-3m-3 3l3 3" />
    </>
  ),
  'trending-up': () => (
    <>
      <path d="M22 7 14.5 14.5 11.5 11.5 2 20" />
      <path d="M17 7h5M22 7V2" />
    </>
  ),
  zap: () => <path d="M13 2 3 14h8l-2 8 11-14h-7l2-8Z" />,
  'git-branch': () => (
    <>
      <path d="M6 3v12M18 9a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM6 21a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM18 9v9a3 3 0 0 1-3 3h-3" />
    </>
  ),
  'chevron-left': () => <path d="M15 18l-6-6 6-6" />,
  'chevron-right': () => <path d="M9 18l6-6-6-6" />,
  'chevron-up': () => <path d="m6 14 6-6 6 6" />,
  'chevron-down': () => <path d="m6 10 6 6 6-6" />,
};

export interface IconProps extends Omit<JSX.SvgSVGAttributes<SVGSVGElement>, 'children'> {
  /** Named icon (recommended) */
  name?: IconName;
  /** File-path icon (legacy usage, via CSS mask-image) */
  src?: string;
  /** Pixel size (width and height), default 16 */
  size?: number;
  class?: string;
}

/**
 * Generic SVG icon component
 * - 1.5px stroke, currentColor follows the parent element color
 * - Uniform 24x24 viewBox
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
