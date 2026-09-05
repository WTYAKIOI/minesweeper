use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tower_http::services::ServeDir;

use crate::engine::{DeterministicEngine, ProbabilityEngine, RegionAnalyzer};
use crate::llm::{LLMClient, LLMConfig, LLMMode, TokenUsage, Translator, UsageStore};
use crate::model::{InferenceIR, PlayerView};

/// 应用状态
#[derive(Clone)]
pub struct AppState {
    pub llm_client: Option<LLMClient>,
    pub mc_iterations: usize,
    /// OCR 微服务地址
    pub ocr_url: String,
    /// Token 用量存储 (JSONL 持久化 + 内存聚合)
    pub usage_store: Arc<UsageStore>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            llm_client: LLMClient::from_env(),
            mc_iterations: 0, // 0 = 动态自适应
            ocr_url: std::env::var("OCR_URL")
                .unwrap_or_else(|_| "http://localhost:5001".to_string()),
            usage_store: Arc::new(UsageStore::open()),
        }
    }
}

/// 分析请求
#[derive(Debug, Deserialize)]
pub struct AnalyzeRequest {
    pub board: Vec<Vec<i32>>,
    pub remaining_mines: u32,
    /// 可选: LLM 模式 (answer/teaching/strategy), 不传则只返回本地分析
    pub mode: Option<String>,
    /// 是否调用 LLM (默认 false, 返回本地转译结果)
    pub use_llm: Option<bool>,
    /// 可选: 运行时 LLM 配置 (覆盖环境变量)
    /// 前端 UI 传入, 不依赖 docker env
    pub llm_config: Option<LLMConfig>,
}

/// 分析响应
#[derive(Debug, Serialize)]
pub struct AnalyzeResponse {
    pub success: bool,
    pub view: PlayerView,
    pub ir: InferenceIR,
    pub analysis: String,
    pub used_llm: bool,
    /// 本次 LLM 调用的 token 用量 (未调用 LLM 或 API 未返回时为 null)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
}

/// OCR 请求 (base64 编码图像)
#[derive(Debug, Deserialize)]
pub struct OcrRequest {
    pub image: String,
}

/// OCR 响应 (来自 Python 微服务)
#[derive(Debug, Deserialize, Serialize)]
pub struct OcrResponse {
    pub success: bool,
    pub board: Vec<Vec<i32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining_mines: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// LLM 测试连接请求
#[derive(Debug, Deserialize)]
pub struct LLMTestRequest {
    pub llm_config: LLMConfig,
}

/// LLM 测试连接响应
#[derive(Debug, Serialize)]
pub struct LLMTestResponse {
    pub success: bool,
    pub model: Option<String>,
    pub error: Option<String>,
}

/// LLM 配置状态查询响应 (不泄露 api_key)
#[derive(Debug, Serialize)]
pub struct LLMStatusResponse {
    /// 环境变量是否已配置 LLM
    pub env_configured: bool,
    pub env_model: Option<String>,
    pub env_base_url: Option<String>,
}

/// 用量日志查询参数
#[derive(Debug, Deserialize)]
pub struct UsageLogsQuery {
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

/// 创建路由
pub fn create_router(state: AppState) -> Router {
    Router::new()
        .route("/api/analyze", post(analyze))
        .route("/api/ocr", post(ocr))
        .route("/api/llm/test", post(test_llm))
        .route("/api/llm/status", get(llm_status))
        .route("/api/usage/stats", get(usage_stats))
        .route("/api/usage/logs", get(usage_logs))
        .route("/api/health", get(health))
        .with_state(Arc::new(state))
        .fallback_service(ServeDir::new("static").fallback(ServeDir::new("static/index.html")))
}

async fn health() -> &'static str {
    "ok"
}

async fn analyze(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AnalyzeRequest>,
) -> Result<Json<AnalyzeResponse>, (StatusCode, String)> {
    let view = PlayerView::from_2d(&req.board, req.remaining_mines);

    if !view.validate() {
        return Err((
            StatusCode::BAD_REQUEST,
            "棋盘不合法: 某数字周围的已标旗数超过该数字".to_string(),
        ));
    }

    // 运行确定性推理 (含子集规则)
    let deterministic = DeterministicEngine::solve_with_subset_rule(&view);

    // 运行蒙特卡洛模拟 (考虑已知结论)
    let known_mines: std::collections::HashSet<_> = deterministic
        .iter()
        .filter(|p| p.conclusion.is_mine)
        .map(|p| p.conclusion.coord)
        .collect();
    let known_safe: std::collections::HashSet<_> = deterministic
        .iter()
        .filter(|p| !p.conclusion.is_mine)
        .map(|p| p.conclusion.coord)
        .collect();

    let probabilities = ProbabilityEngine::compute(&view, &known_mines, &known_safe);

    // 区域分析
    let regions = RegionAnalyzer::analyze(&view, &probabilities);

    let ir = InferenceIR {
        deterministic,
        probabilities,
        regions,
    };

    // 转译分析
    let mode = match req.mode.as_deref() {
        Some("teaching") => LLMMode::Teaching,
        Some("strategy") => LLMMode::Strategy,
        _ => LLMMode::Answer,
    };
    let translator = Translator::new(mode.clone());
    let use_llm = req.use_llm.unwrap_or(false);

    // LLM 客户端优先级: 请求中的运行时配置 > 环境变量 > 无
    let llm_client: Option<LLMClient> = if use_llm {
        if let Some(cfg) = &req.llm_config {
            if cfg.is_valid() {
                Some(LLMClient::new(cfg.clone()))
            } else {
                None
            }
        } else {
            state.llm_client.clone()
        }
    } else {
        None
    };

    // 月度限额: 超限后拒绝 LLM 调用, 回退本地分析 (防止意外超支)
    let mut llm_blocked_msg: Option<String> = None;
    let llm_client = if let Some(client) = &llm_client {
        if state.usage_store.limit_exceeded() {
            llm_blocked_msg = Some(format!(
                "⚠️ 本月 token 用量已达限额 ({}), 本次调用被拒绝并回退到本地分析。\n可调整环境变量 LLM_MONTHLY_TOKEN_LIMIT 后重启。",
                state.usage_store.month_tokens()
            ));
            None
        } else {
            Some(client.clone())
        }
    } else {
        None
    };

    let mut usage: Option<TokenUsage> = None;
    let (analysis, used_llm) = if use_llm {
        if let Some(client) = &llm_client {
            let system_prompt = translator.build_system_prompt();
            let user_message = translator.build_user_message(&view, &ir);
            match client.chat(&system_prompt, &user_message).await {
                Ok(result) => {
                    // 记录 token 用量 (API 未返回 usage 时跳过)
                    if let Some(u) = &result.usage {
                        state.usage_store
                            .record(client.config().model.as_str(), &client.config().provider_label(), u);
                        usage = Some(u.clone());
                    }
                    (result.content, true)
                }
                Err(e) => {
                    let local = translator.local_translate(&ir);
                    (format!("⚠️ LLM 调用失败, 已回退到本地分析\n\n错误: {}\n\n{}", e, local), false)
                }
            }
        } else if let Some(msg) = llm_blocked_msg {
            ("⚠️ ".to_string() + &msg + "\n\n" + &translator.local_translate(&ir), false)
        } else {
            ("⚠️ 未配置 LLM API Key, 回退到本地分析\n\n".to_string() + &translator.local_translate(&ir), false)
        }
    } else {
        (translator.local_translate(&ir), false)
    };

    Ok(Json(AnalyzeResponse {
        success: true,
        view,
        ir,
        analysis,
        used_llm,
        usage,
    }))
}

/// OCR 截图识别代理
///
/// 接收 base64 编码的截图，转发给 Python OCR 微服务，
/// 返回识别后的棋盘二维数组。
async fn ocr(
    State(state): State<Arc<AppState>>,
    Json(req): Json<OcrRequest>,
) -> Result<Json<OcrResponse>, (StatusCode, String)> {
    let client = reqwest::Client::new();
    let url = format!("{}/api/ocr", state.ocr_url);

    let resp = client
        .post(&url)
        .json(&serde_json::json!({ "image": req.image }))
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                format!("OCR 服务不可用: {}。请确保 OCR 微服务已启动 (默认端口 5001)", e),
            )
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err((
            StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            format!("OCR 服务错误: {}", body),
        ));
    }

    let ocr_resp: OcrResponse = resp.json().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("解析 OCR 响应失败: {}", e),
        )
    })?;

    Ok(Json(ocr_resp))
}

/// 测试 LLM 连接 (前端 UI "测试连接" 按钮调用)
async fn test_llm(
    Json(req): Json<LLMTestRequest>,
) -> Result<Json<LLMTestResponse>, (StatusCode, String)> {
    let cfg = req.llm_config;
    if !cfg.is_valid() {
        return Ok(Json(LLMTestResponse {
            success: false,
            model: None,
            error: Some("API Key 不能为空".to_string()),
        }));
    }

    let client = LLMClient::new(cfg);
    match client.test_connection().await {
        Ok(model) => Ok(Json(LLMTestResponse {
            success: true,
            model: Some(model),
            error: None,
        })),
        Err(e) => Ok(Json(LLMTestResponse {
            success: false,
            model: None,
            error: Some(e),
        })),
    }
}

/// 查询 LLM 配置状态 (不泄露 api_key, 只告知环境变量是否已配置)
async fn llm_status() -> Json<LLMStatusResponse> {
    let env_configured = std::env::var("OPENAI_API_KEY").map(|s| !s.is_empty()).unwrap_or(false);
    let env_model = std::env::var("OPENAI_MODEL").ok();
    let env_base_url = std::env::var("OPENAI_BASE_URL").ok();
    Json(LLMStatusResponse {
        env_configured,
        env_model,
        env_base_url,
    })
}

/// Token 用量统计 (今日 / 近7日 / 本月 / 累计 + 模型分布 + 近7天趋势)
async fn usage_stats(State(state): State<Arc<AppState>>) -> Json<crate::llm::UsageStats> {
    Json(state.usage_store.stats())
}

/// Token 用量明细日志 (按时间倒序)
async fn usage_logs(
    State(state): State<Arc<AppState>>,
    Query(q): Query<UsageLogsQuery>,
) -> Json<Vec<crate::llm::UsageRecord>> {
    let limit = q.limit.unwrap_or(100).min(10_000);
    let offset = q.offset.unwrap_or(0);
    Json(state.usage_store.logs(limit, offset))
}
