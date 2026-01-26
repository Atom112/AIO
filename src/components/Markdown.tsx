// Markdown.tsx
import { marked, Tokens } from 'marked';
import { markedHighlight } from "marked-highlight";
import DOMPurify from 'dompurify';
import { createMemo } from 'solid-js';
import hljs from 'highlight.js';
import 'highlight.js/styles/github-dark.css';

// 1. 配置高亮插件 (注意这个顺序)
marked.use(
    markedHighlight({
        langPrefix: 'hljs language-',
        highlight(code, lang) {
            const language = hljs.getLanguage(lang) ? lang : 'plaintext';
            return hljs.highlight(code, { language }).value;
        },
    })
);

// 2. 配置自定义渲染器来包裹按钮
// 我们直接修改全局 renderer 的 code 方法
const renderer = new marked.Renderer();
const originalCodeRenderer = renderer.code.bind(renderer);

renderer.code = (token: Tokens.Code) => {
    // 先获取高亮插件处理后的原始代码块 HTML（包含 <pre><code>...）
    const renderedCode = originalCodeRenderer(token);

    // 返回包裹后的结构
    return `
        <div class="code-wrapper">
            <button class="copy-code-button" title="复制代码">
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

// 3. 配置基础选项
marked.setOptions({ gfm: true, breaks: true });

const Markdown = (props: { content: string }) => {
    const htmlContent = createMemo(() => {
        const rawHtml = marked.parse(props.content || '') as string;

        // 关键修改：DOMPurify 可能删除了 SVG 的内部属性
        return DOMPurify.sanitize(rawHtml, {
            // 允许 SVG 相关的所有标签和常用属性
            ADD_TAGS: ['button', 'svg', 'path', 'span'],
            ADD_ATTR: [
                'target', 'class', 'title', 'draggable',
                'viewBox', 'stroke-width', 'stroke', 'fill',
                'd', 'stroke-linecap', 'stroke-linejoin'
            ],
            USE_PROFILES: { html: true, svg: true } // 显式开启 SVG 配置文件
        });
    });

    const handleCopy = (e: MouseEvent) => {
        const target = e.target as HTMLElement;
        const btn = target.closest('.copy-code-button');
        if (!btn) return;

        // 向下寻找 pre 中的 textContent
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
            });
        }
    };

    return (
        <div class="markdown-body" innerHTML={htmlContent()} onClick={handleCopy} />
    );
};

export default Markdown;