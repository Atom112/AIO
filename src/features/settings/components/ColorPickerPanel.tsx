/**
 * @file ColorPickerPanel.tsx
 * @description Reusable hue-ring color picker (ring + saturation/lightness sliders +
 * preset swatches). Extracted from the theme-color card so every color setting —
 * theme color and background colors — edits through the same custom control.
 * The panel owns only the transient HSL state of its ring; the persisted color is
 * owned (and written) by the calling domain via `onChange`.
 */
import { Component, createEffect, createMemo, createSignal, For, untrack } from 'solid-js';
import { t } from '../../../core/i18n';
import { DEFAULT_THEME_COLOR } from '../../../core/store/store';

export interface ColorPickerPanelProps {
  /** Current color as `#rrggbb` (the single source of truth stays in the calling domain). */
  value: string;
  /** Called with the new `#rrggbb` whenever the ring, a slider or a preset changes. */
  onChange: (hex: string) => void;
}

/** Parse `#rrggbb` into 0-255 channels (invalid input yields black). */
const hexToRgb = (hex: string) => ({
  r: parseInt(hex.slice(1, 3), 16) || 0,
  g: parseInt(hex.slice(3, 5), 16) || 0,
  b: parseInt(hex.slice(5, 7), 16) || 0,
});

/** Convert `#rrggbb` to integer HSL (h 0-360, s/l 0-100). */
const hexToHsl = (hex: string) => {
  let { r, g, b } = hexToRgb(hex);
  r /= 255;
  g /= 255;
  b /= 255;

  const max = Math.max(r, g, b),
    min = Math.min(r, g, b);
  let h = 0,
    s = 0;
  const l = (max + min) / 2;

  if (max !== min) {
    const d = max - min;
    s = l > 0.5 ? d / (2 - max - min) : d / (max + min);

    if (max === r) h = (g - b) / d + (g < b ? 6 : 0);
    else if (max === g) h = (b - r) / d + 2;
    else h = (r - g) / d + 4;

    h /= 6;
  }

  return { h: Math.round(h * 360), s: Math.round(s * 100), l: Math.round(l * 100) };
};

/** Convert HSL (h 0-360, s/l 0-100) to `#rrggbb`. */
const hslToHex = (h: number, s: number, l: number) => {
  l /= 100;
  const a = (s * Math.min(l, 1 - l)) / 100;

  const f = (n: number) => {
    const k = (n + h / 30) % 12;
    const color = l - a * Math.max(Math.min(k - 3, 9 - k, 1), -1);
    return Math.round(255 * color)
      .toString(16)
      .padStart(2, '0');
  };

  return `#${f(0)}${f(8)}${f(4)}`;
};

/**
 * Shared color picker panel.
 * @param props - incoming color plus the persistence callback supplied by the caller
 * @returns ring / sliders / preset swatches bound to the incoming color
 */
const ColorPickerPanel: Component<ColorPickerPanelProps> = (props) => {
  // Seed the ring from the incoming color; later changes are followed below.
  const initial = untrack(() => hexToHsl(props.value));
  const [h, setH] = createSignal(initial.h); // Hue (0-360 degrees)
  const [s, setS] = createSignal(initial.s); // Saturation (0-100%)
  const [l, setL] = createSignal(initial.l); // Lightness (0-100%)

  // Follow external color changes (another picker edited the same value, or the
  // settings card switched the edit target) without clobbering the in-progress
  // HSL while the user is dragging a slider.
  createEffect(() => {
    const currentHex = props.value;
    const stale = untrack(() => currentHex.toLowerCase() !== hslToHex(h(), s(), l()).toLowerCase());
    if (!stale) return;
    const next = hexToHsl(currentHex);
    setH(next.h);
    setS(next.s);
    setL(next.l);
  });

  const rgb = createMemo(() => hexToRgb(props.value));

  const presetThemes = [
    { name: t('app.theme.softBlue'), color: DEFAULT_THEME_COLOR },
    // gate-allow: color preset swatch
    { name: t('app.theme.ashPink'), color: '#b8929e' },
    // gate-allow: color preset swatch
    { name: t('app.theme.warmGray'), color: '#a8a098' },
    // gate-allow: color preset swatch
    { name: t('app.theme.sageGreen'), color: '#9aab9a' },
    // gate-allow: color preset swatch
    { name: t('app.theme.lavender'), color: '#a89cc8' },
  ];

  /**
   * Update one HSL channel and publish the resulting hex.
   * @param type - which channel changed
   * @param val - new channel value
   */
  const updateChannel = (type: 'h' | 's' | 'l', val: number) => {
    let nextH = h();
    let nextS = s();
    let nextL = l();

    if (type === 'h') {
      setH(val);
      nextH = val;
    } else if (type === 's') {
      setS(val);
      nextS = val;
    } else {
      setL(val);
      nextL = val;
    }

    props.onChange(hslToHex(nextH, nextS, nextL));
  };

  /**
   * Map a ring pointer position to a hue angle.
   * @param e - pointer event on the ring
   * @param rect - ring bounds
   */
  const handleRingInteraction = (e: PointerEvent, rect: DOMRect) => {
    const centerX = rect.left + rect.width / 2;
    const centerY = rect.top + rect.height / 2;
    const angle = Math.atan2(e.clientY - centerY, e.clientX - centerX);
    let degree = (angle * 180) / Math.PI + 90;
    if (degree < 0) degree += 360;
    updateChannel('h', Math.round(degree));
  };

  /** Ring indicator position derived from the current hue. */
  const pointerStyle = createMemo(() => {
    const rad = ((h() - 90) * Math.PI) / 180;
    const radius = 102;
    const x = Math.cos(rad) * radius;
    const y = Math.sin(rad) * radius;

    return {
      transform: `translate(${x}px, ${y}px) translate(-50%, -50%)`,
    };
  });

  return (
    <div
      class="flex flex-col gap-[10px]"
      style={{
        '--h': h(),
        '--s': `${s()}%`,
        '--l': `${l()}%`,
      }}
    >
      <div class="grid grid-cols-[100px_240px_100px] gap-8 items-center justify-center py-5">
        <div class="flex flex-col gap-3">
          <div class="bg-white/5 border border-white/10 p-[10px] rounded-[10px] text-center">
            <span class="block text-[16px] text-white/35 mb-2 font-bold">R</span>
            <div class="font-mono text-[16px] text-white font-bold">{rgb().r}</div>
          </div>
          <div class="bg-white/5 border border-white/10 p-[10px] rounded-[10px] text-center">
            <span class="block text-[16px] text-white/35 mb-2 font-bold">G</span>
            <div class="font-mono text-[16px] text-white font-bold">{rgb().g}</div>
          </div>
          <div class="bg-white/5 border border-white/10 p-[10px] rounded-[10px] text-center">
            <span class="block text-[16px] text-white/35 mb-2 font-bold">B</span>
            <div class="font-mono text-[16px] text-white font-bold">{rgb().b}</div>
          </div>
        </div>

        <div class="relative w-[220px] h-[220px] flex items-center justify-center">
          <div
            class="w-full h-full rounded-full cursor-pointer transition-all duration-300"
            style={{
              background:
                'conic-gradient(hsl(0deg var(--s) var(--l)), \n                                        hsl(60deg var(--s) var(--l)), \n                                        hsl(120deg var(--s) var(--l)), \n                                        hsl(180deg var(--s) var(--l)), \n                                        hsl(240deg var(--s) var(--l)), \n                                        hsl(300deg var(--s) var(--l)), \n                                        hsl(360deg var(--s) var(--l)))',
              mask: 'radial-gradient(transparent 59.5%, black 60.5%)',
            }}
            onPointerDown={(e) => {
              const target = e.currentTarget;
              const rect = target.getBoundingClientRect();
              target.setPointerCapture(e.pointerId);
              handleRingInteraction(e, rect);
              const onPointerMove = (ev: PointerEvent) => {
                handleRingInteraction(ev, rect);
              };
              const onPointerUp = (ev: PointerEvent) => {
                target.releasePointerCapture(ev.pointerId);
                target.removeEventListener('pointermove', onPointerMove);
                target.removeEventListener('pointerup', onPointerUp);
              };
              target.addEventListener('pointermove', onPointerMove);
              target.addEventListener('pointerup', onPointerUp);
            }}
          />

          <div
            class="absolute w-[110px] h-[110px] rounded-full flex flex-col items-center justify-center shadow-[0_0_20px_var(--primary-color)] border-2 border-white/20 z-20"
            style={{ background: props.value }}
          >
            <span class="text-[14px] font-extrabold text-white">{props.value.toUpperCase()}</span>
          </div>

          {/* gate-allow: color hue knob glow (hue picker UI primitive, white base) */}
          <div
            class="absolute top-1/2 left-1/2 w-[18px] h-[18px] border-[3px] border-white rounded-full shadow-[0_0_5px_#fff,inset_0_0_10px_#fff] pointer-events-none z-[3]"
            style={pointerStyle()}
          />
        </div>

        <div class="flex flex-col gap-3">
          <For each={presetThemes}>
            {(theme) => (
              <div
                class="w-[40px] h-[40px] rounded-full cursor-pointer transition-transform duration-200 border-2 border-transparent hover:scale-110"
                title={theme.name}
                onClick={() => props.onChange(theme.color)}
                style={{
                  background: theme.color,
                  border:
                    props.value.toLowerCase() === theme.color.toLowerCase()
                      ? '2px solid #fff' // gate-allow: color 2px solid #fff
                      : '2px solid transparent',
                }}
              />
            )}
          </For>
        </div>
      </div>

      <div class="mt-[25px] px-[20px]">
        <div class="mb-2">
          <label class="block text-[12px] text-white/40 mb-[10px] text-center">
            {t('app.theme.saturation')}
          </label>
          <input
            type="range"
            min="0"
            max="100"
            value={s()}
            class="custom-slider sat-slider"
            style={{
              background: `linear-gradient(to right, hsl(${h()}, 0%, ${l()}%), hsl(${h()}, 100%, ${l()}%))`,
            }}
            onInput={(e) => updateChannel('s', parseInt(e.currentTarget.value))}
          />
        </div>

        <div class="mb-2">
          <label class="block text-[12px] text-white/40 mb-[10px] text-center">
            {t('app.theme.lightness')}
          </label>
          <input
            type="range"
            min="0"
            max="100"
            value={l()}
            class="custom-slider light-slider"
            style={{
              background: `linear-gradient(to right, #000, hsl(${h()}, ${s()}%, 50%), #fff)`, // gate-allow: color hue bar black/white anchors
            }}
            onInput={(e) => updateChannel('l', parseInt(e.currentTarget.value))}
          />
        </div>
      </div>
    </div>
  );
};

export default ColorPickerPanel;
