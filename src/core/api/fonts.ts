// Font domain Tauri command wrappers (mirrors commands/fonts.rs).
import { invoke } from '@tauri-apps/api/core';

/** List installed font family names (sorted, de-duplicated) for the app-font picker. */
export const listInstalledFonts = () => invoke<string[]>('list_installed_fonts');
