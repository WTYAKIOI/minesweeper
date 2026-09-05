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
use crate::model::{Conclusion, Coord, InferenceIR, PlayerView, Proof};

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
    /// 可选: 用户在对话区输入的问题 (教学/问答模式携带)
    #[serde(default)]
    pub question: Option<String>,
    /// 可选: 回答语言 ("zh" / "en", 默认 zh)
    #[serde(default)]
    pub language: Option<String>,
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

/// 从网关获取模型列表请求 (复用运行时配置)
#[derive(Debug, Deserialize)]
pub struct LLMModelsRequest {
    pub llm_config: LLMConfig,
}

/// 模型列表响应
#[derive(Debug, Serialize)]
pub struct LLMModelsResponse {
    pub success: bool,
    pub models: Option<Vec<String>>,
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

/// 棋盘编辑操作请求 (前端把状态机语义交给 Rust agent 模块)
#[derive(Debug, Deserialize)]
pub struct BoardOpRequest {
    pub board: Vec<Vec<i32>>,
    pub x: usize,
    pub y: usize,
    /// "cycle" (左键循环) / "flag" (右键插旗) / "clear" (Shift+左键清零)
    pub op: String,
}

#[derive(Debug, Serialize)]
pub struct BoardOpResponse {
    pub success: bool,
    pub board: Vec<Vec<i32>>,
    pub error: Option<String>,
}

/// 文本棋盘解析请求 (原前端 parseTextBoard 语义迁移到 Rust)
#[derive(Debug, Deserialize)]
pub struct BoardParseRequest {
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct BoardParseResponse {
    pub success: bool,
    pub board: Option<Vec<Vec<i32>>>,
    pub rows: Option<usize>,
    pub cols: Option<usize>,
    /// 标准尺寸下推断的剩余雷数 (agent 模块计算, 替代前端重复推断)
    pub inferred_mines: Option<u32>,
    pub error: Option<String>,
}

/// Markdown 报告导出请求
#[derive(Debug, Deserialize)]
pub struct ExportRequest {
    pub board: Vec<Vec<i32>>,
    pub remaining_mines: u32,
    #[serde(default)]
    pub analysis: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ExportResponse {
    pub success: bool,
    pub markdown: Option<String>,
    pub error: Option<String>,
}

/// 创建路由
pub fn create_router(state: AppState) -> Router {
    Router::new()
        .route("/api/analyze", post(analyze))
        .route("/api/board/op", post(board_op))
        .route("/api/board/parse", post(board_parse))
        .route("/api/export", post(export_report))
        .route("/api/ocr", post(ocr))
        .route("/api/llm/test", post(test_llm))
        .route("/api/llm/models", post(llm_models))
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

/// 棋盘编辑操作 (agent 模块状态机, 前端渲染层只调用不改判)
async fn board_op(Json(req): Json<BoardOpRequest>) -> Json<BoardOpResponse> {
    let op = match req.op.as_str() {
        "cycle" => crate::agent::BoardOp::LeftCycle,
        "flag" => crate::agent::BoardOp::ToggleFlag,
        "clear" => crate::agent::BoardOp::ClearToZero,
        other => {
            return Json(BoardOpResponse {
                success: false,
                board: req.board,
                error: Some(format!("未知操作: {} (支持 cycle/flag/clear)", other)),
            })
        }
    };
    if req.board.is_empty() {
        return Json(BoardOpResponse {
            success: false,
            board: req.board,
            error: Some("棋盘为空".to_string()),
        });
    }
    let board = crate::agent::apply_board_op(&req.board, req.x, req.y, op);
    Json(BoardOpResponse { success: true, board, error: None })
}

/// 文本棋盘解析 (agent 模块)
async fn board_parse(Json(req): Json<BoardParseRequest>) -> Json<BoardParseResponse> {
    match crate::agent::parse_text_board(&req.text) {
        Ok(board) => {
            let (rows, cols) = (board.len(), board.first().map(|r| r.len()).unwrap_or(0));
            let inferred_mines = crate::agent::infer_mines_for_size(&board);
            Json(BoardParseResponse {
                success: true,
                board: Some(board),
                rows: Some(rows),
                cols: Some(cols),
                inferred_mines,
                error: None,
            })
        }
        Err(e) => Json(BoardParseResponse {
            success: false,
            board: None,
            rows: None,
            cols: None,
            inferred_mines: None,
            error: Some(e.to_string()),
        }),
    }
}

/// Markdown 报告导出 (agent 模块构建, 前端只负责下载)
async fn export_report(
    Json(req): Json<ExportRequest>,
) -> Result<Json<ExportResponse>, (StatusCode, String)> {
    let (view, ir) = run_local_pipeline(&req.board, req.remaining_mines)?;
    let analysis = match &req.analysis {
        Some(a) if !a.trim().is_empty() => a.clone(),
        _ => {
            // 未附带解读文本时用本地答案模式转译兜底
            Translator::new(LLMMode::Answer).local_translate(&ir)
        }
    };
    let markdown = crate::agent::render_markdown_report(&req.board, &view, &ir, &analysis);
    Ok(Json(ExportResponse { success: true, markdown: Some(markdown), error: None }))
}

/// 本地推理管道: 旗帜正确性验证(容错) → 确定性推理(+子集) → 概率 → 区域。
/// analyze 与 export 共用, 保证两处任务编排一致。
///
/// 容错语义: 用户可能误标旗。verify_board 先把与数字约束矛盾的旗帜降级为
/// "未知"构造容错视图, 推理不再信任矛盾旗; 判定结果随 IR 的
/// flag_verification 字段返回, 供前端高亮矛盾旗并提示纠错。
fn run_local_pipeline(
    board: &[Vec<i32>],
    remaining_mines: u32,
) -> Result<(PlayerView, InferenceIR), (StatusCode, String)> {
    if board.is_empty() || board[0].is_empty() {
        return Err((StatusCode::BAD_REQUEST, "棋盘为空".to_string()));
    }

    // 旗帜正确性判定 (在推理之前执行) — 返回容错视图 + 形式化证明链
    let (view, flag_verification, flag_proofs) = crate::engine::verify_board_full(board, remaining_mines);

    // 运行确定性推理 (含子集规则)
    let mut deterministic = DeterministicEngine::solve_with_subset_rule(&view);

    // 合并旗帜验证与"局部闭包枚举"的形式化结论 (按坐标去重, 保证 已知雷/安全 集合一致)
    let has_coord = |c: &Coord, proofs: &[Proof]| proofs.iter().any(|p| &p.conclusion.coord == c);
    for p in flag_proofs
        .iter()
        .chain(crate::engine::derive_local_forced_proofs(&view).iter())
    {
        if !has_coord(&p.conclusion.coord, &deterministic) {
            deterministic.push(p.clone());
        }
    }
    deterministic.sort_by_key(|p| (p.conclusion.coord.y, p.conclusion.coord.x));

    // 剩余雷数已为 0: 所有未知格必为安全 (总雷数已全部被标出), 直接给出理由并入链
    if view.remaining_mines == 0 {
        for c in &view.unknown {
            let covered = deterministic
                .iter()
                .any(|p| p.conclusion.coord == *c);
            if covered {
                continue;
            }
            deterministic.push(Proof {
                conclusion: Conclusion { coord: *c, is_mine: false },
                depends_on: Vec::new(),
                rule: "剩余雷数已为 0, 该格必为安全".to_string(),
            });
        }
        deterministic.sort_by_key(|p| (p.conclusion.coord.y, p.conclusion.coord.x));
    }

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

    let mut probabilities = ProbabilityEngine::compute(&view, &known_mines, &known_safe);

    // 雷概率 = 0 的格 → 判定为安全并给出理由 (如剩余雷数已为 0, 或约束解析确认
    // 不存在把雷放在该格的可行方案)。这些结论并入确定性链, 并从概率列表中移除,
    // 前端据此以"必安全"高亮并显示理由。
    let mut zero_safe: std::collections::HashSet<Coord> = std::collections::HashSet::new();
    let mut new_proofs: Vec<Proof> = Vec::new();
    for p in &probabilities {
        if p.mine_probability > 0.0 {
            continue;
        }
        if known_mines.contains(&p.coord) || known_safe.contains(&p.coord) {
            continue;
        }
        zero_safe.insert(p.coord);
        let touches_number = view
            .revealed
            .iter()
            .any(|rv| rv.coord.neighbors(view.width, view.height).contains(&p.coord));
        let reason = if view.remaining_mines == 0 {
            "剩余雷数已为 0, 该格必为安全".to_string()
        } else if !touches_number {
            "该格不与任何已翻开数字相邻且雷概率为 0, 表明总雷数已被确认在其他格子区域 (该格必为安全)".to_string()
        } else {
            "约束解析下该格雷概率为 0%, 判定为安全 (不存在把雷放在该格的可行方案)".to_string()
        };
        let depends_on: Vec<Coord> = view
            .revealed
            .iter()
            .filter(|rv| rv.coord.neighbors(view.width, view.height).contains(&p.coord))
            .map(|rv| rv.coord)
            .take(8)
            .collect();
        new_proofs.push(Proof {
            conclusion: Conclusion {
                coord: p.coord,
                is_mine: false,
            },
            depends_on,
            rule: reason,
        });
    }
    deterministic.extend(new_proofs.iter().cloned());
    deterministic.sort_by_key(|p| (p.conclusion.coord.y, p.conclusion.coord.x));
    probabilities.retain(|p| !zero_safe.contains(&p.coord));

    let regions = RegionAnalyzer::analyze(&view, &probabilities);

    Ok((
        view,
        InferenceIR {
            deterministic,
            probabilities,
            regions,
            flag_verification,
        },
    ))
}


/// LLM 翻译编排: 输出完整性重试(教学) + 瞬态网络错误自动重试 + 用量记录。
/// 返回 (analysis, used_llm, usage)
async fn llm_translate_with_retry(
    client: &LLMClient,
    usage_store: &Arc<UsageStore>,
    translator: &Translator,
    view: &PlayerView,
    ir: &InferenceIR,
    question: Option<&str>,
    is_teaching: bool,
) -> (String, bool, Option<TokenUsage>) {
    let system_prompt = translator.build_system_prompt();
    let base_user = translator.build_user_message_with_question(view, ir, question);
    let mut usage: Option<TokenUsage> = None;
    let mut last_err: Option<String> = None;
    let mut last_content: Option<String> = None;

    for attempt in 0..3 {
        let mut user_message = base_user.clone();
        if attempt > 0 && last_content.is_some() {
            user_message.push_str(
                "\n\n【重试提醒】你上一次的输出不完整或为空。请严格按输出格式重新输出完整内容，不要只输出标题、不要留空。",
            );
        }
        match client.chat(&system_prompt, &user_message).await {
            Ok(result) => {
                if let Some(u) = &result.usage {
                    usage_store.record(
                        client.config().model.as_str(),
                        &client.config().provider_label(),
                        u,
                    );
                    usage = Some(u.clone());
                }
                last_content = Some(result.content);
                let incomplete = is_teaching
                    && crate::llm::translator::is_output_incomplete(last_content.as_deref().unwrap_or(""));
                if !incomplete {
                    break;
                }
            }
            Err(e) => {
                let msg = e.to_string();
                // 瞬态错误 (网关 504 / 超时 / 5xx / 连接重置): 立即重试, 最多 3 次
                // (axum handler 要求 future Send, 故不使用 tokio sleep 退避;
                // 有界重试保证不会死循环)
                if attempt < 1 && is_transient_llm_error(&msg) {
                    continue;
                }
                last_err = Some(msg);
                break;
            }
        }
    }

    let (text, ok) = match (last_content, last_err) {
        (Some(content), _) => {
            if is_teaching && crate::llm::translator::is_output_incomplete(&content) {
                let local = translator.local_translate(ir);
                (
                    format!("⚠️ LLM 连续 3 次输出不完整（空或仅标题），已回退到本地引导：\n\n{}", local),
                    true,
                )
            } else {
                (content, true)
            }
        }
        (None, Some(e)) => {
            let local = translator.local_translate(ir);
            let hint = if e.contains("504") || e.contains("Gateway Time-out") {
                "\n\n提示: 网关 504 超时通常是模型响应过慢或上游代理超时。瞬态错误自动重试 1 次后仍失败，可稍后再试；或调大后端环境变量 LLM_TIMEOUT_SECS（默认 120 秒）。"
            } else {
                ""
            };
            (format!("⚠️ LLM 调用失败, 已回退到本地分析\n\n错误: {}{}\n\n{}", e, hint, local), false)
        }
        (None, None) => {
            let local = translator.local_translate(ir);
            (format!("⚠️ LLM 未返回有效内容, 已回退到本地分析\n\n{}", local), false)
        }
    };
    (text, ok, usage)
}

/// 分析局面: 统一任务编排 (校验→推理→(可选)LLM 转译)
async fn analyze(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AnalyzeRequest>,
) -> Result<Json<AnalyzeResponse>, (StatusCode, String)> {
    // 推理管道: 旗帜验证(容错+证明链) → 确定性 → 概率 → 区域
    let (view, ir) = run_local_pipeline(&req.board, req.remaining_mines)?;

    // 转译分析
    let mode = match req.mode.as_deref() {
        Some("teaching") => LLMMode::Teaching,
        Some("strategy") => LLMMode::Strategy,
        _ => LLMMode::Answer,
    };
    let mut translator = Translator::new(mode.clone());
    translator.set_language(crate::llm::translator::Language::parse(req.language.as_deref()));
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

    // 月度限额: 超限后拒绝 LLM 调用, 回退本地分析
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

    let is_teaching = mode == LLMMode::Teaching;
    let usage;
    let (analysis, used_llm) = if use_llm {
        if let Some(client) = &llm_client {
            let (txt, ok, usg) = llm_translate_with_retry(
                client,
                &state.usage_store,
                &translator,
                &view,
                &ir,
                req.question.as_deref(),
                is_teaching,
            )
            .await;
            usage = usg;
            (txt, ok)
        } else if let Some(msg) = llm_blocked_msg {
            usage = None;
            ("⚠️ ".to_string() + &msg + "\n\n" + &translator.local_translate(&ir), false)
        } else {
            usage = None;
            ("⚠️ 未配置 LLM API Key, 回退到本地分析\n\n".to_string() + &translator.local_translate(&ir), false)
        }
    } else {
        usage = None;
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

/// 判断 LLM 错误是否为"瞬态" (值得自动重试):
/// 网关 504 / 上游超时 / 5xx / 连接重置 / 限流等
fn is_transient_llm_error(msg: &str) -> bool {
    let m = msg.to_lowercase();
    m.contains("504")
        || m.contains("gateway time-out")
        || m.contains("timed out")
        || m.contains("timeout")
        || m.contains("connection reset")
        || m.contains("http 500")
        || m.contains("http 502")
        || m.contains("http 503")
        || m.contains("http 529")
        || m.contains("overloaded")
        || m.contains("temporarily unavailable")
        || m.contains("429")
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

/// 从网关获取可用模型列表 (前端「🔍 从网关获取」按钮)
///
/// 通过后端代理请求 {base_url}/models (浏览器直连会触发 CORS),
/// Ollama 等本地网关自动回退到 /api/tags。
async fn llm_models(
    Json(req): Json<LLMModelsRequest>,
) -> Result<Json<LLMModelsResponse>, (StatusCode, String)> {
    let cfg = req.llm_config;
    if cfg.base_url.trim().is_empty() {
        return Ok(Json(LLMModelsResponse {
            success: false,
            models: None,
            error: Some("Base URL 不能为空".to_string()),
        }));
    }
    let client = LLMClient::new(cfg);
    match client.fetch_models().await {
        Ok(models) => Ok(Json(LLMModelsResponse {
            success: true,
            models: Some(models),
            error: None,
        })),
        Err(e) => Ok(Json(LLMModelsResponse {
            success: false,
            models: None,
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
