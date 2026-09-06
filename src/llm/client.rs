use serde::{Deserialize, Serialize};

/// LLM 运行时配置 — 可从前端 UI 传入, 也可从环境变量读取
///
/// 支持任何 OpenAI 兼容 API (OpenAI / DeepSeek / Kimi / Moonshot / 通义千问 / 本地 Ollama 等)
///
/// 注意: api_key 仅在内存中流转 (前端 -> 请求体 -> 客户端), 后端不持久化。
/// 本地 Ollama (localhost / 127.0.0.1) 无需 api_key。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMConfig {
    /// API Key (Bearer token); 本地 Ollama 可为空
    #[serde(default)]
    pub api_key: String,
    /// API 基地址, 如 https://api.openai.com/v1
    pub base_url: String,
    /// 模型名, 如 gpt-4o-mini / deepseek-chat / moonshot-v1-8k
    pub model: String,
    /// 提供商标签 (用于用量统计分组), 如 openai / deepseek / ollama
    #[serde(default)]
    pub provider: Option<String>,
    /// 最大生成 token 数 (None = 使用默认 4096)
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// 采样温度 (None = 使用默认 0.3)
    #[serde(default)]
    pub temperature: Option<f64>,
    /// 厂商前缀策略 (适用于需要 vendor/model 的网关, 如 OpenRouter / 清华 Sub2API 等):
    ///   - None / ""     → 不补前缀
    ///   - Some("auto")  → 按模型名首段自动猜 (openai/anthropic/deepseek/...)
    ///   - Some("thu-ai")→ 使用指定前缀 (模型无 "/" 时尝试 "thu-ai/{model}")
    #[serde(default)]
    pub prefix: Option<String>,
    /// 视觉模型名 (仅截图识别 LLM OCR 使用; 留空 = 与 model 相同)。
    ///
    /// 主模型可以是纯文本模型 (如 glm-5 / deepseek-chat), 只要这里填一个
    /// 支持图像输入的模型 (如 glm-4v / gpt-4o / qwen-vl) 即可用 LLM 识别截图。
    #[serde(default)]
    pub vision_model: Option<String>,
}

/// 默认最大生成 token 数
pub const DEFAULT_MAX_TOKENS: u32 = 4096;

impl Default for LLMConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            base_url: "https://api.openai.com/v1".to_string(),
            model: "gpt-4o-mini".to_string(),
            provider: None,
            max_tokens: None,
            temperature: None,
            prefix: None,
            vision_model: None,
        }
    }
}

impl LLMConfig {
    /// 从环境变量构建: OPENAI_API_KEY, OPENAI_BASE_URL, OPENAI_MODEL
    /// 如果未设置 OPENAI_API_KEY, 返回 None
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("OPENAI_API_KEY").ok()?;
        if api_key.is_empty() {
            return None;
        }
        let base_url = std::env::var("OPENAI_BASE_URL")
            .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
        let model = std::env::var("OPENAI_MODEL")
            .unwrap_or_else(|_| "gpt-4o-mini".to_string());
        Some(Self { api_key, base_url, model, ..Default::default() })
    }

    /// 是否看起来有效
    ///
    /// api_key 非空, 或者 base_url 指向本地服务 (Ollama 等无需鉴权)
    pub fn is_valid(&self) -> bool {
        if self.base_url.is_empty() || self.model.is_empty() {
            return false;
        }
        if !self.api_key.is_empty() {
            return true;
        }
        Self::is_local_url(&self.base_url)
    }

    /// 是否为本地免鉴权地址 (Ollama / LM Studio 等)
    pub fn is_local_url(base_url: &str) -> bool {
        let u = base_url.to_lowercase();
        u.contains("//localhost") || u.contains("//127.0.0.1") || u.contains("[::1]")
    }

    /// 提供商标签: 显式配置优先, 否则从 base_url 推断
    pub fn provider_label(&self) -> String {
        if let Some(p) = &self.provider {
            if !p.is_empty() {
                return p.clone();
            }
        }
        derive_provider(&self.base_url)
    }

    /// 实际生效的最大生成 token 数
    pub fn effective_max_tokens(&self) -> u32 {
        self.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS).clamp(16, 32768)
    }

    /// 实际生效的温度
    pub fn effective_temperature(&self) -> f64 {
        self.temperature.unwrap_or(0.3).clamp(0.0, 2.0)
    }
}

/// 从 base_url 推断提供商标签 (用于用量统计分组)
pub fn derive_provider(base_url: &str) -> String {
    let u = base_url.to_lowercase();
    if u.contains("localhost") || u.contains("127.0.0.1") {
        "ollama".to_string()
    } else if u.contains("deepseek") {
        "deepseek".to_string()
    } else if u.contains("moonshot") {
        "kimi".to_string()
    } else if u.contains("dashscope") || u.contains("aliyun") {
        "qwen".to_string()
    } else if u.contains("openrouter") {
        "openrouter".to_string()
    } else if u.contains("bigmodel") || u.contains("zhipu") {
        "glm".to_string()
    } else if u.contains("openai") {
        "openai".to_string()
    } else {
        "custom".to_string()
    }
}

/// Token 用量 (来自 API 响应的 usage 字段)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    #[serde(default)]
    pub prompt_tokens: u32,
    #[serde(default)]
    pub completion_tokens: u32,
    #[serde(default)]
    pub total_tokens: u32,
}

/// 一次 LLM 调用的结果
#[derive(Debug, Clone)]
pub struct ChatResult {
    pub content: String,
    /// API 返回的 token 用量 (部分本地/兼容服务可能缺失)
    pub usage: Option<TokenUsage>,
}

/// LLM API 客户端
///
/// 通过 HTTP 调用 OpenAI 兼容 API (也支持其他兼容接口如 DeepSeek 等)
#[derive(Clone)]
pub struct LLMClient {
    config: LLMConfig,
    http_client: reqwest::Client,
}

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    temperature: f64,
    max_tokens: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ChatMessage {
    role: String,
    content: String,
    /// 思维链模型 (glm-thinking / deepseek-reasoner 等) 的推理过程字段;
    /// 仅在响应中出现, 请求时序列化会跳过
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reasoning_content: Option<String>,
}

impl ChatMessage {
    fn new(role: &str, content: impl Into<String>) -> Self {
        Self {
            role: role.to_string(),
            content: content.into(),
            reasoning_content: None,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
    #[serde(default)]
    usage: Option<TokenUsage>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

/// 上游错误信息 (OpenAI 兼容格式: {"error":{"message":..., "type":...}})
#[derive(Debug, Deserialize)]
struct ApiErrorBody {
    error: Option<ApiErrorDetail>,
}

/// OpenAI 兼容 /models 响应: { "object": "list", "data": [{"id": ...}, ...] }
#[derive(Debug, Deserialize)]
struct ModelsResponse {
    data: Option<Vec<ModelItem>>,
    /// Ollama 原生 /api/tags 形状: { "models": [{"name": ...}, ...] }
    models: Option<Vec<OllamaModelItem>>,
}

#[derive(Debug, Deserialize)]
struct ModelItem {
    id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OllamaModelItem {
    name: Option<String>,
}

impl LLMClient {
    /// 从网关获取可用模型列表 (GET {base_url}/models, OpenAI 兼容)。
    ///
    /// 部分网关 (如 Ollama) 走本地 /api/tags, 返回 {models:[{name}]},
    /// 此处兼容两种形状。返回模型 id 列表 (保留原始顺序去重)。
    pub async fn fetch_models(&self) -> Result<Vec<String>, String> {
        let base = self.config.base_url.trim_end_matches('/');
        // Ollama 原生接口: 先尝试 /api/tags
        if is_ollama_url(&self.config.base_url) {
            if let Ok(list) = self.fetch_models_url(&format!("{}/api/tags", base)).await {
                if !list.is_empty() {
                    return Ok(list);
                }
            }
        }
        self.fetch_models_url(&format!("{}/models", base)).await
    }

    async fn fetch_models_url(&self, url: &str) -> Result<Vec<String>, String> {
        let mut req = self.http_client.get(url).timeout(std::time::Duration::from_secs(30));
        if !self.config.api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", self.config.api_key));
        }
        let resp = req
            .send()
            .await
            .map_err(|e| format_network_error(&e))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(format_upstream_error(status.as_u16(), &body));
        }
        let parsed: ModelsResponse = resp
            .json()
            .await
            .map_err(|e| format!("解析模型列表失败: {}", e))?;
        let mut out: Vec<String> = Vec::new();
        if let Some(data) = parsed.data {
            for item in data {
                if let Some(id) = item.id {
                    if !id.is_empty() && !out.contains(&id) {
                        out.push(id);
                    }
                }
            }
        }
        if let Some(list) = parsed.models {
            for item in list {
                if let Some(name) = item.name {
                    let name = name.trim_end_matches(":latest").to_string();
                    if !name.is_empty() && !out.contains(&name) {
                        out.push(name);
                    }
                }
            }
        }
        if out.is_empty() {
            return Err("网关未返回可用模型 (响应中无 data/models 字段)".to_string());
        }
        Ok(out)
    }
}

/// 是否为本地 Ollama (模型列表走 /api/tags 而非 OpenAI /models)
fn is_ollama_url(base_url: &str) -> bool {
    let u = base_url.to_lowercase();
    u.contains("localhost") || u.contains("127.0.0.1") || u.contains("[::1]")
}

#[derive(Debug, Deserialize)]
struct ApiErrorDetail {
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    r#type: Option<String>,
}

/// 解析上游错误响应, 提取 message 并附加友好提示
fn format_upstream_error(status: u16, body: &str) -> String {
    let detail = serde_json::from_str::<ApiErrorBody>(body)
        .ok()
        .and_then(|b| b.error)
        .and_then(|e| e.message)
        .unwrap_or_else(|| body.to_string());

    // 常见问题 → 针对性提示
    let hint = if detail.contains("No proxy configuration found for requested model")
        || detail.contains("Model not found")
        || detail.contains("does not exist")
    {
        " — 该模型名在此网关不可用。若使用 OpenRouter / 中转网关, 模型名需带厂商前缀 (如 openai/gpt-4o-mini 而非 gpt-4o-mini), 或换成网关已配置的模型"
    } else if detail.contains("insufficient_quota")
        || detail.contains("Insufficient Balance")
        || detail.contains("quota")
    {
        " — 账户余额不足或额度已用完"
    } else if detail.contains("rate limit") || detail.contains("Too Many Requests") {
        " — 请求频率超限, 请稍后重试"
    } else if detail.contains("invalid api key")
        || detail.contains("Invalid API key")
        || detail.contains("Unauthorized")
    {
        " — API Key 无效或已过期"
    } else if detail.contains("model_not_found") {
        " — 模型名不存在, 请检查模型拼写"
    } else {
        ""
    };
    format!("HTTP {}: {}{}", status, detail.trim(), hint)
}

/// 展开 reqwest 错误的完整原因链。
///
/// reqwest 顶层 Display 只显示 "error sending request for url (...)",
/// 真实原因 (超时 / 连接重置 / TLS 握手失败) 藏在 source() 链中;
/// 不展开会导致提示语误导、且瞬态错误重试判断 (is_transient_llm_error) 失效。
fn format_network_error(e: &reqwest::Error) -> String {
    let mut parts: Vec<String> = vec![e.to_string()];
    let mut src: Option<&dyn std::error::Error> = std::error::Error::source(e);
    while let Some(s) = src {
        let msg = s.to_string();
        if !msg.is_empty() && !parts.contains(&msg) {
            parts.push(msg);
        }
        src = s.source();
    }
    let joined = parts.join(" ← ");
    let lower = joined.to_lowercase();
    let hint = if lower.contains("timed out") || lower.contains("timeout") {
        format!(
            " (请求超时; 当前超时 {} 秒, 可用环境变量 LLM_TIMEOUT_SECS 调大)",
            LLMClient::timeout_secs()
        )
    } else if lower.contains("connection reset") || lower.contains("connection closed") {
        " (连接被重置, 常见于网关拒绝过大/过慢的请求)".to_string()
    } else {
        " (请检查 Base URL 是否可达)".to_string()
    };
    format!("网络错误: {}{}", joined, hint)
}

/// SSE 流式增量累积器 (OpenAI 兼容 chunked 响应)
#[derive(Default)]
struct SseAccumulator {
    content: String,
    reasoning: String,
    usage: Option<TokenUsage>,
}

/// 解析一行 SSE 数据 (`data: {...}` / `data: [DONE]`), 累积到 acc。
/// 非法行静默忽略 (流中可能出现注释/心跳行)。
fn feed_sse_line(line: &str, acc: &mut SseAccumulator) {
    let line = line.trim();
    let Some(data) = line.strip_prefix("data:") else {
        return;
    };
    let data = data.trim();
    if data.is_empty() || data == "[DONE]" {
        return;
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(data) else {
        return;
    };
    // 最终 chunk 可能携带 usage (需 stream_options.include_usage)
    if let Some(u) = v.get("usage").filter(|u| !u.is_null()) {
        if let Ok(parsed) = serde_json::from_value::<TokenUsage>(u.clone()) {
            acc.usage = Some(parsed);
        }
    }
    let Some(delta) = v.pointer("/choices/0/delta") else {
        return;
    };
    if let Some(c) = delta.get("content").and_then(|c| c.as_str()) {
        acc.content.push_str(c);
    }
    // 思维链模型: 流式推理过程字段
    if let Some(r) = delta.get("reasoning_content").and_then(|c| c.as_str()) {
        acc.reasoning.push_str(r);
    }
}

/// 是否为 OpenRouter (或基于其协议的中转网关)
fn is_openrouter_url(base_url: &str) -> bool {
    base_url.to_lowercase().contains("openrouter")
}

/// 常见模型族的厂商前缀猜测 (用于 OpenRouter 自动重试)
fn guess_vendor_prefix(model: &str) -> Option<&'static str> {
    let head = model.split([':', '-', '/']).next()?.to_lowercase();
    match head.as_str() {
        "gpt" | "o1" | "o3" | "o4" | "o5" | "chatgpt" => Some("openai"),
        "claude" => Some("anthropic"),
        "gemini" => Some("google"),
        "llama" => Some("meta-llama"),
        "deepseek" => Some("deepseek"),
        "qwen" => Some("qwen"),
        "glm" => Some("zhipu"),
        "mistral" => Some("mistralai"),
        "grok" => Some("x-ai"),
        "command" => Some("cohere"),
        _ => None,
    }
}

/// 判断错误是否与"模型名不可用"相关 (决定是否值得自动重试)
fn is_model_unavailable_error(msg: &str) -> bool {
    let m = msg.to_lowercase();
    m.contains("no proxy configuration found for requested model")
        || m.contains("model not found")
        || m.contains("does not exist")
        || (m.contains("model") && m.contains("not"))
        || m.contains("model_not_found")
        || m.contains("no endpoints found")
}

/// 判断错误是否为"模型/网关不支持图像输入" (纯文本模型收到 image_url)。
///
/// 典型上游报错:
///   - "messages.content.type 参数非法，取值范围 ['text']" (清华 Sub2API / 部分网关)
///   - "Invalid type for 'messages[1].content[1].type': 'image_url' is not allowed"
pub fn is_vision_unsupported_error(msg: &str) -> bool {
    let m = msg.to_lowercase();
    m.contains("content.type")
        || m.contains("content[1].type")
        || m.contains("image_url")
        || m.contains("multimodal")
        || m.contains("multi-modal")
        || m.contains("不支持图像")
        || m.contains("仅支持文本")
        || (m.contains("image") && (m.contains("not support") || m.contains("unsupported")))
}

/// 从 ChatResponse 提取首个消息内容 (兼容思维链模型回退 reasoning_content)
fn extract_chat_result(chat_resp: ChatResponse) -> Result<ChatResult, String> {
    let usage = chat_resp.usage;
    let msg = chat_resp
        .choices
        .into_iter()
        .next()
        .map(|c| c.message)
        .ok_or("API 返回空响应 (无 choices)")?;
    // 思维链模型兼容: content 为空时回退到 reasoning_content
    let content = if msg.content.trim().is_empty() {
        msg.reasoning_content.unwrap_or_default().trim().to_string()
    } else {
        msg.content
    };
    if content.trim().is_empty() {
        return Err("API 返回空内容 (content 与 reasoning_content 均为空)".to_string());
    }
    Ok(ChatResult { content, usage })
}

impl LLMClient {
    /// LLM HTTP 超时 (秒): 环境变量 LLM_TIMEOUT_SECS, 默认 120。
    /// 校园网关/思维链模型处理长提示较慢, 60s 内可能出不完结果
    /// (超时重试会叠加, 默认值兼顾首次成功率与总等待时长)
    pub fn timeout_secs() -> u64 {
        std::env::var("LLM_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|v| *v >= 10)
            .unwrap_or(120)
    }

    pub fn new(config: LLMConfig) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(Self::timeout_secs()))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { config, http_client }
    }

    /// 从环境变量构建 (向后兼容)
    pub fn from_env() -> Option<Self> {
        LLMConfig::from_env().map(Self::new)
    }

    /// 获取当前配置
    pub fn config(&self) -> &LLMConfig {
        &self.config
    }

    /// 发送一次请求 (不自动重试), 消息体为任意 JSON (支持多模态 content 数组)
    async fn post_request(&self, request: serde_json::Value) -> Result<ChatResponse, String> {
        // LLM_DEBUG=1 时打印请求体大小 (排查网关请求体过大 / 413 / 超时问题)
        if std::env::var("LLM_DEBUG").map(|v| v == "1").unwrap_or(false) {
            let model = request.get("model").and_then(|m| m.as_str()).unwrap_or("?");
            let size = serde_json::to_string(&request).map(|s| s.len()).unwrap_or(0);
            eprintln!("[LLM Request] model={} body={} bytes", model, size);
        }
        // 去掉 base_url 末尾多余的 "/", 再拼接 /chat/completions
        let url = format!("{}/chat/completions", self.config.base_url.trim_end_matches('/'));
        let resp = self
            .http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .json(&request)
            .send()
            .await
            .map_err(|e| format_network_error(&e))?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        // teachmod.md 排查第一步: LLM_DEBUG=1 时打印上游原始响应
        if std::env::var("LLM_DEBUG").map(|v| v == "1").unwrap_or(false) {
            let shown: String = text.chars().take(4000).collect();
            eprintln!("[LLM Raw Response] (HTTP {}):\n{}", status.as_u16(), shown);
            if text.chars().count() > 4000 {
                eprintln!("[LLM Raw Response] ... (截断, 共 {} 字符)", text.chars().count());
            }
        }
        if !status.is_success() {
            return Err(format_upstream_error(status.as_u16(), &text));
        }

        let parsed: ChatResponse = serde_json::from_str(&text)
            .map_err(|e| format!("响应解析失败: {} (原始响应: {})", e, &text[..text.len().min(200)]))?;
        Ok(parsed)
    }

    /// 发送一次纯文本请求 (不自动重试)
    async fn post_once(
        &self,
        model: &str,
        messages: Vec<ChatMessage>,
        temperature: f64,
        max_tokens: u32,
    ) -> Result<ChatResponse, String> {
        let request = serde_json::to_value(ChatRequest {
            model: model.to_string(),
            messages,
            temperature,
            max_tokens,
        })
        .map_err(|e| format!("构建请求失败: {}", e))?;
        self.post_request(request).await
    }

    /// 按模型候选列表依次尝试发请求; 仅"模型名不可用"错误才换前缀重试
    async fn post_with_model_candidates(
        &self,
        build_request: impl Fn(&str) -> serde_json::Value,
    ) -> Result<ChatResult, String> {
        let candidates = self.model_candidates();
        self.post_candidates(&candidates, build_request).await
    }

    /// 是否启用流式请求 (默认开启; LLM_STREAM=0 关闭)。
    ///
    /// 网关 nginx 通常有 60s 级空闲超时 (proxy_read_timeout), 非流式请求
    /// 在模型生成期间无数据流动, 会被网关切断返回 504 — 后端调大超时无用。
    /// 流式下生成内容以 chunk 持续到达, 网关不会判空闲。
    pub fn stream_enabled() -> bool {
        std::env::var("LLM_STREAM").map(|v| v != "0").unwrap_or(true)
    }

    /// 发送聊天请求 (chat / 视觉共用入口): 优先流式, 网关不支持时逐级回退。
    async fn send_chat_request(&self, request: serde_json::Value) -> Result<ChatResult, String> {
        if !Self::stream_enabled() {
            return self
                .post_request(request)
                .await
                .and_then(extract_chat_result);
        }
        // 1) stream + stream_options (OpenAI 协议, 附带 usage 统计)
        let mut req = request.clone();
        req["stream"] = serde_json::Value::Bool(true);
        req["stream_options"] = serde_json::json!({ "include_usage": true });
        match self.send_streaming(req).await {
            Ok(r) => return Ok(r),
            Err(e1) => {
                // 2) 网关不认识 stream_options → 去掉后重试
                if e1.to_lowercase().contains("stream_options") {
                    let mut req = request.clone();
                    req["stream"] = serde_json::Value::Bool(true);
                    match self.send_streaming(req).await {
                        Ok(r) => return Ok(r),
                        Err(e2) => {
                            // 3) 网关完全不支持流式 → 回退普通请求
                            let m = e2.to_lowercase();
                            if m.contains("stream") && (m.contains("http 4") || m.contains("invalid")) {
                                return self
                                    .post_request(request)
                                    .await
                                    .and_then(extract_chat_result);
                            }
                            return Err(e2);
                        }
                    }
                }
                return Err(e1);
            }
        }
    }

    /// 发送一次流式请求并解析 SSE 响应。
    /// 网关忽略 stream 参数返回普通 JSON 时 (content-type 非 event-stream),
    /// 自动按非流式响应解析, 兼容所有 OpenAI 兼容网关。
    async fn send_streaming(&self, request: serde_json::Value) -> Result<ChatResult, String> {
        let url = format!("{}/chat/completions", self.config.base_url.trim_end_matches('/'));
        let resp = self
            .http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .json(&request)
            .send()
            .await
            .map_err(|e| format_network_error(&e))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(format_upstream_error(status.as_u16(), &body));
        }
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_lowercase();

        let mut resp = resp;
        // 网关未走流式 (忽略 stream 参数) → 整体按普通 JSON 解析
        if !content_type.contains("text/event-stream") {
            let text = resp.text().await.unwrap_or_default();
            let parsed: ChatResponse = serde_json::from_str(&text).map_err(|e| {
                format!("响应解析失败: {} (原始响应: {})", e, &text[..text.len().min(200)])
            })?;
            return extract_chat_result(parsed);
        }

        // SSE: 逐 chunk 读取, 按行喂给累积器 (Response::chunk 无需额外依赖)
        let mut acc = SseAccumulator::default();
        let mut buf: Vec<u8> = Vec::new();
        while let Ok(Some(chunk)) = resp.chunk().await {
            buf.extend_from_slice(&chunk);
            while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                let line = String::from_utf8_lossy(&buf[..pos]).into_owned();
                buf.drain(..=pos);
                feed_sse_line(&line, &mut acc);
            }
        }
        if !buf.is_empty() {
            let tail = String::from_utf8_lossy(&buf).into_owned();
            feed_sse_line(&tail, &mut acc);
        }

        let debug = std::env::var("LLM_DEBUG").map(|v| v == "1").unwrap_or(false);
        if debug {
            eprintln!(
                "[LLM Stream] content={} chars, reasoning={} chars, usage={:?}",
                acc.content.chars().count(),
                acc.reasoning.chars().count(),
                acc.usage
            );
        }

        // 思维链模型兼容: content 为空时回退到 reasoning_content
        let content = if acc.content.trim().is_empty() {
            acc.reasoning.trim().to_string()
        } else {
            acc.content
        };
        if content.trim().is_empty() {
            return Err("API 返回空内容 (流式累计 content 与 reasoning 均为空)".to_string());
        }
        Ok(ChatResult { content, usage: acc.usage })
    }

    /// 按给定候选列表依次尝试发请求 (主模型 / 视觉模型共用)
    async fn post_candidates(
        &self,
        candidates: &[String],
        build_request: impl Fn(&str) -> serde_json::Value,
    ) -> Result<ChatResult, String> {
        if candidates.is_empty() {
            return Err("模型名不能为空".to_string());
        }
        let mut last_err = String::new();
        for model in candidates {
            match self.send_chat_request(build_request(model)).await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    last_err = e.clone();
                    // 只有模型名不可用类的错误才值得换前缀重试
                    if !is_model_unavailable_error(&e) {
                        break;
                    }
                }
            }
        }
        Err(last_err)
    }

    /// 是否 OpenRouter 或其协议的中转网关:
    /// - base_url 含 openrouter, 或
    /// - 显式配置了 provider=openrouter (中转域名可能不含 "openrouter")
    fn is_openrouter_gateway(&self) -> bool {
        is_openrouter_url(&self.config.base_url)
            || self.config.provider.as_deref() == Some("openrouter")
    }

    /// 需尝试的模型候选列表。
    ///
    /// 需要厂商前缀的网关 (OpenRouter / 清华 Sub2API 等中转): 模型名须为
    /// vendor/model。若用户填的是无前缀模型 (如 gpt-4o-mini) 且首个请求
    /// 失败, 按以下规则补全重试:
    ///   1. config.prefix 显式指定 (如 "thu-ai") → 使用该前缀
    ///   2. config.prefix = "auto" 或 OpenRouter 类网关 → 按模型名首段自动猜
    fn model_candidates(&self) -> Vec<String> {
        self.candidates_for(&self.config.model)
    }

    /// 任意模型名的候选列表 (含厂商前缀补全), 供主模型与视觉模型共用
    fn candidates_for(&self, model: &str) -> Vec<String> {
        let model = model.trim().to_string();
        if model.is_empty() {
            return Vec::new();
        }
        if model.contains('/') {
            return vec![model];
        }
        let prefix_mode = self.config.prefix.as_deref().unwrap_or("");
        let vendor: Option<&str> = if !prefix_mode.is_empty() && prefix_mode != "auto" {
            Some(prefix_mode)
        } else if prefix_mode == "auto" || self.is_openrouter_gateway() {
            guess_vendor_prefix(&model)
        } else {
            None
        };
        let mut out = vec![model.clone()];
        if let Some(v) = vendor {
            if !v.is_empty() {
                out.push(format!("{}/{}", v, model));
            }
        }
        out
    }

    /// 截图识别实际使用的模型: vision_model 优先 (须支持图像输入), 未配置时与主模型相同
    pub fn effective_vision_model(&self) -> &str {
        self.config
            .vision_model
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .unwrap_or(&self.config.model)
    }

    /// 发送对话请求 (带 OpenRouter 厂商前缀自动重试), 返回内容与 token 用量
    pub async fn chat(&self, system_prompt: &str, user_message: &str) -> Result<ChatResult, Box<dyn std::error::Error>> {
        let messages = vec![
            ChatMessage::new("system", system_prompt),
            ChatMessage::new("user", user_message),
        ];
        let temperature = self.config.effective_temperature();
        let max_tokens = self.config.effective_max_tokens();

        let result = self
            .post_with_model_candidates(|model| {
                serde_json::to_value(ChatRequest {
                    model: model.to_string(),
                    messages: messages.clone(),
                    temperature,
                    max_tokens,
                })
                .unwrap_or_default()
            })
            .await?;
        Ok(result)
    }

    /// 发送多模态请求 (文本 + 图像), 用于截图棋盘识别等视觉任务。
    ///
    /// `image_data_url` 为 data URL 形式 (`data:image/png;base64,....`),
    /// 兼容 OpenAI vision 协议 (image_url content part) 及其兼容网关。
    /// 模型取 effective_vision_model (vision_model 优先, 未配置时与主模型相同),
    /// 带厂商前缀重试, 语义与 chat 一致。
    ///
    /// 注意: 主模型为纯文本模型 (网关报 "content.type 取值范围 ['text']")
    /// 时不支持图像输入, 由调用方按 is_vision_unsupported_error 识别并引导。
    pub async fn chat_with_image(
        &self,
        system_prompt: &str,
        user_text: &str,
        image_data_url: &str,
    ) -> Result<ChatResult, Box<dyn std::error::Error>> {
        let temperature = self.config.effective_temperature();
        let max_tokens = self.config.effective_max_tokens();
        let candidates = self.candidates_for(self.effective_vision_model());

        let result = self
            .post_candidates(&candidates, |model| {
                serde_json::json!({
                    "model": model,
                    "messages": [
                        { "role": "system", "content": system_prompt },
                        { "role": "user", "content": [
                            { "type": "text", "text": user_text },
                            { "type": "image_url", "image_url": { "url": image_data_url } }
                        ]}
                    ],
                    "temperature": temperature,
                    "max_tokens": max_tokens
                })
            })
            .await?;
        Ok(result)
    }

    /// 发送任意多轮消息 (Agent 循环等场景)。
    ///
    /// `messages`: [{role: "system"|"user"|"assistant", content: String}, ...]。
    /// 自动带厂商前缀重试 + 流式 (LLM_STREAM=0 可关), 语义与 chat 一致。
    pub async fn chat_messages(
        &self,
        messages: Vec<serde_json::Value>,
    ) -> Result<ChatResult, Box<dyn std::error::Error>> {
        let temperature = self.config.effective_temperature();
        let max_tokens = self.config.effective_max_tokens();
        let candidates = self.model_candidates();

        let result = self
            .post_candidates(&candidates, |model| {
                serde_json::json!({
                    "model": model,
                    "messages": messages,
                    "temperature": temperature,
                    "max_tokens": max_tokens
                })
            })
            .await?;
        Ok(result)
    }

    /// 测试连接 — 发送一个最小请求验证 API key / base_url / model 是否可用
    ///
    /// 返回 Ok(model_name) 成功, Err(message) 失败
    pub async fn test_connection(&self) -> Result<String, String> {
        let messages = vec![ChatMessage::new("user", "ping")];
        let mut last_err = String::new();
        for model in self.model_candidates() {
            match self.post_once(&model, messages.clone(), 0.0, 1).await {
                Ok(chat_resp) => {
                    if chat_resp.choices.is_empty() {
                        return Err("API 返回空响应".to_string());
                    }
                    return Ok(model);
                }
                Err(e) => {
                    last_err = e.clone();
                    if !is_model_unavailable_error(&e) {
                        break;
                    }
                }
            }
        }
        Err(last_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_env_no_key() {
        std::env::remove_var("OPENAI_API_KEY");
        assert!(LLMConfig::from_env().is_none());
    }

    #[test]
    fn test_config_valid() {
        let c = LLMConfig { api_key: "sk-x".into(), base_url: "https://a.com/v1".into(), model: "m".into(), ..Default::default() };
        assert!(c.is_valid());
        let c2 = LLMConfig::default();
        assert!(!c2.is_valid()); // empty api_key + 非本地
        // 本地 Ollama 无需 key
        let c3 = LLMConfig { api_key: String::new(), base_url: "http://localhost:11434/v1".into(), model: "qwen2.5:7b".into(), ..Default::default() };
        assert!(c3.is_valid());
        assert_eq!(c3.provider_label(), "ollama");
    }

    #[test]
    fn test_config_deserialize_backward_compatible() {
        // 旧版配置 (无新字段) 仍可反序列化
        let json = r#"{"api_key":"sk-test","base_url":"https://api.deepseek.com/v1","model":"deepseek-chat"}"#;
        let c: LLMConfig = serde_json::from_str(json).unwrap();
        assert_eq!(c.api_key, "sk-test");
        assert_eq!(c.model, "deepseek-chat");
        assert_eq!(c.effective_max_tokens(), 4096);
        assert!((c.effective_temperature() - 0.3).abs() < 1e-9);
        // 新字段生效
        let json2 = r#"{"api_key":"k","base_url":"https://x/v1","model":"m","max_tokens":4096,"temperature":0.7,"provider":"custom"}"#;
        let c2: LLMConfig = serde_json::from_str(json2).unwrap();
        assert_eq!(c2.effective_max_tokens(), 4096);
        assert_eq!(c2.provider_label(), "custom");
    }

    #[test]
    fn test_vision_model_config() {
        // 旧版配置 (无 vision_model 字段) 反序列化 → None
        let c: LLMConfig = serde_json::from_str(
            r#"{"api_key":"k","base_url":"https://x/v1","model":"glm-5"}"#,
        )
        .unwrap();
        assert!(c.vision_model.is_none());
        assert_eq!(c.model, "glm-5");

        // 视觉模型生效: 未配置时与主模型相同, 配置后优先
        let client = LLMClient::new(c.clone());
        assert_eq!(client.effective_vision_model(), "glm-5");
        assert_eq!(client.candidates_for("glm-5"), vec!["glm-5".to_string()]);

        let c2 = LLMConfig {
            vision_model: Some("glm-4v".into()),
            prefix: Some("thu-ai".into()),
            ..c.clone()
        };
        let client2 = LLMClient::new(c2);
        assert_eq!(client2.effective_vision_model(), "glm-4v");
        // 视觉模型同样走厂商前缀补全
        assert_eq!(
            client2.candidates_for(client2.effective_vision_model()),
            vec!["glm-4v".to_string(), "thu-ai/glm-4v".to_string()]
        );

        // vision_model 为空白串 → 视为未配置
        let c3 = LLMConfig { vision_model: Some("  ".into()), ..c };
        assert_eq!(LLMClient::new(c3).effective_vision_model(), "glm-5");
    }

    #[test]
    fn test_is_vision_unsupported_error() {
        // 用户实测的清华 Sub2API 报错
        assert!(is_vision_unsupported_error(
            "HTTP 400: messages.content.type 参数非法，取值范围 ['text']"
        ));
        // OpenAI 风格报错
        assert!(is_vision_unsupported_error(
            "Invalid type for 'messages[1].content[1].type': expected one of 'text'"
        ));
        assert!(is_vision_unsupported_error("image_url is not allowed"));
        assert!(is_vision_unsupported_error("该模型不支持图像输入"));
        // 无关错误不误判
        assert!(!is_vision_unsupported_error("Insufficient Balance"));
        assert!(!is_vision_unsupported_error("HTTP 504: Gateway Time-out"));
        assert!(!is_vision_unsupported_error("Invalid API key"));
    }

    #[test]
    fn test_feed_sse_line() {
        let mut acc = SseAccumulator::default();
        // 正常内容增量
        feed_sse_line(r#"data: {"choices":[{"delta":{"content":"你好"}}]}"#, &mut acc);
        feed_sse_line(r#"data: {"choices":[{"delta":{"content":"，扫雷"}}]}"#, &mut acc);
        // 思维链增量
        feed_sse_line(
            r#"data: {"choices":[{"delta":{"reasoning_content":"先分析数字..."}}]}"#,
            &mut acc,
        );
        // usage (最终 chunk, 需 stream_options.include_usage)
        feed_sse_line(
            r#"data: {"choices":[{"delta":{}}],"usage":{"prompt_tokens":100,"completion_tokens":20,"total_tokens":120}}"#,
            &mut acc,
        );
        // 结束标记 / 注释 / 垃圾行 → 忽略
        feed_sse_line("data: [DONE]", &mut acc);
        feed_sse_line(": keep-alive", &mut acc);
        feed_sse_line("event: message", &mut acc);
        feed_sse_line("data: not-json", &mut acc);

        assert_eq!(acc.content, "你好，扫雷");
        assert_eq!(acc.reasoning, "先分析数字...");
        let u = acc.usage.expect("usage parsed");
        assert_eq!(u.total_tokens, 120);
        assert_eq!(u.prompt_tokens, 100);
    }

    #[test]
    fn test_chat_response_parse_usage() {
        let json = r#"{
            "choices": [{"message": {"role":"assistant","content":"hi"}}],
            "usage": {"prompt_tokens": 120, "completion_tokens": 45, "total_tokens": 165}
        }"#;
        let resp: ChatResponse = serde_json::from_str(json).unwrap();
        let u = resp.usage.expect("usage should parse");
        assert_eq!(u.total_tokens, 165);
        assert_eq!(u.prompt_tokens, 120);
        // usage 缺失也能解析
        let json2 = r#"{"choices":[{"message":{"role":"assistant","content":"hi"}}]}"#;
        let resp2: ChatResponse = serde_json::from_str(json2).unwrap();
        assert!(resp2.usage.is_none());
    }

    #[test]
    fn test_derive_provider() {
        assert_eq!(derive_provider("https://api.openai.com/v1"), "openai");
        assert_eq!(derive_provider("https://api.deepseek.com/v1"), "deepseek");
        assert_eq!(derive_provider("https://api.moonshot.cn/v1"), "kimi");
        assert_eq!(derive_provider("https://dashscope.aliyuncs.com/compatible-mode/v1"), "qwen");
        assert_eq!(derive_provider("http://localhost:11434/v1"), "ollama");
        assert_eq!(derive_provider("https://example.com/v1"), "custom");
    }

    #[test]
    fn test_format_upstream_error() {
        // OpenRouter / 中转网关典型错误 → 解析 message 并附厂商前缀提示
        let e = format_upstream_error(
            500,
            r#"{"error":{"type":"api_error","message":"No proxy configuration found for requested model"}}"#,
        );
        assert!(e.contains("No proxy configuration found for requested model"));
        assert!(e.contains("厂商前缀"));
        assert!(e.contains("openai/gpt-4o-mini"));

        // 余额不足提示
        let e2 = format_upstream_error(402, r#"{"error":{"message":"Insufficient Balance"}}"#);
        assert!(e2.contains("余额不足"));

        // 非 JSON 体也能展示
        let e3 = format_upstream_error(502, "Bad Gateway");
        assert!(e3.contains("Bad Gateway"));
    }

    #[test]
    fn test_is_model_unavailable_error() {
        assert!(is_model_unavailable_error("No proxy configuration found for requested model"));
        assert!(is_model_unavailable_error("Model not found"));
        assert!(is_model_unavailable_error("The model `xxx` does not exist"));
        assert!(!is_model_unavailable_error("Insufficient Balance"));
        assert!(!is_model_unavailable_error("Invalid API key"));
    }

    #[test]
    fn test_guess_vendor_prefix() {
        assert_eq!(guess_vendor_prefix("gpt-4o-mini"), Some("openai"));
        assert_eq!(guess_vendor_prefix("claude-3-5-sonnet"), Some("anthropic"));
        assert_eq!(guess_vendor_prefix("deepseek-chat"), Some("deepseek"));
        assert_eq!(guess_vendor_prefix("llama-3.3-70b"), Some("meta-llama"));
        assert_eq!(guess_vendor_prefix("gemini-2.0-flash"), Some("google"));
        assert_eq!(guess_vendor_prefix("unknown-model"), None);
    }

    #[test]
    fn test_model_candidates() {
        let base = LLMConfig {
            api_key: "sk-x".into(),
            base_url: "https://openrouter.ai/api/v1".into(),
            model: "gpt-4o-mini".into(),
            ..Default::default()
        };
        // OpenRouter + 无前缀 → 原样 + 厂商前缀补全
        let client = LLMClient::new(base.clone());
        assert_eq!(
            client.model_candidates(),
            vec!["gpt-4o-mini".to_string(), "openai/gpt-4o-mini".to_string()]
        );

        // 已带前缀 → 只试原样
        let cfg2 = LLMConfig { model: "anthropic/claude-3-5-sonnet".into(), ..base.clone() };
        let client2 = LLMClient::new(cfg2);
        assert_eq!(client2.model_candidates(), vec!["anthropic/claude-3-5-sonnet".to_string()]);

        // 非 OpenRouter (如 OpenAI 官方) → 不补前缀, 避免改成错误模型名
        let cfg3 = LLMConfig { base_url: "https://api.openai.com/v1".into(), ..base.clone() };
        let client3 = LLMClient::new(cfg3);
        assert_eq!(client3.model_candidates(), vec!["gpt-4o-mini".to_string()]);

        // 中转域名不含 openrouter 但显式标 provider=openrouter → 同样补全
        let cfg4 = LLMConfig {
            base_url: "https://gateway.example.com/v1".into(),
            provider: Some("openrouter".into()),
            ..base.clone()
        };
        let client4 = LLMClient::new(cfg4);
        assert_eq!(
            client4.model_candidates(),
            vec!["gpt-4o-mini".to_string(), "openai/gpt-4o-mini".to_string()]
        );

        // 任意网关 + 显式前缀 (如清华 Sub2API 的 thu-ai) → 自动补全该前缀
        let cfg5 = LLMConfig {
            base_url: "https://sub2api.example.edu.cn/v1".into(),
            model: "glm-5".into(),
            prefix: Some("thu-ai".into()),
            ..base.clone()
        };
        let client5 = LLMClient::new(cfg5);
        assert_eq!(
            client5.model_candidates(),
            vec!["glm-5".to_string(), "thu-ai/glm-5".to_string()]
        );

        // 模型名已带 "/" → 不重复补全
        let cfg6 = LLMConfig {
            base_url: "https://sub2api.example.edu.cn/v1".into(),
            model: "thu-ai/glm-5".into(),
            prefix: Some("thu-ai".into()),
            ..base.clone()
        };
        let client6 = LLMClient::new(cfg6);
        assert_eq!(client6.model_candidates(), vec!["thu-ai/glm-5".to_string()]);

        // prefix 未配置且非 openrouter → 不补全
        let cfg7 = LLMConfig {
            base_url: "https://api.deepseek.com/v1".into(),
            model: "deepseek-chat".into(),
            ..base.clone()
        };
        let client7 = LLMClient::new(cfg7);
        assert_eq!(client7.model_candidates(), vec!["deepseek-chat".to_string()]);
    }
}
