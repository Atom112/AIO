import { marked, Tokens } from 'marked';
import { markedHighlight } from 'marked-highlight';
import DOMPurify from 'dompurify';
import { convertFileSrc } from '@tauri-apps/api/core';
import { createMemo, Component, Index, Show } from 'solid-js';
import hljs from 'highlight.js';
import 'highlight.js/styles/github-dark.css';
import ThinkBlock from './ThinkBlock';
import { t } from '../../core/i18n';

// 配置 Marked 高亮和渲染器
marked.use(
  markedHighlight({
    langPrefix: 'hljs language-',
    highlight(code, lang) {
      const language = hljs.getLanguage(lang) ? lang : 'plaintext';
      return hljs.highlight(code, { language }).value;
    },
  }),
);

const renderer = new marked.Renderer();
const originalCodeRenderer = renderer.code.bind(renderer);

// ══════════════════════════════════════════════
//  Catppuccin VSCode Icons — language → SVG
//  Source: https://github.com/catppuccin/vscode-icons (MIT)
//  CSS variables defined in src/index.css (Mocha palette)
// ══════════════════════════════════════════════

const _s = (inner: string) =>
  `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" class="inline-block shrink-0 align-middle w-3.5 h-3.5">${inner}</svg>`;

const ICONS: Record<string, string> = {
  javascript: _s(
    `<g fill="none" stroke="var(--vscode-ctp-yellow)" stroke-linecap="round" stroke-linejoin="round"><path d="M4.5 11c0 .828.672 1.5 1.5 1.5s1.5-.672 1.5-1.5V7.5M10.7 7.5h-.6c-.663 0-1.2.56-1.2 1.25S9.437 10 10.1 10h.6c.663 0 1.2.56 1.2 1.25s-.537 1.25-1.2 1.25h-.6c-.663 0-1.2-.56-1.2-1.25"/><path d="M4 1.5h8c1.385 0 2.5 1.115 2.5 2.5v8c0 1.385-1.115 2.5-2.5 2.5H4c-1.385 0-2.5-1.115-2.5-2.5V4c0-1.385 1.115-2.5 2.5-2.5z"/></g>`,
  ),
  js: _s(
    `<g fill="none" stroke="var(--vscode-ctp-yellow)" stroke-linecap="round" stroke-linejoin="round"><path d="M4.5 11c0 .828.672 1.5 1.5 1.5s1.5-.672 1.5-1.5V7.5M10.7 7.5h-.6c-.663 0-1.2.56-1.2 1.25S9.437 10 10.1 10h.6c.663 0 1.2.56 1.2 1.25s-.537 1.25-1.2 1.25h-.6c-.663 0-1.2-.56-1.2-1.25"/><path d="M4 1.5h8c1.385 0 2.5 1.115 2.5 2.5v8c0 1.385-1.115 2.5-2.5 2.5H4c-1.385 0-2.5-1.115-2.5-2.5V4c0-1.385 1.115-2.5 2.5-2.5z"/></g>`,
  ),
  jsx: _s(
    `<g fill="none" stroke="var(--vscode-ctp-yellow)" stroke-linecap="round" stroke-linejoin="round"><path d="M4.5 11c0 .828.672 1.5 1.5 1.5s1.5-.672 1.5-1.5V7.5M10.7 7.5h-.6c-.663 0-1.2.56-1.2 1.25S9.437 10 10.1 10h.6c.663 0 1.2.56 1.2 1.25s-.537 1.25-1.2 1.25h-.6c-.663 0-1.2-.56-1.2-1.25"/><path d="M4 1.5h8c1.385 0 2.5 1.115 2.5 2.5v8c0 1.385-1.115 2.5-2.5 2.5H4c-1.385 0-2.5-1.115-2.5-2.5V4c0-1.385 1.115-2.5 2.5-2.5z"/></g>`,
  ),
  typescript: _s(
    `<g fill="none" stroke="var(--vscode-ctp-blue)" stroke-linecap="round" stroke-linejoin="round"><path d="M4 1.5h8A2.5 2.5 0 0114.5 4v8a2.5 2.5 0 01-2.5 2.5H4A2.5 2.5 0 011.5 12V4A2.5 2.5 0 014 1.5"/><path d="M12.5 8.75c0-.69-.54-1.25-1.2-1.25h-.6c-.66 0-1.2.56-1.2 1.25S10.04 10 10.7 10h.6c.66 0 1.2.56 1.2 1.25s-.54 1.25-1.2 1.25h-.6c-.66 0-1.2-.56-1.2-1.25m-3-3.75v5M5 7.5h3"/></g>`,
  ),
  ts: _s(
    `<g fill="none" stroke="var(--vscode-ctp-blue)" stroke-linecap="round" stroke-linejoin="round"><path d="M4 1.5h8A2.5 2.5 0 0114.5 4v8a2.5 2.5 0 01-2.5 2.5H4A2.5 2.5 0 011.5 12V4A2.5 2.5 0 014 1.5"/><path d="M12.5 8.75c0-.69-.54-1.25-1.2-1.25h-.6c-.66 0-1.2.56-1.2 1.25S10.04 10 10.7 10h.6c.66 0 1.2.56 1.2 1.25s-.54 1.25-1.2 1.25h-.6c-.66 0-1.2-.56-1.2-1.25m-3-3.75v5M5 7.5h3"/></g>`,
  ),
  tsx: _s(
    `<g fill="none" stroke="var(--vscode-ctp-blue)" stroke-linecap="round" stroke-linejoin="round"><path d="M4 1.5h8A2.5 2.5 0 0114.5 4v8a2.5 2.5 0 01-2.5 2.5H4A2.5 2.5 0 011.5 12V4A2.5 2.5 0 014 1.5"/><path d="M12.5 8.75c0-.69-.54-1.25-1.2-1.25h-.6c-.66 0-1.2.56-1.2 1.25S10.04 10 10.7 10h.6c.66 0 1.2.56 1.2 1.25s-.54 1.25-1.2 1.25h-.6c-.66 0-1.2-.56-1.2-1.25m-3-3.75v5M5 7.5h3"/></g>`,
  ),
  python: _s(
    `<g fill="none" stroke-linecap="round" stroke-linejoin="round"><path stroke="var(--vscode-ctp-blue)" d="M8.5 5.5h-3m6 0V3c0-.8-.7-1.5-1.5-1.5H7c-.8 0-1.5.7-1.5 1.5v2.5H3c-.8 0-1.5.7-1.5 1.5v2c0 .8.7 1.5 1.48 1.5"/><path stroke="var(--vscode-ctp-yellow)" d="M10.5 10.5h-3m-3 0V13c0 .8.7 1.5 1.5 1.5h3c.8 0 1.5-.7 1.5-1.5v-2.5H13c.8 0 1.5-.7 1.5-1.5V7c0-.8-.7-1.5-1.48-1.5H11.5c0 1.5 0 2-1 2h-2"/><path stroke="var(--vscode-ctp-blue)" d="M2.98 10.5H4.5c0-1.5 0-2 1-2h2M7.5 3.5v0"/><path stroke="var(--vscode-ctp-yellow)" d="m8.5 12.5v0"/></g>`,
  ),
  py: _s(
    `<g fill="none" stroke-linecap="round" stroke-linejoin="round"><path stroke="var(--vscode-ctp-blue)" d="M8.5 5.5h-3m6 0V3c0-.8-.7-1.5-1.5-1.5H7c-.8 0-1.5.7-1.5 1.5v2.5H3c-.8 0-1.5.7-1.5 1.5v2c0 .8.7 1.5 1.48 1.5"/><path stroke="var(--vscode-ctp-yellow)" d="M10.5 10.5h-3m-3 0V13c0 .8.7 1.5 1.5 1.5h3c.8 0 1.5-.7 1.5-1.5v-2.5H13c.8 0 1.5-.7 1.5-1.5V7c0-.8-.7-1.5-1.48-1.5H11.5c0 1.5 0 2-1 2h-2"/><path stroke="var(--vscode-ctp-blue)" d="M2.98 10.5H4.5c0-1.5 0-2 1-2h2M7.5 3.5v0"/><path stroke="var(--vscode-ctp-yellow)" d="m8.5 12.5v0"/></g>`,
  ),
  rust: _s(
    `<g fill="none" stroke="var(--vscode-ctp-peach)" stroke-linecap="round" stroke-linejoin="round"><path d="M15.5 9.5Q8 13.505.5 9.5l1-1-1-2 2-.5V4.5h2l.5-2 1.5 1 1.5-2 1.5 2 1.5-1 .5 2h2V6l2 .5-1 2z"/><path d="M6.5 7.5a1 1 0 01-1 1 1 1 0 01-1-1 1 1 0 011-1 1 1 0 011 1m5 0a1 1 0 01-1 1 1 1 0 01-1-1 1 1 0 011-1 1 1 0 011 1M4 11.02c-.67.37-1.5.98-1.5 2.23s1.22 1.22 2 1.25v-2M12 11c.67.37 1.5 1 1.5 2.25s-1.22 1.22-2 1.25v-2"/></g>`,
  ),
  rs: _s(
    `<g fill="none" stroke="var(--vscode-ctp-peach)" stroke-linecap="round" stroke-linejoin="round"><path d="M15.5 9.5Q8 13.505.5 9.5l1-1-1-2 2-.5V4.5h2l.5-2 1.5 1 1.5-2 1.5 2 1.5-1 .5 2h2V6l2 .5-1 2z"/><path d="M6.5 7.5a1 1 0 01-1 1 1 1 0 01-1-1 1 1 0 011-1 1 1 0 011 1m5 0a1 1 0 01-1 1 1 1 0 01-1-1 1 1 0 011-1 1 1 0 011 1M4 11.02c-.67.37-1.5.98-1.5 2.23s1.22 1.22 2 1.25v-2M12 11c.67.37 1.5 1 1.5 2.25s-1.22 1.22-2 1.25v-2"/></g>`,
  ),
  go: _s(
    `<path fill="none" stroke="var(--vscode-ctp-sapphire)" stroke-linecap="round" stroke-linejoin="round" d="m15.48 8.06-4.85.48m4.85-.48a4.98 4.98 0 01-4.54 5.42 5 5 0 112.95-8.66l-1.7 1.84a2.5 2.5 0 00-4.18 2.06c.05.57.3 1.1.69 1.51.25.27 1 .83 1.78.82.8-.02 1.58-.25 2.07-.81 0 0 .8-.96.68-1.88M2.5 8.5l-2 .01m1.5 2h1.5m-2-3.99 2-.02"/>`,
  ),
  java: _s(
    `<g fill="none" stroke-linecap="round" stroke-linejoin="round"><path stroke="var(--vscode-ctp-text)" d="M10.73 8.41c.57 3 1.59 5.83 2.77 7.09-6.63-3.45-9.76-1.75-10.5 0-.66-3.4-.54-5.74.09-7.78"/><path stroke="var(--vscode-ctp-red)" d="M8.5 7c.63.34 1.82 1.07 2.24 1.41-.54-2.9-.64-5.96-.74-7.91-2.13.58-5.73 1.98-6.9 7.22.52-.69 1.72-1.05 2.4-1.22"/><path stroke="var(--vscode-ctp-red)" d="M5.5 7A1.5 1.5 0 007 8.5 1.5 1.5 0 008.5 7 1.5 1.5 0 007 5.5 1.5 1.5 0 005.5 7"/></g>`,
  ),
  html: _s(
    `<g fill="none" stroke-linecap="round" stroke-linejoin="round"><path stroke="var(--vscode-ctp-peach)" d="M1.5 1.5h13L13 13l-5 2-5-2z"/><path stroke="var(--vscode-ctp-text)" d="M11 4.5H5l.25 3h5.5l-.25 3-2.5 1-2.5-1-.08-1"/></g>`,
  ),
  css: _s(
    `<g fill="none" stroke="var(--vscode-ctp-mauve)" stroke-linecap="round" stroke-linejoin="round"><path d="M4 1.5h8c1.38 0 2.5 1.12 2.5 2.5v8c0 1.38-1.12 2.5-2.5 2.5H4c-1.38 0-2.5-1.12-2.5-2.5V4c0-1.38 1.12-2.5 2.5-2.5z"/><path stroke-width=".814" d="M10.24 11.53c0 .58.437 1.039.96 1.035l.452-.003c.522-.004.949-.451.949-1.033 0-.581-.427-1.066-.949-1.066l-.452.001c-.522 0-.949-.486-.949-1.066s.427-1.038.949-1.038h.452c.522 0 .952.458.952 1.038M6.8 11.53c0 .58.437 1.039.96 1.035l.465-.003c.522-.004.936-.451.936-1.032 0-.58-.41-1.066-.932-1.066h-.47c-.522 0-.949-.485-.949-1.065 0-.58.427-1.038.95-1.038h.451c.522 0 .963.458.963 1.038M3.407 11.53c0 .58.438 1.052.96 1.052h.452c.523 0 .95-.457.95-1.038m.011-2.132c0-.58-.437-1.038-.96-1.038l-.452.001c-.523 0-.96.468-.96 1.05v2.118"/></g>`,
  ),
  scss: _s(
    `<path fill="none" stroke="var(--vscode-ctp-pink)" stroke-linecap="round" stroke-linejoin="round" d="M4 1.5h8c1.38 0 2.5 1.12 2.5 2.5v8c0 1.38-1.12 2.5-2.5 2.5H4c-1.38 0-2.5-1.12-2.5-2.5V4c0-1.38 1.12-2.5 2.5-2.5zM4.5 4.5l3.5 2-3.5 2M7 5.5h4.51"/>`,
  ),
  json: _s(
    `<path fill="none" stroke="var(--vscode-ctp-yellow)" stroke-linecap="round" stroke-linejoin="round" d="M4.5 2.5H4c-.75 0-1.5.75-1.5 1.5v2c0 1.1-1 2-1.83 2 .83 0 1.83.9 1.83 2v2c0 .75.75 1.5 1.5 1.5h.5m7-11h.5c.75 0 1.5.75 1.5 1.5v2c0 1.1 1 2 1.83 2-.83 0-1.83.9-1.83 2v2c0 .74-.75 1.5-1.5 1.5h-.5m-6.5-3a.5.5 0 100-1 .5.5 0 000 1m3 0a.5.5 0 100-1 .5.5 0 000 1m3 0a.5.5 0 100-1 .5.5 0 000 1"/>`,
  ),
  bash: _s(
    `<g fill="none" stroke="var(--vscode-ctp-green)" stroke-linecap="round" stroke-linejoin="round"><path d="M2 15.5c-.7 0-1.5-.8-1.5-1.5V5c0-.7.8-1.5 1.5-1.5h9c.7 0 1.5.8 1.5 1.5v9c0 .7-.8 1.5-1.5 1.5z"/><path d="m1.2 3.8 3.04-2.5S5.17.5 5.7.5h8.4c.66 0 1.4.73 1.4 1.4v7.73a2.7 2.7 0 01-.7 1.75l-2.68 3.51"/><path d="M6 8.75c0-.69-.54-1.25-1.2-1.25h-.6c-.66 0-1.2.56-1.2 1.25S3.54 10 4.2 10h.6c.66 0 1.2.56 1.2 1.25s-.54 1.25-1.2 1.25h-.6c-.66 0-1.2-.56-1.2-1.25M4.5 6.5v1m0 5v1"/></g>`,
  ),
  shell: _s(
    `<g fill="none" stroke="var(--vscode-ctp-green)" stroke-linecap="round" stroke-linejoin="round"><path d="M2 15.5c-.7 0-1.5-.8-1.5-1.5V5c0-.7.8-1.5 1.5-1.5h9c.7 0 1.5.8 1.5 1.5v9c0 .7-.8 1.5-1.5 1.5z"/><path d="m1.2 3.8 3.04-2.5S5.17.5 5.7.5h8.4c.66 0 1.4.73 1.4 1.4v7.73a2.7 2.7 0 01-.7 1.75l-2.68 3.51"/><path d="M6 8.75c0-.69-.54-1.25-1.2-1.25h-.6c-.66 0-1.2.56-1.2 1.25S3.54 10 4.2 10h.6c.66 0 1.2.56 1.2 1.25s-.54 1.25-1.2 1.25h-.6c-.66 0-1.2-.56-1.2-1.25M4.5 6.5v1m0 5v1"/></g>`,
  ),
  sh: _s(
    `<g fill="none" stroke="var(--vscode-ctp-green)" stroke-linecap="round" stroke-linejoin="round"><path d="M2 15.5c-.7 0-1.5-.8-1.5-1.5V5c0-.7.8-1.5 1.5-1.5h9c.7 0 1.5.8 1.5 1.5v9c0 .7-.8 1.5-1.5 1.5z"/><path d="m1.2 3.8 3.04-2.5S5.17.5 5.7.5h8.4c.66 0 1.4.73 1.4 1.4v7.73a2.7 2.7 0 01-.7 1.75l-2.68 3.51"/><path d="M6 8.75c0-.69-.54-1.25-1.2-1.25h-.6c-.66 0-1.2.56-1.2 1.25S3.54 10 4.2 10h.6c.66 0 1.2.56 1.2 1.25s-.54 1.25-1.2 1.25h-.6c-.66 0-1.2-.56-1.2-1.25M4.5 6.5v1m0 5v1"/></g>`,
  ),
  zsh: _s(
    `<g fill="none" stroke="var(--vscode-ctp-green)" stroke-linecap="round" stroke-linejoin="round"><path d="M2 15.5c-.7 0-1.5-.8-1.5-1.5V5c0-.7.8-1.5 1.5-1.5h9c.7 0 1.5.8 1.5 1.5v9c0 .7-.8 1.5-1.5 1.5z"/><path d="m1.2 3.8 3.04-2.5S5.17.5 5.7.5h8.4c.66 0 1.4.73 1.4 1.4v7.73a2.7 2.7 0 01-.7 1.75l-2.68 3.51"/><path d="M6 8.75c0-.69-.54-1.25-1.2-1.25h-.6c-.66 0-1.2.56-1.2 1.25S3.54 10 4.2 10h.6c.66 0 1.2.56 1.2 1.25s-.54 1.25-1.2 1.25h-.6c-.66 0-1.2-.56-1.2-1.25M4.5 6.5v1m0 5v1"/></g>`,
  ),
  powershell: _s(
    `<g fill="none" stroke="var(--vscode-ctp-blue)" stroke-linecap="round" stroke-linejoin="round"><path d="M2 15.5c-.7 0-1.5-.8-1.5-1.5V5c0-.7.8-1.5 1.5-1.5h9c.7 0 1.5.8 1.5 1.5v9c0 .7-.8 1.5-1.5 1.5z"/><path d="m1.2 3.8 3.04-2.5S5.17.5 5.7.5h8.4c.66 0 1.4.73 1.4 1.4v7.73a2.7 2.7 0 01-.7 1.75l-2.68 3.51"/><path d="M9 6.5h3.5M3.5 9.5h9M5.5 12.5h5"/></g>`,
  ),
  pwsh: _s(
    `<g fill="none" stroke="var(--vscode-ctp-blue)" stroke-linecap="round" stroke-linejoin="round"><path d="M2 15.5c-.7 0-1.5-.8-1.5-1.5V5c0-.7.8-1.5 1.5-1.5h9c.7 0 1.5.8 1.5 1.5v9c0 .7-.8 1.5-1.5 1.5z"/><path d="m1.2 3.8 3.04-2.5S5.17.5 5.7.5h8.4c.66 0 1.4.73 1.4 1.4v7.73a2.7 2.7 0 01-.7 1.75l-2.68 3.51"/><path d="M9 6.5h3.5M3.5 9.5h9M5.5 12.5h5"/></g>`,
  ),
  markdown: _s(
    `<path fill="none" stroke="var(--vscode-ctp-sapphire)" stroke-linecap="round" stroke-linejoin="round" d="m9.25 8.25 2.25 2.25 2.25-2.25M3.5 11V5.5l2.04 3 1.96-3V11m4-.5V5M1.65 2.5h12.7c.59 0 1.15.49 1.15 1v9c0 .51-.56 1-1.15 1H1.65c-.59 0-1.15-.49-1.15-1V3.58c0-.5.56-1.08 1.15-1.08"/>`,
  ),
  md: _s(
    `<path fill="none" stroke="var(--vscode-ctp-sapphire)" stroke-linecap="round" stroke-linejoin="round" d="m9.25 8.25 2.25 2.25 2.25-2.25M3.5 11V5.5l2.04 3 1.96-3V11m4-.5V5M1.65 2.5h12.7c.59 0 1.15.49 1.15 1v9c0 .51-.56 1-1.15 1H1.65c-.59 0-1.15-.49-1.15-1V3.58c0-.5.56-1.08 1.15-1.08"/>`,
  ),
  yaml: _s(
    `<path fill="none" stroke="var(--vscode-ctp-red)" stroke-linecap="round" stroke-linejoin="round" d="M2.5 1.5h3l3 4 3-4h3l-9 13h-3L7 8z"/>`,
  ),
  yml: _s(
    `<path fill="none" stroke="var(--vscode-ctp-red)" stroke-linecap="round" stroke-linejoin="round" d="M2.5 1.5h3l3 4 3-4h3l-9 13h-3L7 8z"/>`,
  ),
  toml: _s(
    `<path fill="none" stroke="var(--vscode-ctp-maroon)" stroke-linecap="round" stroke-linejoin="round" d="M3.5 1.5h-2v13h2m9-13h2v13h-2m-8-11h7v3h-2v6h-3v-6h-2z"/>`,
  ),
  xml: _s(
    `<path fill="none" stroke="var(--vscode-ctp-peach)" stroke-linecap="round" stroke-linejoin="round" d="M4.5 4.5 1 8 4.5 11.5M11.5 4.5 15 8 11.5 11.5M9.5 2 6.5 14"/>`,
  ),
  svg: _s(
    `<path fill="none" stroke="var(--vscode-ctp-peach)" stroke-linecap="round" stroke-linejoin="round" d="M4.5 4.5 1 8 4.5 11.5M11.5 4.5 15 8 11.5 11.5M9.5 2 6.5 14"/>`,
  ),
  dockerfile: _s(
    `<path fill="none" stroke="var(--vscode-ctp-blue)" stroke-linecap="round" stroke-linejoin="round" d="M.5 8.5H11l.75-.5a5.35 5.35 0 010-3.5c1 .6 1 1.88 1.74 2 .77-.09 1.23.01 2 .52 0 0-.97 1.77-2.5 1.98-1.93 3.65-4.5 5.5-6.98 5.5C0 14.5.5 8.5.5 8.5m1 0v-2m0 0h8m-6 2v-4m0 0h4m-2-2h2m-2 6v-6m2 6v-6m2 6v-2"/>`,
  ),
  docker: _s(
    `<path fill="none" stroke="var(--vscode-ctp-blue)" stroke-linecap="round" stroke-linejoin="round" d="M.5 8.5H11l.75-.5a5.35 5.35 0 010-3.5c1 .6 1 1.88 1.74 2 .77-.09 1.23.01 2 .52 0 0-.97 1.77-2.5 1.98-1.93 3.65-4.5 5.5-6.98 5.5C0 14.5.5 8.5.5 8.5m1 0v-2m0 0h8m-6 2v-4m0 0h4m-2-2h2m-2 6v-6m2 6v-6m2 6v-2"/>`,
  ),
  c: _s(
    `<path fill="none" stroke="var(--vscode-ctp-blue)" stroke-linecap="round" stroke-linejoin="round" d="M4.056 12.952c2.746 2.734 7.198 2.734 9.944 0l-1.79-1.783c-1.757 1.75-4.607 1.75-6.364 0-1.757-1.75-1.757-4.588 0-6.338 1.757-1.75 4.607-1.75 6.364 0l.895-.891.895-.891c-2.746-2.735-7.198-2.735-9.944 0-2.746 2.734-2.746 7.168 0 9.903z"/>`,
  ),
  cpp: _s(
    `<g fill="none" stroke="var(--vscode-ctp-blue)" stroke-linecap="round" stroke-linejoin="round"><path d="M2.556 12.952c2.746 2.734 7.198 2.734 9.944 0l-1.79-1.783c-1.757 1.75-4.607 1.75-6.364 0-1.757-1.75-1.757-4.588 0-6.338 1.757-1.75 4.607-1.75 6.364 0l.895-.891.895-.891c-2.746-2.735-7.198-2.735-9.944 0-2.746 2.734-2.746 7.168 0 9.903z"/><path d="M7.5 6v4M5.514 8h4M13.486 6v4M11.5 8h4"/></g>`,
  ),
  csharp: _s(
    `<path fill="none" stroke="var(--vscode-ctp-blue)" stroke-linecap="round" stroke-linejoin="round" d="M6.666 1.01c.543.09.912.607.821 1.15L7.181 4h2.972l.36-2.167c.09-.544.606-.913 1.15-.822.543.09.912.606.821 1.15L12.181 4H14c.553 0 1 .447 1 1s-.447 1-1 1h-2.153L11.181 10H13c.553 0 1 .447 1 1s-.447 1-1 1h-2.153l-.36 2.164c-.09.544-.606.913-1.15.822-.543-.09-.912-.606-.821-1.15l.306-1.834h-2.975l-.36 2.167c-.09.543-.606.912-1.15.821-.543-.09-.912-.606-.821-1.15l.303-1.84H2c-.553 0-1-.447-1-1s.447-1 1-1h2.153l.666-4H3c-.553 0-1-.447-1-1s.447-1 1-1h2.153l.36-2.166c.09-.544.606-.913 1.15-.822zM6.847 6l-.666 4h2.972l.666-4z"/>`,
  ),
  cs: _s(
    `<path fill="none" stroke="var(--vscode-ctp-blue)" stroke-linecap="round" stroke-linejoin="round" d="M6.666 1.01c.543.09.912.607.821 1.15L7.181 4h2.972l.36-2.167c.09-.544.606-.913 1.15-.822.543.09.912.606.821 1.15L12.181 4H14c.553 0 1 .447 1 1s-.447 1-1 1h-2.153L11.181 10H13c.553 0 1 .447 1 1s-.447 1-1 1h-2.153l-.36 2.164c-.09.544-.606.913-1.15.822-.543-.09-.912-.606-.821-1.15l.306-1.834h-2.975l-.36 2.167c-.09.543-.606.912-1.15.821-.543-.09-.912-.606-.821-1.15l.303-1.84H2c-.553 0-1-.447-1-1s.447-1 1-1h2.153l.666-4H3c-.553 0-1-.447-1-1s.447-1 1-1h2.153l.36-2.166c.09-.544.606-.913 1.15-.822zM6.847 6l-.666 4h2.972l.666-4z"/>`,
  ),
  php: _s(
    `<path fill="none" stroke="var(--vscode-ctp-blue)" stroke-linecap="round" stroke-linejoin="round" d="M.5 12.5v.74C.5 14 1.274 14.5 2 14.5c.938 0 1.5-.5 1.5-1.255V6c0-1.715 1.494-3.478 3.65-3.5C9.494 2.5 11 4.058 11 5.5c.166 2.99-1.422 4.137-3.504 5v4h8.002V9c.041-.635-.56-1.844-1.367-2.5C13.194 5.808 12.058 5.503 11 5.5M11.5 14.5v-3M6 6.5a.5.5 0 100-1 .5.5 0 000 1z"/>`,
  ),
  ruby: _s(
    `<path fill="none" stroke="var(--vscode-ctp-red)" stroke-linecap="round" stroke-linejoin="round" d="M1.5 9.06v2.5c.02.86.36 1.61.9 2.15 1.76 1.76 5.71.65 8.84-2.47s4.23-7.08 2.47-8.84a3.1 3.1 0 00-2.15-.9h-2.5M14.5 4l-.25 10.25L4 14.5m4.39-6.11c2.34-2.35 3.29-5.2 2.12-6.37S6.49 1.8 4.14 4.14C1.8 6.5.85 9.34 2.02 10.51s4.02.22 6.37-2.12M5.5 14.5l.25-3.75L11 11l-.25-5.25 3.75-.25"/>`,
  ),
  rb: _s(
    `<path fill="none" stroke="var(--vscode-ctp-red)" stroke-linecap="round" stroke-linejoin="round" d="M1.5 9.06v2.5c.02.86.36 1.61.9 2.15 1.76 1.76 5.71.65 8.84-2.47s4.23-7.08 2.47-8.84a3.1 3.1 0 00-2.15-.9h-2.5M14.5 4l-.25 10.25L4 14.5m4.39-6.11c2.34-2.35 3.29-5.2 2.12-6.37S6.49 1.8 4.14 4.14C1.8 6.5.85 9.34 2.02 10.51s4.02.22 6.37-2.12M5.5 14.5l.25-3.75L11 11l-.25-5.25 3.75-.25"/>`,
  ),
  swift: _s(
    `<path fill="none" stroke="var(--vscode-ctp-peach)" stroke-linecap="round" stroke-linejoin="round" d="M14.34 10.2c.34-1.08 1.1-5.07-4.45-8.62a.48.48 0 00-.6.07.44.44 0 00-.02.6c.03.02 2.07 2.5 1.34 5.34-1.26-.86-6.24-4.81-6.24-4.81L7.25 7.5 1.9 4.05S5.68 8.7 8 10.45c-1.12.4-3.56.82-6.78-1.18a.48.48 0 00-.58.06.44.44 0 00-.08.56c.11.18 2.7 4.36 8.14 4.36 1.5 0 2.37-.42 3.08-.77.43-.2.77-.37 1.14-.37.93 0 1.54.92 1.54.93.1.14.27.22.44.21a.46.46 0 00.4-.28c.67-1.55-.49-3.2-.96-3.78z"/>`,
  ),
  kotlin: _s(
    `<g fill="none" stroke-linecap="round" stroke-linejoin="round"><path stroke="var(--vscode-ctp-mauve)" d="M2.5 13.5h11L8 8"/><path stroke="var(--vscode-ctp-peach)" d="M8.03 2.5h5.47l-8 8"/><path stroke="var(--vscode-ctp-red)" d="M2.5 13.5V8"/><path stroke="var(--vscode-ctp-sapphire)" d="M8 2.5H2.5V8l3-2.5"/></g>`,
  ),
  scala: _s(
    `<path fill="none" stroke="var(--vscode-ctp-red)" stroke-linecap="round" stroke-linejoin="round" d="M8 1.5c-3.59 0-6.5 2.91-6.5 6.5s2.91 6.5 6.5 6.5 6.5-2.91 6.5-6.5c0-.7-.1-1.38-.32-2-.37.5-.83.75-1.43.75-.5 0-.96-.21-1.38-.63-1.21 1.04-2.2 1.15-2.38 1.15-.79 0-1.43-.63-1.43-1.42V5.44c-.4.3-.95.51-1.56.56v2.25c0 .76-.63 1.38-1.4 1.38-.76 0-1.39-.62-1.39-1.38s.63-1.38 1.4-1.38c.15 0 .3.03.43.07V5.15c-.36-.28-.64-.66-.64-1.12 0-.84.67-1.53 1.51-1.53.84 0 1.51.68 1.51 1.53 0 .63-.36 1.16-.93 1.41l2.47 3.1c.19-.27.35-.55.45-.86-.29-.13-.49-.42-.49-.76 0-.46.38-.84.84-.84s.84.38.84.84a.86.86 0 01-.46.76c.06.24.1.52.1.78 0 2.01-1.18 3.73-2.88 4.52 1.15-.58 2.18-2.12 2.18-4.52 0-.36-.04-.7-.1-1.02"/>`,
  ),
  lua: _s(
    `<g fill="none" stroke-linecap="round" stroke-linejoin="round"><path stroke="var(--vscode-ctp-text)" d="M10.5 7A1.5 1.5 0 019 8.5 1.5 1.5 0 017.5 7 1.5 1.5 0 019 5.5 1.5 1.5 0 0110.5 7"/><path stroke="var(--vscode-ctp-blue)" d="M7 2.5a6.5 6.5 0 100 13 6.5 6.5 0 000-13m7-2a1.5 1.5 0 100 3 1.5 1.5 0 000-3"/></g>`,
  ),
  graphql: _s(
    `<path fill="none" stroke="var(--vscode-ctp-pink)" stroke-linecap="round" stroke-linejoin="round" d="M9 1.5a1 1 0 01-1 1 1 1 0 01-1-1 1 1 0 011-1 1 1 0 011 1m-5.5 3a1 1 0 01-1 1 1 1 0 01-1-1 1 1 0 011-1 1 1 0 011 1m0 7a1 1 0 01-1 1 1 1 0 01-1-1 1 1 0 011-1 1 1 0 011 1m11 0a1 1 0 01-1 1 1 1 0 01-1-1 1 1 0 011-1 1 1 0 011 1m-5.5 3a1 1 0 01-1 1 1 1 0 01-1-1 1 1 0 011-1 1 1 0 011 1m5.5-10a1 1 0 01-1 1 1 1 0 01-1-1 1 1 0 011-1 1 1 0 011 1m-12 1v5m11-5v5m-10 1h9m-6 2.5-3-1.5m6 1.5 3-1.5m-9-2 4-8m5 8-4-8m-5 1 3-1.5m3 0 3 1.5"/>`,
  ),
  diff: _s(
    `<path fill="none" stroke="var(--vscode-ctp-green)" stroke-linecap="round" stroke-linejoin="round" d="M4 8h8M8 4v8M1.5 2.5h13v11h-13z"/>`,
  ),
  nix: _s(
    `<g fill="none" stroke-linecap="round" stroke-linejoin="round"><path stroke="var(--vscode-ctp-sapphire)" d="M.5 7.5H4m1.39-2L2.05 11"/><path stroke="var(--vscode-ctp-blue)" d="M4 1.5 5.5 4m3.5.5H2.55"/><path stroke="var(--vscode-ctp-sapphire)" d="m12 1.5-1.5 3m1.01 2.6L8.5 1.5"/><path stroke="var(--vscode-ctp-blue)" d="M15.5 8.52 12 8.5m-1.38 2L14 5"/><path stroke="var(--vscode-ctp-sapphire)" d="m12.5 14.5-2.5-3m-2.97.02 6.48-.02"/><path stroke="var(--vscode-ctp-blue)" d="m4 14.5 1.5-3M4.53 9l2.97 5.5"/></g>`,
  ),
  cmake: _s(
    `<g fill="none" stroke="var(--vscode-ctp-green)" stroke-linecap="round" stroke-linejoin="round"><path d="M3.5 1.5h-2v13h2m9-13h2v13h-2m-8-11h7v3h-2v6h-3v-6h-2z"/></g>`,
  ),
};

const _FALLBACK = _s(
  `<path fill="none" stroke="var(--vscode-ctp-text)" stroke-linecap="round" stroke-linejoin="round" d="M13.5 6.5v6a2 2 0 01-2 2h-7a2 2 0 01-2-2v-9c0-1.1.9-2 2-2h4.01m-.01 0 5 5h-4a1 1 0 01-1-1z"/>`,
);

// Aliases
ICONS.mjs = ICONS.javascript;
ICONS.cjs = ICONS.javascript;
ICONS.mts = ICONS.typescript;
ICONS.golang = ICONS.go;
ICONS.jsonc = ICONS.json;
ICONS.mdx = ICONS.markdown;
ICONS.gql = ICONS.graphql;
ICONS.plaintext = _FALLBACK;
ICONS.text = _FALLBACK;
ICONS.txt = _FALLBACK;
ICONS.makefile = _s(
  `<path fill="none" stroke="var(--vscode-ctp-peach)" stroke-linecap="round" stroke-linejoin="round" d="M3.5 1.5h-2v13h2m9-13h2v13h-2m-8-11h7v3h-2v6h-3v-6h-2z"/>`,
);

export function getLangIcon(lang: string): string {
  const key = lang.toLowerCase().trim();
  return ICONS[key] ?? _FALLBACK;
}

/**
 * 重写代码块渲染规则（参考 LobeHub 风格）
 * @param {Tokens.Code} token - 代码块 Token
 * @returns {string} 带 header 栏和复制按钮的 HTML
 */
renderer.code = (token: Tokens.Code) => {
  const renderedCode = originalCodeRenderer(token);
  const lang = token.lang || 'plaintext';

  return `
        <div class="group relative code-block my-5 rounded-xl overflow-clip bg-white/[0.04] border border-white/[0.06]">
            <div class="flex items-center h-6 px-4 select-none bg-white/[0.03] border-b border-white/[0.05] rounded-b-xl">
                <span class="inline-flex items-center gap-1.5 text-xs font-mono tracking-wide text-white/35 lowercase">${getLangIcon(lang)}${lang}</span>
            </div>
            <div class="sticky top-3 z-10 h-0">
                <button class="copy-code-button absolute right-3 top-1.5 flex items-center gap-1.5 px-2 py-0.5 rounded-md text-xs cursor-pointer transition-all duration-200 text-white/40 bg-[rgba(0,0,0,0.15)] backdrop-blur-sm border border-white/[0.06] opacity-0 group-hover:opacity-100 hover:text-white/80 hover:bg-[rgba(0,0,0,0.35)] hover:border-white/[0.12] active:scale-[0.96]" title="${t('chat.markdown.copyCode')}" aria-label="Copy code">
                    <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" class="w-3.5 h-3.5">
                        <path stroke-linecap="round" stroke-linejoin="round" d="M15.75 17.25v3.375c0 .621-.504 1.125-1.125 1.125h-9.75a1.125 1.125 0 01-1.125-1.125V7.875c0-.621.504-1.125 1.125-1.125H6.75a9.06 9.06 0 011.5.124m7.5 10.376h3.375c.621 0 1.125-.504 1.125-1.125V11.25c0-4.46-3.243-8.161-7.5-8.876a9.06 9.06 0 00-1.5-.124H9.375c-.621 0-1.125.504-1.125 1.125v3.5m7.5 10.375H9.375a1.125 1.125 0 01-1.125-1.125v-9.25m12 6.625v-1.875a3.375 3.375 0 00-3.375-3.375h-1.5a1.125 1.125 0 01-1.125-1.125v-1.5a3.375 3.375 0 00-3.375-3.375H9.75" />
                    </svg>
                    <span class="copy-text">${t('common.copy')}</span>
                </button>
            </div>
            <div class="code-body overflow-x-auto">
                ${renderedCode}
            </div>
        </div>
    `;
};

// aio-image://<base64(absPath)> → convertFileSrc 可显示的本地资源 URL
renderer.image = ({ href, title, text }: Tokens.Image) => {
  let src = href || '';
  if (src.startsWith('aio-image://')) {
    try {
      const bytes = Uint8Array.from(atob(src.slice('aio-image://'.length)), (c) => c.charCodeAt(0));
      src = convertFileSrc(new TextDecoder().decode(bytes));
    } catch {
      /* 非法 token 原样保留 */
    }
  }
  const alt = text ? ` alt="${text}"` : '';
  const ttl = title ? ` title="${title}"` : '';
  return `<img src="${src}"${alt}${ttl} class="aio-chat-image" loading="lazy" />`;
};

marked.use({ renderer });

marked.setOptions({ gfm: true, breaks: true });

type Segment =
  { type: 'markdown'; content: string } | { type: 'think'; content: string; isStreaming: boolean };

const isStreamingSegment = (segment: Segment) => segment.type === 'think' && segment.isStreaming;

/**
 * 将消息文本解析为分段: think 块 / markdown 段 (保留位置顺序)
 * - 支持多个完整 <think>...</think> 块
 * - 支持未闭合的尾部 <think> (流式中)
 */
const parseSegments = (text: string): Segment[] => {
  const segments: Segment[] = [];
  const OPEN = '<think>';
  const CLOSE = '</think>';
  const openLen = OPEN.length;
  const closeLen = CLOSE.length;
  let lastIndex = 0;
  let cursor = 0;

  while (cursor < text.length) {
    const start = text.indexOf(OPEN, cursor);
    if (start === -1) break;

    if (start > lastIndex) {
      segments.push({ type: 'markdown', content: text.substring(lastIndex, start) });
    }

    const end = text.indexOf(CLOSE, start + openLen);
    if (end === -1) {
      const content = text.substring(start + openLen);
      if (content.trim()) {
        segments.push({ type: 'think', content: content.trim(), isStreaming: true });
      }
      lastIndex = text.length;
      break;
    } else {
      const content = text.substring(start + openLen, end);
      if (content.trim()) {
        segments.push({ type: 'think', content: content.trim(), isStreaming: false });
      }
      lastIndex = end + closeLen;
      cursor = lastIndex;
    }
  }

  if (lastIndex < text.length) {
    segments.push({ type: 'markdown', content: text.substring(lastIndex) });
  }

  return segments;
};

// PERF-01：有限大小的呈现缓存（内容→HTML），跳过对未变化分段重复执行 marked+hljs+DOMPurify。
// 有界（LRU 退化版：满则清空），避免缓存自身造成内存增长。
const RENDER_CACHE_MAX = 64;
const renderCache = new Map<string, string>();

const renderMarkdownHtml = (raw: string): string => {
  if (!raw.trim()) return '';
  const cached = renderCache.get(raw);
  if (cached !== undefined) return cached;
  const html = marked.parse(raw) as string;
  const out = DOMPurify.sanitize(html, {
    ADD_TAGS: [
      'button',
      'svg',
      'path',
      'span',
      // 表格标签
      'table',
      'thead',
      'tbody',
      'tfoot',
      'tr',
      'th',
      'td',
      'col',
      'colgroup',
      'caption',
    ],
    ADD_ATTR: [
      'target',
      'class',
      'title',
      'draggable',
      'viewBox',
      'stroke-width',
      'stroke',
      'fill',
      'd',
      'stroke-linecap',
      'stroke-linejoin',
      // 表格属性
      'colspan',
      'rowspan',
      'align',
      'valign',
      'scope',
      'headers',
    ],
    USE_PROFILES: { html: true, svg: true },
  });
  if (renderCache.size >= RENDER_CACHE_MAX) {
    renderCache.clear();
  }
  renderCache.set(raw, out);
  return out;
};

interface MarkdownProps {
  content: string;
}

/**
 * Markdown 渲染组件 (含思考过程块)
 * @param {MarkdownProps} props - 组件属性
 * @returns {JSX.Element} 渲染后的 HTML 元素
 */
const Markdown: Component<MarkdownProps> = (props) => {
  const segments = createMemo(() => parseSegments(props.content || ''));

  /**
   * 处理代码块复制
   * @param {MouseEvent} e - 点击事件
   */
  const handleCopy = (e: MouseEvent) => {
    const target = e.target as HTMLElement;
    const btn = target.closest('.copy-code-button');
    if (!btn) return;

    const wrapper = btn.closest('.code-block');
    const codeElement = wrapper?.querySelector('.code-body pre code');
    if (codeElement) {
      const textToCopy = (codeElement as HTMLElement).innerText;

      navigator.clipboard
        .writeText(textToCopy)
        .then(() => {
          const span = btn.querySelector('.copy-text') as HTMLElement | null;
          if (span) {
            const oldText = span.innerText || '';
            span.innerText = t('chat.markdown.copied');
            (btn as HTMLElement).style.color = '#3fb950';
            (btn as HTMLElement).style.borderColor = 'rgba(63,185,80,0.3)';
            (btn as HTMLElement).style.backgroundColor = 'rgba(46,160,67,0.15)';
            (btn as HTMLElement).style.opacity = '1';

            setTimeout(() => {
              span.innerText = oldText;
              (btn as HTMLElement).style.color = '';
              (btn as HTMLElement).style.borderColor = '';
              (btn as HTMLElement).style.backgroundColor = '';
              (btn as HTMLElement).style.opacity = '';
            }, 2000);
          }
        })
        .catch((err) => {
          console.error('无法复制代码: ', err);
        });
    }
  };

  return (
    <div class="markdown-body" onClick={handleCopy}>
      <Index each={segments()}>
        {(seg) => (
          <Show
            when={seg().type === 'think'}
            fallback={
              <>
                {/* eslint-disable-next-line solid/no-innerhtml -- renderMarkdownHtml sanitizes content */}
                <div class="markdown-segment" innerHTML={renderMarkdownHtml(seg().content)} />
              </>
            }
          >
            <ThinkBlock content={seg().content} isStreaming={isStreamingSegment(seg())} />
          </Show>
        )}
      </Index>
    </div>
  );
};

export default Markdown;
