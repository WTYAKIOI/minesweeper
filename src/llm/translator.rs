use crate::model::{InferenceIR, PlayerView, Coord};
use std::collections::HashSet;

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
                "## 语言要求\n请全程使用简体中文回答。",
                r#"## 输出硬性规则 (违反视为不合格)
1. 坐标引用: 只允许引用 IR 中实际出现的坐标 (deterministic[].conclusion.coord / probabilities[].coord /
   文本给出的未知格坐标清单); 一律用 (行,列) 形式; 禁止编造坐标、禁止"左边/附近/那一片"等模糊描述。
2. 确定性结论: 必须逐条附带依据 (引用 depends_on 中的数字坐标与 rule); 禁止"我认为/可能"句式;
   不得自行添加 IR 中不存在的结论。
3. 概率结论: 数值直接来自 IR 的 probabilities (字段名 mine_probability, 即 0~1 的小数,
   展示时换算为百分比, 不四舍五入成整数); 只可给 IR 中存在的格赋概率; 每个概率格附建议
   (优先点击 / 可考虑标旗 / 暂缓处理)。
4. 旗帜验证: 若 flag_verification 中存在 Contradicted/未确认(Suspected) 旗, 必须逐条列出;
   无问题则写"所有旗帜与当前数字约束一致"。
5. 旗帜问题警告: 当存在矛盾旗或未确认旗时, 显式提示"存在旗帜问题, 以下概率与推理基于容错假设,
   数值可能出错, 建议先核对/移除问题旗帜后再做关键决策"。
6. 长度控制: 普通模式总输出 ≤500 字, 表格每类 ≤10 行 (超 10 行只列前 5 并注明"余 N 个略")。
7. 确定性结论用 Markdown 表格呈现; 概率参考用表格; 其余用列表; 不用过渡客套话。
8. deterministic 为空 → 不输出确定性结论表格, 只给概率参考; probabilities 为空 → 写"概率数据不可用"。\n"#,
            ),
            Language::English => (
                "## Language Requirement\nRespond entirely in English.",
                r#"## Hard Output Rules (violations are rejected)
1. Coordinates: only cite coordinates present in the IR (deterministic[].conclusion.coord /
   probabilities[].coord / the listed unknown coordinates); format "(x, y)"; never invent
   coordinates or use vague descriptions.
2. Deterministic claims: must cite evidence (depends_on number coordinates and rule); never say
   "I think"; never add conclusions not in the IR.
3. Probabilities: take values directly from IR probabilities (field mine_probability, a 0..1 float;
   show as percentage without rounding to integers); only for cells present in the IR; attach a
   recommendation (click first / consider flagging / wait).
4. Flag verification: if flag_verification contains Contradicted or Suspected flags, list them all;
   otherwise state "all flags are consistent with the board constraints".
5. Flag warning: when contradicted or unverified flags exist, explicitly warn that probabilities
   may be wrong because reasoning runs on a fault-tolerant assumption; suggest fixing flags first.
6. Length: ≤500 characters overall in normal modes; ≤10 table rows per class (list first 5 and
   note "N more omitted").
7. Present deterministic results and probability references as Markdown tables.
8. If deterministic is empty, give only probability reference; if probabilities empty, say
   "probability data unavailable".\n"#,
            ),
        };

        let base = format!(
            "你是一个扫雷推理助手。你的唯一任务是把 Rust 引擎输出的推理中间语言 (IR) 翻译成符合规范的{}分析报告。你会收到 JSON IR (deterministic / probabilities / regions / flag_verification) 与局面文本说明。

## 数据防火墙原则
你收到的数据中绝不包含未翻开格子的真实雷藏信息。所有概率都是基于已知信息的数学推断。

## 输入字段说明 (防止读错)
- deterministic[].conclusion.is_mine: true=必雷结论 / false=必安全结论; depends_on[]: 依赖的数字坐标
- probabilities[].mine_probability: 雷概率(0~1 小数); info_gain: 信息增益
- flag_verification.flags[]: 每面旗的 status (Verified=确认正确 / Contradicted=矛盾 /
  Suspected=未确认) 与 reason(中文理由)

{}
{}
",
            match self.language { Language::Chinese => "中文", Language::English => "English" },
            lang_rule,
            spec
        );

        match &self.mode {
            LLMMode::Teaching => self.teaching_prompt(&base),
            LLMMode::Answer => self.answer_prompt(&base),
            LLMMode::Strategy => self.strategy_prompt(&base),
        }
    }

    /// 答案模式模板 (llm-output.md 模板 A: 确定性结论表 + 概率参考表 + 旗帜检查 + 下一步建议)
    fn answer_prompt(&self, base: &str) -> String {
        match self.language {
            Language::Chinese => format!(
                r##"{}

## 当前模式：答案模式 — 输出模板 (按序, 缺失章节省略并说明原因)
## 🎯 确定性结论（必有依据）
### 💣 必雷格（共 N 个）[若无则此小节不出现]
| 坐标 | 依据 |
|------|------|
| (行,列) | 依据 rule / depends_on |
### ✅ 安全格（共 N 个）[同上]
| 坐标 | 依据 |
|------|------|

## 📊 概率参考（无确定结论时给出; 有确定结论也可附高价值格）
| 坐标 | 雷概率 | 信息增益 | 建议 |
|------|--------|----------|------|

## 🚩 旗帜检查
- ✅ / ⚠️ 说明; 疑似误标按坐标列出原因

## 💡 下一步建议
[1-2 句, 给出最高优先级操作]

若用户带了具体提问(见消息末尾【用户提问】):
- "为什么/原因/解释" → 先答标题「## 直接原因」1-3 句并引用坐标, 再给「## 思考引导」1 问;
- 其他提问 → 先按上方模板输出当前局面结论, 再以「## 回答用户提问」小节直接回答。"##,
                base
            ),
            Language::English => format!(
                r##"{}

## Current Mode: Answer — Output Template
## 🎯 Deterministic conclusions (with evidence)
### 💣 Must-be-mine (N)
| coord | evidence |
### ✅ Must-be-safe (N)
| coord | evidence |

## 📊 Probability reference
| coord | mine prob | info gain | advice |

## 🚩 Flag check
list contradicted/suspected flags or state all flags consistent

## 💡 Next step
1-2 sentences.

If the user asked a question: "why" questions first answer with the heading ## Direct Reason (1-3 sentences citing
coordinates) then the heading ## Think Further (1 question); other questions: output the template then answer in the
## Answer section."##,
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
        let ir_json = serde_json::to_string_pretty(ir).unwrap_or_else(|e| format!("序列化失败: {}", e));
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

        let mut result = String::new();

        if !mines.is_empty() {
            result.push_str("【必雷格子】\n");
            for proof in &ir.deterministic {
                if proof.conclusion.is_mine {
                    result.push_str(&format!(
                        "- 坐标{} 是雷。依据: {}\n",
                        proof.conclusion.coord, proof.rule
                    ));
                }
            }
        }

        if !safe.is_empty() {
            result.push_str("\n【必安全格子】\n");
            for proof in &ir.deterministic {
                if !proof.conclusion.is_mine {
                    result.push_str(&format!(
                        "- 坐标{} 安全。依据: {}\n",
                        proof.conclusion.coord, proof.rule
                    ));
                }
            }
        }

        if !ir.probabilities.is_empty() && mines.is_empty() && safe.is_empty() {
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
        assert!(result.contains("必雷格子"));
        assert!(result.contains("必安全格子"));
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
        assert!(prompt.contains("输出硬性规则"));   // llm-output.md 规则块
        assert!(prompt.contains("坐标引用"));
        assert!(prompt.contains("mine_probability"));
        assert!(prompt.contains("下一步建议"));
        // 中文模式下禁止超过 500 字 & 必列表格
        assert!(prompt.contains("500"));
        assert!(prompt.contains("旗帜检查"));
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
