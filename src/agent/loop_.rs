//! Agent 循环 (ReAct) — 模型判断 → 工具调用 → 观察结果 → 再次判断, 直到完成或停止。
//!
//! 安全护栏 (9.2.7.2 要求):
//! - 最大步数: 请求可传 max_steps, 硬上限 AGENT_MAX_STEPS (默认 8, 上限 15)
//! - 取消机制: HTTP 请求断开 (前端 AbortController) → axum 丢弃 handler future → 循环在
//!   下一次 await 处中止
//! - 超时: 每次 LLM 调用受客户端超时约束 (LLM_TIMEOUT_SECS); 另有整体时间预算
//!   time_budget_secs (默认 180s), 超时以 reason=timeout 结束
//! - 费用上限: 每轮调用前检查月度 token 限额 (UsageStore::limit_exceeded),
//!   超限以 reason=budget 结束; 每次用量即时记录
//!
//! 协议 (纯文本 JSON, 兼容任意 OpenAI 兼容网关, 无需原生 function calling):
//!   模型输出 {"action":"tool","name":..,"args":{..},"reason":".."} 或
//!           {"action":"final","answer":".."}
//!   Rust 校验参数 → 执行 (见 tools.rs) → 观察结果作为下一条 user 消息写回。

use serde::Serialize;
use serde_json::{json, Value};

use crate::agent::tools::{tool_definitions, AgentBoard};
use crate::engine::{board_summary, run_local_pipeline};
use crate::llm::usage::UsageStore;
use crate::llm::{ChatResult, TokenUsage};

/// 网关未返回 usage 时的粗略估算 (中文约 0.5-1 token/字, 取中间值)。
/// 仅用于前端回显, 不进入用量统计库。
pub fn estimate_token_usage(prompt_chars: usize, completion_chars: usize) -> TokenUsage {
    TokenUsage {
        prompt_tokens: ((prompt_chars as u64 * 3) / 4) as u32,
        completion_tokens: ((completion_chars as u64 * 3) / 4) as u32,
        total_tokens: (((prompt_chars + completion_chars) as u64 * 3) / 4) as u32,
    }
}

/// 对话能力抽象 (便于单元测试注入假 LLM)。
/// 显式返回 Send Future, 满足 axum handler 的 Send 约束。
pub trait ChatCall {
    fn model_label(&self) -> (String, String);
    #[allow(clippy::type_complexity)]
    fn call(
        &self,
        messages: Vec<Value>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ChatResult, Box<dyn std::error::Error>>> + Send + '_>,
    >;
}

impl ChatCall for crate::llm::LLMClient {
    fn model_label(&self) -> (String, String) {
        (
            self.config().model.clone(),
            self.config().provider_label(),
        )
    }

    #[allow(clippy::type_complexity)]
    fn call(
        &self,
        messages: Vec<Value>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ChatResult, Box<dyn std::error::Error>>> + Send + '_>,
    > {
        Box::pin(async move { self.chat_messages(messages).await })
    }
}

/// 单步轨迹 (前端时间线展示)
#[derive(Debug, Clone, Serialize)]
pub struct AgentStep {
    pub step: u32,
    /// 模型在 JSON 中声明的理由 (≤30 字引导)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// 模型完整回复 (原样, 排查用)
    pub reply: String,
    /// 执行的动作摘要, 如 flag_cell(28,12)
    pub action: String,
    /// 工具观察结果 (截断)
    pub observation: String,
}

/// 循环结束原因
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentEndReason {
    /// 模型给出最终回答
    Final,
    /// 达到最大步数
    MaxSteps,
    /// 整体时间预算耗尽
    Timeout,
    /// token 月度限额
    Budget,
    /// LLM 调用连续失败
    LlmError,
}

/// 一次 Agent 运行的完整结果
#[derive(Debug, Clone, Serialize)]
pub struct AgentRunOutcome {
    pub success: bool,
    pub steps: Vec<AgentStep>,
    pub final_answer: String,
    pub reason: AgentEndReason,
    /// 工作棋盘 (已包含 agent 的标旗操作), 供前端载入
    pub board: Vec<Vec<i32>>,
    pub remaining_mines: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// 各步 token 用量 (累计)
    pub total_tokens: u32,
}

/// Agent 系统提示词: 角色 + 工具清单 + 协议 + 知识库指针 + 纪律
pub fn build_agent_system_prompt() -> String {
    let tools = serde_json::to_string_pretty(&tool_definitions()).unwrap_or_default();
    format!(
        "你是一个扫雷求解 Agent (操作器), 不是聊天助手。你通过调用工具来分析并逐步解决残局; \
         你无法直接访问棋盘, 所有信息必须来自工具返回的观察结果。\n\n\
【可用工具】\n{}\n\n\
【知识库】有 get_knowledge 工具可查询扫雷模式 (1-2-1 / 子集 / 组合枚举 等)。需要引用模式时先查询。\n\n\
【工作流程 (Skills)】\n\
1. 先用 analyze_board 了解局面 (或直接利用上一轮工具结果)。\n\
2. 对每个确定性结论: 必雷未标旗 → flag_cell; 必安全未翻开 → 记入候选并准备最终建议。\n\
3. 拿不准时用 get_constraints / verify_flag / check_safe / list_regions 取证。\n\
4. 全部可证明操作完成后, 输出最终回答 (建议玩家点开的安全格、需移除的矛盾旗、剩余风险与概率判断)。\n\n\
【输出协议】(每轮只输出一个 JSON, 不要输出 JSON 以外的任何文字, 不要思考自白)\n\
- 调用工具: {{\"action\":\"tool\",\"name\":\"flag_cell\",\"args\":{{\"x\":28,\"y\":12}},\"reason\":\"数字3@(27,11)剩1雷,该格唯一\"}}\n\
- 完成任务: {{\"action\":\"final\",\"answer\":\"建议先点击 (29,13) ...\"}}\n\n\
【硬性约束】\n\
1. 语言: 全程简体中文 (reason/answer 用中文, 工具名与坐标用英文/数字)。\n\
2. 每次只调用一个工具; reason 不超过 30 字。\n\
3. 禁止编造观察结果; 局面信息只能来自工具返回。\n\
4. flag_cell 只对工具结果中标注\"必雷(未标旗)\"的格执行; 被拒绝后不得原样重试同一格。\n\
5. 不要重复执行已完成的动作或查询同样的内容; 若上一轮工具报\"错误\", 先修正再继续。\n\
6. 当 analyze_board 显示没有新的必雷/必安全结论, 或已标完所有可证必雷格时, 立即输出 final。\n\
7. 不要把概率当结论: 只有引擎给出的\"必雷/必安全\"才能操作; 其余只写进 final 的风险提示。\n\n\
【输出纪律】\n\
禁止 \"Let me\" / \"I think\" / \"我先分析\" 等思考过程; 只输出 JSON。",
        tools
    )
}

/// 模型单轮回复的解析结果
#[derive(Debug, Clone, PartialEq)]
enum Decision {
    ToolCall { name: String, args: Value, reason: Option<String> },
    Final(String),
    /// 需要把"格式错误"作为观察喂回去
    Malformed(String),
}

/// 解析模型回复 (容忍 ```json 包裹 / 前后缀废话, 但不接受非 JSON 的"聊天"回复)
fn parse_decision(reply: &str) -> Decision {
    let t = reply.trim();
    let json_str: Option<&str> = if t.starts_with('{') {
        Some(t)
    } else {
        crate::llm::vision::extract_json_from_text(t)
    };
    let Some(json_str) = json_str else {
        // 无 JSON → 视为闲聊, 告知必须 JSON
        let head: String = t.chars().take(120).collect();
        return Decision::Malformed(format!("未找到 JSON 协议对象, 收到文本: {}", head));
    };
    let Ok(v) = serde_json::from_str::<Value>(json_str) else {
        return Decision::Malformed("JSON 解析失败, 请严格按协议输出".to_string());
    };
    let action = v
        .get("action")
        .and_then(|a| a.as_str())
        .unwrap_or_default();
    match action {
        "tool" => {
            let name = v
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or_default()
                .to_string();
            let args = v.get("args").cloned().unwrap_or(json!({}));
            if name.is_empty() {
                return Decision::Malformed("tool 调用缺少 name 字段".to_string());
            }
            let reason = v.get("reason").and_then(|r| r.as_str()).map(String::from);
            Decision::ToolCall { name, args, reason }
        }
        "final" => {
            let answer = v
                .get("answer")
                .and_then(|a| a.as_str())
                .unwrap_or_default()
                .to_string();
            if answer.trim().is_empty() {
                Decision::Malformed("final 缺少 answer 字段".to_string())
            } else {
                Decision::Final(answer)
            }
        }
        other => Decision::Malformed(format!(
            "action 字段应为 \"tool\" 或 \"final\", 实际为 {:?}",
            other
        )),
    }
}

/// Agent 运行参数
pub struct AgentLoopConfig {
    pub max_steps: u32,
    pub time_budget_secs: u64,
}

impl Default for AgentLoopConfig {
    fn default() -> Self {
        // 最大步数: 环境变量 AGENT_MAX_STEPS, 默认 8
        let max_steps = std::env::var("AGENT_MAX_STEPS")
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(8)
            .clamp(1, 15);
        Self { max_steps, time_budget_secs: 180 }
    }
}

/// ReAct 主循环
pub async fn run_agent_loop<C: ChatCall>(
    chat: &C,
    initial_board: Vec<Vec<i32>>,
    remaining_mines: u32,
    goal: &str,
    cfg: &AgentLoopConfig,
    usage_store: Option<&UsageStore>,
) -> AgentRunOutcome {
    let started = std::time::Instant::now();
    let mut board = initial_board.clone();
    let mut mines = remaining_mines;
    let mut steps: Vec<AgentStep> = Vec::new();
    let mut total_tokens: u32 = 0;

    let finish = |success: bool,
                  steps: Vec<AgentStep>,
                  final_answer: String,
                  reason: AgentEndReason,
                  board: Vec<Vec<i32>>,
                  mines: u32,
                  total: u32,
                  error: Option<String>| -> AgentRunOutcome {
        AgentRunOutcome {
            success,
            steps,
            final_answer,
            reason,
            board,
            remaining_mines: mines,
            total_tokens: total,
            error,
        }
    };

    let system_prompt = build_agent_system_prompt();

    // 历史: [(模型回复, 观察结果)]
    let mut history: Vec<(String, String)> = Vec::new();

    for step_no in 1..=cfg.max_steps {
        // ---- 超时预算 ----
        if started.elapsed().as_secs() >= cfg.time_budget_secs {
            return finish(
                true, steps,
                "⏱️ Agent 运行超过时间预算, 已停止。请重新运行或缩小目标。".into(),
                AgentEndReason::Timeout, board, mines, total_tokens, None,
            );
        }
        // ---- 费用上限 ----
        if let Some(store) = usage_store {
            if store.limit_exceeded() {
                let local = summarize_locally(&board, mines);
                return finish(
                    true, steps,
                    format!("⚠️ 本月 token 限额已到, Agent 提前停止。\n\n{}", local),
                    AgentEndReason::Budget, board, mines, total_tokens, None,
                );
            }
        }

        // ---- 组消息: system + 任务开头 + 历史(截断到最近 6 对) + 请继续 ----
        let mut messages: Vec<Value> = vec![json!({"role": "system", "content": system_prompt})];
        // 开局引导 (含当前棋盘摘要)
        let (view0, ir0) = match run_local_pipeline(&board, mines) {
            Ok(x) => x,
            Err(e) => {
                return finish(false, steps, String::new(), AgentEndReason::LlmError, board, mines, total_tokens, Some(e))
            }
        };
        let opening = format!(
            "【任务】{}\n\n【当前局面】{}\n\n请输出你的决策 JSON。{}",
            goal,
            board_summary(&view0, &ir0),
            if history.is_empty() { "首次调用建议先 analyze_board。" } else { "" }
        );
        messages.push(json!({"role": "user", "content": opening}));
        for (i, (reply, obs)) in history.iter().enumerate() {
            if history.len() > 6 && i < history.len() - 6 {
                continue; // 只保留最近 6 对, 防止上下文无限膨胀
            }
            messages.push(json!({"role": "assistant", "content": reply}));
            messages.push(json!({"role": "user", "content": obs}));
        }
        messages.push(json!({
            "role": "user",
            "content": "请基于上述信息继续 (输出决策 JSON)。若已无确定性操作可做, 直接输出 final。"
        }));

        // ---- 调用模型 (错误先转 String, 避免非 Send 错误盒跨 await) ----
        let call_outcome: Result<String, String> = match chat.call(messages.clone()).await {
            Ok(r) => {
                if let Some(u) = &r.usage {
                    if let Some(store) = usage_store {
                        let (model, provider) = chat.model_label();
                        store.record(&model, &provider, u);
                    }
                    total_tokens = total_tokens.saturating_add(u.total_tokens);
                }
                Ok(r.content)
            }
            Err(e) => Err(e.to_string()),
        };
        let reply = match call_outcome {
            Ok(content) => content,
            Err(msg) => {
                if step_no == 1 {
                    // 首轮瞬态失败: 立即重试一次 (与分析路径一致); 仍失败才停止
                    match chat.call(messages).await {
                        Ok(r) => {
                            if let Some(u) = &r.usage {
                                if let Some(store) = usage_store {
                                    let (model, provider) = chat.model_label();
                                    store.record(&model, &provider, u);
                                }
                                total_tokens = total_tokens.saturating_add(u.total_tokens);
                            }
                            r.content
                        }
                        Err(e2) => {
                            return finish(
                                false, steps, String::new(), AgentEndReason::LlmError, board,
                                mines, total_tokens, Some(format!("LLM 调用失败: {}", e2)),
                            );
                        }
                    }
                } else {
                    // 后续轮次出错: 尝试最后一次本地兜底回答
                    let local = summarize_locally(&board, mines);
                    return finish(
                        true, steps,
                        format!("⚠️ Agent 循环在第 {} 步 LLM 调用失败, 已停止: {}\n\n{}", step_no, msg, local),
                        AgentEndReason::LlmError, board, mines, total_tokens, None,
                    );
                }
            }
        };
        let reply_trim = reply.trim().to_string();

        // ---- 解析决策 ----
        let decision = parse_decision(&reply_trim);
        match decision {
            Decision::Final(answer) => {
                return finish(
                    true, steps, answer.clone(), AgentEndReason::Final, board, mines,
                    total_tokens, None,
                );
            }
            Decision::Malformed(problem) => {
                history.push((
                    reply_trim.clone(),
                    format!("【协议错误】{}。请重新输出严格 JSON 决策。", problem),
                ));
                steps.push(AgentStep {
                    step: step_no,
                    reason: Some("格式错误".into()),
                    reply: reply_trim.clone(),
                    action: "!格式错误".into(),
                    observation: problem.clone(),
                });
                continue;
            }
            Decision::ToolCall { name, args, reason } => {
                // ---- 执行 (校验失败也作为观察返回, 让模型修正) ----
                let mut agent = AgentBoard::new(board.clone(), mines);
                let result = crate::agent::tools::execute_tool(&mut agent, &name, &args);
                match result {
                    Ok(obs) => {
                        board = agent.board.clone();
                        mines = agent.remaining_mines;
                        let action_label = describe_call(&name, &args);
                        let obs_capped = cap_chars(&obs, 700);
                        history.push((
                            reply_trim.clone(),
                            format!("【工具结果】{} 成功\n{}", action_label, obs_capped),
                        ));
                        steps.push(AgentStep {
                            step: step_no,
                            reason,
                            reply: reply_trim.clone(),
                            action: action_label.clone(),
                            observation: obs_capped.clone(),
                        });
                    }
                    Err(e) => {
                        let obs = cap_chars(&e, 300);
                        history.push((
                            reply_trim.clone(),
                            format!("【工具结果】{} 被拒绝\n{}", describe_call(&name, &args), obs),
                        ));
                        steps.push(AgentStep {
                            step: step_no,
                            reason,
                            reply: reply_trim.clone(),
                            action: format!("{} (拒绝)", describe_call(&name, &args)),
                            observation: obs.clone(),
                        });
                    }
                }
            }
        }
    }

    // ---- 达到最大步数 ----
    let local = summarize_locally(&board, mines);
    finish(
        true,
        steps,
        format!(
            "⏹️ Agent 已达最大步数 {} 次, 提前结束。\n\n{}",
            cfg.max_steps, local
        ),
        AgentEndReason::MaxSteps,
        board,
        mines,
        total_tokens,
        None,
    )
}

/// 描述一次工具调用, 如 flag_cell(28, 12)
fn describe_call(name: &str, args: &Value) -> String {
    let x = args.get("x").and_then(|v| v.as_u64());
    let y = args.get("y").and_then(|v| v.as_u64());
    match (x, y) {
        (Some(x), Some(y)) => format!("{}({}, {})", name, x, y),
        _ => name.to_string(),
    }
}

/// 两个工具调用是否针对同一格 (去重判断)
fn same_cell_args(a: &Value, b: &Value) -> bool {
    let ax = a.get("x").and_then(|v| v.as_u64());
    let ay = a.get("y").and_then(|v| v.as_u64());
    let bx = b.get("x").and_then(|v| v.as_u64());
    let by = b.get("y").and_then(|v| v.as_u64());
    match (ax, ay, bx, by) {
        (Some(ax), Some(ay), Some(bx), Some(by)) => ax == bx && ay == by,
        _ => a == b,
    }
}

fn cap_chars(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n).collect::<String>())
    }
}

/// 本地兜底总结 (预算/超时/最大步数/LLM 失败时给出可读结论)
fn summarize_locally(board: &[Vec<i32>], mines: u32) -> String {
    match run_local_pipeline(board, mines) {
        Ok((view, ir)) => {
            let t = crate::llm::Translator::new(crate::llm::LLMMode::Answer);
            format!(
                "{}\n\n{}",
                crate::engine::board_summary(&view, &ir),
                t.local_translate(&ir)
            )
        }
        Err(e) => format!("本地总结失败: {}", e),
    }
}

// ---------------------------------------------------------------------------
// 分析型 Agent — 应用于「分析局面」(答案模式 + 调用 LLM)
// ---------------------------------------------------------------------------
// LLM 不接收完整 IR, 而是通过与本地推理库工具 (analyze_board / get_constraints /
// verify_flag / check_safe / list_regions / get_knowledge) 逐轮交互来理解局面,
// 最后产出一段简洁、自然、有重点的讲解 — 不逐字罗列工具输出。

/// 分析型 Agent 的返回
pub struct AnalysisOutcome {
    pub analysis: String,
    /// 工作棋盘 (Agent 成功标旗后写回; 若未标旗则与原棋盘一致)
    pub board: Vec<Vec<i32>>,
    pub remaining_mines: u32,
    pub steps: u32,
    /// 真实用量合计; 网关缺失时由调用方估算
    pub real_usage: Option<TokenUsage>,
    /// 累计输入/输出字符 (用于估算与诊断)
    pub prompt_chars: usize,
    pub completion_chars: usize,
    /// 结束原因
    pub reason: AgentEndReason,
    /// 步骤轨迹 (供调试/时间线; 默认不展示给普通分析)
    pub trace: Vec<AgentStep>,
}

#[derive(Clone, Copy)]
pub struct AnalysisAgentConfig {
    pub max_steps: u32,
    pub time_budget_secs: u64,
    /// 是否允许对"引擎证明的必雷格"执行 flag_cell (写回工作棋盘)
    pub allow_flagging: bool,
    /// final 过长 (超过此字符数) 时追加一次"压缩重写"
    pub compress_threshold: usize,
}

impl Default for AnalysisAgentConfig {
    fn default() -> Self {
        Self {
            max_steps: std::env::var("AGENT_ANALYSIS_STEPS")
                .ok()
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(6)
                .clamp(3, 12),
            time_budget_secs: 150,
            allow_flagging: true,
            compress_threshold: 1200,
        }
    }
}

/// 分析型 Agent 系统提示词 — 人味、简洁、不罗列
pub fn build_analysis_system_prompt(english: bool) -> String {
    let lang_line = if english {
        "Language: answer the final report in English (tools/replies stay as-is)."
    } else {
        "语言: 全程简体中文。"
    };
    let spec = if english {
        r#"You are a minesweeper reasoning Agent. You learn a position by querying the LOCAL ENGINE
LIBRARY through tools, step by step, then explain it to the player like a skilled veteran —
concise, natural, and focused. Tool observations are your INTERNAL material: never copy or
enumerate them verbatim, never say "according to IR/tools".

Tools (one per turn, reason <=30 chars): analyze_board / flag_cell(x,y) only for engine-proven
mines / check_safe(x,y) / explain_cell(x,y) get the full verifiable material for a cell
(engine verdict + local constraint table with remaining mines and candidate lists) —
call it BEFORE writing a final-verdict reasoning chain / verify_flag(x,y) /
get_constraints(x,y) / list_regions / get_knowledge(topic).

Typical flow: analyze_board → verify the 1-2 most interesting cells (verify_flag / check_safe /
explain_cell / get_knowledge) → final.

FINAL REPORT RULES (the only text the player sees; total <=260 words):
  - Tone: like a veteran player coaching you; no machine-translation flavor.
  - KNOWLEDGE TIERS: verified flags are ALREADY-KNOWN conditions, never "findings". A finding is
    ONLY an engine-proven mine that is NOT yet flagged, or a not-yet-revealed safe cell, or a
    contradicted flag. If the tool report shows NO such finding, write one sentence
    "No new deterministic conclusion this turn" and move on to strategy.
  - When there is NO new finding, use this shape (skip the reasoning chain):
    Position summary (1-2 sentences incl. mines/unknowns/flag check result in ONE sentence) /
    Strategy (top region or top 2-3 probability cells with mine% and why) / Move suggestion (1-2).
  - When there IS a finding, give a VERIFIABLE CHAIN, numbered steps "Step 1 / Step 2 ..." (2-5
    steps). Each step MUST cite concrete numbers, coordinates and candidate counts that match the
    explain_cell constraint table, e.g.:
      Step 1: digit 3@(27,11) has 1 flag, needs 2, 5 candidates (28,10)(28,11)(26,12)(27,12)(28,12)
      → exactly 2 mines in those 5.
      Step 2: digit 2@(28,9) needs 1 over (28,10)(29,10) → ... → one cell is forced safe.
    Then end the report with one line: Final verdict: (x,y) is a mine OR is NOT a mine.
  - FORBIDDEN vague words in chains: "combining surrounding constraints", "via elimination",
    "overall analysis" — every step must name the exact digits/cells and counts used.
  - NEVER enumerate constraints, flag-verification proofs, or tool observations outside the chain;
    never repeat a conclusion twice; total <=260 words including the chain.

Protocol (reply JSON only, no extra words): {action:"tool",name,args,reason} or
{action:"final",answer:"<final report>"}. No thinking aloud."#
    } else {
        r#"你是扫雷推理 Agent。你要通过与"本地推理库"工具逐轮交互来弄清局面, 最后像一位资深玩家
那样给玩家一段简洁、自然、有重点的讲解。工具的观察结果是你的内部素材: 严禁逐字转抄或罗列,
严禁说"根据工具结果/IR 显示"这类话, 严禁把每条约束都念一遍。

【工具】每轮只调一个, reason ≤30 字:
- analyze_board 全局面推理摘要 (只调一次; 棋盘未变化时重复调用会被拒绝)
- flag_cell(x,y) 仅对引擎证明必雷且未标旗的格执行 (会推进局面)
- check_safe(x,y) 查某格必雷/必安全/概率 (同格只查一次)
- explain_cell(x,y) 获取某格的完整可验证素材: 引擎判定 + 局部约束表 (每个相关数字的
  旗数/剩余需雷数/候选格)。要在最终回答给出某格"是雷/不是雷"的分步推理前, 先调用它;
  之后每个步骤的数字与候选格都必须与这张表一致 (同格只查一次)
- verify_flag(x,y) 查旗帜是否矛盾 (同格只查一次)
- get_constraints(x,y) 查数字约束细节 (同格只查一次)
- list_regions 分区概览
- get_knowledge(topic) 扫雷知识库 (1-2-1/子集/组合枚举…)

【典型流程】analyze_board 总览 → 挑 1-2 个最值得讲的点取证 (explain_cell / verify_flag /
check_safe) → final。

【结论分级】(最重要)
- 新发现 = 引擎报告的"必雷未标旗" / "必安全未翻开" / 矛盾旗。
- 已标旗且验证通过的雷是"已知条件": 不逐条取证, 不写进"新发现", 摘要一句带过。
- 若 analyze_board 显示"本次无新增确定性结论", 不罗列旧结论, 直接转策略分析。

【最终回答规范】(玩家唯一看到的文本)
有新发现、需要给出某格结论时 — 输出"可验证的分步推理链" (全文 ≤600 字):
  用「第 1 步」「第 2 步」… 编号 (2-5 步), 每步必须写清三件事:
    · 哪个数字: 数字N@(a,b) 已有几旗、还剩几雷、候选格是哪几个 (来自 explain_cell 的局部约束表)
    · 推导: 候选格共几格放几雷 / 与上一步的候选关系 (共享格、包含、互斥)
    · 结论: 本步推出什么 (某格必雷/必安全/某区域恰有 k 雷)
  示例风格:
    第 1 步: 数字3@(27,11) 剩2雷, 5个候选格 (28,10)(28,11)(26,12)(27,12)(28,12) 中恰有2雷。
    第 2 步: 数字2@(28,9) 剩1雷, 候选 (28,10)(29,10); 其中 (28,10) 与第1步共享, 故 (28,10)(28,11) 恰1雷…
    第 3 步: 于是 (26,12)(27,12)(28,12) 中还有1雷; 又 (26,12)(27,12) 恰1雷 (数字1@(25,12)…) → (28,12) 被排除。
  最后单独一行: 最终结论: (x,y) 是雷  或  最终结论: (x,y) 不是雷
  铁律: 每步的"数字、旗数、剩余雷、候选格"必须与 explain_cell 返回的约束表一致;
        严禁"联动排除/结合周边约束综合分析/排除后可知"等省略推导过程的黑话;
        严禁编造约束表里不存在的数字或候选格。

无新发现时 (全文 ≤350 字, 不要硬凑推理链):
  🎯 局面摘要 (1-2 句)
  📊 策略分析 (最多 2 个区域或 2-3 个概率格)
  💡 决策建议 (1-2 条)

铁律: 禁止在最终回答中罗列约束明细、旗标验证过程、或任何工具观察原文; 同一结论不得重复。

【协议】每轮只输出一个 JSON: {"action":"tool","name":"...","args":{...},"reason":"..."}
或 {"action":"final","answer":"<最终回答全文>"}。不要 JSON 以外的文字, 不要思考自白。"#
    };
    format!(
        "{}\n\n{}\n\n【可用工具】\n{}",
        lang_line, spec, serde_json::to_string_pretty(&tool_definitions()).unwrap_or_default()
    )
}

/// 运行"分析型 Agent 循环"。
///
/// 返回 AnalysisOutcome; 失败 (LLM 首轮不可用等) 时 reason=LlmError 且 analysis 为空,
/// 由调用方回退本地转译。
pub async fn run_analysis_agent<C: ChatCall>(
    chat: &C,
    initial_board: Vec<Vec<i32>>,
    remaining_mines: u32,
    question: Option<&str>,
    english: bool,
    cfg: &AnalysisAgentConfig,
    usage_store: Option<&UsageStore>,
) -> AnalysisOutcome {
    let started = std::time::Instant::now();
    let mut agent = AgentBoard::new(initial_board, remaining_mines);
    let mut trace: Vec<AgentStep> = Vec::new();
    let mut history: Vec<(String, String)> = Vec::new();
    let mut real_usage: Option<TokenUsage> = None;
    let mut prompt_chars = 0usize;
    let mut completion_chars = 0usize;
    let mut steps_done: u32 = 0;

    let acc_usage = |u: &TokenUsage, real: &mut Option<TokenUsage>| {
        let t = real.get_or_insert_with(TokenUsage::default);
        t.prompt_tokens = t.prompt_tokens.saturating_add(u.prompt_tokens);
        t.completion_tokens = t.completion_tokens.saturating_add(u.completion_tokens);
        t.total_tokens = t.total_tokens.saturating_add(u.total_tokens);
    };

    let question_block = question
        .filter(|q| !q.trim().is_empty())
        .map(|q| format!("\n\n【用户提问】{}", q))
        .unwrap_or_default();

    let system_prompt = build_analysis_system_prompt(english);

    // 防空转/去重: analyze_board 状态指纹; 已成功执行的查询随指纹失效
    let mut last_analysis_fp: Option<u64> = None;
    let mut executed: Vec<(String, u64, Value)> = Vec::new();

    for step_no in 1..=cfg.max_steps {
        steps_done = step_no;
        if started.elapsed().as_secs() >= cfg.time_budget_secs {
            break;
        }
        if let Some(store) = usage_store {
            if store.limit_exceeded() {
                break;
            }
        }
        let (view0, ir0) = match run_local_pipeline(&agent.board, agent.remaining_mines) {
            Ok(x) => x,
            Err(_) => break,
        };
        let summary = board_summary(&view0, &ir0);

        // 首轮提示词给出任务与抽查指引; 之后提示继续 (压缩轮走专用消息)
        let user_content = if step_no == 1 {
            format!(
                "【任务】分析当前残局并给出讲解。{}用工具逐步核实, 不要臆测。\n\n\
                 【当前局面】{}\n\n请输出你的决策 JSON。",
                question_block, summary
            )
        } else {
            "请基于最新工具结果继续。若已理解关键点, 输出 final。".to_string()
        };

        // 组消息: system + 首轮任务 + 最近历史对 + 本轮回合提示
        let mut messages: Vec<Value> =
            vec![json!({"role": "system", "content": system_prompt})];
        messages.push(json!({"role": "user", "content": user_content}));
        for (i, (rep, obs)) in history.iter().enumerate() {
            if history.len() > 6 && i < history.len() - 6 {
                continue;
            }
            messages.push(json!({"role": "assistant", "content": rep}));
            messages.push(json!({"role": "user", "content": obs}));
        }
        messages.push(json!({"role": "user", "content": "请继续 (输出决策 JSON)。"}));
        for m in &messages {
            prompt_chars += m
                .get("content")
                .and_then(|c| c.as_str())
                .map(|s| s.chars().count())
                .unwrap_or(0);
        }

        let call_outcome: Result<String, String> = match chat.call(messages.clone()).await {
            Ok(r) => {
                if let Some(u) = &r.usage {
                    if let Some(store) = usage_store {
                        let (model, provider) = chat.model_label();
                        store.record(&model, &provider, u);
                    }
                    acc_usage(u, &mut real_usage);
                }
                completion_chars += r.content.chars().count();
                Ok(r.content)
            }
            Err(e) => Err(e.to_string()),
        };
        let reply = match call_outcome {
            Ok(c) => c,
            Err(_msg) => {
                if step_no == 1 {
                    // 首轮失败重试一次
                    match chat.call(messages).await {
                        Ok(r) => {
                            if let Some(u) = &r.usage {
                                if let Some(store) = usage_store {
                                    let (model, provider) = chat.model_label();
                                    store.record(&model, &provider, u);
                                }
                                acc_usage(u, &mut real_usage);
                            }
                            completion_chars += r.content.chars().count();
                            r.content
                        }
                        Err(_) => {
                            return AnalysisOutcome {
                                analysis: String::new(),
                                board: agent.board.clone(),
                                remaining_mines: agent.remaining_mines,
                                steps: 0,
                                real_usage,
                                prompt_chars,
                                completion_chars,
                                reason: AgentEndReason::LlmError,
                                trace,
                            };
                        }
                    }
                } else {
                    break; // 中途失败: 用已收集内容兜底 (见末尾)
                }
            }
        };
        let reply_trim = reply.trim().to_string();
        let decision = parse_decision(&reply_trim);
        match decision {
            Decision::Final(answer) => {
                let answer = answer.trim().to_string();
                let mut final_text = answer.clone();
                // 过长 → 追加一次压缩重写 (Final 分支随即返回, 天然只执行一次)
                if final_text.chars().count() > cfg.compress_threshold {
                    // 只把前 1500 字给压缩器 (防原文回流导致再次超长)
                    let mut preview: String = final_text.chars().take(1500).collect();
                    if let Some(vi) = final_text.find("最终结论:") {
                        let v: String = final_text[vi..].chars().take(80).collect();
                        if !preview.contains("最终结论:") {
                            preview.push_str("\n…(结尾原结论行)\n");
                            preview.push_str(&v);
                        }
                    }
                    let compress_prompt = if english {
                        format!(
                            "The previous final report is too long / too mechanical. Rewrite it as a \
                             concise veteran-style report (at most 220 words): position summary (1-2 sentences), \
                             ONE key reasoning chain (3-6 arrows), up to 3 move suggestions. NEVER enumerate \
                             constraints, flag proofs or tool observations. Output ONLY the rewritten text, no JSON.\n\nPrevious (first 1500 chars):\n{}",
                            preview
                        )
                    } else {
                        format!(
                            "你上一条最终回答过长且偏向机械罗列。请压缩为 ≤350 字、资深玩家口吻的讲解: \
                             局势要点 1-2 句 + 一条关键推理链 (3-6 步) + ≤3 条行动建议; \
                             严禁罗列约束/旗标证明/工具观察; 若原文有「最终结论」行, 结尾必须原样保留一行。\
                             只输出改写后的文本, 不要 JSON。\n\n上一条 (前1500字):\n{}",
                            preview
                        )
                    };
                    let msgs = vec![
                        json!({"role": "system", "content": system_prompt}),
                        json!({"role": "user", "content": compress_prompt}),
                    ];
                    if let Ok(r) = chat.call(msgs.clone()).await {
                        if let Some(u) = &r.usage {
                            if let Some(store) = usage_store {
                                let (model, provider) = chat.model_label();
                                store.record(&model, &provider, u);
                            }
                            acc_usage(u, &mut real_usage);
                        }
                        completion_chars += r.content.chars().count();
                        let c = r.content.trim();
                        if !c.is_empty() {
                            final_text = c.to_string();
                        }
                    }
                    // 压缩仍超长 → 硬截断 (保留结论行)
                    if final_text.chars().count() > 1300 {
                        final_text =
                            crate::llm::translator::truncate_llm_output(&final_text, 1000);
                    }
                }
                trace.push(AgentStep {
                    step: step_no,
                    reason: None,
                    reply: reply_trim.clone(),
                    action: "final".into(),
                    observation: cap_chars(&final_text, 200),
                });
                return AnalysisOutcome {
                    analysis: final_text,
                    board: agent.board.clone(),
                    remaining_mines: agent.remaining_mines,
                    steps: step_no,
                    real_usage,
                    prompt_chars,
                    completion_chars,
                    reason: AgentEndReason::Final,
                    trace,
                };
            }
            Decision::Malformed(problem) => {
                history.push((
                    reply_trim.clone(),
                    format!("【协议错误】{}。请重新输出严格 JSON 决策。", problem),
                ));
                continue;
            }
            Decision::ToolCall { name, args, reason } => {
                // flag_cell 仅在允许时执行, 否则当作建议由 final 表达
                if name == "flag_cell" && !cfg.allow_flagging {
                    history.push((
                        reply_trim.clone(),
                        "【工具结果】flag_cell 在分析模式下被禁用; 把标旗建议写进 final 回答即可。"
                            .to_string(),
                    ));
                    continue;
                }
                // 防空转: 棋盘未变化时重复 analyze_board → 拒绝 (2.5 节)
                let fp_now = crate::agent::tools::board_fingerprint(
                    &agent.board,
                    agent.remaining_mines,
                );
                if name == "analyze_board" && last_analysis_fp == Some(fp_now) {
                    let obs = "analyze_board 已在本轮执行过且棋盘未变化 (没有新的标旗/编辑), 重复分析只会浪费时间。\
                               请直接输出 final, 或对「必雷(未标旗)」格执行 flag_cell 后再重新分析。"
                        .to_string();
                    history.push((
                        reply_trim.clone(),
                        format!("【工具结果】analyze_board 被拒绝\n{}", obs),
                    ));
                    trace.push(AgentStep {
                        step: step_no,
                        reason,
                        reply: reply_trim.clone(),
                        action: "analyze_board (去重拒绝)".into(),
                        observation: obs,
                    });
                    continue;
                }
                // 去重: 同一指纹下重复的查询 (verify_flag/get_constraints/check_safe 同格) → 拒绝
                if matches!(name.as_str(), "verify_flag" | "get_constraints" | "check_safe" | "explain_cell") {
                    let dup = executed.iter().any(|(n, fp, a)| {
                        n == &name && *fp == fp_now && same_cell_args(a, &args)
                    });
                    if dup {
                        let obs = format!(
                            "{} 已在本轮对同一格查询过 (棋盘未变化), 结果不变。请基于已有观察继续, 不要重复查询。",
                            name
                        );
                        history.push((
                            reply_trim.clone(),
                            format!("【工具结果】{} 被拒绝\n{}", name, obs),
                        ));
                        trace.push(AgentStep {
                            step: step_no,
                            reason,
                            reply: reply_trim.clone(),
                            action: format!("{} (去重拒绝)", name),
                            observation: obs,
                        });
                        continue;
                    }
                }
                let result = crate::agent::tools::execute_tool(&mut agent, &name, &args);
                match result {
                    Ok(obs) => {
                        // 成功执行后登记 (analyze_board 更新指纹)
                        executed.push((name.clone(), fp_now, args.clone()));
                        if name == "analyze_board" {
                            last_analysis_fp = Some(fp_now);
                        }
                        let label = describe_call(&name, &args);
                        let obs_c = cap_chars(&obs, if name == "explain_cell" { 1400 } else { 500 });
                        history.push((
                            reply_trim.clone(),
                            format!("【工具结果】{}\n{}", label, obs_c),
                        ));
                        trace.push(AgentStep {
                            step: step_no,
                            reason,
                            reply: reply_trim.clone(),
                            action: label.clone(),
                            observation: obs_c.clone(),
                        });
                    }
                    Err(e) => {
                        let obs = cap_chars(&e, 200);
                        history.push((
                            reply_trim.clone(),
                            format!("【工具结果】{} 被拒绝\n{}", describe_call(&name, &args), obs),
                        ));
                        trace.push(AgentStep {
                            step: step_no,
                            reason,
                            reply: reply_trim.clone(),
                            action: format!("{} (拒绝)", describe_call(&name, &args)),
                            observation: obs.clone(),
                        });
                    }
                }
            }
        }
    }

    // 未 final 结束: 用本地转译兜底生成一段简洁分析
    let analysis = summarize_locally(&agent.board, agent.remaining_mines);
    AnalysisOutcome {
        analysis: format!(
            "⚠️ Agent 分析达到停止条件 (步数 {} / 时间 / 限额), 已给出本地推理摘要:\n\n{}",
            steps_done, analysis
        ),
        board: agent.board.clone(),
        remaining_mines: agent.remaining_mines,
        steps: steps_done,
        real_usage,
        prompt_chars,
        completion_chars,
        reason: AgentEndReason::MaxSteps,
        trace,
    }
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
#[derive(Clone)]
struct FakeChat {
    /// 依次返回的回复 (最后一个会重复)
    replies: Vec<String>,
    calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

#[cfg(test)]
impl FakeChat {
    #[cfg(test)]
    fn new(replies: Vec<String>) -> Self {
        Self { replies, calls: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)) }
    }
}

#[cfg(test)]
#[allow(clippy::type_complexity)]
impl ChatCall for FakeChat {
    fn model_label(&self) -> (String, String) {
        ("fake".into(), "fake".into())
    }
    fn call(
        &self,
        _m: Vec<Value>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ChatResult, Box<dyn std::error::Error>>> + Send + '_>,
    > {
        let calls = self.calls.clone();
        let replies = self.replies.clone();
        Box::pin(async move {
            let i = calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let reply = replies
                .get(i)
                .cloned()
                .unwrap_or_else(|| replies.last().cloned().unwrap_or_default());
            Ok(ChatResult { content: reply, usage: None })
        })
    }
}

#[cfg(test)]
fn test_board() -> Vec<Vec<i32>> {
    // 3x3: (0,0)=3, 其 3 个邻居全未知 → 必雷 x3
    vec![vec![3, -1, -1], vec![-1, -1, -1], vec![-1, -1, -1]]
}

#[tokio::test]
async fn test_parse_decision() {
    let d = parse_decision(r#"{"action":"tool","name":"flag_cell","args":{"x":1,"y":0},"reason":"唯一"}"#);
    assert_eq!(
        d,
        Decision::ToolCall {
            name: "flag_cell".into(),
            args: serde_json::json!({"x":1,"y":0}),
            reason: Some("唯一".into())
        }
    );
    let d2 = parse_decision("```json\n{\"action\":\"final\",\"answer\":\"点 (2,2)\"}\n```");
    assert_eq!(d2, Decision::Final("点 (2,2)".into()));
    // 闲聊文本 → Malformed
    assert!(matches!(parse_decision("好的, 我看看这个局面..."), Decision::Malformed(_)));
    assert!(matches!(parse_decision(r#"{"action":"fly","x":1}"#), Decision::Malformed(_)));
}

#[tokio::test]
async fn test_agent_loop_flags_proven_mines_then_final() {
    // 剧本: 第1步 analyze_board, 第2步 flag_cell(1,0), 第3步 flag_cell(0,1), 第4步 final
    let fake = FakeChat::new(vec![
        r#"{"action":"tool","name":"analyze_board","args":{},"reason":"了解局面"}"#.into(),
        r#"{"action":"tool","name":"flag_cell","args":{"x":1,"y":0},"reason":"数字3剩3雷"}"#.into(),
        r#"{"action":"tool","name":"flag_cell","args":{"x":0,"y":1},"reason":"同理由"}"#.into(),
        r#"{"action":"final","answer":"已标完可证雷, 建议点其余未知格"}"#.into(),
    ]);
    let cfg = AgentLoopConfig { max_steps: 8, time_budget_secs: 60 };
    let out = run_agent_loop(&fake, test_board(), 3, "求解残局", &cfg, None).await;
    assert_eq!(out.reason, crate::agent::loop_::AgentEndReason::Final);
    assert_eq!(out.board[0][1], -2);
    assert_eq!(out.board[1][0], -2);
    assert_eq!(out.remaining_mines, 1);
    assert!(out.final_answer.contains("建议"));
    // 轨迹记录每步动作
    let actions: Vec<&str> = out.steps.iter().map(|s| s.action.as_str()).collect();
    assert_eq!(actions[0], "analyze_board");
    assert!(actions[1].starts_with("flag_cell(1, 0)"));
}

#[tokio::test]
async fn test_agent_loop_max_steps() {
    // 模型只会 flag 同一格? 不: 剧本始终 analyze_board → 重复? 用 check 再 flag… 简单: 全部回 analyze_board
    let fake = FakeChat::new(vec![
        r#"{"action":"tool","name":"analyze_board","args":{},"reason":"看"}"#.into(),
        r#"{"action":"tool","name":"flag_cell","args":{"x":1,"y":0},"reason":"雷"}"#.into(),
    ]);
    let cfg = AgentLoopConfig { max_steps: 3, time_budget_secs: 60 };
    let out = run_agent_loop(&fake, test_board(), 3, "求解", &cfg, None).await;
    assert_eq!(out.reason, AgentEndReason::MaxSteps);
    assert!(out.final_answer.contains("最大步数"));
    assert!(out.board[0][1] == -2); // flag 被执行成功, 状态被写回
}

#[tokio::test]
async fn test_agent_loop_rejects_invalid_flag_and_recovers() {
    // 第1步 flag 已翻开格(0,0) → 被拒; 第2步正确 flag; 第3步 final
    let fake = FakeChat::new(vec![
        r#"{"action":"tool","name":"flag_cell","args":{"x":0,"y":0},"reason":"错试"}"#.into(),
        r#"{"action":"tool","name":"flag_cell","args":{"x":1,"y":0},"reason":"正确"}"#.into(),
        r#"{"action":"final","answer":"完成"}"#.into(),
    ]);
    let cfg = AgentLoopConfig { max_steps: 8, time_budget_secs: 60 };
    let out = run_agent_loop(&fake, test_board(), 3, "求解", &cfg, None).await;
    assert_eq!(out.reason, AgentEndReason::Final);
    // 第一次被拒的记录在轨迹中
    assert!(out.steps[0].action.contains("拒绝"));
    assert_eq!(out.board[0][1], -2);
}

#[tokio::test]
async fn test_analysis_agent_produces_concise_final_and_flags() {
    // 剧本: analyze_board → flag_cell(1,0) → get_constraints(取证) → final (简洁人话)
    let fake = FakeChat::new(vec![
        r#"{"action":"tool","name":"analyze_board","args":{},"reason":"总览"}"#.into(),
        r#"{"action":"tool","name":"flag_cell","args":{"x":1,"y":0},"reason":"必雷"}"#.into(),
        r#"{"action":"tool","name":"get_constraints","args":{"x":1,"y":0},"reason":"核实"}"#.into(),
        r#"{"action":"final","answer":"这棋很简单: (1,0) 已证必雷并标旗。建议点 (2,2)。\n最终结论: (1, 0) 是雷"}"#.into(),
    ]);
    let cfg = AnalysisAgentConfig::default();
    let out = run_analysis_agent(&fake, test_board(), 3, None, false, &cfg, None).await;
    assert_eq!(out.reason, AgentEndReason::Final);
    assert_eq!(out.steps, 4);
    // Agent 标旗写回工作棋盘并扣减剩余雷
    assert_eq!(out.board[0][1], -2);
    assert_eq!(out.remaining_mines, 2);
    // 最终回答保持原样、简洁 (未超压缩阈值), 含最终结论行
    assert!(out.analysis.contains("最终结论: (1, 0) 是雷"));
    assert!(out.analysis.chars().count() < 400);
    // trace: 3 个工具步 + 1 个 final
    assert_eq!(out.trace.len(), 4);
    assert!(out.trace[1].action.starts_with("flag_cell"));
    assert_eq!(out.trace[3].action, "final");
}

#[tokio::test]
async fn test_analysis_agent_compress_when_overlong() {
    // 超长 final (带结论行) → 触发压缩重写调用 (第二次对话), 返回压缩文本
    let long_answer = format!(
        "{}。\n最终结论: (1, 0) 是雷",
        "这是一段故意写得非常啰嗦并且像机械罗列的长文本, 把每一条约束都重复好几遍来测试压缩逻辑的有效性。".repeat(15)
    );
    let fake = FakeChat::new(vec![
        r#"{"action":"tool","name":"analyze_board","args":{},"reason":"看"}"#.into(),
        r#"{"action":"final","answer":"PLACEHOLDER"}"#.into(),
        // 第三次调用 = 压缩请求的响应
        "简洁版: 关键推理已给出。\n最终结论: (1, 0) 是雷".to_string(),
    ]);
    // 修改剧本: 第二个回复放超长 final
    let fake2 = FakeChat {
        replies: vec![
            r#"{"action":"tool","name":"analyze_board","args":{},"reason":"看"}"#.into(),
            serde_json::json!({"action":"final","answer":long_answer.clone()}).to_string(),
            "简洁版: 关键推理已给出。\n最终结论: (1, 0) 是雷".to_string(),
        ],
        calls: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
    };
    let cfg = AnalysisAgentConfig { compress_threshold: 200, ..AnalysisAgentConfig::default() };
    let out = run_analysis_agent(&fake2, test_board(), 3, None, false, &cfg, None).await;
    assert_eq!(out.reason, AgentEndReason::Final);
    assert!(out.analysis.contains("简洁版"));
    assert!(out.analysis.contains("最终结论: (1, 0) 是雷"));
    let _ = fake;
}

#[tokio::test]
async fn test_analysis_agent_no_idle_reanalyze() {
    // 模型连续两次 analyze_board (棋盘未变) → 第二次被去重拒绝; 然后 final
    let fake = FakeChat::new(vec![
        r#"{"action":"tool","name":"analyze_board","args":{},"reason":"第一次"}"#.into(),
        r#"{"action":"tool","name":"analyze_board","args":{},"reason":"空转"}"#.into(),
        r#"{"action":"final","answer":"无新结论, 建议处理低概率格。\n最终结论: (1, 0) 是雷"}"#.into(),
    ]);
    let cfg = AnalysisAgentConfig::default();
    let out = run_analysis_agent(&fake, test_board(), 3, None, false, &cfg, None).await;
    assert_eq!(out.reason, AgentEndReason::Final);
    // 步骤 2 被去重拒绝
    assert!(out.trace[1].action.contains("去重拒绝"));
    assert!(out.analysis.contains("最终结论: (1, 0) 是雷"));
}

#[tokio::test]
async fn test_analysis_agent_dedup_queries_but_allow_after_flag() {
    // get_constraints(1,0) 两次 → 第二次拒绝; flag_cell 推进后同格查询允许 (指纹变化)
    let fake = FakeChat::new(vec![
        r#"{"action":"tool","name":"get_constraints","args":{"x":1,"y":0},"reason":"看"}"#.into(),
        r#"{"action":"tool","name":"get_constraints","args":{"x":1,"y":0},"reason":"重复"}"#.into(),
        r#"{"action":"tool","name":"flag_cell","args":{"x":1,"y":0},"reason":"必雷"}"#.into(),
        r#"{"action":"tool","name":"get_constraints","args":{"x":1,"y":0},"reason":"标旗后再看"}"#.into(),
        r#"{"action":"final","answer":"完成。"}"#.into(),
    ]);
    let cfg = AnalysisAgentConfig::default();
    let out = run_analysis_agent(&fake, test_board(), 3, None, false, &cfg, None).await;
    assert_eq!(out.board[0][1], -2);
    // step2 拒绝, step4 (指纹变化后) 允许 → trace actions
    assert!(out.trace[1].action.contains("去重拒绝"));
    assert!(!out.trace[3].action.contains("拒绝"));
}

// ---------------------------------------------------------------------------
// 自主 ReAct 问答 (React QA) — 面向 /api/chat
// 用户任意提问 → LLM 自行决定查哪些工具、何时停止; Rust 只校验/执行/回写。
// 规则路由 (user_asked_cell) 不再用于决策; LLM 在系统提示词里获得"定点问题"工作流。
// ---------------------------------------------------------------------------

pub struct ReactQaConfig {
    pub max_steps: u32,
    pub time_budget_secs: u64,
}

impl Default for ReactQaConfig {
    fn default() -> Self {
        Self {
            max_steps: std::env::var("AGENT_QA_STEPS")
                .ok()
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(6)
                .clamp(3, 10),
            time_budget_secs: 120,
        }
    }
}

pub struct ReactQaOutcome {
    pub answer: String,
    pub steps: u32,
    pub real_usage: Option<TokenUsage>,
    pub prompt_chars: usize,
    pub completion_chars: usize,
    pub reason: AgentEndReason,
}

/// 自主问答系统提示词 — 定位: 回答问题优先, 只查所需, 可答即答
pub fn build_react_system_prompt(english: bool) -> String {
    let spec = if english {
        r#"You are a minesweeper Q&A Agent. The user asks a QUESTION about the current position.
Decide yourself which tools you need and in which order; stop as soon as you can answer.
Do not re-analyze the whole board unless the question is about the whole board.

Guidance:
- "(x,y) why is it a mine / is it safe / state?" → call get_cell_info(x,y); if a verdict needs
  proving, call explain_cell(x,y) to obtain its local constraint table; then answer.
- "next move?" → call analyze_board once; if no deterministic move, use get_probability on a few
  low-risk cells / list_regions and recommend.
- "how many mines in this area?" → call get_local_board(x,y,radius) and count flags/unknowns.
- Question mentions no coordinates → you still decide (analyze_board / list_regions / ...).

FINAL ANSWER RULES (the only text the user sees; <=350 words):
1. Answer the question directly and first.
2. Use numbered steps (Step 1/2...) only when you prove a verdict; cite the concrete digit@coord,
   flags, remaining mines and candidates that your tools returned. No vague "by elimination".
3. When the question targets one cell and a verdict exists, end with one line:
   Final verdict: (x,y) is a mine OR is NOT a mine; if unprovable, write the probability.
4. Never dump global summaries unless the question asks for them; never repeat tool outputs
   verbatim; never mention the whole-board analysis flow.

Protocol (reply JSON only): {"action":"tool","name":..,"args":{..},"reason":"<=30 chars"} or
{"action":"final","answer":"<final answer>"}. One tool per turn. No thinking aloud."#
    } else {
        r#"你是扫雷问答 Agent。用户就当前局面提出一个问题。由你自己决定需要查询哪些工具、
按什么顺序查、何时足够并直接回答; 不要为了回答问题而重做全盘分析 (除非问题就是关于全局)。

【按问题类型的工作流】
- "(x,y) 为什么是雷 / 安全吗 / 什么状态?" → 先 get_cell_info(x,y) 看状态与引擎结论;
  需要证明时再 explain_cell(x,y) 取局部约束表, 然后作答 (步骤引用表中数字)。
- "下一步怎么走 / 哪个好?" → analyze_board 一次; 无确定性操作时用 get_probability 挑
  低风险格或 list_regions 看区域, 给出建议。
- "这片区域有几颗雷 / 什么布局?" → get_local_board(x,y,radius) 数旗与未知。
- 提问不含坐标 → 由你判断需要什么 (analyze_board / list_regions / get_knowledge…)。

【最终回答规范】(用户唯一看到的文本, ≤400 字)
1. 先直接回答问题, 再给必要依据。
2. 需要证明某格结论时用「第 1 步/第 2 步…」编号 (2-4 步), 每步引用工具返回的
   具体 数字@坐标/旗数/剩余雷/候选格; 禁止"联动排除"式黑话。
3. 问题针对单格且有判定时, 末尾一行: 最终结论: (x,y) 是雷 / 不是雷;
   无法证明时写: 最终结论: (x,y) 无法确定为雷/安全, 雷概率 N%。
4. 除非用户问全局, 否则不输出全局摘要; 不转抄工具原文; 不描述"我调用了哪些工具"。

【协议】每轮只输出一个 JSON: {"action":"tool","name":..,"args":{..},"reason":"≤30字"}
或 {"action":"final","answer":"<最终回答>"}。每次只调一个工具。不要思考自白。"#
    };
    format!(
        "{}\n\n{}\n\n【可用工具】\n{}",
        if english {
            "Language: answer in English."
        } else {
            "语言: 全程简体中文。"
        },
        spec,
        serde_json::to_string_pretty(&tool_definitions()).unwrap_or_default()
    )
}

/// 自主 ReAct 问答主循环
pub async fn run_react_qa<C: ChatCall>(
    chat: &C,
    initial_board: Vec<Vec<i32>>,
    remaining_mines: u32,
    question: &str,
    english: bool,
    cfg: &ReactQaConfig,
    usage_store: Option<&UsageStore>,
) -> ReactQaOutcome {
    let started = std::time::Instant::now();
    let mut agent = AgentBoard::new(initial_board, remaining_mines);
    let mut history: Vec<(String, String)> = Vec::new();
    let mut real_usage: Option<TokenUsage> = None;
    let mut prompt_chars = 0usize;
    let mut completion_chars = 0usize;

    let acc_usage = |u: &TokenUsage, real: &mut Option<TokenUsage>| {
        let t = real.get_or_insert_with(TokenUsage::default);
        t.prompt_tokens = t.prompt_tokens.saturating_add(u.prompt_tokens);
        t.completion_tokens = t.completion_tokens.saturating_add(u.completion_tokens);
        t.total_tokens = t.total_tokens.saturating_add(u.total_tokens);
    };

    let system_prompt = build_react_system_prompt(english);
    let mut last_steps: u32 = 0;

    for step_no in 1..=cfg.max_steps {
        last_steps = step_no;
        if started.elapsed().as_secs() >= cfg.time_budget_secs {
            break;
        }
        if let Some(store) = usage_store {
            if store.limit_exceeded() {
                break;
            }
        }
        let (view0, ir0) = match run_local_pipeline(&agent.board, agent.remaining_mines) {
            Ok(x) => x,
            Err(_) => break,
        };
        let summary = board_summary(&view0, &ir0);
        let opening = format!(
            "【用户提问】{}\n\n【当前局面(一行摘要)】{}\n\n请决定第一个动作 (查询工具或直接 final)。",
            question, summary
        );

        let mut messages: Vec<Value> =
            vec![json!({"role": "system", "content": system_prompt})];
        messages.push(json!({"role": "user", "content": opening}));
        for (i, (rep, obs)) in history.iter().enumerate() {
            if history.len() > 5 && i < history.len() - 5 {
                continue;
            }
            messages.push(json!({"role": "assistant", "content": rep}));
            messages.push(json!({"role": "user", "content": obs}));
        }
        messages.push(json!({"role": "user", "content": "请继续 (决策 JSON)。能回答时请输出 final。"}));
        for m in &messages {
            prompt_chars += m
                .get("content")
                .and_then(|c| c.as_str())
                .map(|s| s.chars().count())
                .unwrap_or(0);
        }

        let call_outcome: Result<String, String> = match chat.call(messages.clone()).await {
            Ok(r) => {
                if let Some(u) = &r.usage {
                    if let Some(store) = usage_store {
                        let (model, provider) = chat.model_label();
                        store.record(&model, &provider, u);
                    }
                    acc_usage(u, &mut real_usage);
                }
                completion_chars += r.content.chars().count();
                Ok(r.content)
            }
            Err(e) => Err(e.to_string()),
        };
        let reply = match call_outcome {
            Ok(c) => c,
            Err(msg) => {
                if step_no == 1 {
                    match chat.call(messages).await {
                        Ok(r) => {
                            if let Some(u) = &r.usage {
                                if let Some(store) = usage_store {
                                    let (model, provider) = chat.model_label();
                                    store.record(&model, &provider, u);
                                }
                                acc_usage(u, &mut real_usage);
                            }
                            completion_chars += r.content.chars().count();
                            r.content
                        }
                        Err(_) => {
                            return ReactQaOutcome {
                                answer: format!("⚠️ LLM 调用失败, 无法回答该问题: {}", msg),
                                steps: 0,
                                real_usage,
                                prompt_chars,
                                completion_chars,
                                reason: AgentEndReason::LlmError,
                            };
                        }
                    }
                } else {
                    break;
                }
            }
        };
        let reply_trim = reply.trim().to_string();
        match parse_decision(&reply_trim) {
            Decision::Final(answer) => {
                let mut final_text = answer.trim().to_string();
                // 净化 + 上限 (保留最终结论行)
                let (cleaned, _) = crate::llm::translator::clean_llm_output(&final_text);
                final_text = if cleaned.trim().is_empty() {
                    final_text
                } else {
                    crate::llm::translator::truncate_llm_output(&cleaned, 1500)
                };
                return ReactQaOutcome {
                    answer: final_text,
                    steps: step_no,
                    real_usage,
                    prompt_chars,
                    completion_chars,
                    reason: AgentEndReason::Final,
                };
            }
            Decision::Malformed(problem) => {
                history.push((
                    reply_trim.clone(),
                    format!("【协议错误】{}。请重新输出严格 JSON 决策。", problem),
                ));
            }
            Decision::ToolCall { name, args, .. } => {
                if name == "flag_cell" {
                    // 问答模式不改动棋盘; 标旗类只作为建议写进 final
                    history.push((
                        reply_trim.clone(),
                        "【工具结果】flag_cell 在问答模式禁用; 如需建议标旗, 请直接写进 final。".to_string(),
                    ));
                    continue;
                }
                let result = crate::agent::tools::execute_tool(&mut agent, &name, &args);
                match result {
                    Ok(obs) => {
                        let label = describe_call(&name, &args);
                        let obs_c = cap_chars(&obs, 700);
                        history.push((
                            reply_trim.clone(),
                            format!("【工具结果】{}\n{}", label, obs_c),
                        ));
                    }
                    Err(e) => {
                        let obs = cap_chars(&e, 300);
                        history.push((
                            reply_trim.clone(),
                            format!("【工具结果】{} 被拒绝\n{}", describe_call(&name, &args), obs),
                        ));
                    }
                }
            }
        }
    }

    // 未 final: 本地兜底一句
    let local = summarize_locally(&agent.board, agent.remaining_mines);
    ReactQaOutcome {
        answer: format!(
            "⚠️ 自主问答达到停止条件 (步数 {} / 时间 / 限额), 未能给出针对性回答。本地摘要:\n{}",
            last_steps, local
        ),
        steps: last_steps,
        real_usage,
        prompt_chars,
        completion_chars,
        reason: AgentEndReason::MaxSteps,
    }
}

#[tokio::test]
async fn test_react_qa_answers_targeted_question() {
    // 剧本: 提问 (1,0) 为什么是雷 → get_cell_info → explain_cell → final
    let fake = FakeChat::new(vec![
        r#"{"action":"tool","name":"get_cell_info","args":{"x":1,"y":0},"reason":"看状态"}"#.into(),
        r#"{"action":"tool","name":"explain_cell","args":{"x":1,"y":0},"reason":"取约束"}"#.into(),
        r#"{"action":"final","answer":"第 1 步: 数字3@(0,0) 剩3雷 候选3格含 (1,0) …\n最终结论: (1, 0) 是雷"}"#.into(),
    ]);
    let cfg = ReactQaConfig::default();
    let out = run_react_qa(
        &fake,
        test_board(),
        3,
        "(1,0) 为什么必定是雷?",
        false,
        &cfg,
        None,
    )
    .await;
    assert_eq!(out.reason, AgentEndReason::Final);
    assert_eq!(out.steps, 3);
    assert!(out.answer.contains("最终结论: (1, 0) 是雷"));
    // 不会全盘输出 (无"剩余雷"全局摘要字样出现在答案中…剧本内容不含)
    assert!(!out.answer.contains("本次无新增确定性结论"));
}

#[tokio::test]
async fn test_react_qa_uses_local_board_tool() {
    let fake = FakeChat::new(vec![
        r#"{"action":"tool","name":"get_local_board","args":{"x":1,"y":1,"radius":1},"reason":"看局部"}"#.into(),
        r#"{"action":"final","answer":"这一小片就 3 个未知格, 建议先开低概率的。"}"#.into(),
    ]);
    let cfg = ReactQaConfig::default();
    let out = run_react_qa(&fake, test_board(), 3, "这里大概什么布局", false, &cfg, None).await;
    assert_eq!(out.reason, AgentEndReason::Final);
    assert!(out.answer.contains("未知格"));
}

#[test]
fn test_react_prompt_mentions_workflows() {
    let zh = build_react_system_prompt(false);
    assert!(zh.contains("explain_cell"));
    assert!(zh.contains("get_local_board"));
    assert!(zh.contains("先直接回答问题"));
    let en = build_react_system_prompt(true);
    assert!(en.contains("Decide yourself which tools"));
}
