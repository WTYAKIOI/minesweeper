use crate::model::{InferenceIR, PlayerView, Coord, CellProb, Proof, CellState};
use std::collections::{HashSet, VecDeque};

/// 抽查验证: 引擎已证明、且 LLM 需要给出"是雷/不是雷"最终结论的格子。
///
/// 候选规则 (答案模式默认执行):
/// 1. 必雷且未标旗的未知格 (最优先 — "该插旗了"最可操作)
/// 2. 必安全的未知格
/// 3. 引擎判定矛盾的旗 (必安全)
///
/// 平凡局面过滤: 若候选格所在未知连通块整块全是必雷 (≥2 格), 结论是"白送"的,
/// 不能体现推理, 排除。
fn pick_spot_check_target(view: &PlayerView, ir: &InferenceIR) -> Option<Coord> {
    let mine_set: HashSet<Coord> = ir
        .deterministic
        .iter()
        .filter(|p| p.conclusion.is_mine)
        .map(|p| p.conclusion.coord)
        .collect();
    let state = view.to_state_map();
    let richness = |p: &Proof| (p.depends_on.len(), p.rule.chars().count());

    // 类别 1: 必雷未标旗 (未知格), 且连通块非"整块全雷"
    let mut mines: Vec<&Proof> = ir
        .deterministic
        .iter()
        .filter(|p| {
            p.conclusion.is_mine
                && matches!(state.get(&p.conclusion.coord), Some(CellState::Unknown))
        })
        .collect();
    mines.retain(|p| {
        !is_trivial_all_mines_component(view, p.conclusion.coord, &mine_set)
    });
    mines.sort_by_key(|p| richness(p));
    if let Some(p) = mines.last() {
        return Some(p.conclusion.coord);
    }

    // 类别 2: 必安全的未知格
    let mut safes: Vec<&Proof> = ir
        .deterministic
        .iter()
        .filter(|p| {
            !p.conclusion.is_mine
                && matches!(state.get(&p.conclusion.coord), Some(CellState::Unknown))
        })
        .collect();
    safes.sort_by_key(|p| richness(p));
    if let Some(p) = safes.last() {
        return Some(p.conclusion.coord);
    }

    // 类别 3: 矛盾旗 (引擎证明必安全)
    let flag = ir
        .flag_verification
        .flags
        .iter()
        .find(|f| f.status == crate::model::FlagVerifyStatus::Contradicted)
        .map(|f| f.coord);
    if let Some(c) = flag {
        return Some(c);
    }

    None
}

/// 候选格所在未知连通块 (8 邻接) 是否"整块全是必雷"且 ≥2 格 (平凡结论, 无推理价值)
fn is_trivial_all_mines_component(
    view: &PlayerView,
    start: Coord,
    mine_set: &HashSet<Coord>,
) -> bool {
    let state = view.to_state_map();
    if !matches!(state.get(&start), Some(CellState::Unknown)) {
        return false;
    }
    let mut seen: HashSet<Coord> = HashSet::new();
    let mut queue: VecDeque<Coord> = VecDeque::new();
    seen.insert(start);
    queue.push_back(start);
    let mut count = 0usize;
    while let Some(c) = queue.pop_front() {
        count += 1;
        if !mine_set.contains(&c) {
            return false; // 块内存在非必雷格 → 非平凡
        }
        for n in c.neighbors(view.width, view.height) {
            if matches!(state.get(&n), Some(CellState::Unknown)) && seen.insert(n) {
                queue.push_back(n);
            }
        }
    }
    count >= 2
}

/// 某格是否被引擎证明 (确定性结论覆盖; 含矛盾旗降级结论)
fn is_engine_proven(ir: &InferenceIR, c: Coord) -> bool {
    ir.deterministic.iter().any(|p| p.conclusion.coord == c)
}

/// 是否启用"抽查验证" (答案模式专用; 教学/策略/区域任务是独立流程, 不掺和)
fn spot_check_enabled(mode: &LLMMode, cmd: Option<&UserCommand>) -> bool {
    if *mode != LLMMode::Answer {
        return false;
    }
    !matches!(cmd, Some(UserCommand::Region) | Some(UserCommand::Teach))
}

/// 确定抽查目标: 用户已在提问中指定 (指令或纯文本坐标) → 用该格;
/// 提问完全不含坐标 (或未提问) → 默认自动挑选一个代表性格子。
fn resolve_spot_target(
    mode: &LLMMode,
    view: &PlayerView,
    ir: &InferenceIR,
    cmd: Option<&UserCommand>,
    question: Option<&str>,
) -> Option<Coord> {
    if !spot_check_enabled(mode, cmd) {
        return None;
    }
    let question_coord = question
        .and_then(|q| extract_coords(q).first().cloned())
        .and_then(|s| parse_coord_str(&s));
    let specified = match cmd {
        Some(UserCommand::Why(c)) | Some(UserCommand::Check(c)) => Some(*c),
        _ => question_coord,
    };
    if let Some(c) = specified {
        if is_engine_proven(ir, c) {
            return Some(c); // 用户指谁就验证谁
        }
        // 提问里带了坐标但不是引擎可证格 (如问已翻开数字含义) →
        // 不做抽查, 免得答非所问
        return None;
    }
    // 完全没带坐标 → 默认抽查一个代表格 (答案模式自动附带)
    pick_spot_check_target(view, ir)
}

/// 抽查验证任务块: 勒令 LLM 对目标格做完整推导, 最后给出"是/不是雷"最终结论
fn build_spot_check_block(view: &PlayerView, ir: &InferenceIR, coord: Coord) -> String {
    let state = view.to_state_map();
    let proven_mine = ir
        .deterministic
        .iter()
        .any(|p| p.conclusion.coord == coord && p.conclusion.is_mine);
    let kind_desc = match state.get(&coord) {
        Some(CellState::Flagged) if proven_mine => {
            "引擎判定该格必为雷 (你的旗帜正确)"
        }
        Some(CellState::Flagged) => "你当前在该格插了旗, 但引擎判定该格必为安全 (矛盾旗)",
        _ if proven_mine => "引擎判定该格必为雷 (且你尚未插旗)",
        _ => "引擎判定该格必为安全 (尚未翻开)",
    };
    format!(
        "\n\n【抽查验证任务】(强制完成, 放在回答的最后)\n\
         目标格: {} — {}\n\
         要求:\n\
         1. 在回答最后输出「## 🔬 抽查验证」章节, 用 3-6 步可复现推理链 (引用上方数字约束表, \
         每步用 → 连接, 说明所用推理类型: 单约束 / 子集 / 组合枚举), 复现引擎为何判定该格。\n\
         2. 推理完成后以单独一行收尾 (二选一, 必须与引擎判定一致, 不得推翻):\n\
         {}",
        format_coord(&coord),
        kind_desc,
        if proven_mine {
            format!("最终结论: {} 是雷", format_coord(&coord))
        } else {
            format!("最终结论: {} 不是雷", format_coord(&coord))
        }
    )
}

/// IR 精简上限 (LLM 输出规则本身限定表格每类 ≤10 行, 全量 IR 是浪费)
const MAX_PROOFS: usize = 15;
const MAX_PROB_EACH_SIDE: usize = 10;
const MAX_FLAG_DETAILS: usize = 20;

/// IR 精简为 LLM 转译所需的代表性内容, 大幅减小请求体
/// (16×30 棋盘全量 pretty IR 可达 30KB+, 校园网关对大请求体易超时/重置连接)。
///
/// 策略 (对应裁剪方案):
/// - deterministic: 前 15 条 + omitted 计数 (输出规则本就只列前几条)
/// - probabilities: 雷概率最低 10 + 最高 10 (其余以 total 计数说明),
///   不足 2×10 条时全量保留
/// - flags: 只保留需注意的旗 (矛盾/未确认), Verified 旗由 summary 概括
/// - regions: 保留 (区域数量少, 策略模式需要)
///
/// 返回 (紧凑 JSON, 是否发生了裁剪)
fn compact_ir_json(ir: &InferenceIR) -> (String, bool) {
    let mut trimmed = false;

    // 确定性证明链: 截断
    let det_total = ir.deterministic.len();
    let deterministic: Vec<&crate::model::Proof> =
        ir.deterministic.iter().take(MAX_PROOFS).collect();
    let det_omitted = det_total.saturating_sub(deterministic.len());
    if det_omitted > 0 {
        trimmed = true;
    }

    // 概率: 最低 N + 最高 N (升序), 小棋盘全量保留
    let prob_total = ir.probabilities.len();
    let probabilities: Vec<CellProb> = if prob_total <= MAX_PROB_EACH_SIDE * 2 {
        ir.probabilities.clone()
    } else {
        trimmed = true;
        let mut sorted: Vec<&CellProb> = ir.probabilities.iter().collect();
        sorted.sort_by(|a, b| {
            a.mine_probability
                .partial_cmp(&b.mine_probability)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        sorted
            .iter()
            .take(MAX_PROB_EACH_SIDE)
            .chain(sorted.iter().rev().take(MAX_PROB_EACH_SIDE))
            .map(|p| (*p).clone())
            .collect()
    };

    // 旗帜: 只保留需注意的旗 (矛盾/未确认), Verified 由 summary 概括
    let flags: Vec<&crate::model::FlagStatus> = ir
        .flag_verification
        .flags
        .iter()
        .filter(|f| f.status != crate::model::FlagVerifyStatus::Verified)
        .take(MAX_FLAG_DETAILS)
        .collect();
    if flags.len() < ir.flag_verification.flags.len() {
        trimmed = true;
    }

    let json = serde_json::json!({
        "deterministic": deterministic,
        "deterministic_total": det_total,
        "deterministic_omitted": det_omitted,
        "probabilities": probabilities,
        "probabilities_total": prob_total,
        "regions": ir.regions,
        "flag_verification": {
            "flags": flags,
            "flags_total": ir.flag_verification.flags.len(),
            "summary": ir.flag_verification.summary,
            "has_contradiction": ir.flag_verification.has_contradiction,
        }
    });
    (serde_json::to_string(&json).unwrap_or_default(), trimmed)
}

/// LLM 工作模式
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LLMMode {
    /// 答案模式：将证明链转为流畅的"因为...所以..."文字
    Answer,
    /// 教学模式：将证明链倒置，生成逐步引导问题
    Teaching,
    /// 策略模式：基于概率+收益数据，生成区域比较建议
    Strategy,
}

/// 回答语言
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Chinese,
    English,
}

impl Language {
    /// 解析前端传入值: "en"/"english" → English, 其余默认中文
    pub fn parse(s: Option<&str>) -> Self {
        match s.map(str::trim).unwrap_or("").to_lowercase().as_str() {
            "en" | "english" | "en-us" | "en-gb" => Language::English,
            _ => Language::Chinese,
        }
    }
}

/// 教学模式完整性阈值: 标题后仅剩 1 行正文且总长 < 此值 → 视为"输出被吃"
/// (自适应教学中"## 直接原因"+一句话也可能偏短, 阈值不宜过大)
pub const MIN_TEACHING_OUTPUT_CHARS: usize = 30;

/// 输出完整性检测 (教学模式):
/// 空文本 → 不完整;
/// 以 "##" 开头但标题后无正文, 或仅 1 行过短正文 (<30 字符) → 视为"输出被吃"
pub fn is_output_incomplete(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return true;
    }
    if !t.starts_with("##") {
        return false;
    }
    let body: Vec<&str> = t
        .lines()
        .skip(1)
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();
    if body.is_empty() {
        return true;
    }
    if body.len() == 1 && t.chars().count() < MIN_TEACHING_OUTPUT_CHARS {
        return true;
    }
    false
}

/// 思考自白/自言自语前缀 (行首匹配, 小写化后比较)。命中即视为"思考过程泄漏"。
const THOUGHT_PREFIXES: &[&str] = &[
    "let me", "let's", "i think", "i believe", "actually", "wait", "hmm", "ok,", "okay",
    "i realize", "i need to", "i will", "i'll", "让我", "我们来看", "我们先", "首先分析",
    "嗯，", "好，现在", "等一下", "我想", "先看看",
];

/// 净化 LLM 原始输出:
/// 1. 剔除"思考自白"行 (Let me / I think / 让我... 等)
/// 2. 丢弃首个 "##" 区块之前的大段引言 (模型常以 "Let me analyze this position carefully"
///    起头整段自言自语, 这些不是任何区块)
/// 3. 压缩 3+ 连续空行
///
/// 返回 (净化后的文本, 剔除的自白行数)
pub fn clean_llm_output(raw: &str) -> (String, usize) {
    let mut kept: Vec<&str> = Vec::new();
    let mut dropped = 0usize;
    for line in raw.lines() {
        let low = line.trim_start().to_lowercase();
        if THOUGHT_PREFIXES.iter().any(|m| low.starts_with(m)) {
            dropped += 1;
            continue;
        }
        kept.push(line);
    }
    let joined = kept.join("\n");

    // 丢弃首个 "## " 区块前的非空引言
    let text = if let Some(pos) = joined.find("## ") {
        let before = joined[..pos].trim();
        if !before.is_empty() && !before.contains("##") {
            joined[pos..].to_string()
        } else {
            joined
        }
    } else {
        joined
    };

    // 压缩连续空行 (每个空行段只保留一个换行)
    let mut out = String::with_capacity(text.len());
    let mut prev_blank = false;
    for line in text.lines() {
        if line.trim().is_empty() {
            if !prev_blank {
                out.push('\n');
            }
            prev_blank = true;
            continue;
        }
        prev_blank = false;
        out.push_str(line);
        out.push('\n');
    }
    (out.trim().to_string(), dropped)
}

/// 输出是否"混乱" (值得用纪律提醒重试一次):
/// 自白行 ≥2, 或以思考自白开头, 或完全没有 "##" 区块结构
pub fn is_output_chaotic(raw: &str) -> bool {
    let t = raw.trim();
    if t.is_empty() {
        return true;
    }
    let marker_lines = t
        .lines()
        .filter(|l| {
            let low = l.trim_start().to_lowercase();
            THOUGHT_PREFIXES.iter().any(|m| low.starts_with(m))
        })
        .count();
    if marker_lines >= 2 {
        return true;
    }
    let low = t.to_lowercase();
    if THOUGHT_PREFIXES.iter().any(|m| low.starts_with(m)) {
        return true;
    }
    if !t.contains("##") && !t.contains('→') {
        return true;
    }
    false
}

/// 输出过长时截断, 尽量保留末尾"最终结论"行 (抽查验证的判定不能被截掉)。
/// 追加简短说明 (不附带原文总字数, 避免 "共 N 字" 刷屏)。
pub fn truncate_llm_output(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    const VERDICT_KEY: &str = "最终结论:";
    let verdict_start = text.rfind(VERDICT_KEY);
    let verdict_tail: Option<String> = verdict_start.map(|vi| text[vi..].chars().take(80).collect());
    let tail_len = verdict_tail.as_ref().map(|s| s.chars().count()).unwrap_or(0);

    let reserve = if verdict_tail.is_some() { tail_len + 12 } else { 0 };
    let keep = max_chars.saturating_sub(reserve);
    let head: String = text.chars().take(keep).collect();
    let mut out = format!("{}…", head);
    if verdict_tail.is_some() {
        out.push_str("\n[已精简过长内容, 保留最终结论]");
    } else {
        out.push_str("\n[已精简过长内容]");
    }
    if let Some(v) = verdict_tail {
        out.push('\n');
        out.push_str(&v);
    }
    out
}

/// 解析用户针对"单个格子"的提问目标:
/// - @why/@check (x,y) 指令
/// - 自然语言中第一个 (x,y) 坐标 (如 "(18,15) 为什么必定是雷?")
/// - 其余返回 None (全盘分析 / 策略问题)。
pub fn user_asked_cell(question: &str) -> Option<Coord> {
    if let Some(cmd) = parse_user_command(question) {
        match cmd {
            UserCommand::Why(c) | UserCommand::Check(c) => return Some(c),
            _ => return None, // @region/@teach/@analyze 不是定点问题
        }
    }
    extract_coords(question)
        .first()
        .and_then(|s| parse_coord_str(s))
}

/// 定点解释系统提示词: 只回答用户指定格, 禁止全盘分析 (routes 定点路径使用)
pub fn build_explain_system_prompt(english: bool) -> String {
    if english {
        r#"You are a minesweeper reasoning expert. The user asks about ONE specific cell (with
coordinates). Your ONLY job is to answer that cell — NEVER run or output a whole-board analysis.

The "engine material" below contains: the target cell state, the engine verdict
(must-be-mine / must-be-safe / probability only), the related constraint table
(each digit: flags / remaining mines / candidate list), and a contradiction comparison
when the cell is a wrong flag.

Answer format:
  Step 1 / Step 2 ... (2-4 steps; every step must cite the concrete digit@coord, flags,
  remaining mines and candidate cells from the material — no vague "by elimination" phrasing).
  Last line: Final verdict: (x,y) is a mine  OR  Final verdict: (x,y) is NOT a mine
  If the engine has NO deterministic verdict, write:
  Final verdict: (x,y) cannot be proven; mine probability N%

Rules:
1. Only discuss the requested cell and its directly related constraints; nothing about the rest
   of the board, no global summaries.
2. If the engine verdict contradicts the user's assumption, trust the engine material and say so.
3. ≤500 characters. No thinking aloud."#
    } else {
        r#"你是扫雷推理专家。用户针对"单个格子"(含坐标)提问, 你的唯一任务是回答该格,
禁止做全盘分析、禁止输出全局摘要 (剩余雷数、其他区域一律不提)。

下方"引擎素材"包含: 目标格状态、引擎判定 (必雷/必安全/仅有概率)、相关数字约束表
(每个数字: 旗数/剩余需雷数/候选格)、矛盾旗对照 (若该格是误标旗)。

【回答格式】
第 1 步 / 第 2 步 … (2-4 步; 每步引用素材中的具体 数字@坐标、旗数、剩余雷、候选格,
禁止"联动排除/综合分析"等黑话)
最后一行 (二选一, 或第三情形):
  最终结论: (x,y) 是雷
  最终结论: (x,y) 不是雷
  若引擎无确定性判定: 最终结论: (x,y) 无法确定为雷/安全, 雷概率 N%

【约束】
1. 只分析用户指定坐标及其直接相关约束, 不涉及棋盘其余部分。
2. 若引擎判定与用户假设冲突 (用户以为必雷但素材显示安全/仅概率), 以引擎素材为准并明确指出。
3. 全文 ≤500 字; 简体中文; 不要思考自白。"#
    }
    .to_string()
}

/// 旗帜问题专项分析系统提示词: 棋盘存在误标旗时, 只解释"第一面问题旗为什么有问题",
/// 禁止全盘分析 (routes 旗帜问题特判路径使用)
pub fn build_flag_problem_system_prompt(english: bool) -> String {
    if english {
        r#"You are a minesweeper reasoning expert. The board contains PROBLEM FLAGS (flags that
conflict with the digit constraints). This analysis handles exactly ONE of them — your ONLY job
is to explain WHY this flag is a problem. NEVER run a whole-board analysis and never discuss
other problem flags.

The "engine material" below contains: the target flag's engine verdict, its judged reason,
the related constraint table (each digit: flags / remaining mines / candidate cells), and a
keep-flag vs remove-flag comparison when the flag is contradicted.

Answer format:
  Step 1 / Step 2 ... (2-4 steps; every step must cite the concrete digit@coord, flags,
  remaining mines and candidate cells from the material — point out explicitly which digit
  constraint becomes unsatisfiable if this flag is treated as a mine; no vague wording).
  Last line (choose one):
    Final verdict: flag (x,y) is misplaced — the cell is provably safe, remove it
    or (if the material shows the flag is only unconfirmed): Final verdict: flag (x,y)
    cannot be confirmed, mine probability N%, verify it manually

Rules:
1. Only discuss this flag and its directly related constraints; nothing about the rest of
   the board, no global summaries, no other problem flags.
2. Trust the engine material; where possible, briefly explain why this flag was likely
   misplaced in the first place (misread digit, wrong neighborhood, etc.).
3. ≤500 characters. No thinking aloud."#
    } else {
        r#"你是扫雷推理专家。用户棋盘上存在"问题旗帜"(与数字约束矛盾的误标旗), 本次分析只处理其中一面 —
你的唯一任务是解释"这面旗为什么有问题", 禁止做全盘分析、禁止讨论其他问题旗。

下方"引擎素材"包含: 目标旗的引擎判定、判定理由、相关数字约束表
(每个数字: 旗数/剩余需雷数/候选格)、矛盾旗对照 (保留旗 vs 撤旗的约束变化, 若为矛盾旗)。

【回答格式】
第 1 步 / 第 2 步 … (2-4 步; 每步引用素材中的具体 数字@坐标、旗数、剩余雷、候选格;
关键步骤必须点明"把该旗当作雷时哪个数字约束无法满足", 禁止"联动排除/综合分析"等黑话)
最后一行 (二选一):
  最终结论: 旗 (x,y) 是误标 — 该格必安全, 建议移除
  (若素材显示该旗仅是无法确认/概率存疑): 最终结论: 旗 (x,y) 无法确认, 雷概率 N%, 建议人工核对

【约束】
1. 只分析该旗及其直接相关约束, 不涉及棋盘其余部分。
2. 以引擎素材为准; 适当解释这面旗当初为什么容易被误标 (如数字看错/邻域混淆)。
3. 全文 ≤500 字; 简体中文; 不要思考自白。"#
    }
    .to_string()
}

/// 推理 IR → 自然语言转译器
///
/// 将 Rust 推理引擎的计算结果翻译为人类可理解的策略语言。
/// 支持三种模式：答案模式、教学模式、策略模式。
pub struct Translator {
    mode: LLMMode,
    language: Language,
}

impl Translator {
    pub fn new(mode: LLMMode) -> Self {
        Self { mode, language: Language::Chinese }
    }

    pub fn set_mode(&mut self, mode: LLMMode) {
        self.mode = mode;
    }

    /// 设置回答语言 (默认中文)
    pub fn set_language(&mut self, language: Language) {
        self.language = language;
    }

    /// 当前语言
    pub fn language(&self) -> Language {
        self.language
    }

    /// 将 IR 转为给 LLM 的系统提示词 (llm-output.md 规范)
    /// 输出模板 + 硬性规则全模式共享; 答案/策略/教学按模式附加模板.
    pub fn build_system_prompt(&self) -> String {
        let (lang_rule, spec) = match self.language {
            Language::Chinese => (
                "## 语言要求\n必须全程使用简体中文。若思考中出现任何其他语言, 一律不得输出; 只输出中文。",
                r#"## 输出纪律 (最高优先级, 违反任一条即视为输出非法)
1. 禁止思考自白: 严禁输出 "Let me ..." / "I think ..." / "Actually ..." / "Wait, ..." /
   "Hmm" / "OK" / "让我..." / "我想..." / "先分析..." 等思考过程与自言自语。推理只在内部完成,
   输出只呈现最终结论与推理链, 不要解释"你是如何分析的"。
2. 语言: 全文必须是简体中文 (含表格、推理链、坐标说明); 禁止混入英文句子; 禁止中英夹杂。
3. 长度: 常规输出 ≤400 字; 消息含【抽查验证任务】时 ≤600 字; 表格每类 ≤5 行 (超 5 行只列前 3 并注明"余 N 个略")。
4. 结构: 只按当前模式模板规定的区块顺序输出; 区块之间禁止插入零散分析段落; 表格必须完整闭合, 禁止截断。
5. 推理链: 一律用箭头序列 (→) 呈现, 每步 ≤15 字, 总 3-6 步; 同一推理节点只展开一次, 禁止重复表述。

## 内容硬性规则 (违反视为不合格)
1. 坐标引用: 只允许引用 IR / 数字约束表 / 坐标清单中实际出现的坐标; 一律用 (x, y) 形式
   (与 IR 及约束表一致); 禁止编造坐标、禁止"左边/附近/那一片"等模糊描述。
2. 推理链展开: 关键结论 (必雷/必安全/矛盾旗) 必须附可复现的推导 (箭头链 3-6 步),
   每步引用数字约束表的原始约束 (数字X@(a,b): 旗f 剩r 未知[...]); 只做填表式翻译视为不合格。
3. 确定性结论: 必须与 IR 结论一致并可由约束表复现; 禁止添加 IR 与约束推导不支持的结论;
   表述确定结论时禁止"我认为/可能/大概/也许"句式。
4. 概率结论: 数值直接来自 IR 的 probabilities (字段名 mine_probability, 即 0~1 的小数,
   展示时换算为百分比, 不四舍五入成整数); 只可给 IR 中存在的格赋概率; 每个概率格附建议
   (优先点击 / 可考虑标旗 / 暂缓处理)。
5. 旗帜验证: 若 flag_verification 中存在 Contradicted/未确认(Suspected) 旗, 必须逐条列出;
   矛盾旗必须展开"为什么矛盾"的推导, 并明确指出"你的旗帜 (x,y) 与数字冲突——该格实际必安全, 因为...";
   无问题则写"所有旗帜与当前数字约束一致"。
6. 旗帜问题警告: 当存在矛盾旗或未确认旗时, 显式提示"存在旗帜问题, 以下概率与推理基于容错假设,
   数值可能出错, 建议先核对/移除问题旗帜后再做关键决策"。
7. 区块省略: deterministic 为空 → 不输出确定性结论表格, 只给概率参考; probabilities 为空 → 写"概率数据不可用"。
8. deterministic_omitted / probabilities_total 字段标明省略数量, 被省略条目不得引用或编造, 汇总时以省略计数说明即可。\n"#,
            ),
            Language::English => (
                "## Language Requirement\nRespond entirely in English. Never include text in other languages.",
                r#"## Output Discipline (highest priority; any violation invalidates the answer)
1. No thinking aloud: never output "Let me ..." / "I think ..." / "Actually ..." / "Wait, ..." /
   "Hmm" / "OK" or similar self-talk. Reason internally; output only conclusions and reasoning chains.
2. Language: the entire answer must be in English (tables, chains, coordinates included).
3. Length: ≤400 characters normally; ≤600 when the message contains a spot-check task; ≤5 table rows
   per class (list first 3 and note "N more omitted").
4. Structure: output only the blocks defined by the current mode template, in order; no stray
   paragraphs between blocks; never truncate a table.
5. Chains: single arrow (→) sequence, each step ≤15 words, 3-6 arrows total; never repeat a node.

## Content Rules (violations are rejected)
1. Coordinates: only cite coordinates present in the IR / constraint table / listed coordinates;
   format "(x, y)"; never invent coordinates or use vague descriptions.
2. Reasoning chains: key conclusions must include a reproducible derivation (3-6 arrows), each step
   citing raw constraints from the constraint table.
3. Deterministic claims: must agree with the IR and be reproducible from constraints; never say
   "I think / maybe / probably"; never add conclusions not supported by the IR.
4. Probabilities: take values directly from IR probabilities (field mine_probability, a 0..1 float;
   show as percentage without rounding); only for cells present in the IR; attach a recommendation.
5. Flag verification: if flag_verification contains Contradicted or Suspected flags, list them all
   and derive why each contradicted flag is wrong; otherwise state "all flags are consistent".
6. Flag warning: when contradicted or unverified flags exist, explicitly warn that probabilities
   may be wrong because reasoning runs on a fault-tolerant assumption; suggest fixing flags first.
7. If deterministic is empty, give only probability reference; if probabilities empty, say
   "probability data unavailable".
8. Respect deterministic_omitted / probabilities_total counts; never cite omitted entries.\n"#,
            ),
        };

        let base = format!(
            "你是一位世界级扫雷推理专家，拥有 20 年残局分析经验。你收到的不是\"待翻译的数据\"，而是 Rust 推理引擎的计算结果 (JSON IR)、数字约束表与局面说明；你的任务不是填表式翻译，而是像人类专家一样展开可复现的推理链，让玩家学会推理。

## 推理原则
1. 扫雷的核心是\"逻辑必然性\"——结论必须被证明，而不是猜测 (概率结论须明确标注为概率)。
2. 三种推理武器:
   - 单约束: 数字剩余需雷数 (=数字-周围已标旗数) 为 0 → 其未知邻居全部安全; 剩余需雷数 = 未知邻居数 → 未知邻居全部是雷。
   - 子集: 数字 A 的未知邻居 ⊆ 数字 B 的未知邻居时, B剩 - A剩 = B 专属格中的雷数; 若两数相等, B 的专属格全部安全。
   - 组合/枚举: 1-2-1 等模式, 或对局部约束联合枚举所有可行布雷方案, 取恒定结论 (所有方案中该格恒为雷/恒安全)。
3. 推理链格式 (用 → 连接, 完整可复现):
   \"数字X@(a,b): 旗f 剩r 未知[(c,d),...] → [单约束/子集/枚举推导] → 结论: (x,y) 必为雷/安全\"。

## 数据防火墙原则
你收到的数据中绝不包含未翻开格子的真实雷藏信息。所有概率都是基于已知信息的数学推断。

## 输入字段说明 (防止读错)
- deterministic[].conclusion.is_mine: true=必雷结论 / false=必安全结论; depends_on[]: 依据的数字坐标;
  rule: 引擎给出的结论摘要 (你的任务是把 rule 背后的约束推导展开成推理链)
- probabilities[].mine_probability: 雷概率(0~1 小数); info_gain: 信息增益
- flag_verification.flags[]: 每面旗的 status (Verified=确认正确 / Contradicted=矛盾,必为误标 /
  Suspected=未确认) 与 reason(中文理由)
- 数字约束表: 每个前沿数字一行的原始约束 (坐标=数字, 旗数, 剩余需雷数, 未知邻居) — 展开推理链的原料

{}
{}
",
            lang_rule,
            spec
        );

        match &self.mode {
            LLMMode::Teaching => self.teaching_prompt(&base),
            LLMMode::Answer => self.answer_prompt(&base),
            LLMMode::Strategy => self.strategy_prompt(&base),
        }
    }

    /// 答案模式模板 (输出纪律版: 只允许按序输出下列区块, 区块外零文字)
    fn answer_prompt(&self, base: &str) -> String {
        match self.language {
            Language::Chinese => format!(
                r##"{}

## 当前模式：答案模式 — 严格输出模板
只允许按以下顺序输出区块; 无内容可写的区块整块省略 (省略时不要解释); 区块之间与区块外禁止任何文字、禁止思考过程。

区块 1. ## 🎯 局面评估
1-2 句: 剩余雷数 / 未知格数 / 关键区域位置。禁止超过 2 句。

区块 2. ## 🧠 核心推理链
只列 1 个最关键节点, 用单行箭头序列呈现 (3-6 步, 每步 ≤15 字, 引用数字约束表), 例如:
数字2@(28,9)与数字3@(27,10)共享(28,10) → (28,10)与(28,11)恰1雷 → (26,12)恒安全
若没有值得展开的新推理, 此区块只写: 本次分析无新增推理链

区块 3. ## ✅ 确定性结论
### 💣 必雷格 (共 N 个; 表格 ≤5 行, 超 5 行只列前 3 并注明"余 N 个略")
| 坐标 | 依据 |
### ✅ 安全格 (同上) [无则整小节省略]

区块 4. ## 🚩 旗帜检查
仅当存在矛盾/未确认旗时输出; 每面矛盾旗用 1-2 句给出"为什么矛盾"; 无问题旗则整块省略。

区块 5. ## 📊 概率参考
前 5 个最低雷概率格 (升序); 可再附至多 2 个高概率格。表格列: 坐标 | 雷概率 | 建议。
无概率数据时写: 概率数据不可用

区块 6. ## 💡 下一步建议
1 句, 指出最优操作。

区块 7. ## 🔬 抽查验证
仅当消息末尾含【抽查验证任务】时输出 (该任务会指定目标格): 单行箭头链 3-6 步后,
输出最后一行(必须是最末一行): 最终结论: (x,y) 是雷  或  最终结论: (x,y) 不是雷

若用户带了具体提问(见消息末尾【用户提问】): 先按区块 1-6 输出当前局面,
再以区块 8. ## 回答用户提问 作答 (同样遵守输出纪律: 简体中文、无自白、≤200 字)。"##,
                base
            ),
            Language::English => format!(
                r##"{}

## Current Mode: Answer — Strict Output Template
Output ONLY the following blocks in order. Omit a block entirely when it has nothing to show.
No text outside blocks; no thinking aloud; English only.

Block 1. ## 🎯 Position assessment (1-2 sentences: mines left / unknown cells / key region)
Block 2. ## 🧠 Key reasoning chain
One critical node only, as a single arrow sequence (3-6 arrows, each ≤15 words, citing the
constraint table). If nothing new: write exactly "No new reasoning chain this turn".
Block 3. ## ✅ Deterministic conclusions
### 💣 Must-be-mine (N; ≤5 rows, list first 3 and note "N more omitted")
| coord | evidence |
### ✅ Must-be-safe (omit subsection when empty)
Block 4. ## 🚩 Flag check (only when contradicted/suspected flags exist; 1-2 lines each)
Block 5. ## 📊 Probability reference
5 lowest-mine cells ascending (table: coord | mine prob | advice), up to 2 highest optional.
If none: write "probability data unavailable".
Block 6. ## 💡 Next step (1 sentence)
Block 7. ## 🔬 Spot check
Only when the message ends with a spot-check task: single arrow chain (3-6 arrows) then the very
last line must be: Final verdict: (x,y) is a mine   OR   Final verdict: (x,y) is NOT a mine
If the user asked a question: output blocks 1-6, then answer in Block 8. ## Answer (≤200 words,
no self-talk, English only)."##,
                base
            ),
        }
    }

    /// 策略模式模板 (llm-output.md 模板 C)
    fn strategy_prompt(&self, base: &str) -> String {
        match self.language {
            Language::Chinese => format!(
                r##"{}

## 当前模式：策略比较 — 输出模板
## 策略比较
### 方案 A：处理区域 [坐标范围]
- 收益: [可翻开格数等具体数字]
- 风险: [雷概率等具体数字]
- 预期结果: [简述]
### 方案 B：处理区域 [坐标范围]
- 收益: ...
- 风险: ...
### 建议
推荐方案 [A/B], 理由: [1-2 句]
补充: 若存在旗帜问题, 先在结论处提示概率可能出错并建议先处理旗帜。"##,
                base
            ),
            Language::English => format!(
                r##"{}

## Current Mode: Strategy — Output Template
## Strategy comparison
### Option A: region [coordinate range]
- gain / risk / expected
### Option B: region ...
### Recommendation
Option [A/B], reason.
Mention flag issues first if any."##,
                base
            ),
        }
    }
    /// 教学模式提示词: 智能自适应 (teachmod2.md) — 不是复读机,
    /// 根据用户提问在"直接解释"与"引导问题"之间切换, 双语
    fn teaching_prompt(&self, base: &str) -> String {
        match self.language {
            Language::Chinese => format!(
                r##"{}

## 当前模式：教学模式（智能引导）
你是扫雷教练。用户开启了教学模式，但你**不是复读机**：根据用户的提问动态调整回答策略。

## 核心规则
规则 1：若用户问"为什么 / 原因 / 解释"类问题
→ 先给出直接、清晰的解释（1-3 句话，引用具体数字与坐标），随后追加 1 个思考引导问题。
输出格式：
## 直接原因
[简洁解释核心原因]

## 思考引导
[1 个针对性问题]

规则 2：若用户问"怎么做 / 下一步 / 该点哪里"类问题，或问题不明确
→ 只输出 3-5 个引导性问题，不直接给出答案。
输出格式：
## 引导问题
1. [问题]
2. [问题]
3. [问题]
（总问题数 3~5 个，不超过 5 个；每题 ≤30 字并含精确坐标）

规则 3：若用户说"我懂了 / 继续 / 下一个"类问题
→ 进入更深层次的推理引导，提出 1-2 个进阶问题。
输出格式：
## 进阶引导
1. [问题]
2. [问题]

## 重要约束
- 所有回答必须使用简体中文
- 只引用 IR 中存在的坐标 (x, y)，禁止编造
- 每次回答不超过 5 个问题，保持简洁
- 禁止空标题、空回答或未完结尾"##,
                base
            ),
            Language::English => format!(
                r##"{}

## Current Mode: Teaching (Adaptive)
You are a Minesweeper coach. Teaching mode is enabled, but you are **not a parrot**: adapt your reply to the user's question.

## Core Rules
Rule 1: If the user asks "why / reason / explain" questions
→ First give a direct, clear explanation (1-3 sentences citing exact numbers and coordinates), then add 1 thought-provoking question.
Output format:
## Direct Reason
[concise explanation]

## Think Further
[1 targeted question]

Rule 2: If the user asks "what should I do / next step / where to click" questions, or the question is unclear
→ Give only 3-5 guiding questions, never the answer directly.
Output format:
## Guiding Questions
1. [question]
2. [question]
3. [question]
(3 to 5 questions total, never more than 5; each within 30 words and containing an exact coordinate)

Rule 3: If the user says "I see / continue / next"
→ Guide deeper, asking 1-2 advanced questions.
Output format:
## Advanced Guidance
1. [question]
2. [question]

## Hard constraints
- Respond entirely in English
- Only cite coordinates (x, y) that exist in the IR; never invent coordinates
- Never exceed 5 questions per reply; stay concise
- No empty headings, empty answers, or unfinished endings"##,
                base
            ),
        }
    }

    /// 将 IR + PlayerView 序列化为给 LLM 的用户消息
    pub fn build_user_message(&self, view: &PlayerView, ir: &InferenceIR) -> String {
        self.build_user_message_with_question(view, ir, None)
    }

    /// 同上, 但附带用户在对话区输入的问题 (教学模式问答的关键:
    /// 不带问题时 LLM 只能泛泛引导, 无法针对用户提问作答)
    pub fn build_user_message_with_question(
        &self,
        view: &PlayerView,
        ir: &InferenceIR,
        question: Option<&str>,
    ) -> String {
        let (ir_json, trimmed) = compact_ir_json(ir);
        let view_info = format!(
            "棋盘: {}x{}, 剩余雷数: {}, 已翻开格子数: {}, 已标旗数: {}, 未知格数: {}",
            view.width,
            view.height,
            view.remaining_mines,
            view.revealed.len(),
            view.flagged.len(),
            view.unknown.len()
        );

        // 列出所有合法坐标，供 LLM 参考
        let valid_coords: Vec<String> = view.unknown.iter().map(format_coord).collect();
        let revealed_coords: Vec<String> = view.revealed.iter().map(|c| format_coord(&c.coord)).collect();

        let mut msg = format!(
            "当前局面信息:\n{}\n\n已翻开数字坐标: {}\n未知格子坐标(你的建议只能在这些中选): {}\n\n推理结果 (IR):\n```json\n{}\n```\n\n请根据以上信息给出分析。记住：你引用的每个坐标必须在上述列表中。",
            view_info, revealed_coords.join(", "), valid_coords.join(", "), ir_json
        );

        // 数字约束表: 前沿数字的原始约束 (旗数/剩余需雷数/未知邻居),
        // LLM 据此展开"因为...所以..."的专家推理链而非填表翻译
        let cmd = question.and_then(parse_user_command);
        let focus = match cmd {
            Some(UserCommand::Why(c)) | Some(UserCommand::Check(c)) => Some(c),
            _ => None,
        };
        let constraints = build_constraint_lines(view, focus, 60);
        if !constraints.is_empty() {
            let omitted = if constraints.len() == 60 { " (超过 60 行已截断)" } else { "" };
            msg.push_str(&format!(
                "\n\n数字约束表 (坐标=数字, 已标旗数, 剩余需雷数=数字-旗数, 未知邻居){}:\n{}",
                omitted,
                constraints.join("\n")
            ));
        }

        // 指令任务: 对话区 @ 命令 → 针对性任务 + 目标格上下文。
        // @why/@check 的目标格已被引擎证明时, 交由消息末尾【抽查验证任务】统一执行
        // (单指令源, 避免两处任务要求冲突导致输出混乱); 未证明时给"概率性作答"软指令。
        if let Some(cmd) = cmd {
            let delegated_to_spot = match cmd {
                UserCommand::Why(c) | UserCommand::Check(c) => is_engine_proven(ir, c),
                _ => false,
            };
            if !delegated_to_spot {
                msg.push_str(&format!("\n\n【指令任务】{}", command_directive(&cmd)));
                if let Some(c) = focus {
                    msg.push_str(&focus_context(view, ir, c));
                }
            }
        }

        // IR 已精简: 明确告知省略数量, 防止 LLM 编造被省略的条目
        if trimmed {
            msg.push_str(
                "\n注: IR 为精简版 (省略数量见 deterministic_omitted / probabilities_total 字段); \
                 被省略的条目不要引用、不要编造, 汇总时以省略计数说明即可。",
            );
        }

        // 用户提问: 让 LLM 围绕该问题作答 (教学模式下用提问引导, 其他模式直接解答)
        if let Some(q) = question {
            let q = q.trim();
            if !q.is_empty() {
                msg.push_str(&format!(
                    "\n\n【用户提问】{}\n请围绕上述提问给出回答：教学模式用引导问题逐步启发（严禁直接说出必雷/必安全结论）；答案/策略模式可直接解答。",
                    q
                ));
            }
        }

        // 旗帜可信度上下文: 用户标注的旗帜未必正确, LLM 不得把"矛盾旗/未确认旗"
        // 当作已知雷引用或断言, 必须把验证结论纳入推理解释 (flag_verification 在推理
        // 前由 Rust 完成, 此处仅转述)
        let intro = match self.language {
            Language::Chinese => "【旗帜验证说明】用户标注的旗帜未必全部正确, 推理前已做旗帜验证:",
            Language::English => "[Flag Verification] User-placed flags may be wrong; flags were verified before reasoning:",
        };
        let fv = &ir.flag_verification;
        let contrad: Vec<_> = fv
            .flags
            .iter()
            .filter(|f| f.status == crate::model::FlagVerifyStatus::Contradicted)
            .collect();
        let suspects: Vec<_> = fv
            .flags
            .iter()
            .filter(|f| f.status == crate::model::FlagVerifyStatus::Suspected)
            .collect();
        let attention: Vec<_> = fv
            .flags
            .iter()
            .filter(|f| f.needs_attention)
            .collect();
        if !fv.flags.is_empty() {
            msg.push_str(&format!("\n\n{} {}", intro, fv.summary));
            if !contrad.is_empty() {
                msg.push_str("\n矛盾旗（该格必非雷, 严禁将其当作已知雷引用/推荐保持标旗）:");
                for f in contrad.iter().take(12) {
                    msg.push_str(&format!("\n- 坐标 {}: {}", f.coord, f.reason));
                }
            }
            if !suspects.is_empty() {
                let coords: Vec<String> = suspects.iter().take(15).map(|f| format!("{}", f.coord)).collect();
                msg.push_str(&format!(
                    "\n未确认旗（推理无法断定其是否为雷, 只能在概率意义上讨论, 不得断言）: {}",
                    coords.join(", ")
                ));
            }
            if contrad.is_empty() && suspects.is_empty() {
                msg.push_str("\n结论: 所有旗帜均与数字约束自洽, 可当作已知雷使用。");
            }
        }
        // 旗帜问题警告: 概率可能出错 (LLM 必须显式告知用户并谨慎引用概率)
        if !attention.is_empty() {
            let coords: Vec<String> = attention.iter().map(|f| format!("{}", f.coord)).collect();
            msg.push_str(&format!(
                "\n\n⚠️ 重要警告: 当前存在 {} 面问题旗帜 {} (矛盾或极可能误标), 需要用户处理。\n以下概率与推理基于\"问题旗降级为未知、其余旗视为已知雷\"的容错假设, 数值可能出错; 请在回答开头提示用户先核对/移除问题旗帜, 并避免基于可能受影响的概率做出决定性断言。",
                attention.len(),
                coords.join(", ")
            ));
        }
        // 0% 概率语义提示: 未与数字相邻的未知格概率为 0, 通常意味着总雷数已在其他区域确认
        msg.push_str(
            "\n\n提示: 若某未知格不与任何已翻开数字相邻但雷概率为 0, 通常表示总雷数已被确认在其他格子区域 (该格必为安全); 引用此类结论时说明原因。",
        );

        // 抽查验证 (强制, 放在消息最末 → LLM 输出末尾): 对引擎已证明的格子
        // 给出完整推理链并以"最终结论: (x,y) 是雷/不是雷"收尾。
        // 默认自动挑选 1 个代表格; 用户在提问中指定坐标则验证该格。
        if let Some(spot) = resolve_spot_target(&self.mode, view, ir, cmd.as_ref(), question) {
            msg.push_str(&build_spot_check_block(view, ir, spot));
            // 目标格上下文 (状态/引擎结论/概率/旗验证/相邻约束) —
            // 指令委托时未在【指令任务】附过, 此处始终附带
            msg.push_str(&focus_context(view, ir, spot));
        }
        msg
    }

    /// 验证 LLM 输出是否只引用了 IR 中存在的坐标
    /// 返回 (是否有越界坐标, 越界坐标列表)
    pub fn validate_llm_output(_ir: &InferenceIR, view: &PlayerView, llm_response: &str) -> (bool, Vec<String>) {
        let valid_set: HashSet<String> = view.unknown.iter()
            .chain(view.flagged.iter())
            .chain(view.revealed.iter().map(|c| &c.coord))
            .map(|c| format!("({}, {})", c.x, c.y))
            .collect();

        let mut hallucinated: Vec<String> = Vec::new();

        for coord_str in extract_coords(llm_response) {
            if !valid_set.contains(&coord_str) {
                hallucinated.push(coord_str);
            }
        }

        (!hallucinated.is_empty(), hallucinated)
    }

    /// 本地转译（不调用LLM）：直接从IR生成简单文本
    /// 用于离线模式或LLM不可用时的回退
    pub fn local_translate(&self, ir: &InferenceIR) -> String {
        match &self.mode {
            LLMMode::Answer => self.local_answer(ir),
            LLMMode::Teaching => self.local_teaching(ir),
            LLMMode::Strategy => self.local_strategy(ir),
        }
    }

    fn local_answer(&self, ir: &InferenceIR) -> String {
        let mines = ir.all_mines();
        let safe = ir.all_safe();

        // 分级原则: 已标旗是"已知条件"。必雷列表只展示"未标旗的新发现";
        // 已标旗验证结果用摘要一句话带过, 不再逐条展开。
        let flagged_set: HashSet<Coord> = ir
            .flag_verification
            .flags
            .iter()
            .map(|f| f.coord)
            .collect();
        let mines_new: Vec<&Proof> = ir
            .deterministic
            .iter()
            .filter(|p| p.conclusion.is_mine && !flagged_set.contains(&p.conclusion.coord))
            .collect();
        let safe_new: Vec<&Proof> = ir
            .deterministic
            .iter()
            .filter(|p| !p.conclusion.is_mine && !flagged_set.contains(&p.conclusion.coord))
            .collect();

        let mut result = String::new();

        // 旗帜摘要 (已有旗验证结论; 有矛盾旗时给出可操作提示)
        let contrad: Vec<&crate::model::FlagStatus> = ir
            .flag_verification
            .flags
            .iter()
            .filter(|f| f.status == crate::model::FlagVerifyStatus::Contradicted)
            .collect();
        if !flagged_set.is_empty() {
            let fv = &ir.flag_verification;
            result.push_str(&format!("【旗帜检查】{}", fv.summary));
            if !contrad.is_empty() {
                let coords: Vec<String> = contrad
                    .iter()
                    .take(5)
                    .map(|f| format!("{}", f.coord))
                    .collect();
                let more = contrad.len().saturating_sub(5);
                let more = if more > 0 { format!(" 等共 {} 面", contrad.len()) } else { String::new() };
                result.push_str(&format!("\n- 矛盾旗 (实际必安全, 建议移除): {}{}", coords.join(", "), more));
            }
            result.push('\n');
        } else if !mines.is_empty() || !safe.is_empty() {
            // 无旗时无需小结
        }

        if !mines_new.is_empty() {
            result.push_str("【新发现·必雷 (未标旗)】\n");
            for proof in mines_new.iter().take(5) {
                result.push_str(&format!(
                    "- 坐标{} 是雷。依据: {}\n",
                    proof.conclusion.coord, proof.rule
                ));
            }
            let more = mines_new.len().saturating_sub(5);
            if more > 0 {
                result.push_str(&format!("- …其余 {} 个略\n", more));
            }
        } else if !mines.is_empty() {
            // 必雷全部已标旗 → 无需罗列 (已知条件)
            result.push_str("【新发现·必雷 (未标旗)】\n本次无新增必雷结论 (必雷均已标旗)。\n");
        }

        if !safe_new.is_empty() {
            result.push_str("\n【必安全·未翻开】\n");
            for proof in safe_new.iter().take(5) {
                result.push_str(&format!(
                    "- 坐标{} 安全。依据: {}\n",
                    proof.conclusion.coord, proof.rule
                ));
            }
            let more = safe_new.len().saturating_sub(5);
            if more > 0 {
                result.push_str(&format!("- …其余 {} 个略\n", more));
            }
        }

        if !ir.probabilities.is_empty() && mines_new.is_empty() && safe_new.is_empty() {
            result.push_str("【概率分析】\n");
            let mut sorted: Vec<_> = ir.probabilities.iter().collect();
            sorted.sort_by(|a, b| a.mine_probability.partial_cmp(&b.mine_probability).unwrap());

            result.push_str("最安全的格子:\n");
            for p in sorted.iter().take(3) {
                result.push_str(&format!(
                    "- 坐标{}: 雷概率 {:.1}%, 信息增益 {:.1}\n",
                    p.coord, p.mine_probability * 100.0, p.info_gain
                ));
            }
            if sorted.len() > 3 {
                result.push_str("最危险的格子:\n");
                for p in sorted.iter().rev().take(3) {
                    result.push_str(&format!(
                        "- 坐标{}: 雷概率 {:.1}%\n",
                        p.coord, p.mine_probability * 100.0
                    ));
                }
            }
        }

        if result.is_empty() {
            "当前局面无法做出确定性判断。建议在概率最低的格子中随机点击。".to_string()
        } else {
            result
        }
    }

    fn local_teaching(&self, ir: &InferenceIR) -> String {
        let mut result = String::new();
        result.push_str("让我们一步步分析这个局面：\n\n");

        for (i, proof) in ir.deterministic.iter().enumerate() {
            result.push_str(&format!("{}. {}\n", i + 1, proof.rule));
            if proof.conclusion.is_mine {
                result.push_str("   → 你觉得这个格子是什么？\n\n");
            } else {
                result.push_str("   → 想想看，这个格子是雷还是安全的？\n\n");
            }
        }

        if ir.deterministic.is_empty() {
            result.push_str("当前没有确定的推理。请看每个数字周围还有几个未知格，思考：\n");
            result.push_str("- 剩余雷数是否等于未知格数？\n");
            result.push_str("- 是否有子集关系可以利用？\n");
        }

        result
    }

    fn local_strategy(&self, ir: &InferenceIR) -> String {
        let mut result = String::new();

        if ir.regions.is_empty() {
            return "当前没有可供策略分析的区域。".to_string();
        }

        result.push_str("【区域策略分析】\n\n");

        let mut sorted_regions: Vec<_> = ir.regions.iter().collect();
        sorted_regions.sort_by(|a, b| {
            a.features.avg_mine_prob.partial_cmp(&b.features.avg_mine_prob).unwrap()
        });

        for (i, region) in sorted_regions.iter().enumerate() {
            result.push_str(&format!(
                "区域{}: {}个未知格, 平均雷概率 {:.1}%, 最低雷概率 {:.1}%, 边界长度 {}, 估计雷数 {:.1}\n",
                i + 1,
                region.features.unknown_count,
                region.features.avg_mine_prob * 100.0,
                region.features.min_mine_prob * 100.0,
                region.features.boundary_length,
                region.features.estimated_mines
            ));
        }

        if let Some(best) = sorted_regions.first() {
            result.push_str(&format!(
                "\n建议: 优先处理平均雷概率最低的区域（{:.1}%），该区域更可能有安全格。",
                best.features.avg_mine_prob * 100.0
            ));
        }

        result
    }
}

/// 格式化坐标显示
pub fn format_coord(coord: &Coord) -> String {
    format!("({}, {})", coord.x, coord.y)
}

/// 解析 "(x, y)" 字符串为 Coord (与 format_coord 互逆)
fn parse_coord_str(s: &str) -> Option<Coord> {
    let inner = s.trim().trim_start_matches('(').trim_end_matches(')');
    let (x, y) = inner.split_once(',')?;
    let x: u32 = x.trim().parse().ok()?;
    let y: u32 = y.trim().parse().ok()?;
    Some(Coord::new(x, y))
}

/// 用户指令 (对话区以 @ 开头触发, 让 Agent 具备主动推理任务)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserCommand {
    /// @why (x,y) — 推导某格为什么是雷/安全
    Why(Coord),
    /// @check (x,y) — 验证某面旗是否正确
    Check(Coord),
    /// @region — 分析分区情况
    Region,
    /// @teach — 苏格拉底教学模式
    Teach,
    /// @analyze — 全局推理分析
    Analyze,
}

/// 从用户输入解析指令; 非指令输入返回 None。
/// 支持 "@why (28,12)" / "@why(28, 12)" / "@check (28,12)" 等。
pub fn parse_user_command(input: &str) -> Option<UserCommand> {
    let t = input.trim();
    let lower = t.to_lowercase();
    if !lower.starts_with('@') {
        return None;
    }
    let (name, rest) = match lower.find(|c: char| c.is_whitespace() || c == '(') {
        Some(pos) => (&lower[..pos], &t[pos..]),
        None => (lower.as_str(), ""),
    };
    match name {
        "@why" | "@why?" => extract_coords(rest)
            .first()
            .and_then(|s| parse_coord_str(s))
            .map(UserCommand::Why),
        "@check" | "@flag" => extract_coords(rest)
            .first()
            .and_then(|s| parse_coord_str(s))
            .map(UserCommand::Check),
        "@region" | "@regions" => Some(UserCommand::Region),
        "@teach" | "@teaching" => Some(UserCommand::Teach),
        "@analyze" | "@analysis" => Some(UserCommand::Analyze),
        _ => None,
    }
}

/// 推理步骤 (从 LLM 自由文本中提取, 供前端逐步高亮)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReasoningStep {
    /// 步骤文本 (截断)
    pub text: String,
    /// 该步骤涉及的合法坐标 (点击步骤时棋盘高亮)
    pub coords: Vec<Coord>,
}

/// 从 LLM 回答中提取推理节点: 含 ≥1 个合法坐标且带推理标记的行。
/// 标记: → / 因为 / 所以 / 必(雷|安全) / 剩余需雷 / 若 / 约束 / 概率
pub fn extract_reasoning_nodes(response: &str, view: &PlayerView) -> Vec<ReasoningStep> {
    let valid: HashSet<String> = view
        .unknown
        .iter()
        .chain(view.flagged.iter())
        .chain(view.revealed.iter().map(|c| &c.coord))
        .map(format_coord)
        .collect();
    let mut steps = Vec::new();
    for line in response.lines() {
        if steps.len() >= 12 {
            break;
        }
        let line = line.trim();
        if line.is_empty() || line.starts_with('|') && line.contains("---") {
            continue;
        }
        let coords: Vec<Coord> = extract_coords(line)
            .into_iter()
            .filter_map(|s| parse_coord_str(&s))
            .filter(|c| valid.contains(&format_coord(c)))
            .collect();
        if coords.is_empty() {
            continue;
        }
        let marker = line.contains('→')
            || line.contains("第 ") && (line.contains("步"))
            || line.contains("步骤")
            || line.contains("候选")
            || line.contains("因为")
            || line.contains("所以")
            || line.contains("必")
            || line.contains("剩余需雷")
            || line.contains("剩")
            || line.contains("若")
            || line.contains("约束")
            || line.contains("概率");
        if !marker {
            continue;
        }
        let text: String = line.chars().take(120).collect();
        steps.push(ReasoningStep { text, coords });
    }
    steps
}

/// 数字约束表行: 前沿数字的原始约束 (LLM 展开专家推理链的原料)。
/// focus 的相邻数字优先且保证不被截断。
fn build_constraint_lines(view: &PlayerView, focus: Option<Coord>, max_lines: usize) -> Vec<String> {
    let state = view.to_state_map();
    let line_for = |cell: &crate::model::RevealedCell| -> Option<String> {
        let neighbors = cell.coord.neighbors(view.width, view.height);
        let flagged = neighbors
            .iter()
            .filter(|n| matches!(state.get(n), Some(crate::model::CellState::Flagged)))
            .count() as i32;
        let unknowns: Vec<String> = neighbors
            .iter()
            .filter(|n| matches!(state.get(n), Some(crate::model::CellState::Unknown)))
            .map(format_coord)
            .collect();
        if unknowns.is_empty() {
            return None;
        }
        Some(format!(
            "数字{}@{}: 旗{} 剩{} 未知[{}]",
            cell.number,
            format_coord(&cell.coord),
            flagged,
            cell.number as i32 - flagged,
            unknowns.join(",")
        ))
    };

    let mut focused: Vec<String> = Vec::new();
    if let Some(f) = focus {
        for n in f.neighbors(view.width, view.height) {
            if let Some(crate::model::CellState::Revealed(_)) = state.get(&n) {
                if let Some(cell) = view.revealed.iter().find(|c| c.coord == n) {
                    if let Some(line) = line_for(cell) {
                        focused.push(line);
                    }
                }
            }
        }
    }

    let mut rest: Vec<String> = Vec::new();
    for cell in &view.revealed {
        if focus.is_some_and(|f| f.neighbors(view.width, view.height).contains(&cell.coord)) {
            continue; // 已在 focused 中
        }
        if let Some(line) = line_for(cell) {
            rest.push(line);
        }
    }
    let mut lines = focused;
    let budget = max_lines.saturating_sub(lines.len());
    lines.extend(rest.into_iter().take(budget));
    lines
}

/// 指令的任务描述 (追加在用户消息中, 引导 LLM 执行针对性任务; 需遵守输出纪律)
fn command_directive(cmd: &UserCommand) -> String {
    match cmd {
        UserCommand::Why(c) => format!(
            "用户询问格子 {} 为什么是雷/安全 (该格未被引擎确定性证明)。作答要求: \
             以「## 回答用户指令」为标题 (位于模板区块之后); 若确定性结论相关则引用其依据; \
             若只有概率, 给出雷概率与决策建议; 禁止编造确定性结论。",
            format_coord(c)
        ),
        UserCommand::Check(c) => format!(
            "用户要求检查格子 {} 上的旗帜。作答要求: 以「## 回答用户指令」为标题 (位于模板区块之后); \
             对照数字约束表检查该旗是否与相邻数字一致; 无法证明必雷时给出概率判断并建议; \
             禁止编造\"必雷/必安全\"结论。",
            format_coord(c)
        ),
        UserCommand::Region => String::from(
            "用户要求分析棋盘分区: 以「## 回答用户指令」为标题 (位于模板区块之后)。基于 IR 的 \
             regions (每区域: 未知格数 / 估计雷数 / 边界长度 / 平均与最低雷概率), 指出孤立区域、\
             风险最低与最高的区域、推荐处理顺序。用列表呈现, 简明, ≤200 字。",
        ),
        UserCommand::Teach => String::from(
            "用户要求进入苏格拉底教学: 不直接给答案, 围绕当前局面最关键的 1 个推理节点\
             (优先矛盾旗或必安全格), 用最多 2 个引导问题让用户自己发现推理; 用户答错时给 1 条提示。",
        ),
        UserCommand::Analyze => String::from(
            "用户要求全局推理分析: 按输出模板完整执行, 推理链区块只展开 1 个最关键节点。",
        ),
    }
    .to_string()
}

/// 目标格上下文: 当前状态 / 引擎结论 / 概率 / 旗验证 / 全部相邻数字约束
fn focus_context(view: &PlayerView, ir: &InferenceIR, c: Coord) -> String {
    let mut ctx = format!("\n\n【目标格 {} 上下文】", format_coord(&c));
    match view.to_state_map().get(&c) {
        Some(crate::model::CellState::Unknown) => ctx.push_str("\n当前状态: 未翻开"),
        Some(crate::model::CellState::Flagged) => ctx.push_str("\n当前状态: 已标旗"),
        Some(crate::model::CellState::Revealed(n)) => {
            ctx.push_str(&format!("\n当前状态: 已翻开数字 {}", n))
        }
        None => ctx.push_str("\n当前状态: 越界 (请指出坐标不合法)"),
    }
    if let Some(p) = ir.deterministic.iter().find(|p| p.conclusion.coord == c) {
        ctx.push_str(&format!(
            "\n引擎结论: {} — {} (依据数字: {})",
            if p.conclusion.is_mine { "必雷" } else { "必安全" },
            p.rule,
            p.depends_on.iter().map(format_coord).collect::<Vec<_>>().join(", ")
        ));
    }
    if let Some(prob) = ir.prob_of(&c) {
        ctx.push_str(&format!(
            "\n引擎概率: 雷 {:.1}% (信息增益 {:.1})",
            prob.mine_probability * 100.0,
            prob.info_gain
        ));
    }
    if let Some(f) = ir.flag_verification.flags.iter().find(|f| f.coord == c) {
        ctx.push_str(&format!(
            "\n旗验证: {:?} — {}",
            f.status, f.reason
        ));
    }
    let neighbor_constraints = build_constraint_lines(view, Some(c), usize::MAX);
    if !neighbor_constraints.is_empty() {
        ctx.push_str("\n该格全部相邻数字约束:");
        for l in &neighbor_constraints {
            ctx.push_str(&format!("\n- {}", l));
        }
    }
    ctx
}

/// 从文本中提取所有 (x, y) 格式的坐标
fn extract_coords(text: &str) -> Vec<String> {
    let mut result = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'(' {
            let mut j = i + 1;
            let mut found_comma = false;
            let mut found_close = false;
            while j < bytes.len() && j < i + 30 {
                if bytes[j] == b',' {
                    found_comma = true;
                }
                if bytes[j] == b')' {
                    found_close = true;
                    break;
                }
                j += 1;
            }
            if found_comma && found_close {
                let candidate = &text[i..=j];
                let inner = &candidate[1..candidate.len()-1];
                let parts: Vec<&str> = inner.split(',').map(|s| s.trim()).collect();
                if parts.len() == 2 {
                    let p0 = parts[0].trim_start_matches('(').trim();
                    let p1 = parts[1].trim_end_matches(')').trim();
                    if p0.chars().all(|c| c.is_ascii_digit())
                        && p1.chars().all(|c| c.is_ascii_digit())
                        && !p0.is_empty() && !p1.is_empty()
                    {
                        result.push(format!("({}, {})", p0, p1));
                    }
                }
            }
        }
        i += 1;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn make_test_ir() -> InferenceIR {
        InferenceIR {
            deterministic: vec![
                Proof {
                    conclusion: Conclusion { coord: Coord::new(1, 0), is_mine: true },
                    depends_on: vec![Coord::new(0, 0)],
                    rule: "数字3周围剩余3个未知格，还需3雷 → 全部为雷".to_string(),
                },
                Proof {
                    conclusion: Conclusion { coord: Coord::new(0, 1), is_mine: false },
                    depends_on: vec![Coord::new(0, 2)],
                    rule: "数字0周围剩余雷数为0 → 该格安全".to_string(),
                },
            ],
            probabilities: vec![
                CellProb { coord: Coord::new(2, 0), mine_probability: 0.33, info_gain: 5.0 },
                CellProb { coord: Coord::new(2, 1), mine_probability: 0.17, info_gain: 3.0 },
            ],
            regions: vec![
                Region {
                    cells: vec![Coord::new(2, 0), Coord::new(2, 1)],
                    features: RegionFeature {
                        unknown_count: 2,
                        estimated_mines: 0.5,
                        boundary_length: 2,
                        avg_mine_prob: 0.25,
                        min_mine_prob: 0.17,
                    },
                },
            ],
            flag_verification: crate::model::FlagVerificationResult::empty(),
        }
    }

    #[test]
    fn test_answer_mode() {
        let t = Translator::new(LLMMode::Answer);
        let ir = make_test_ir();
        let result = t.local_translate(&ir);
        // 分级标题: 未标旗必雷作为"新发现"; 安全未翻开单独一节
        assert!(result.contains("必雷 (未标旗)"));
        assert!(result.contains("必安全·未翻开"));
        assert!(result.contains("(1, 0)"));
    }

    #[test]
    fn test_teaching_mode() {
        let t = Translator::new(LLMMode::Teaching);
        let ir = make_test_ir();
        let result = t.local_translate(&ir);
        assert!(result.contains("让我们一步步"));
        assert!(result.contains("？"));
    }

    #[test]
    fn test_strategy_mode() {
        let t = Translator::new(LLMMode::Strategy);
        let ir = make_test_ir();
        let result = t.local_translate(&ir);
        assert!(result.contains("区域策略分析"));
        assert!(result.contains("建议"));
    }

    #[test]
    fn test_system_prompt() {
        let t = Translator::new(LLMMode::Answer);
        let prompt = t.build_system_prompt();
        assert!(prompt.contains("数据防火墙"));
        assert!(prompt.contains("答案模式"));
        assert!(prompt.contains("输出纪律"));   // 输出纪律规则块
        assert!(prompt.contains("坐标引用"));
        assert!(prompt.contains("mine_probability"));
        assert!(prompt.contains("下一步建议"));
        // 输出纪律: 禁止自白 / 限长 400 / 严格区块
        assert!(prompt.contains("Let me"));
        assert!(prompt.contains("400"));
        assert!(prompt.contains("最终结论"));
        assert!(prompt.contains("抽查验证"));
    }

    #[test]
    fn test_teaching_prompt_requires_full_output() {
        // 教学模式提示词: 智能自适应 (不是复读机), 强制结构化 + 限长
        let t = Translator::new(LLMMode::Teaching);
        let prompt = t.build_system_prompt();
        assert!(prompt.contains("不是**复读机**") || prompt.contains("不是复读机"));
        assert!(prompt.contains("规则 1"));
        assert!(prompt.contains("## 直接原因"));
        assert!(prompt.contains("## 引导问题"));
        assert!(prompt.contains("3-5 个引导性问题"));
        assert!(prompt.contains("不超过 5 个"));
        assert!(prompt.contains("30 字"));
        assert!(prompt.contains("禁止空标题"));
        assert!(prompt.contains("简体中文"));
    }

    #[test]
    fn test_teaching_prompt_english() {
        let mut t = Translator::new(LLMMode::Teaching);
        t.set_language(Language::English);
        let prompt = t.build_system_prompt();
        assert!(prompt.contains("## Guiding Questions"));
        assert!(prompt.contains("3 to 5 questions"));
        assert!(prompt.contains("30 words"));
        assert!(prompt.contains("Respond entirely in English"));
        assert!(prompt.contains("## Direct Reason"));
    }

    #[test]
    fn test_language_parse() {
        assert_eq!(Language::parse(Some("en")), Language::English);
        assert_eq!(Language::parse(Some("EN")), Language::English);
        assert_eq!(Language::parse(Some("english")), Language::English);
        assert_eq!(Language::parse(Some("zh")), Language::Chinese);
        assert_eq!(Language::parse(None), Language::Chinese);
        assert_eq!(Language::parse(Some("de")), Language::Chinese);
    }

    #[test]
    fn test_output_incompleteness() {
        // 空输出
        assert!(is_output_incomplete(""));
        assert!(is_output_incomplete("   \n  "));
        // 以 ## 开头但标题后无正文 → "输出被吃"
        assert!(is_output_incomplete("## "));
        assert!(is_output_incomplete("## 引导问题"));
        // 完整结构化输出 → 正常
        let full = "## 引导问题\n\n1. 【坐标(0,1)数字3周围还剩几个未知格?】\n2. 【(1,2)周围已标几面旗?】\n3. 【剩余雷数=数字-旗数, 还差多少?】";
        assert!(!is_output_incomplete(full));
        // 自适应规则1: "## 直接原因"+一段充分解释 → 正常 (不误判为被吃)
        let reason = "## 直接原因\n\n因为坐标(0,1)的数字3周围只剩1个未知格且仍需1雷, 该格必雷";
        assert!(!is_output_incomplete(reason));
        // 普通长文本 (非 ## 开头) → 正常
        assert!(!is_output_incomplete("坐标(0, 1)是安全的，因为……"));
    }

    #[test]
    fn test_user_message_carries_question() {
        let view = PlayerView {
            width: 3, height: 3,
            revealed: vec![],
            flagged: vec![],
            unknown: vec![Coord::new(1, 0), Coord::new(2, 1)],
            remaining_mines: 1,
        };
        let ir = InferenceIR {
            deterministic: vec![],
            probabilities: vec![],
            regions: vec![],
            flag_verification: FlagVerificationResult {
                flags: vec![FlagStatus {
                    coord: Coord::new(1, 0),
                    status: FlagVerifyStatus::Contradicted,
                    reason: "数字1位于(0,0)周围旗数超限".into(),
                    needs_attention: true,
                }],
                summary: "发现 1 面可能标错".into(),
                has_contradiction: true,
            },
        };
        let t = Translator::new(LLMMode::Teaching);
        let msg = t.build_user_message_with_question(&view, &ir, Some("目前的局面怎么进行解决"));
        assert!(msg.contains("【用户提问】"));
        assert!(msg.contains("目前的局面怎么进行解决"));
        assert!(msg.contains("旗帜验证说明"));
        assert!(msg.contains("矛盾旗"));
        assert!(msg.contains("(1, 0)"));
        // 不带提问时不出现该块
        let plain = t.build_user_message(&view, &ir);
        assert!(!plain.contains("【用户提问】"));
        assert!(plain.contains("旗帜验证说明"));
    }

    #[test]
    fn test_compact_ir_small_kept_full() {
        // 小 IR: 全量保留, 不触发裁剪提示
        let ir = InferenceIR {
            deterministic: vec![Proof {
                conclusion: Conclusion { coord: Coord::new(0, 0), is_mine: false },
                depends_on: vec![],
                rule: "测试".into(),
            }],
            probabilities: vec![CellProb {
                coord: Coord::new(1, 0),
                mine_probability: 0.25,
                info_gain: 3.0,
            }],
            regions: vec![],
            flag_verification: FlagVerificationResult::empty(),
        };
        let (json, trimmed) = compact_ir_json(&ir);
        assert!(!trimmed);
        assert!(json.contains("\"deterministic_total\":1"));
        assert!(json.contains("\"probabilities_total\":1"));
    }

    #[test]
    fn test_compact_ir_large_trimmed() {
        // 大 IR: 60 条证明 + 200 条概率 + 80 面 Verified 旗 → 裁剪
        let deterministic: Vec<Proof> = (0..60)
            .map(|i| Proof {
                conclusion: Conclusion { coord: Coord::new(i % 30, i / 30), is_mine: i % 2 == 0 },
                depends_on: vec![Coord::new(0, 0)],
                rule: format!("规则 #{}", i),
            })
            .collect();
        let probabilities: Vec<CellProb> = (0..200)
            .map(|i| CellProb {
                coord: Coord::new(i % 30, i / 30),
                mine_probability: (i % 100) as f64 / 100.0,
                info_gain: 2.0,
            })
            .collect();
        let flags: Vec<FlagStatus> = (0..80)
            .map(|i| FlagStatus {
                coord: Coord::new(i % 30, i / 30),
                status: FlagVerifyStatus::Verified,
                reason: "无矛盾".into(),
                needs_attention: false,
            })
            .collect();
        let ir = InferenceIR {
            deterministic,
            probabilities,
            regions: vec![],
            flag_verification: FlagVerificationResult {
                flags,
                summary: "已检查 80 面旗".into(),
                has_contradiction: false,
            },
        };
        let (json, trimmed) = compact_ir_json(&ir);
        assert!(trimmed);
        // 计数字段保留全量信息
        assert!(json.contains("\"deterministic_total\":60"));
        assert!(json.contains("\"deterministic_omitted\":45"));
        assert!(json.contains("\"probabilities_total\":200"));
        assert!(json.contains("\"flags_total\":80"));
        // Verified 旗明细被省略
        assert!(!json.contains("无矛盾"));
        // 裁剪后体积显著小于全量 pretty 序列化
        let full = serde_json::to_string_pretty(&ir).unwrap();
        assert!(json.len() * 3 < full.len(), "compact={} full={}", json.len(), full.len());
    }

    #[test]
    fn test_user_message_notes_trimming() {
        let deterministic: Vec<Proof> = (0..30)
            .map(|i| Proof {
                conclusion: Conclusion { coord: Coord::new(i, 0), is_mine: true },
                depends_on: vec![],
                rule: "r".into(),
            })
            .collect();
        let ir = InferenceIR {
            deterministic,
            probabilities: vec![],
            regions: vec![],
            flag_verification: FlagVerificationResult::empty(),
        };
        let view = PlayerView {
            width: 30, height: 1,
            revealed: vec![],
            flagged: vec![],
            unknown: vec![Coord::new(0, 0)],
            remaining_mines: 10,
        };
        let t = Translator::new(LLMMode::Answer);
        let msg = t.build_user_message(&view, &ir);
        assert!(msg.contains("deterministic_omitted"));
        assert!(msg.contains("精简版"));
    }

    #[test]
    fn test_parse_user_command() {
        assert_eq!(
            parse_user_command("@why (28,12)"),
            Some(UserCommand::Why(Coord::new(28, 12)))
        );
        assert_eq!(
            parse_user_command("@Why(28, 12) 为什么不是雷"),
            Some(UserCommand::Why(Coord::new(28, 12)))
        );
        assert_eq!(
            parse_user_command("@check (28,12)"),
            Some(UserCommand::Check(Coord::new(28, 12)))
        );
        assert_eq!(parse_user_command("@region"), Some(UserCommand::Region));
        assert_eq!(parse_user_command("@teach"), Some(UserCommand::Teach));
        assert_eq!(parse_user_command("@analyze"), Some(UserCommand::Analyze));
        // 非指令 / 未知指令 / 缺坐标 → None
        assert_eq!(parse_user_command("为什么 (28,12) 是雷"), None);
        assert_eq!(parse_user_command("@unknown"), None);
        assert_eq!(parse_user_command("@why"), None);
    }

    #[test]
    fn test_extract_reasoning_nodes() {
        let view = PlayerView {
            width: 5, height: 5,
            revealed: vec![crate::model::RevealedCell { coord: Coord::new(0, 0), number: 1 }],
            flagged: vec![],
            unknown: vec![Coord::new(1, 0), Coord::new(2, 0)],
            remaining_mines: 1,
        };
        let response = "## 直接原因\n\
            数字1@(0, 0): 旗0 剩1 未知[(1,0),(2,0)] → 剩余需雷数=1, 但 (2,0) 已被约束排除 → (1,0) 必为雷\n\
            这行没有坐标不被提取\n\
            所以 (2,0) 必安全\n";
        let steps = extract_reasoning_nodes(response, &view);
        assert_eq!(steps.len(), 2);
        assert!(steps[0].text.contains("数字1@(0, 0)"));
        assert!(steps[0].coords.contains(&Coord::new(1, 0)));
        assert!(steps[1].coords.contains(&Coord::new(2, 0)));
        // 越界/不存在坐标被过滤 (99,99 不在棋盘)
        assert!(extract_reasoning_nodes("(99,99) 必安全", &view).is_empty());
    }

    #[test]
    fn test_build_constraint_lines() {
        // 3x3: (0,0)=1 与 (2,0)=1, (2,0) 旁有一面旗
        let view = PlayerView {
            width: 3, height: 3,
            revealed: vec![
                crate::model::RevealedCell { coord: Coord::new(0, 0), number: 1 },
                crate::model::RevealedCell { coord: Coord::new(2, 0), number: 1 },
            ],
            flagged: vec![Coord::new(2, 1)],
            unknown: vec![Coord::new(1, 0), Coord::new(1, 1)],
            remaining_mines: 1,
        };
        let lines = build_constraint_lines(&view, None, 60);
        assert_eq!(lines.len(), 2); // 两个数字都有未知邻居
        assert!(lines[0].contains("数字1@(0, 0): 旗0 剩1"));
        assert!(lines.iter().any(|l| l.contains("数字1@(2, 0): 旗1 剩0")));
        // focus (1,1): 其相邻数字 (0,0) 与 (2,0) 的约束都在且排最前
        let focused = build_constraint_lines(&view, Some(Coord::new(1, 1)), 60);
        assert_eq!(focused.len(), 2);
        assert!(focused[0].contains("@(0, 0)") || focused[0].contains("@(2, 0)"));
        // 截断到 1 行
        assert_eq!(build_constraint_lines(&view, None, 1).len(), 1);
    }

    #[test]
    fn test_user_message_with_command_and_constraints() {
        let view = PlayerView {
            width: 3, height: 3,
            revealed: vec![crate::model::RevealedCell { coord: Coord::new(0, 0), number: 1 }],
            flagged: vec![Coord::new(1, 0)],
            unknown: vec![Coord::new(1, 1)],
            remaining_mines: 1,
        };
        let ir = InferenceIR {
            deterministic: vec![Proof {
                conclusion: Conclusion { coord: Coord::new(1, 0), is_mine: false },
                depends_on: vec![Coord::new(0, 0)],
                rule: "局部枚举: 恒安全".into(),
            }],
            probabilities: vec![],
            regions: vec![],
            flag_verification: FlagVerificationResult {
                flags: vec![FlagStatus {
                    coord: Coord::new(1, 0),
                    status: FlagVerifyStatus::Contradicted,
                    reason: "枚举恒安全".into(),
                    needs_attention: true,
                }],
                summary: "1 面误标".into(),
                has_contradiction: true,
            },
        };
        let t = Translator::new(LLMMode::Answer);
        // 目标格 (1,0) 已被引擎证明 → 委托给消息末尾的抽查验证块 (单一任务源)
        let msg = t.build_user_message_with_question(&view, &ir, Some("@why (1,0)"));
        // 数字约束表存在: (0,0)=1 的邻居含旗(1,0)与未知(1,1) → 旗1 剩0
        assert!(msg.contains("数字约束表"));
        assert!(msg.contains("数字1@(0, 0): 旗1 剩0"));
        // 不再输出【指令任务】, 由【抽查验证任务】统一要求推导 + 最终结论
        assert!(!msg.contains("【指令任务】"));
        assert!(msg.contains("【抽查验证任务】"));
        assert!(msg.contains("【目标格 (1, 0) 上下文】"));
        assert!(msg.contains("当前状态: 已标旗"));
        assert!(msg.contains("引擎结论: 必安全"));
        assert!(msg.contains("旗验证"));
        assert!(msg.contains("最终结论: (1, 0) 不是雷"));
        // 无指令时约束表仍在, 也无【指令任务】 (但矛盾旗可被自动抽查, 见另一测试)
        let plain = t.build_user_message(&view, &ir);
        assert!(plain.contains("数字约束表"));
        assert!(!plain.contains("【指令任务】"));

        // @why 指向未被引擎证明的格 → 保留【指令任务】软指令 (概率性作答)
        let unproven_view = PlayerView {
            width: 3, height: 3,
            revealed: vec![crate::model::RevealedCell { coord: Coord::new(0, 0), number: 1 }],
            flagged: vec![],
            unknown: vec![Coord::new(1, 1)],
            remaining_mines: 1,
        };
        let m = t.build_user_message_with_question(&unproven_view, &ir, Some("@why (1,1)"));
        assert!(m.contains("【指令任务】"));
        assert!(m.contains("禁止编造"));
    }

    #[test]
    fn test_pick_spot_check_prefers_unflagged_mine() {
        // 3x3: (1,0) 必雷未标旗, (1,1) 必安全 → 应优先选 (1,0)
        let view = PlayerView {
            width: 3, height: 3,
            revealed: vec![
                crate::model::RevealedCell { coord: Coord::new(0, 0), number: 2 },
                crate::model::RevealedCell { coord: Coord::new(0, 1), number: 1 },
            ],
            flagged: vec![],
            unknown: vec![Coord::new(1, 0), Coord::new(1, 1), Coord::new(2, 1)],
            remaining_mines: 1,
        };
        let ir = InferenceIR {
            deterministic: vec![
                Proof {
                    conclusion: Conclusion { coord: Coord::new(1, 0), is_mine: true },
                    depends_on: vec![Coord::new(0, 0), Coord::new(0, 1)],
                    rule: "组合枚举".into(),
                },
                Proof {
                    conclusion: Conclusion { coord: Coord::new(1, 1), is_mine: false },
                    depends_on: vec![Coord::new(0, 0)],
                    rule: "枚举恒安全".into(),
                },
            ],
            probabilities: vec![],
            regions: vec![],
            flag_verification: FlagVerificationResult::empty(),
        };
        assert_eq!(pick_spot_check_target(&view, &ir), Some(Coord::new(1, 0)));
    }

    #[test]
    fn test_pick_spot_excludes_trivial_all_mines_block() {
        // 平凡块: 3x3 右列三格全为未知且全被证明必雷 → 整块必雷, 全部排除 → 无候选
        let view = PlayerView {
            width: 3, height: 3,
            revealed: vec![
                crate::model::RevealedCell { coord: Coord::new(0, 0), number: 3 },
                crate::model::RevealedCell { coord: Coord::new(0, 1), number: 3 },
                crate::model::RevealedCell { coord: Coord::new(0, 2), number: 3 },
            ],
            flagged: vec![],
            unknown: vec![Coord::new(1, 0), Coord::new(1, 1), Coord::new(1, 2)],
            remaining_mines: 3,
        };
        let mine_of = |c: Coord, n: u32| Proof {
            conclusion: Conclusion { coord: c, is_mine: true },
            depends_on: vec![Coord::new(0, n)],
            rule: "必雷".into(),
        };
        let ir = InferenceIR {
            deterministic: vec![
                mine_of(Coord::new(1, 0), 0),
                mine_of(Coord::new(1, 1), 1),
                mine_of(Coord::new(1, 2), 2),
            ],
            probabilities: vec![],
            regions: vec![],
            flag_verification: FlagVerificationResult::empty(),
        };
        assert_eq!(pick_spot_check_target(&view, &ir), None);

        // 同块但 (1,2) 改为必安全 → 块含非必雷格, 非平凡 → 必有未标旗必雷可选 (并列取序末)
        let mut ir2 = ir.clone();
        ir2.deterministic[2].conclusion.is_mine = false;
        ir2.deterministic[2].rule = "枚举恒安全".into();
        assert_eq!(pick_spot_check_target(&view, &ir2), Some(Coord::new(1, 1)));

        // 只留安全结论, 无未标旗必雷 → 回退到必安全格 (1,2)
        ir2.deterministic.retain(|p| !p.conclusion.is_mine);
        assert_eq!(pick_spot_check_target(&view, &ir2), Some(Coord::new(1, 2)));
    }

    #[test]
    fn test_spot_check_block_in_user_message() {
        let view = PlayerView {
            width: 3, height: 3,
            revealed: vec![crate::model::RevealedCell { coord: Coord::new(0, 0), number: 1 }],
            flagged: vec![],
            unknown: vec![Coord::new(1, 0), Coord::new(1, 1)],
            remaining_mines: 1,
        };
        let ir = InferenceIR {
            deterministic: vec![
                Proof {
                    conclusion: Conclusion { coord: Coord::new(1, 1), is_mine: true },
                    depends_on: vec![Coord::new(0, 0)],
                    rule: "组合枚举: 该格若安全则数字1无雷可放".into(),
                },
                Proof {
                    conclusion: Conclusion { coord: Coord::new(1, 0), is_mine: false },
                    depends_on: vec![Coord::new(0, 0)],
                    rule: "枚举: 该格若雷则数字1邻居超限".into(),
                },
            ],
            probabilities: vec![],
            regions: vec![],
            flag_verification: FlagVerificationResult::empty(),
        };
        // 答案模式默认 (无提问): 自动抽查 (1,1), 强制最终结论行
        let t = Translator::new(LLMMode::Answer);
        let msg = t.build_user_message(&view, &ir);
        assert!(msg.contains("【抽查验证任务】"));
        assert!(msg.contains("最终结论: (1, 1) 是雷"));
        assert!(msg.contains("## 🔬 抽查验证"));
        // 用户指定 (1,0) → 只验证 (1,0)
        let msg2 = t.build_user_message_with_question(&view, &ir, Some("那 (1,0) 呢?"));
        assert!(msg2.contains("最终结论: (1, 0) 不是雷"));
        assert!(!msg2.contains("最终结论: (1, 1)"));
        // 提问无坐标 → 默认自动
        let msg3 = t.build_user_message_with_question(&view, &ir, Some("下一步怎么走?"));
        assert!(msg3.contains("最终结论: (1, 1) 是雷"));
        // 教学/策略模式不输出抽查
        let teach = Translator::new(LLMMode::Teaching);
        assert!(!teach.build_user_message(&view, &ir).contains("【抽查验证任务】"));
        let strategy = Translator::new(LLMMode::Strategy);
        assert!(!strategy.build_user_message(&view, &ir).contains("最终结论:"));
    }

    #[test]
    fn test_spot_check_specified_unproven_suppressed() {
        // 提问带坐标但该格未被引擎证明 → 不抽查, 不做默认自动 (答非所问)
        let view = PlayerView {
            width: 3, height: 3,
            revealed: vec![crate::model::RevealedCell { coord: Coord::new(0, 0), number: 1 }],
            flagged: vec![],
            unknown: vec![Coord::new(1, 0), Coord::new(1, 1)],
            remaining_mines: 1,
        };
        let ir = InferenceIR {
            deterministic: vec![Proof {
                conclusion: Conclusion { coord: Coord::new(1, 1), is_mine: true },
                depends_on: vec![Coord::new(0, 0)],
                rule: "必雷".into(),
            }],
            probabilities: vec![],
            regions: vec![],
            flag_verification: FlagVerificationResult::empty(),
        };
        let t = Translator::new(LLMMode::Answer);
        // (0,0) 是已翻开数字, 引擎无结论 → 不生成抽查块
        let msg = t.build_user_message_with_question(&view, &ir, Some("数字 (0,0) 说明什么?"));
        assert!(!msg.contains("【抽查验证任务】"));
    }

    #[test]
    fn test_clean_llm_output() {
        // 自白行剔除 + 首个 ## 前的引言段丢弃
        let raw = "Let me analyze this minesweeper position carefully.\n\
                   Wait, I need to check the numbers around (28,12) first...\n\
                   ## 🎯 局面评估\n残局, 剩 9 雷。\n\n\n\n## 💡 下一步建议\n翻开 (28,12)。";
        let (cleaned, dropped) = clean_llm_output(raw);
        assert_eq!(dropped, 2);
        assert!(!cleaned.contains("Let me"));
        assert!(!cleaned.contains("Wait,"));
        assert!(!cleaned.starts_with("I need"));
        assert!(cleaned.starts_with("## 🎯"));
        assert!(cleaned.contains("翻开 (28,12)"));
        // 多余空行被压缩
        assert!(!cleaned.contains("\n\n\n"));
        // 正常输出不受影响 (行内坐标等非行首标记)
        let ok_raw = "## 🎯 局面评估\n残局阶段。\n最终结论: (28, 12) 不是雷";
        let (cleaned2, d2) = clean_llm_output(ok_raw);
        assert_eq!(d2, 0);
        assert!(cleaned2.contains("最终结论"));
    }

    #[test]
    fn test_is_output_chaotic() {
        // 混乱: 大段英文思考自白, 无区块
        assert!(is_output_chaotic(
            "Let me analyze this minesweeper position carefully.\nActually, wait... I think the best move is..."
        ));
        // 混乱: 自白 + 区块混杂 (≥2 行标记)
        assert!(is_output_chaotic(
            "Let me think...\n## 🎯 局面评估\n残局。\nI think (28,12) is safe."
        ));
        // 正常: 结构良好中文输出 → 不判定混乱
        assert!(!is_output_chaotic(
            "## 🎯 局面评估\n残局阶段, 剩 9 雷。\n## 💡 下一步建议\n翻开 (28,12)。"
        ));
        // 纯箭头链文本 (抽查验证偶尔不带 ##) → 不判定混乱
        assert!(!is_output_chaotic("数字3@(27,11) → (28,12) 恒安全"));
    }

    #[test]
    fn test_truncate_llm_output_keeps_verdict() {
        let long = "字".repeat(2000);
        let cut1 = truncate_llm_output(&long, 1200);
        assert!(cut1.contains("已精简"));
        assert!(cut1.chars().count() < 2000);
        // 未超长时不截断
        assert_eq!(truncate_llm_output("短文本", 1200), "短文本");
        // 含最终结论的过长输出 → 截断后保留结论行
        let mut raw = "x".repeat(2000);
        raw.push_str("\n最终结论: (28, 12) 不是雷");
        let cut = truncate_llm_output(&raw, 800);
        assert!(cut.contains("最终结论: (28, 12) 不是雷"));
        assert!(cut.contains("已精简"));
        // 不再出现大段 "共 N 字" 统计 (用户反馈的刷屏问题)
        assert!(!cut.contains("共 2"));
    }

    #[test]
    fn test_extract_coords() {
        let text = "坐标(2, 3)是雷，而 (0, 1)安全。注意 (abc, def) 不是坐标。";
        let coords = extract_coords(text);
        assert!(coords.contains(&"(2, 3)".to_string()));
        assert!(coords.contains(&"(0, 1)".to_string()));
        assert!(!coords.iter().any(|c| c.contains("abc")));
    }

    #[test]
    fn test_validate_llm_output_no_hallucination() {
        let view = PlayerView {
            width: 3, height: 3,
            revealed: vec![],
            flagged: vec![],
            unknown: vec![Coord::new(1, 0), Coord::new(2, 1)],
            remaining_mines: 1,
        };
        let ir = InferenceIR {
            deterministic: vec![],
            probabilities: vec![],
            regions: vec![],
        
            flag_verification: crate::model::FlagVerificationResult::empty(),};
        let response = "坐标(1, 0)和(2, 1)需要分析";
        let (has_bad, bad) = Translator::validate_llm_output(&ir, &view, response);
        assert!(!has_bad, "unexpected hallucinated coords: {:?}", bad);
    }

    #[test]
    fn test_validate_llm_output_detects_hallucination() {
        let view = PlayerView {
            width: 3, height: 3,
            revealed: vec![],
            flagged: vec![],
            unknown: vec![Coord::new(1, 0)],
            remaining_mines: 1,
        };
        let ir = InferenceIR {
            deterministic: vec![],
            probabilities: vec![],
            regions: vec![],
        
            flag_verification: crate::model::FlagVerificationResult::empty(),};
        let response = "坐标(5, 5)是雷";
        let (has_bad, bad) = Translator::validate_llm_output(&ir, &view, response);
        assert!(has_bad);
        assert!(bad.contains(&"(5, 5)".to_string()));
    }
}

#[cfg(test)]
mod intent_tests {
    use super::*;

    #[test]
    fn test_user_asked_cell_natural_language() {
        // 用户自然语言提问 (修复场景: 之前被当成全盘分析)
        assert_eq!(
            user_asked_cell("(18,15) 为什么必定是雷?"),
            Some(Coord::new(18, 15))
        );
        assert_eq!(
            user_asked_cell("(18, 15) 是安全的吗"),
            Some(Coord::new(18, 15))
        );
        // @why/@check 指令
        assert_eq!(user_asked_cell("@why (28,12)"), Some(Coord::new(28, 12)));
        assert_eq!(user_asked_cell("@check (28,12) 对吗"), Some(Coord::new(28, 12)));
        // 无坐标 / 全局问题 → None
        assert_eq!(user_asked_cell("下一步怎么走比较好"), None);
        assert_eq!(user_asked_cell("这个局面怎么分析"), None);
        // @region/@analyze 非定点
        assert_eq!(user_asked_cell("@region"), None);
    }

    #[test]
    fn test_explain_prompt_binds_single_cell() {
        let zh = build_explain_system_prompt(false);
        assert!(zh.contains("禁止做全盘分析"));
        assert!(zh.contains("第 1 步"));
        assert!(zh.contains("最终结论"));
        let en = build_explain_system_prompt(true);
        assert!(en.contains("ONE specific cell"));
        assert!(en.contains("Final verdict"));
    }

    #[test]
    fn test_flag_problem_prompt_binds_single_flag() {
        let zh = build_flag_problem_system_prompt(false);
        assert!(zh.contains("只处理其中一面"));
        assert!(zh.contains("禁止做全盘分析"));
        assert!(zh.contains("第 1 步"));
        assert!(zh.contains("是误标"));
        let en = build_flag_problem_system_prompt(true);
        assert!(en.contains("exactly ONE of them"));
        assert!(en.contains("misplaced"));
        assert!(en.contains("Final verdict"));
    }
}
