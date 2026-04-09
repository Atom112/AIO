
import { marked, Tokens } from 'marked';
import { markedHighlight } from "marked-highlight";
import DOMPurify from 'dompurify';
import { createMemo, Component } from 'solid-js';
import hljs from 'highlight.js';
import 'highlight.js/styles/github-dark.css';

// 配置 Marked 高亮和渲染器
marked.use(
    markedHighlight({
        langPrefix: 'hljs language-',
        highlight(code, lang) {
            const language = hljs.getLanguage(lang) ? lang : 'plaintext';
            return hljs.highlight(code, { language }).value;
        },
    })
);

const renderer = new marked.Renderer();
const originalCodeRenderer = renderer.code.bind(renderer);

/**
 * 重写代码块渲染规则
 * @param {Tokens.Code} token - 代码块 Token
 * @returns {string} 带复制按钮的 HTML
 */
renderer.code = (token: Tokens.Code) => {
    const renderedCode = originalCodeRenderer(token);

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

marked.use({ renderer });

marked.setOptions({ gfm: true, breaks: true });

interface MarkdownProps {
    content: string; // Markdown 原始字符串
}

/**
 * Markdown 渲染组件
 * @param {MarkdownProps} props - 组件属性
 * @returns {JSX.Element} 渲染后的 HTML 元素
 */
const Markdown: Component<MarkdownProps> = (props) => {
    const htmlContent = createMemo(() => {
        const rawHtml = marked.parse(props.content || '') as string;

        return DOMPurify.sanitize(rawHtml, {
            ADD_TAGS: ['button', 'svg', 'path', 'span'],
            ADD_ATTR: [
                'target', 'class', 'title', 'draggable',
                'viewBox', 'stroke-width', 'stroke', 'fill',
                'd', 'stroke-linecap', 'stroke-linejoin'
            ],
            USE_PROFILES: { html: true, svg: true }
        });
    });

    /**
     * 处理代码块复制
     * @param {MouseEvent} e - 点击事件
     */
    const handleCopy = (e: MouseEvent) => {
        const target = e.target as HTMLElement;
        const btn = target.closest('.copy-code-button');
        if (!btn) return;

        const codeElement = btn.parentElement?.querySelector('pre code');
        if (codeElement) {
            const textToCopy = (codeElement as HTMLElement).innerText;

            navigator.clipboard.writeText(textToCopy).then(() => {
                const span = btn.querySelector('span');
                if (span) {
                    const oldText = span.innerText;
                    span.innerText = '已复制!';
                    btn.classList.add('copied');

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