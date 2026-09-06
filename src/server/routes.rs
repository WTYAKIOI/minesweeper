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


use crate::llm::{
    vision, LLMClient, LLMConfig, LLMMode, TokenUsage, Translator, UsageStore,
};
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
    /// 从回答中提取的推理步骤 (含坐标, 前端逐步高亮); 无则省略
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reasoning_steps: Vec<crate::llm::translator::ReasoningStep>,
}

/// LLM 视觉棋盘识别请求 (base64 图像 + 可选运行时配置)
#[derive(Debug, Deserialize)]
pub struct LlmOcrRequest {
    /// base64 编码截图 (无 data: 前缀)
    pub image: String,
    /// 可选: 运行时 LLM 配置 (覆盖环境变量), 模型须支持视觉输入
    pub llm_config: Option<LLMConfig>,
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

/// Agent 循环运行请求 (9.2.7: 工具调用 + ReAct)
#[derive(Debug, Deserialize)]
pub struct AgentRunRequest {
    pub board: Vec<Vec<i32>>,
    pub remaining_mines: u32,
    /// 任务目标 (默认: 执行可证明操作后给出建议)
    #[serde(default)]
    pub goal: Option<String>,
    /// 最大步数 (≤15; 默认 AGENT_MAX_STEPS 或 8)
    #[serde(default)]
    pub max_steps: Option<u32>,
    /// 运行时 LLM 配置 (覆盖环境变量)
    #[serde(default)]
    pub llm_config: Option<LLMConfig>,
}

/// Agent 状态查询响应 (免 LLM 调用)
#[derive(Debug, Serialize)]
pub struct AgentStatusResponse {
    pub available: bool,
    pub tools: Vec<String>,
    pub max_steps: u32,
    pub max_steps_limit: u32,
}

/// Agent 运行响应
#[derive(Debug, Serialize)]
pub struct AgentRunResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<crate::agent::AgentRunOutcome>,
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
        .route("/api/llm/ocr", post(llm_ocr))
        .route("/api/llm/test", post(test_llm))
        .route("/api/llm/models", post(llm_models))
        .route("/api/llm/status", get(llm_status))
        .route("/api/usage/stats", get(usage_stats))
        .route("/api/usage/logs", get(usage_logs))
        .route("/api/agent/run", post(agent_run))
        .route("/api/agent/status", get(agent_status))
        .route("/api/chat", post(chat_question))
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
        "del_row" => crate::agent::BoardOp::DeleteRow,
        "del_col" => crate::agent::BoardOp::DeleteCol,
        other => {
            return Json(BoardOpResponse {
                success: false,
                board: req.board,
                error: Some(format!(
                    "未知操作: {} (支持 cycle/flag/clear/del_row/del_col)",
                    other
                )),
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
    // 轻量结构校验 (矩形/尺寸/取值), 防止畸形输入进入编辑状态机
    let cells = req.board.len() * req.board[0].len();
    if let Err(e) = crate::engine::validate_board_input(&req.board, cells as u32) {
        return Json(BoardOpResponse {
            success: false,
            board: req.board,
            error: Some(e),
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
            if rows > crate::engine::MAX_BOARD_DIM || cols > crate::engine::MAX_BOARD_DIM {
                return Json(BoardParseResponse {
                    success: false,
                    board: None,
                    rows: None,
                    cols: None,
                    inferred_mines: None,
                    error: Some(format!(
                        "文本棋盘尺寸 {}x{} 超过上限 {}x{}",
                        rows,
                        cols,
                        crate::engine::MAX_BOARD_DIM,
                        crate::engine::MAX_BOARD_DIM
                    )),
                });
            }
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
    let (view, ir) = crate::engine::run_local_pipeline(&req.board, req.remaining_mines)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
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

/// LLM 翻译编排: 输出完整性重试(教学) + 输出混乱纪律重试 + 瞬态网络错误自动重试 + 用量记录。
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
    let mut last_chaotic = false;

    for attempt in 0..3 {
        let mut user_message = base_user.clone();
        if attempt > 0 {
            if last_chaotic && !is_teaching {
                // 输出混乱 (思考自白/语言/结构违规) → 纪律提醒重试
                user_message.push_str(
                    "\n\n【重试提醒】你上一次的输出违反输出纪律: 包含思考过程自白 (Let me / I think 等)、\
                     语言非简体中文、或结构混乱。请直接按模板输出简洁结论: 不得展示思考过程, \
                     全文简体中文, 只输出模板区块。",
                );
            } else if last_content.is_some() {
                user_message.push_str(
                    "\n\n【重试提醒】你上一次的输出不完整或为空。请严格按输出格式重新输出完整内容，不要只输出标题、不要留空。",
                );
            }
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
                let content = result.content;
                last_content = Some(content.clone());
                // 教学: 输出不完整才重试; 其他模式: 输出混乱 (自白/无结构) 则纪律提醒重试
                last_chaotic = !is_teaching
                    && crate::llm::translator::is_output_chaotic(&content);
                let incomplete = is_teaching
                    && crate::llm::translator::is_output_incomplete(&content);
                if !incomplete && !last_chaotic {
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
                // 净化: 剔除残留思考自白行/引言段, 过长则截断 (保留最终结论行)
                let (cleaned, _dropped) = crate::llm::translator::clean_llm_output(&content);
                if cleaned.trim().is_empty() {
                    let local = translator.local_translate(ir);
                    (
                        format!("⚠️ LLM 输出全为思考过程, 已净化并回退到本地分析：\n\n{}", local),
                        true,
                    )
                } else {
                    let cleaned = crate::llm::translator::truncate_llm_output(&cleaned, 1200);
                    (cleaned, true)
                }
            }
        }
        (None, Some(e)) => {
            let local = translator.local_translate(ir);
            let hint = if e.contains("504") || e.contains("Gateway Time-out") {
                "\n\n提示: 网关 504 是网关自身空闲超时 (与后端超时设置无关), 已默认改用流式请求规避。\
                 若仍 504: ① 换更快的模型 (如 deepseek-chat / gpt-4o-mini, glm-5 思维链较慢); \
                 ② 在 UI 调小最大 Token; ③ 稍后重试。"
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
    let (view, ir) = crate::engine::run_local_pipeline(&req.board, req.remaining_mines)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    // 转译分析; @teach 指令强制教学模式 (对话区一键切换, 免去手动点开关)
    let mut mode = match req.mode.as_deref() {
        Some("teaching") => LLMMode::Teaching,
        Some("strategy") => LLMMode::Strategy,
        _ => LLMMode::Answer,
    };
    if let Some(q) = req.question.as_deref() {
        if matches!(
            crate::llm::translator::parse_user_command(q),
            Some(crate::llm::translator::UserCommand::Teach)
        ) {
            mode = LLMMode::Teaching;
        }
    }
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
    // 答案模式 + LLM → 内置 Agent 分析循环 (模型与本地推理库逐轮交互, 见 agent/loop_.rs);
    // 教学/策略模式保持单轮转译 (教学不得提前给结论)。
    let use_agent_analysis = use_llm && mode == LLMMode::Answer;

    // 答案模式走 Agent 循环时, 结果棋盘 (含已证必雷格标旗) 会更新, 需重算本地管道
    let mut view = view;
    let mut ir = ir;

    let usage;
    let (mut analysis, used_llm) = if use_llm {
        if let Some(client) = &llm_client {
            if use_agent_analysis {
                let english = translator.language() == crate::llm::translator::Language::English;
                // ---- 意图路由: 含坐标的提问 → 定点解释 (单次小调用), 不走全盘 Agent 循环 ----
                let point_target = req
                    .question
                    .as_deref()
                    .and_then(crate::llm::translator::user_asked_cell);
                if let Some(target) = point_target {
                    let (txt, ok, usg) = point_explain(
                        client,
                        &state.usage_store,
                        req.question.as_deref().unwrap_or(""),
                        target,
                        &req.board,
                        req.remaining_mines,
                        english,
                    )
                    .await;
                    usage = usg;
                    (txt, ok)
                } else {
                    let cfg = crate::agent::AnalysisAgentConfig::default();
                    let outcome = crate::agent::run_analysis_agent(
                        client,
                        req.board.clone(),
                        req.remaining_mines,
                        req.question.as_deref(),
                        english,
                        &cfg,
                        Some(&state.usage_store),
                    )
                    .await;
                    if outcome.reason == crate::agent::AgentEndReason::LlmError
                        && outcome.analysis.trim().is_empty()
                    {
                        // LLM 不可用 → 本地回退 (与旧行为一致)
                        usage = None;
                        (
                            format!(
                                "⚠️ LLM 调用失败, Agent 分析已回退到本地分析\n\n{}",
                                translator.local_translate(&ir)
                            ),
                            false,
                        )
                    } else {
                        // 写回工作棋盘 (含 Agent 已证明雷格的标旗), 重算本地 IR 与展示
                        if let Ok((v2, ir2)) = crate::engine::run_local_pipeline(
                            &outcome.board,
                            outcome.remaining_mines,
                        ) {
                            view = v2;
                            ir = ir2;
                        }
                        usage = outcome.real_usage.clone().or_else(|| {
                            Some(crate::agent::estimate_token_usage(
                                outcome.prompt_chars,
                                outcome.completion_chars,
                            ))
                        });
                        (outcome.analysis, true)
                    }
                }
            } else {
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
                // 网关缺失 usage 时按消息长度估算, 保证前端可回显 token
                usage = usg.or_else(|| {
                    let prompt_chars = translator
                        .build_user_message_with_question(&view, &ir, req.question.as_deref())
                        .chars()
                        .count();
                    Some(crate::agent::estimate_token_usage(prompt_chars, txt.chars().count()))
                });
                (txt, ok)
            }
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

    // 最终净化: 回答过长/含机器味 → 截断并保留末尾"最终结论"行 (目标 ≤800 字左右)
    if used_llm && !is_teaching {
        let (cleaned, _) = crate::llm::translator::clean_llm_output(&analysis);
        analysis = if cleaned.trim().is_empty() {
            analysis
        } else {
            crate::llm::translator::truncate_llm_output(&cleaned, 1400)
        };
    }
    // 推理步骤提取: 从回答文本中抽出含坐标的推理行, 前端逐步高亮
    let reasoning_steps =
        crate::llm::translator::extract_reasoning_nodes(&analysis, &view);

    Ok(Json(AnalyzeResponse {
        success: true,
        view,
        ir,
        analysis,
        used_llm,
        usage,
        reasoning_steps,
    }))
}

/// 定点解释: 用户提问指定格 (如 "(18,15) 为什么是雷") → 取该格引擎素材 +
/// 单次小 LLM 调用按定点模板作答; 失败时直接呈现引擎素材。
/// 返回 (analysis, used_llm, usage)
async fn point_explain(
    client: &LLMClient,
    usage_store: &Arc<UsageStore>,
    question: &str,
    target: crate::model::Coord,
    board: &[Vec<i32>],
    remaining_mines: u32,
    english: bool,
) -> (String, bool, Option<TokenUsage>) {
    let mut agent_board = crate::agent::AgentBoard::new(board.to_vec(), remaining_mines);
    let material = crate::agent::tools::execute_tool(
        &mut agent_board,
        "explain_cell",
        &serde_json::json!({"x": target.x, "y": target.y}),
    )
    .unwrap_or_else(|e| format!("该格无法解释: {}", e));

    let system_prompt = crate::llm::translator::build_explain_system_prompt(english);
    let user_msg = format!(
        "用户提问: {}\n\n【引擎素材】\n{}",
        question, material
    );

    let mut usage: Option<TokenUsage> = None;
    match client.chat(&system_prompt, &user_msg).await {
        Ok(result) => {
            if let Some(u) = &result.usage {
                usage_store.record(
                    client.config().model.as_str(),
                    &client.config().provider_label(),
                    u,
                );
                usage = Some(u.clone());
            }
            usage = usage.or_else(|| {
                Some(crate::agent::estimate_token_usage(
                    user_msg.chars().count(),
                    result.content.chars().count(),
                ))
            });
            let (cleaned, _) = crate::llm::translator::clean_llm_output(&result.content);
            let analysis = if cleaned.trim().is_empty() {
                result.content
            } else {
                crate::llm::translator::truncate_llm_output(&cleaned, 1200)
            };
            (analysis, true, usage)
        }
        Err(e) => {
            // LLM 不可用 → 引擎素材本身即为可读定点说明
            (
                format!(
                    "⚠️ LLM 调用失败 ({})。以下为引擎对该格的直接判定与相关约束:\n\n{}",
                    e, material
                ),
                false,
                None,
            )
        }
    }
}

/// 判断 LLM 错误是否为"瞬态" (值得自动重试):
/// 网关 504 / 上游超时 / 5xx / 连接重置 / 连接层发送失败 / 限流等
fn is_transient_llm_error(msg: &str) -> bool {
    let m = msg.to_lowercase();
    m.contains("504")
        || m.contains("gateway time-out")
        || m.contains("timed out")
        || m.contains("timeout")
        // reqwest 连接层失败 (超时/连接重置的原因链可能被网关吞掉), 重试一次代价低
        || m.contains("error sending request")
        || m.contains("connection reset")
        || m.contains("connection closed")
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

/// 从 base64 前缀嗅探图像 MIME 类型 (默认 png)
fn sniff_image_mime(b64: &str) -> &'static str {
    use base64::Engine;
    let prefix: String = b64.chars().take(16).collect();
    if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(prefix.as_bytes()) {
        if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
            return "image/png";
        }
        if bytes.starts_with(&[0xFF, 0xD8]) {
            return "image/jpeg";
        }
        if bytes.starts_with(b"GIF8") {
            return "image/gif";
        }
        if bytes.starts_with(b"RIFF") {
            return "image/webp";
        }
    }
    "image/png"
}

/// 错误信息截断 (超长上游报错只保留前 300 字符, 便于前端展示)
fn truncate_err(msg: &str) -> String {
    if msg.chars().count() <= 300 {
        msg.to_string()
    } else {
        let head: String = msg.chars().take(300).collect();
        format!("{}…", head)
    }
}

/// LLM 视觉棋盘识别编排:
/// 多模态 LLM 自行检测行列数并输出 JSON → Rust 解析/归一化 →
/// 格式校验 + 扫雷规则校验 → 转换为棋盘二维数组。
///
/// 校验失败不抛异常, 返回 success=false 由前端回退本地 OCR
/// (设计文档 §5/§7: 双重校验 + 故障回退)。
async fn llm_ocr(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LlmOcrRequest>,
) -> Result<Json<OcrResponse>, (StatusCode, String)> {
    let fail = |msg: String| {
        Json(OcrResponse {
            success: false,
            board: Vec::new(),
            remaining_mines: None,
            error: Some(msg),
        })
    };

    if req.image.trim().is_empty() {
        return Ok(fail("图像数据为空".to_string()));
    }

    // LLM 客户端优先级: 请求运行时配置 > 环境变量 > 无
    let client: Option<LLMClient> = match &req.llm_config {
        Some(cfg) if cfg.is_valid() => Some(LLMClient::new(cfg.clone())),
        Some(_) => None,
        None => state.llm_client.clone(),
    };
    let Some(client) = client else {
        return Ok(fail(
            "未配置 LLM (请在 API 配置面板填写支持视觉的模型, 或设置 OPENAI_API_KEY 环境变量)".to_string(),
        ));
    };

    // 月度限额: 超限拒绝, 回退本地 OCR
    if state.usage_store.limit_exceeded() {
        return Ok(fail(format!(
            "本月 token 用量已达限额 ({}), 本次识别被拒绝",
            state.usage_store.month_tokens()
        )));
    }

    let data_url = format!(
        "data:{};base64,{}",
        sniff_image_mime(&req.image),
        req.image.trim()
    );
    let result = match client
        .chat_with_image(
            vision::BOARD_RECOGNITION_SYSTEM_PROMPT,
            vision::BOARD_RECOGNITION_USER_PROMPT,
            &data_url,
        )
        .await
    {
        Ok(r) => r,
        Err(e) => {
            let msg = e.to_string();
            // 纯文本模型收到 image_url → 明确指引切换视觉模型, 而非笼统报错
            if crate::llm::client::is_vision_unsupported_error(&msg) {
                return Ok(fail(format!(
                    "模型「{}」为纯文本模型, 不支持图像输入 (上游: {})。\
                     请在 API 配置中把「视觉模型」设为该网关上支持图像的模型\
                     (如 glm-4v / gpt-4o / qwen-vl 系列, 可点「🔍 获取模型」查看列表),\
                     或关闭「🤖 LLM 识别」改用本地 OCR",
                    client.effective_vision_model(),
                    truncate_err(&msg)
                )));
            }
            return Ok(fail(format!("LLM 识别请求失败: {}", truncate_err(&msg))));
        }
    };

    if let Some(u) = &result.usage {
        state.usage_store.record(
            client.config().model.as_str(),
            &client.config().provider_label(),
            u,
        );
    }

    // 解析 → 归一化 (含尺寸修正) → 格式校验 → 规则校验
    let parsed = vision::parse_llm_board_response(&result.content)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    if !vision::validate_llm_output(&parsed) {
        return Ok(fail("LLM 输出格式校验失败 (维度不自洽或含非法格子值)".to_string()));
    }
    let board = parsed.to_board();
    if !vision::validate_minesweeper_rules(&board) {
        return Ok(fail(
            "LLM 识别结果未通过扫雷规则校验 (数字与周围旗数/未知格矛盾), 可能为误识别".to_string(),
        ));
    }

    let remaining_mines = crate::agent::infer_mines_for_size(&board);
    Ok(Json(OcrResponse {
        success: true,
        board,
        remaining_mines,
        error: None,
    }))
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

/// Agent 状态 (工具清单 / 步数上限), 供前端按钮渲染
async fn agent_status() -> Json<AgentStatusResponse> {
    let names: Vec<String> = vec![
        "analyze_board".into(),
        "flag_cell".into(),
        "check_safe".into(),
        "explain_cell".into(),
        "verify_flag".into(),
        "get_constraints".into(),
        "list_regions".into(),
        "get_knowledge".into(),
    ];
    Json(AgentStatusResponse {
        available: true,
        tools: names,
        max_steps: crate::agent::AgentLoopConfig::default().max_steps,
        max_steps_limit: 15,
    })
}

/// Agent 循环: 模型判断 → 工具调用 → 观察 → 再判断 (ReAct)
///
/// 安全护栏: 步数上限 / HTTP 断开即取消 / 每步 LLM 超时 / 月度 token 限额;
/// 工具只由 Rust 校验并执行 (见 src/agent/tools.rs)。
async fn agent_run(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AgentRunRequest>,
) -> Json<AgentRunResponse> {
    let fail = |msg: String| Json(AgentRunResponse { success: false, outcome: None, error: Some(msg) });
    if req.board.is_empty() || req.board[0].is_empty() {
        return fail("棋盘为空".to_string());
    }
    // LLM 客户端: 运行时配置 > 环境变量 > 无
    let client: Option<LLMClient> = match &req.llm_config {
        Some(cfg) if cfg.is_valid() => Some(LLMClient::new(cfg.clone())),
        Some(_) => None,
        None => state.llm_client.clone(),
    };
    let Some(client) = client else {
        return fail(
            "未配置 LLM (请在 API 配置面板填写模型, 或设置 OPENAI_API_KEY 环境变量)".to_string(),
        );
    };
    if state.usage_store.limit_exceeded() {
        return fail(format!(
            "本月 token 用量已达限额 ({}), Agent 无法启动",
            state.usage_store.month_tokens()
        ));
    }
    // 步数护栏: 请求值 ≤15
    let mut cfg = crate::agent::AgentLoopConfig::default();
    if let Some(n) = req.max_steps {
        cfg.max_steps = n.clamp(1, 15);
    }
    let goal = req
        .goal
        .filter(|g| !g.trim().is_empty())
        .unwrap_or_else(|| "分析当前棋盘: 用工具逐步执行可证明的标旗操作, 结束后给出建议点击的安全格、需移除的矛盾旗与剩余风险提示。".to_string());

    let outcome = crate::agent::run_agent_loop(
        &client,
        req.board.clone(),
        req.remaining_mines,
        &goal,
        &cfg,
        Some(&state.usage_store),
    )
    .await;

    Json(AgentRunResponse { success: outcome.success, outcome: Some(outcome), error: None })
}

/// 自主 ReAct 问答请求
#[derive(Debug, Deserialize)]
pub struct ChatQuestionRequest {
    pub board: Vec<Vec<i32>>,
    pub remaining_mines: u32,
    pub question: String,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub max_steps: Option<u32>,
    #[serde(default)]
    pub llm_config: Option<LLMConfig>,
}

/// 自主 ReAct 问答响应
#[derive(Debug, Serialize)]
pub struct ChatQuestionResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steps: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<crate::agent::AgentEndReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// 自主 ReAct 问答: LLM 自己决定查哪些工具、何时回答 (不再按坐标硬路由)。
/// 与 analyze 不同: 只针对用户问题, 不做全盘分析输出。
async fn chat_question(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ChatQuestionRequest>,
) -> Json<ChatQuestionResponse> {
    if req.board.is_empty() || req.board[0].is_empty() {
        return Json(ChatQuestionResponse {
            success: false,
            answer: None,
            steps: None,
            reason: None,
            usage: None,
            error: Some("棋盘为空".to_string()),
        });
    }
    if req.question.trim().is_empty() {
        return Json(ChatQuestionResponse {
            success: false,
            answer: None,
            steps: None,
            reason: None,
            usage: None,
            error: Some("提问为空".to_string()),
        });
    }
    let client: Option<LLMClient> = match &req.llm_config {
        Some(cfg) if cfg.is_valid() => Some(LLMClient::new(cfg.clone())),
        Some(_) => None,
        None => state.llm_client.clone(),
    };
    let Some(client) = client else {
        return Json(ChatQuestionResponse {
            success: false,
            answer: None,
            steps: None,
            reason: None,
            usage: None,
            error: Some("未配置 LLM (请在 API 配置面板填写模型, 或设置 OPENAI_API_KEY 环境变量)".to_string()),
        });
    };
    if state.usage_store.limit_exceeded() {
        return Json(ChatQuestionResponse {
            success: false,
            answer: None,
            steps: None,
            reason: None,
            usage: None,
            error: Some(format!(
                "本月 token 用量已达限额 ({})",
                state.usage_store.month_tokens()
            )),
        });
    }
    let mut cfg = crate::agent::ReactQaConfig::default();
    if let Some(n) = req.max_steps {
        cfg.max_steps = n.clamp(1, 10);
    }
    let english = crate::llm::translator::Language::parse(req.language.as_deref())
        == crate::llm::translator::Language::English;

    let outcome = crate::agent::run_react_qa(
        &client,
        req.board,
        req.remaining_mines,
        req.question.trim(),
        english,
        &cfg,
        Some(&state.usage_store),
    )
    .await;

    let usage = outcome.real_usage.clone().or_else(|| {
        Some(crate::agent::estimate_token_usage(
            outcome.prompt_chars,
            outcome.completion_chars,
        ))
    });
    Json(ChatQuestionResponse {
        success: true,
        answer: Some(outcome.answer),
        steps: Some(outcome.steps),
        reason: Some(outcome.reason),
        usage,
        error: None,
    })
}
