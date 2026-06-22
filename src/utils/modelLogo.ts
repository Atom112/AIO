/**
 * 模型 / Provider logo 工具
 * @description 统一所有调用方对 logo 的获取逻辑
 *
 * 来源：@lobehub/icons-static-svg (通过 Vite ?raw 嵌入 bundle)
 * 优先级：
 * 1. provider id 直接查表（最准确）
 * 2. model id 文本匹配（兜底，处理 custom provider + 未知来源）
 * 3. 通用 neutral 图标 (FALLBACK_LOGO)
 */
import aihubmixColorSvg from '@lobehub/icons-static-svg/icons/aihubmix-color.svg?raw';
import ai302ColorSvg from '@lobehub/icons-static-svg/icons/ai302-color.svg?raw';
import alibabaColorSvg from '@lobehub/icons-static-svg/icons/alibaba-color.svg?raw';
import alibabacloudColorSvg from '@lobehub/icons-static-svg/icons/alibabacloud-color.svg?raw';
import awsSvg from '@lobehub/icons-static-svg/icons/aws.svg?raw';
import azureSvg from '@lobehub/icons-static-svg/icons/azure.svg?raw';
import bailianColorSvg from '@lobehub/icons-static-svg/icons/bailian-color.svg?raw';
import basetenSvg from '@lobehub/icons-static-svg/icons/baseten.svg?raw';
import bedrockColorSvg from '@lobehub/icons-static-svg/icons/bedrock-color.svg?raw';
import cerebrasColorSvg from '@lobehub/icons-static-svg/icons/cerebras-color.svg?raw';
import claudeColorSvg from '@lobehub/icons-static-svg/icons/claude-color.svg?raw';
import cloudflareColorSvg from '@lobehub/icons-static-svg/icons/cloudflare-color.svg?raw';
import cohereColorSvg from '@lobehub/icons-static-svg/icons/cohere-color.svg?raw';
import deepinfraColorSvg from '@lobehub/icons-static-svg/icons/deepinfra-color.svg?raw';
import deepseekColorSvg from '@lobehub/icons-static-svg/icons/deepseek-color.svg?raw';
import doubaoColorSvg from '@lobehub/icons-static-svg/icons/doubao-color.svg?raw';
import fireworksColorSvg from '@lobehub/icons-static-svg/icons/fireworks-color.svg?raw';
import friendliSvg from '@lobehub/icons-static-svg/icons/friendli.svg?raw';
import geminiColorSvg from '@lobehub/icons-static-svg/icons/gemini-color.svg?raw';
import githubcopilotSvg from '@lobehub/icons-static-svg/icons/githubcopilot.svg?raw';
import googleSvg from '@lobehub/icons-static-svg/icons/google.svg?raw';
import grokSvg from '@lobehub/icons-static-svg/icons/grok.svg?raw';
import groqSvg from '@lobehub/icons-static-svg/icons/groq.svg?raw';
import huggingfaceColorSvg from '@lobehub/icons-static-svg/icons/huggingface-color.svg?raw';
import inceptionSvg from '@lobehub/icons-static-svg/icons/inception.svg?raw';
import inferenceSvg from '@lobehub/icons-static-svg/icons/inference.svg?raw';
import kimiColorSvg from '@lobehub/icons-static-svg/icons/kimi-color.svg?raw';
import lmstudioSvg from '@lobehub/icons-static-svg/icons/lmstudio.svg?raw';
import metaColorSvg from '@lobehub/icons-static-svg/icons/meta-color.svg?raw';
import minimaxColorSvg from '@lobehub/icons-static-svg/icons/minimax-color.svg?raw';
import mistralColorSvg from '@lobehub/icons-static-svg/icons/mistral-color.svg?raw';
import modelscopeColorSvg from '@lobehub/icons-static-svg/icons/modelscope-color.svg?raw';
import moonshotSvg from '@lobehub/icons-static-svg/icons/moonshot.svg?raw';
import morphColorSvg from '@lobehub/icons-static-svg/icons/morph-color.svg?raw';
import nebiusSvg from '@lobehub/icons-static-svg/icons/nebius.svg?raw';
import novaColorSvg from '@lobehub/icons-static-svg/icons/nova-color.svg?raw';
import novitaColorSvg from '@lobehub/icons-static-svg/icons/novita-color.svg?raw';
import nvidiaColorSvg from '@lobehub/icons-static-svg/icons/nvidia-color.svg?raw';
import ollamaSvg from '@lobehub/icons-static-svg/icons/ollama.svg?raw';
import openaiSvg from '@lobehub/icons-static-svg/icons/openai.svg?raw';
import opencodeSvg from '@lobehub/icons-static-svg/icons/opencode.svg?raw';
import openrouterSvg from '@lobehub/icons-static-svg/icons/openrouter.svg?raw';
import perplexityColorSvg from '@lobehub/icons-static-svg/icons/perplexity-color.svg?raw';
import poeColorSvg from '@lobehub/icons-static-svg/icons/poe-color.svg?raw';
import qiniuSvg from '@lobehub/icons-static-svg/icons/qiniu.svg?raw';
import qwenColorSvg from '@lobehub/icons-static-svg/icons/qwen-color.svg?raw';
import snowflakeColorSvg from '@lobehub/icons-static-svg/icons/snowflake-color.svg?raw';
import stepfunColorSvg from '@lobehub/icons-static-svg/icons/stepfun-color.svg?raw';
import submodelColorSvg from '@lobehub/icons-static-svg/icons/submodel-color.svg?raw';
import tencentcloudColorSvg from '@lobehub/icons-static-svg/icons/tencentcloud-color.svg?raw';
import togetherColorSvg from '@lobehub/icons-static-svg/icons/together-color.svg?raw';
import upstageColorSvg from '@lobehub/icons-static-svg/icons/upstage-color.svg?raw';
import v0Svg from '@lobehub/icons-static-svg/icons/v0.svg?raw';
import veniceColorSvg from '@lobehub/icons-static-svg/icons/venice-color.svg?raw';
import vercelSvg from '@lobehub/icons-static-svg/icons/vercel.svg?raw';
import vertexaiColorSvg from '@lobehub/icons-static-svg/icons/vertexai-color.svg?raw';
import xaiSvg from '@lobehub/icons-static-svg/icons/xai.svg?raw';
import zaiSvg from '@lobehub/icons-static-svg/icons/zai.svg?raw';
import zhipuColorSvg from '@lobehub/icons-static-svg/icons/zhipu-color.svg?raw';
import zenmuxSvg from '@lobehub/icons-static-svg/icons/zenmux.svg?raw';
import replitColorSvg from '@lobehub/icons-static-svg/icons/replit-color.svg?raw';

/** SVG 字符串 → data URL（base64 编码, UTF-8 安全） */
function svgToDataUrl(svg: string): string {
    const b64 = btoa(unescape(encodeURIComponent(svg)));
    return `data:image/svg+xml;base64,${b64}`;
}

const PROVIDER_LOGOS: Record<string, string> = {
    // ===== 一线大厂 =====
    openai:        svgToDataUrl(openaiSvg),
    anthropic:     svgToDataUrl(claudeColorSvg),       // Claude = Anthropic
    google:        svgToDataUrl(geminiColorSvg),
    'google-vertex': svgToDataUrl(vertexaiColorSvg),
    'google-vertex-anthropic': svgToDataUrl(claudeColorSvg),  // Vertex 上的 Claude
    deepseek:      svgToDataUrl(deepseekColorSvg),
    mistral:       svgToDataUrl(mistralColorSvg),
    xai:           svgToDataUrl(xaiSvg),                // Grok
    grok:          svgToDataUrl(grokSvg),
    cohere:        svgToDataUrl(cohereColorSvg),
    groq:          svgToDataUrl(groqSvg),
    meta:          svgToDataUrl(metaColorSvg),            // Meta / Llama
    llama:         svgToDataUrl(metaColorSvg),
    nvidia:        svgToDataUrl(nvidiaColorSvg),          // NIM
    cerebras:      svgToDataUrl(cerebrasColorSvg),
    perplexity:    svgToDataUrl(perplexityColorSvg),
    'perplexity-agent': svgToDataUrl(perplexityColorSvg),
    together:      svgToDataUrl(togetherColorSvg),
    'together-ai': svgToDataUrl(togetherColorSvg),
    fireworks:     svgToDataUrl(fireworksColorSvg),
    'fireworks-ai': svgToDataUrl(fireworksColorSvg),
    firepass:      svgToDataUrl(fireworksColorSvg),      // Fireworks Firepass 子产品
    deepinfra:     svgToDataUrl(deepinfraColorSvg),
    huggingface:   svgToDataUrl(huggingfaceColorSvg),
    replit:        svgToDataUrl(replitColorSvg),

    // ===== 中国大厂 =====
    alibaba:       svgToDataUrl(alibabaColorSvg),
    alibabacloud:  svgToDataUrl(alibabacloudColorSvg),
    'alibaba-cn': svgToDataUrl(alibabacloudColorSvg),
    'alibaba-token-plan': svgToDataUrl(alibabacloudColorSvg),
    'alibaba-token-plan-cn': svgToDataUrl(alibabacloudColorSvg),
    'alibaba-coding-plan': svgToDataUrl(alibabacloudColorSvg),
    'alibaba-coding-plan-cn': svgToDataUrl(alibabacloudColorSvg),
    qwen:          svgToDataUrl(qwenColorSvg),            // 通义千问
    qiniu:         svgToDataUrl(qiniuSvg),
    'qiniu-ai': svgToDataUrl(qiniuSvg),
    modelscope:    svgToDataUrl(modelscopeColorSvg),
    moonshot:      svgToDataUrl(moonshotSvg),            // Kimi
    kimi:          svgToDataUrl(kimiColorSvg),           // 月之暗面
    moonshotai:    svgToDataUrl(moonshotSvg),
    'moonshotai-cn': svgToDataUrl(moonshotSvg),
    'kimi-for-coding': svgToDataUrl(kimiColorSvg),
    zhipu:         svgToDataUrl(zhipuColorSvg),          // 智谱 GLM
    zhipuai:       svgToDataUrl(zhipuColorSvg),
    'zhipuai-coding-plan': svgToDataUrl(zhipuColorSvg),
    doubao:        svgToDataUrl(doubaoColorSvg),          // 字节豆包
    bytedance:     svgToDataUrl(doubaoColorSvg),         // 字节 = 豆包母公司
    stepfun:       svgToDataUrl(stepfunColorSvg),
    'stepfun-ai': svgToDataUrl(stepfunColorSvg),
    minimax:       svgToDataUrl(minimaxColorSvg),
    'minimax-cn': svgToDataUrl(minimaxColorSvg),
    'minimax-coding-plan': svgToDataUrl(minimaxColorSvg),
    'minimax-cn-coding-plan': svgToDataUrl(minimaxColorSvg),
    tencent:       svgToDataUrl(tencentcloudColorSvg),   // 腾讯云
    tencentcloud:  svgToDataUrl(tencentcloudColorSvg),
    'tencent-tokenhub': svgToDataUrl(tencentcloudColorSvg),
    'tencent-coding-plan': svgToDataUrl(tencentcloudColorSvg),
    zai:           svgToDataUrl(zaiSvg),                  // 智谱 Z.AI
    'zai-coding-plan': svgToDataUrl(zaiSvg),
    bailing:       svgToDataUrl(bailianColorSvg),         // 百聆 = 阿里 Bailian
    '302ai': svgToDataUrl(ai302ColorSvg),                // 302.AI

    // ===== 云 / 基础设施 =====
    aws:           svgToDataUrl(awsSvg),
    'amazon-bedrock': svgToDataUrl(bedrockColorSvg),
    azure:         svgToDataUrl(azureSvg),
    'azure-cognitive-services': svgToDataUrl(azureSvg),
    cloudflare:    svgToDataUrl(cloudflareColorSvg),
    'cloudflare-workers-ai': svgToDataUrl(cloudflareColorSvg),
    'cloudflare-ai-gateway': svgToDataUrl(cloudflareColorSvg),
    vercel:        svgToDataUrl(vercelSvg),
    'vercel-ai-gateway': svgToDataUrl(vercelSvg),
    snowflake:     svgToDataUrl(snowflakeColorSvg),
    'snowflake-cortex': svgToDataUrl(snowflakeColorSvg),
    github:        svgToDataUrl(githubcopilotSvg),
    'github-models': svgToDataUrl(githubcopilotSvg),
    'github-copilot': svgToDataUrl(githubcopilotSvg),
    v0:            svgToDataUrl(v0Svg),                  // Vercel v0
    lmstudio:      svgToDataUrl(lmstudioSvg),
    ollama:        svgToDataUrl(ollamaSvg),
    'ollama-cloud': svgToDataUrl(ollamaSvg),
    opencode:      svgToDataUrl(opencodeSvg),
    'opencode-go': svgToDataUrl(opencodeSvg),
    venice:        svgToDataUrl(veniceColorSvg),
    nebius:        svgToDataUrl(nebiusSvg),
    baseten:       svgToDataUrl(basetenSvg),
    inception:     svgToDataUrl(inceptionSvg),
    inference:     svgToDataUrl(inferenceSvg),
    friendli:      svgToDataUrl(friendliSvg),
    poe:           svgToDataUrl(poeColorSvg),
    upstage:       svgToDataUrl(upstageColorSvg),
    morph:         svgToDataUrl(morphColorSvg),
    submodel:      svgToDataUrl(submodelColorSvg),
    novita:        svgToDataUrl(novitaColorSvg),
    'novita-ai': svgToDataUrl(novitaColorSvg),
    zenmux:        svgToDataUrl(zenmuxSvg),
    aihubmix:      svgToDataUrl(aihubmixColorSvg),
    nova:          svgToDataUrl(novaColorSvg),

    // ===== 聚合 / 路由器 =====
    openrouter:    svgToDataUrl(openrouterSvg),

    // ===== 兜底映射 (尚未匹配的真实 logo 留作字母占位) =====
    google_brand: svgToDataUrl(googleSvg),               // 备用的 Google 文字 logo
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
    { match: n => n.includes('llama'), logo: PROVIDER_LOGOS.llama },
    { match: n => n.includes('whisper'), logo: PROVIDER_LOGOS.groq },
    { match: n => n.includes('command') || n.includes('cohere') || n.includes('embed-v'), logo: PROVIDER_LOGOS.cohere },
    { match: n => n.includes('kimi') || n.includes('moonshot'), logo: PROVIDER_LOGOS.moonshot },
    { match: n => n.includes('glm') || n.includes('chatglm') || n.includes('zhipu'), logo: PROVIDER_LOGOS.zhipu },
    { match: n => n.includes('qwen') || n.includes('qwq') || n.includes('qvq'), logo: PROVIDER_LOGOS.qwen },
    { match: n => n.includes('doubao') || n.includes('seedream') || n.includes('volcengine'), logo: PROVIDER_LOGOS.doubao },
];

/** 找不到 logo 时返回 undefined, UI 会自动渲染字母占位 */
export const FALLBACK_LOGO: string | undefined = undefined;

export function getProviderLogo(providerId: string | null | undefined): string | undefined {
    if (!providerId) return FALLBACK_LOGO;
    return PROVIDER_LOGOS[providerId.toLowerCase()] ?? FALLBACK_LOGO;
}

export function getModelLogoByName(modelId: string | null | undefined): string | undefined {
    if (!modelId) return FALLBACK_LOGO;
    const name = modelId.toLowerCase();
    for (const rule of TEXT_RULES) {
        if (rule.match(name)) return rule.logo;
    }
    return FALLBACK_LOGO;
}

export function getLogo(providerId: string | null | undefined, modelId: string | null | undefined): string | undefined {
    const fromProvider = getProviderLogo(providerId);
    if (fromProvider) return fromProvider;
    return getModelLogoByName(modelId);
}
