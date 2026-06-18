/**
 * 模型 / Provider logo 工具
 * @description 统一所有调用方对 logo 的获取逻辑
 *
 * 来源：@lobehub/icons-static-svg (通过 Vite ?raw 嵌入 bundle, 0 文件依赖)
 * 优先级：
 * 1. provider id 直接查表（最准确）
 * 2. model id 文本匹配（兜底，处理 custom provider + 未知来源）
 * 3. 通用 neutral 图标
 */

import openaiSvg from '@lobehub/icons-static-svg/icons/openai.svg?raw';
import claudeSvg from '@lobehub/icons-static-svg/icons/claude-color.svg?raw';
import geminiSvg from '@lobehub/icons-static-svg/icons/gemini-color.svg?raw';
import googleSvg from '@lobehub/icons-static-svg/icons/google.svg?raw';
import deepseekSvg from '@lobehub/icons-static-svg/icons/deepseek-color.svg?raw';
import groqSvg from '@lobehub/icons-static-svg/icons/groq.svg?raw';
import groqTextSvg from '@lobehub/icons-static-svg/icons/groq-text.svg?raw';
import mistralSvg from '@lobehub/icons-static-svg/icons/mistral-color.svg?raw';
import xaiSvg from '@lobehub/icons-static-svg/icons/xai.svg?raw';
import grokSvg from '@lobehub/icons-static-svg/icons/grok.svg?raw';
import cohereSvg from '@lobehub/icons-static-svg/icons/cohere-color.svg?raw';
import moonshotSvg from '@lobehub/icons-static-svg/icons/moonshot.svg?raw';
import zhipuSvg from '@lobehub/icons-static-svg/icons/zhipu-color.svg?raw';
import qwenSvg from '@lobehub/icons-static-svg/icons/qwen-color.svg?raw';
import ollamaSvg from '@lobehub/icons-static-svg/icons/ollama.svg?raw';
import doubaoSvg from '@lobehub/icons-static-svg/icons/doubao-color.svg?raw';
import fallbackSvg from '../../public/icons/model-logo/logo.svg?raw';

/** SVG 字符串 → data URL（base64 编码, UTF-8 安全） */
function svgToDataUrl(svg: string): string {
  // 用 encodeURIComponent + unescape 兼容 UTF-8 字符（部分 SVG 含中文/特殊符号）
  const b64 = btoa(unescape(encodeURIComponent(svg)));
  return `data:image/svg+xml;base64,${b64}`;
}

const PROVIDER_LOGOS: Record<string, string> = {
  openai:       svgToDataUrl(openaiSvg),
  anthropic:    svgToDataUrl(claudeSvg),
  google:       svgToDataUrl(geminiSvg),
  google_brand: svgToDataUrl(googleSvg),
  deepseek:     svgToDataUrl(deepseekSvg),
  groq:         svgToDataUrl(groqSvg),
  groq_text:    svgToDataUrl(groqTextSvg),
  mistral:      svgToDataUrl(mistralSvg),
  xai:          svgToDataUrl(xaiSvg),
  grok:         svgToDataUrl(grokSvg),
  cohere:       svgToDataUrl(cohereSvg),
  moonshot:     svgToDataUrl(moonshotSvg),
  kimi:         svgToDataUrl(moonshotSvg),
  zhipu:        svgToDataUrl(zhipuSvg),
  qwen:         svgToDataUrl(qwenSvg),
  ollama:       svgToDataUrl(ollamaSvg),
  doubao:       svgToDataUrl(doubaoSvg),
  openrouter:   svgToDataUrl(openaiSvg),  // 兜底用 openai 图标
  custom:       svgToDataUrl(fallbackSvg),
};

interface TextRule {
  match: (name: string) => boolean;
  logo: string;
}

const TEXT_RULES: TextRule[] = [
  { match: n => n.includes('gpt') || n.includes('o1') || n.includes('o3') || n.includes('o4') || n.includes('chatgpt'), logo: PROVIDER_LOGOS.openai },
  { match: n => n.includes('claude'), logo: PROVIDER_LOGOS.anthropic },
  { match: n => n.includes('gemini') || n.includes('gemma') || n.includes('palm'), logo: PROVIDER_LOGOS.google },
  { match: n => n.includes('grok'), logo: PROVIDER_LOGOS.grok },
  { match: n => n.includes('deepseek'), logo: PROVIDER_LOGOS.deepseek },
  { match: n => n.includes('mixtral') || n.includes('mistral') || n.includes('codestral') || n.includes('pixtral'), logo: PROVIDER_LOGOS.mistral },
  { match: n => n.includes('groq'), logo: PROVIDER_LOGOS.groq },
  { match: n => n.includes('llama'), logo: PROVIDER_LOGOS.groq },
  { match: n => n.includes('whisper'), logo: PROVIDER_LOGOS.groq },
  { match: n => n.includes('command') || n.includes('cohere') || n.includes('embed-v'), logo: PROVIDER_LOGOS.cohere },
  { match: n => n.includes('kimi') || n.includes('moonshot'), logo: PROVIDER_LOGOS.moonshot },
  { match: n => n.includes('glm') || n.includes('chatglm') || n.includes('zhipu'), logo: PROVIDER_LOGOS.zhipu },
  { match: n => n.includes('qwen') || n.includes('qwq') || n.includes('qvq'), logo: PROVIDER_LOGOS.qwen },
  { match: n => n.includes('doubao') || n.includes('seedream') || n.includes('volcengine'), logo: PROVIDER_LOGOS.doubao },
];

export const FALLBACK_LOGO = PROVIDER_LOGOS.custom;

export function getProviderLogo(providerId: string | null | undefined): string {
  if (!providerId) return FALLBACK_LOGO;
  return PROVIDER_LOGOS[providerId.toLowerCase()] ?? FALLBACK_LOGO;
}

export function getModelLogoByName(modelId: string | null | undefined): string {
  if (!modelId) return FALLBACK_LOGO;
  const name = modelId.toLowerCase();
  for (const rule of TEXT_RULES) {
    if (rule.match(name)) return rule.logo;
  }
  return FALLBACK_LOGO;
}

export function getLogo(providerId: string | null | undefined, modelId: string | null | undefined): string {
  const fromProvider = getProviderLogo(providerId);
  if (fromProvider !== FALLBACK_LOGO) return fromProvider;
  return getModelLogoByName(modelId);
}
