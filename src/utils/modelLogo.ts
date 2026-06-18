/**
 * 模型 / Provider logo 工具
 * @description 统一所有调用方对 logo 的获取逻辑
 *
 * 来源：public/icons/model-logo/ (vendored from @lobehub/icons-static-svg)
 * 优先级：
 * 1. provider id 直接查表（最准确）
 * 2. model id 文本匹配（兜底，处理 custom provider + 未知来源）
 * 3. 通用 neutral 图标
 */

const FALLBACK_LOGO = '/icons/model-logo/logo.svg';

const PROVIDER_LOGOS: Record<string, string> = {
  openai:     '/icons/model-logo/openai.svg',
  anthropic:  '/icons/model-logo/claude-color.svg',
  google:     '/icons/model-logo/gemini-color.svg',
  google_brand: '/icons/model-logo/google.svg',
  deepseek:   '/icons/model-logo/deepseek-color.svg',
  groq:       '/icons/model-logo/groq.svg',
  groq_text:  '/icons/model-logo/groq-text.svg',
  mistral:    '/icons/model-logo/mistral-color.svg',
  xai:        '/icons/model-logo/xai.svg',
  grok:       '/icons/model-logo/grok.svg',
  cohere:     '/icons/model-logo/cohere-color.svg',
  moonshot:   '/icons/model-logo/moonshot.svg',
  kimi:       '/icons/model-logo/moonshot.svg',
  zhipu:      '/icons/model-logo/zhipu-color.svg',
  qwen:       '/icons/model-logo/qwen-color.svg',
  ollama:     '/icons/model-logo/ollama.svg',
  doubao:     '/icons/model-logo/doubao-color.svg',
  openrouter: '/icons/model-logo/openai.svg',
  custom:     FALLBACK_LOGO,
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
