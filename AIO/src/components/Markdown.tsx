import { marked } from 'marked';
import { markedHighlight } from "marked-highlight"; // 引入高亮插件
import DOMPurify from 'dompurify';
import { createMemo } from 'solid-js';
import hljs from 'highlight.js';
import 'highlight.js/styles/github-dark.css';

// 1. 使用插件模式配置 marked 
marked.use(
    markedHighlight({
        langPrefix: 'hljs language-',
        highlight(code, lang) {
            const language = hljs.getLanguage(lang) ? lang : 'plaintext';
            return hljs.highlight(code, { language }).value;
        },
    })
);

// 2. 配置 marked 选项（启用表格、换行等 GFM 特性）
marked.setOptions({
    gfm: true,
    breaks: true,
});

interface MarkdownProps {
    content: string;
}

const Markdown = (props: MarkdownProps) => {
    const htmlContent = createMemo(() => {
        // 解析 Markdown
        const rawHtml = marked.parse(props.content || '') as string;

        // 关键：确保 DOMPurify 允许 hljs 使用的 class 属性
        return DOMPurify.sanitize(rawHtml, {
            ADD_ATTR: ['target'], // 如果有链接可以保留
            USE_PROFILES: { html: true }
        });
    });

    return (
        <div
            class="markdown-body"
            innerHTML={htmlContent()}
        />
    );
};

export default Markdown;