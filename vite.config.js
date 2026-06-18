import { defineConfig } from "vite";
import solid from "vite-plugin-solid";
import { copyFile, mkdir } from "node:fs/promises";
import { existsSync } from "node:fs";
import { resolve, dirname } from "node:path";

const host = process.env.TAURI_DEV_HOST;

const LOBE_ICONS_FILES = [
  "openai.svg",
  "claude-color.svg",
  "gemini-color.svg",
  "google.svg",
  "deepseek-color.svg",
  "groq.svg",
  "groq-text.svg",
  "mistral-color.svg",
  "xai.svg",
  "grok.svg",
  "cohere-color.svg",
  "moonshot.svg",
  "zhipu-color.svg",
  "qwen-color.svg",
  "ollama.svg",
  "doubao-color.svg",
];

/**
 * 把 node_modules/@lobehub/icons-static-svg/icons/ 下的指定 SVG
 * 扁平化拷贝到 public/icons/model-logo/，让 dist 输出保持干净路径
 *
 * dev: buildStart hook 启动时立即执行，dev server 立即可用
 * build: buildStart hook build 阶段执行
 */
function lobeIconsPlugin() {
  const projectRoot = process.cwd();
  const srcDir = resolve(projectRoot, "node_modules/@lobehub/icons-static-svg/icons");
  const outDir = resolve(projectRoot, "public/icons/model-logo");

  return {
    name: "aio-lobe-icons-copy",
    async buildStart() {
      if (!existsSync(srcDir)) {
        console.warn(`[aio-lobe-icons] source dir not found: ${srcDir}`);
        return;
      }
      await mkdir(outDir, { recursive: true });
      let copied = 0;
      for (const file of LOBE_ICONS_FILES) {
        const src = resolve(srcDir, file);
        const dest = resolve(outDir, file);
        if (existsSync(src)) {
          await copyFile(src, dest);
          copied++;
        } else {
          console.warn(`[aio-lobe-icons] source file missing: ${src}`);
        }
      }
      console.log(`[aio-lobe-icons] copied ${copied} SVG icons to public/icons/model-logo/`);
    },
    // 监听 node_modules 中 SVG 变化时自动重新复制
    watchChange(id) {
      if (id.startsWith(srcDir) && id.endsWith(".svg")) {
        const file = id.slice(srcDir.length + 1);
        const dest = resolve(outDir, file);
        if (existsSync(id)) {
          copyFile(id, dest).catch(() => {});
        }
      }
    },
  };
}

export default defineConfig(async () => ({
  plugins: [
    solid(),
    lobeIconsPlugin(),
  ],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? { protocol: "ws", host, port: 1421 }
      : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
}));
