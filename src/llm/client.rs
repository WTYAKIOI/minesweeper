use serde::{Deserialize, Serialize};

/// LLM 运行时配置 — 可从前端 UI 传入, 也可从环境变量读取
///
/// 支持任何 OpenAI 兼容 API (OpenAI / DeepSeek / Kimi / Moonshot / 本地 Ollama 等)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMConfig {
    /// API Key (Bearer token)
    pub api_key: String,
    /// API 基地址, 如 https://api.openai.com/v1
    pub base_url: String,
    /// 模型名, 如 gpt-4o-mini / deepseek-chat / moonshot-v1-8k
    pub model: String,
}

impl Default for LLMConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            base_url: "https://api.openai.com/v1".to_string(),
            model: "gpt-4o-mini".to_string(),
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
        Some(Self { api_key, base_url, model })
    }

    /// 是否看起来有效 (api_key 非空)
    pub fn is_valid(&self) -> bool {
        !self.api_key.is_empty() && !self.base_url.is_empty() && !self.model.is_empty()
    }
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
    max_tokens: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

impl LLMClient {
    pub fn new(config: LLMConfig) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
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

    /// 发送对话请求
    pub async fn chat(&self, system_prompt: &str, user_message: &str) -> Result<String, Box<dyn std::error::Error>> {
        let request = ChatRequest {
            model: self.config.model.clone(),
            messages: vec![
                ChatMessage {
                    role: "system".to_string(),
                    content: system_prompt.to_string(),
                },
                ChatMessage {
                    role: "user".to_string(),
                    content: user_message.to_string(),
                },
            ],
            temperature: 0.3,
            max_tokens: Some(2048),
        };

        let url = format!("{}/chat/completions", self.config.base_url);
        let resp = self.http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .json(&request)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("API error {}: {}", status, body).into());
        }

        let chat_resp: ChatResponse = resp.json().await?;
        chat_resp
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .ok_or("No response from API".into())
    }

    /// 测试连接 — 发送一个最小请求验证 API key / base_url / model 是否可用
    ///
    /// 返回 Ok(model_name) 成功, Err(message) 失败
    pub async fn test_connection(&self) -> Result<String, String> {
        let request = ChatRequest {
            model: self.config.model.clone(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: "ping".to_string(),
            }],
            temperature: 0.0,
            max_tokens: Some(1),
        };

        let url = format!("{}/chat/completions", self.config.base_url);
        let resp = self.http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .json(&request)
            .send()
            .await
            .map_err(|e| format!("网络错误: {}", e))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            // 提取常见错误
            let hint = if status.as_u16() == 401 {
                " — API Key 无效或已过期"
            } else if status.as_u16() == 404 {
                " — base_url 错误或模型名不存在"
            } else if status.as_u16() == 429 {
                " — 请求频率超限, 请稍后重试"
            } else {
                ""
            };
            return Err(format!("HTTP {}{}: {}", status, hint, &body[..body.len().min(200)]));
        }

        let chat_resp: ChatResponse = resp.json().await
            .map_err(|e| format!("响应解析失败: {}", e))?;

        if chat_resp.choices.is_empty() {
            return Err("API 返回空响应".to_string());
        }

        Ok(self.config.model.clone())
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
        let c = LLMConfig { api_key: "sk-x".into(), base_url: "https://a.com/v1".into(), model: "m".into() };
        assert!(c.is_valid());
        let c2 = LLMConfig::default();
        assert!(!c2.is_valid()); // empty api_key
    }

    #[test]
    fn test_config_deserialize() {
        let json = r#"{"api_key":"sk-test","base_url":"https://api.deepseek.com/v1","model":"deepseek-chat"}"#;
        let c: LLMConfig = serde_json::from_str(json).unwrap();
        assert_eq!(c.api_key, "sk-test");
        assert_eq!(c.model, "deepseek-chat");
    }
}
