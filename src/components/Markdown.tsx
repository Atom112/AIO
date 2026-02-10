/**
 * @file Markdown.tsx
 * @description 基于 Solid.js 的 Markdown 渲染组件。
 * 具备功能：
 * 1. 语法高亮 (highlight.js)
 * 2. 代码块内嵌“复制”按钮
 * 3. 安全的 HTML 过滤 (DOMPurify)
 * 4. 样式兼容 Github Dark 主题
 */

import { marked, Tokens } from 'marked';
import { markedHighlight } from "marked-highlight";
import DOMPurify from 'dompurify';
import { createMemo, Component } from 'solid-js';
import hljs from 'highlight.js';

// 导入代码高亮样式
import 'highlight.js/styles/github-dark.css';

/** ---------------------------------------------------------
 * 1. 配置 Marked 扩展与高亮逻辑
 * --------------------------------------------------------- */

marked.use(
    markedHighlight({
        // 指定高亮后的 class 前缀
        langPrefix: 'hljs language-',
        /**
         * 高亮处理函数
         * @param code 原始代码字符串
         * @param lang 编程语言标识符
         */
        highlight(code, lang) {
            const language = hljs.getLanguage(lang) ? lang : 'plaintext';
            return hljs.highlight(code, { language }).value;
        },
    })
);

/** ---------------------------------------------------------
 * 2. 自定义代码块渲染器
 * 用于在 <pre><code> 结构外层包裹功能按钮容器
 * --------------------------------------------------------- */

const renderer = new marked.Renderer();
const originalCodeRenderer = renderer.code.bind(renderer);

/**
 * 重写代码块渲染规则
 * @param token Marked 解析出的代码块 Token
 * @returns 带有“复制”按钮的 HTML 字符串
 */
renderer.code = (token: Tokens.Code) => {
    // 调用原始渲染器获取经过高亮后的 HTML
    const renderedCode = originalCodeRenderer(token);

    // 返回经过自定义包裹的结构
    // .code-wrapper 用于相对定位
    // .copy-code-button 为浮动在右上角的按钮
    return `
        <div class="code-wrapper">
            <button class="copy-code-button" title="复制代码" aria-label="Copy code">
                <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor">
                    <path stroke-linecap="round" stroke-linejoin="round" d="M15.75 17.25v3.375c0 .621-.504 1.125-1.125 1.125h-9.75a1.125 1.125 0 01-1.125-1.125V7.875c0-.621.504-1.125 1.125-1.125H6.75a9.06 9.06 0 011.5.124m7.5 10.376h3.375c.621 0 1.125-.504 1.125-1.125V11.25c0-4.46-3.243-8.161-7.5-8.876a9.06 9.06 0 00-1.5-.124H9.375c-.621 0-1.125.504-1.125 1.125v3.5m7.5 10.375H9.375a1.125 1.125 0 01-1.125-1.125v-9.25m12 6.625v-1.875a3.375 3.375 0 00-3.375-3.375h-1.5a1.125 1.125 0 01-1.125-1.125v-1.5a3.375 3.375 0 00-3.375-3.375H9.75" />
                </svg>
                <span>复制</span>
            </button>
            ${renderedCode}
        </div>
    `;
};

// 应用自定义渲染器
marked.use({ renderer });

// 配置基础解析选项：允许 GFM 规范，允许回车换行
marked.setOptions({ gfm: true, breaks: true });

/** ---------------------------------------------------------
 * 3. Markdown 组件实现
 * --------------------------------------------------------- */

interface MarkdownProps {
    /** Markdown 原始字符串 */
    content: string;
}

/**
 * Markdown 渲染组件
 */
const Markdown: Component<MarkdownProps> = (props) => {
    /**
     * 计算并转换 HTML 内容
     * 使用 createMemo 确保仅在内容变化时重新解析
     */
    const htmlContent = createMemo(() => {
        const rawHtml = marked.parse(props.content || '') as string;

        /**
         * 关键步骤：清理 XSS 风险并保留自定义组件
         * DOMPurify 默认会过滤掉 SVG 和自定义属性，因此需要显式授权
         */
        return DOMPurify.sanitize(rawHtml, {
            // 允许自定义的标签（包括 SVG）
            ADD_TAGS: ['button', 'svg', 'path', 'span'],
            // 允许 SVG 绘图相关的属性及基础样式类
            ADD_ATTR: [
                'target', 'class', 'title', 'draggable',
                'viewBox', 'stroke-width', 'stroke', 'fill',
                'd', 'stroke-linecap', 'stroke-linejoin'
            ],
            // 启用 HTML 和 SVG 配置文件以支持渲染
            USE_PROFILES: { html: true, svg: true }
        });
    });

    /**
     * 全局事件代理：处理代码块复制逻辑
     * 避免在每个按钮上重复绑定事件
     */
    const handleCopy = (e: MouseEvent) => {
        const target = e.target as HTMLElement;
        // 查找点击流中最近的复制按钮
        const btn = target.closest('.copy-code-button');
        if (!btn) return;

        // 获取当前容器内的代码文本内容
        const codeElement = btn.parentElement?.querySelector('pre code');
        if (codeElement) {
            const textToCopy = (codeElement as HTMLElement).innerText;

            // 使用现代剪贴板 API
            navigator.clipboard.writeText(textToCopy).then(() => {
                const span = btn.querySelector('span');
                if (span) {
                    const oldText = span.innerText;
                    // 反馈状态 UI
                    span.innerText = '已复制!';
                    btn.classList.add('copied');

                    // 2秒后恢复原始状态
                    setTimeout(() => {
                        span.innerText = oldText;
                        btn.classList.remove('copied');
                    }, 2000);
                }
            }).catch(err => {
                console.error('无法复制代码: ', err);
            });
        }
    };

    return (
        <div
            class="markdown-body"
            innerHTML={htmlContent()}
            onClick={handleCopy}
        />
    );
};

export default Markdown;