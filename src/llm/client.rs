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
            .map_err(|e| format!("网络错误: {} (请检查 Base URL 是否可达)", e))?;
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

impl LLMClient {
    /// LLM HTTP 超时 (秒): 环境变量 LLM_TIMEOUT_SECS, 默认 60
    /// (60s 内大多数模型可完成回答; 超时重试会叠加, 默认值不宜过大)
    pub fn timeout_secs() -> u64 {
        std::env::var("LLM_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|v| *v >= 10)
            .unwrap_or(60)
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

    /// 发送一次请求 (不自动重试)
    async fn post_once(
        &self,
        model: &str,
        messages: Vec<ChatMessage>,
        temperature: f64,
        max_tokens: u32,
    ) -> Result<ChatResponse, String> {
        let request = ChatRequest {
            model: model.to_string(),
            messages,
            temperature,
            max_tokens,
        };

        // 去掉 base_url 末尾多余的 "/", 再拼接 /chat/completions
        let url = format!("{}/chat/completions", self.config.base_url.trim_end_matches('/'));
        let resp = self
            .http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .json(&request)
            .send()
            .await
            .map_err(|e| format!("网络错误: {} (请检查 Base URL 是否可达)", e))?;

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
        let model = self.config.model.trim().to_string();
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

    /// 发送对话请求 (带 OpenRouter 厂商前缀自动重试), 返回内容与 token 用量
    pub async fn chat(&self, system_prompt: &str, user_message: &str) -> Result<ChatResult, Box<dyn std::error::Error>> {
        let messages = vec![
            ChatMessage::new("system", system_prompt),
            ChatMessage::new("user", user_message),
        ];
        let temperature = self.config.effective_temperature();
        let max_tokens = self.config.effective_max_tokens();

        let mut last_err = String::new();
        for model in self.model_candidates() {
            match self.post_once(&model, messages.clone(), temperature, max_tokens).await {
                Ok(chat_resp) => {
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
                        return Err("API 返回空内容 (content 与 reasoning_content 均为空)".into());
                    }
                    return Ok(ChatResult { content, usage: chat_resp.usage });
                }
                Err(e) => {
                    last_err = e.clone();
                    // 只有模型名不可用类的错误才值得换前缀重试
                    if !is_model_unavailable_error(&e) {
                        break;
                    }
                }
            }
        }
        Err(last_err.into())
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
