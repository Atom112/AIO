/**
 * 模型 / Provider logo 工具
 * @description 统一所有调用方对 logo 的获取逻辑，避免重复定义和不一致
 *
 * 优先级：
 * 1. provider id 直接查表（最准确）
 * 2. model id 文本匹配（兜底，处理 custom provider + 未知来源）
 * 3. 通用 neutral 图标
 */

const FALLBACK = '/icons/model-logo/logo.svg';

/** 已知 provider id → logo 路径 */
const PROVIDER_LOGOS: Record<string, string> = {
  openai:     '/icons/model-logo/openai.svg',
  anthropic:  '/icons/model-logo/claude-color.svg',
  google:     '/icons/model-logo/gemini-color.svg',
  deepseek:   '/icons/model-logo/deepseek-color.svg',
  groq:       '/icons/model-logo/groq-color.svg',
  mistral:    '/icons/model-logo/mistral-color.svg',
  xai:        '/icons/model-logo/grok.svg',
  cohere:     '/icons/model-logo/cohere-color.svg',
  moonshot:   '/icons/model-logo/moonshot.svg',
  zhipu:      '/icons/model-logo/zhipu-color.svg',
  qwen:       '/icons/model-logo/qwen-color.svg',
  ollama:     '/icons/model-logo/ollama.svg',
  doubao:     '/icons/model-logo/doubao-color.svg',
  openrouter: '/icons/model-logo/openai.svg',  // 没有专门 logo 时用中性图标
  custom:     FALLBACK,
};

/** model id 文本匹配规则（按 provider 排序，最具体的优先） */
interface TextRule {
  match: (name: string) => boolean;
  logo: string;
}

const TEXT_RULES: TextRule[] = [
  { match: n => n.includes('gpt') || n.includes('o1') || n.includes('o3') || n.includes('o4') || n.includes('chatgpt'), logo: PROVIDER_LOGOS.openai },
  { match: n => n.includes('claude'), logo: PROVIDER_LOGOS.anthropic },
  { match: n => n.includes('gemini') || n.includes('gemma') || n.includes('palm'), logo: PROVIDER_LOGOS.google },
  { match: n => n.includes('grok'), logo: PROVIDER_LOGOS.xai },
  { match: n => n.includes('deepseek'), logo: PROVIDER_LOGOS.deepseek },
  { match: n => n.includes('mixtral') || n.includes('mistral') || n.includes('codestral') || n.includes('pixtral'), logo: PROVIDER_LOGOS.mistral },
  { match: n => n.includes('llama') || n.includes('groq') || n.includes('whisper'), logo: PROVIDER_LOGOS.groq },
  { match: n => n.includes('command') || n.includes('cohere') || n.includes('embed-v'), logo: PROVIDER_LOGOS.cohere },
  { match: n => n.includes('kimi') || n.includes('moonshot'), logo: PROVIDER_LOGOS.moonshot },
  { match: n => n.includes('glm') || n.includes('chatglm') || n.includes('zhipu'), logo: PROVIDER_LOGOS.zhipu },
  { match: n => n.includes('qwen') || n.includes('qwq') || n.includes('qvq'), logo: PROVIDER_LOGOS.qwen },
  { match: n => n.includes('doubao') || n.includes('seedream') || n.includes('volcengine'), logo: PROVIDER_LOGOS.doubao },
  { match: n => n.includes('llama3.1') || n.includes('llama-3.1') || n.includes('llama-3.3') || n.includes('llama-3.2'), logo: PROVIDER_LOGOS.groq },
  { match: n => n.includes('qwen'), logo: PROVIDER_LOGOS.qwen },
];

/**
 * 根据 provider id 获取 logo 路径
 * @returns logo 路径字符串，找不到则返回 fallback
 */
export function getProviderLogo(providerId: string | null | undefined): string {
  if (!providerId) return FALLBACK;
  return PROVIDER_LOGOS[providerId.toLowerCase()] ?? FALLBACK;
}

/**
 * 根据 model id 文本匹配获取 logo 路径
 * @returns logo 路径字符串，找不到则返回 fallback
 */
export function getModelLogoByName(modelId: string | null | undefined): string {
  if (!modelId) return FALLBACK;
  const name = modelId.toLowerCase();
  for (const rule of TEXT_RULES) {
    if (rule.match(name)) return rule.logo;
  }
  return FALLBACK;
}

/**
 * 统一入口：优先用 provider id，失败时回退到 model id 文本匹配
 * @param providerId - provider 配置中的 id (如 'openai', 'mistral', 'custom-xxx')
 * @param modelId - 模型 id (如 'gpt-4o', 'claude-sonnet-4-5')
 */
export function getLogo(providerId: string | null | undefined, modelId: string | null | undefined): string {
  const fromProvider = getProviderLogo(providerId);
  if (fromProvider !== FALLBACK) return fromProvider;
  return getModelLogoByName(modelId);
}

/** 导出 fallback 给特殊场景 */
export const FALLBACK_LOGO = FALLBACK;
