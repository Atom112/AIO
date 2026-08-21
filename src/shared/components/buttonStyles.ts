/**
 * 统一按钮样式常量（自进化项目约定）
 *
 * 同一种类的按钮必须复用本文件中的常量，禁止在组件里手写等价样式，
 * 以保证 各页面/组件/弹窗 中「同种类的按钮样式一致」。
 *
 * 用法：
 *   <button class={btnPrimary}>{t('common.confirm')}</button>
 *   如需额外布局类（w-full / ml-auto / flex-1 等），在调用处拼接：
 *   <button class={'w-full ' + btnPrimarySm}>...</button>
 *
 * 尺寸约定（按场景选择，不要在同一场景混用）：
 *   - btnPrimary / btnSecondary        弹窗底部主次操作
 *   - btnPrimarySm / btnSecondarySm    设置页内嵌操作（保存/测试/连接等）
 *   - btnDanger / btnBadge             删除等危险操作、小胶囊内联操作
 *   - btnGhost / btnIcon / btnClose    文字按钮 / 图标按钮 / 弹窗关闭按钮
 *   - btnTabMd / btnTabXs + tabActive/tabInactive  分段控件/选项卡
 */

/** 主操作按钮（弹窗底部确认/保存/创建等） */
export const btnPrimary =
  'inline-flex items-center justify-center gap-1.5 px-4 py-2 rounded-lg text-sm font-medium bg-pri text-black hover:opacity-90 active:scale-95 transition-all duration-200 disabled:opacity-50 disabled:cursor-not-allowed cursor-pointer';

/** 主操作按钮（紧凑，设置页内嵌操作） */
export const btnPrimarySm =
  'inline-flex items-center justify-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium bg-pri text-black hover:opacity-90 active:scale-95 transition-all duration-200 disabled:opacity-50 disabled:cursor-not-allowed cursor-pointer';

/** 次操作按钮（弹窗底部取消/稍后等） */
export const btnSecondary =
  'inline-flex items-center justify-center gap-1.5 px-4 py-2 rounded-lg text-sm font-medium bg-white/[0.06] border border-white/10 text-white/60 hover:bg-white/[0.1] hover:text-white/85 active:scale-95 transition-all duration-200 disabled:opacity-50 disabled:cursor-not-allowed cursor-pointer';

/** 次操作按钮（紧凑，设置页内嵌操作） */
export const btnSecondarySm =
  'inline-flex items-center justify-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium bg-white/[0.06] border border-white/10 text-white/60 hover:bg-white/[0.1] hover:text-white/85 active:scale-95 transition-all duration-200 disabled:opacity-50 disabled:cursor-not-allowed cursor-pointer';

/** 危险操作按钮（删除/移除等） */
export const btnDanger =
  'inline-flex items-center justify-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium border border-danger/30 bg-danger/10 text-danger hover:bg-danger/20 active:scale-95 transition-all duration-200 disabled:opacity-50 disabled:cursor-not-allowed cursor-pointer';

/** 成功/通过操作按钮（测试连接、批准等） */
export const btnSuccess =
  'inline-flex items-center justify-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium border border-green-400/30 bg-green-400/10 text-green-400 hover:bg-green-400/20 active:scale-95 transition-all duration-200 disabled:opacity-50 disabled:cursor-not-allowed cursor-pointer';

/** 小胶囊内联操作（标签/小按钮） */
export const btnBadge =
  'inline-flex items-center justify-center gap-1 px-2 py-1 rounded-md text-[11px] font-medium bg-white/[0.06] border border-white/10 text-white/60 hover:bg-white/[0.1] hover:text-white/85 transition-all duration-200 disabled:opacity-50 disabled:cursor-not-allowed cursor-pointer';

/** 幽灵按钮（无边框文字按钮） */
export const btnGhost =
  'inline-flex items-center justify-center bg-transparent border-none text-white/60 hover:text-white/90 transition-colors duration-200 disabled:opacity-50 disabled:cursor-not-allowed cursor-pointer';

/** 图标按钮（无边框方形，工具栏/行内操作） */
export const btnIcon =
  'flex items-center justify-center bg-transparent border-none rounded-md transition-all duration-200 disabled:opacity-50 disabled:cursor-not-allowed cursor-pointer';

/** 模态框右上角关闭按钮 */
export const btnClose =
  'w-8 h-8 rounded-lg bg-transparent border-none text-xl cursor-pointer leading-none p-0 transition-all duration-200 text-white/40 hover:text-white hover:bg-danger/80';

/** 分段控件/选项卡：中等尺寸基类 */
export const btnTabMd =
  'px-3 py-1.5 rounded-md text-sm transition-colors duration-200 cursor-pointer';

/** 分段控件/选项卡：小尺寸基类 */
export const btnTabXs =
  'px-2.5 py-1 rounded-md text-xs transition-colors duration-200 cursor-pointer';

/** 分段控件/选项卡：选中态 */
export const tabActive = 'bg-pri-20 text-white';

/** 分段控件/选项卡：未选中态 */
export const tabInactive = 'text-white/50 hover:text-white/75';
