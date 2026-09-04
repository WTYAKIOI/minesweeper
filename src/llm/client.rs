use serde::{Deserialize, Serialize};

/// LLM API 客户端
///
/// 通过 HTTP 调用 OpenAI 兼容 API (也支持其他兼容接口如 DeepSeek 等)
#[derive(Clone)]
pub struct LLMClient {
    api_key: String,
    base_url: String,
    model: String,
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
    pub fn new(api_key: impl Into<String>, base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: base_url.into(),
            model: model.into(),
            http_client: reqwest::Client::new(),
        }
    }

    /// 从环境变量构建: OPENAI_API_KEY, OPENAI_BASE_URL, OPENAI_MODEL
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("OPENAI_API_KEY").ok()?;
        let base_url = std::env::var("OPENAI_BASE_URL")
            .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
        let model = std::env::var("OPENAI_MODEL")
            .unwrap_or_else(|_| "gpt-4o-mini".to_string());
        Some(Self::new(api_key, base_url, model))
    }

    /// 发送对话请求
    pub async fn chat(&self, system_prompt: &str, user_message: &str) -> Result<String, Box<dyn std::error::Error>> {
        let request = ChatRequest {
            model: self.model.clone(),
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

        let url = format!("{}/chat/completions", self.base_url);
        let resp = self.http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_env_no_key() {
        // Ensure no env var is set
        std::env::remove_var("OPENAI_API_KEY");
        assert!(LLMClient::from_env().is_none());
    }
}
