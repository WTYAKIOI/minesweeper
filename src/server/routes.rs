use axum::{
    extract::State,
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tower_http::services::ServeDir;

use crate::engine::{DeterministicEngine, MonteCarloEngine, RegionAnalyzer};
use crate::llm::{LLMClient, LLMMode, Translator};
use crate::model::{InferenceIR, PlayerView};

/// 应用状态
#[derive(Clone)]
pub struct AppState {
    pub llm_client: Option<LLMClient>,
    pub mc_iterations: usize,
    /// OCR 微服务地址
    pub ocr_url: String,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            llm_client: LLMClient::from_env(),
            mc_iterations: 0, // 0 = 动态自适应
            ocr_url: std::env::var("OCR_URL")
                .unwrap_or_else(|_| "http://localhost:5001".to_string()),
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
}

/// 分析响应
#[derive(Debug, Serialize)]
pub struct AnalyzeResponse {
    pub success: bool,
    pub view: PlayerView,
    pub ir: InferenceIR,
    pub analysis: String,
    pub used_llm: bool,
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

/// 创建路由
pub fn create_router(state: AppState) -> Router {
    Router::new()
        .route("/api/analyze", post(analyze))
        .route("/api/ocr", post(ocr))
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

    let mc = MonteCarloEngine::new(state.mc_iterations);
    let probabilities = mc.simulate_with_deductions(&view, &known_mines, &known_safe);

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

    let (analysis, used_llm) = if use_llm {
        if let Some(ref client) = state.llm_client {
            let system_prompt = translator.build_system_prompt();
            let user_message = translator.build_user_message(&view, &ir);
            match client.chat(&system_prompt, &user_message).await {
                Ok(resp) => (resp, true),
                Err(e) => (format!("LLM 调用失败: {}, 回退到本地分析\n\n{}", e, translator.local_translate(&ir)), false),
            }
        } else {
            ("未配置 OPENAI_API_KEY, 回退到本地分析\n\n".to_string() + &translator.local_translate(&ir), false)
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
