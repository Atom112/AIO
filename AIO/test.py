import requests
import os

# 从环境变量中读取 API Key（推荐方式）
API_KEY = "sk-KwmAR4Az6SHLAgEr19FbC79531124d449cF18b2aF35f34Ea"

if not API_KEY:
    raise ValueError("请设置环境变量 AIHUBMIX_API_KEY，例如：export AIHUBMIX_API_KEY='your-api-key'")

# AIHubMix 兼容 OpenAI 的 API 端点
url = "https://api.aihubmix.com/v1/models"

headers = {
    "Authorization": f"Bearer {API_KEY}",
    "Content-Type": "application/json"
}

try:
    response = requests.get(url, headers=headers)
    response.raise_for_status()  # 如果状态码不是 2xx，抛出异常

    data = response.json()
    models = data.get("data", [])

    print(f"✅ 成功获取 {len(models)} 个模型：\n")
    for model in sorted(models, key=lambda x: x.get("id", "")):
        model_id = model.get("id", "N/A")
        owned_by = model.get("owned_by", "Unknown")
        created = model.get("created", 0)
        print(f"- ID: {model_id:<30} | 提供方: {owned_by:<15} | 创建时间戳: {created}")

except requests.exceptions.RequestException as e:
    print(f"❌ 请求失败: {e}")
except KeyError as e:
    print(f"❌ 响应格式异常，缺少字段: {e}")
except Exception as e:
    print(f"❌ 发生未知错误: {e}")