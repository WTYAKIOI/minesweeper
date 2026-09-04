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

/// 推理 IR → 自然语言转译器
///
/// 将 Rust 推理引擎的计算结果翻译为人类可理解的策略语言。
/// 支持三种模式：答案模式、教学模式、策略模式。
pub struct Translator {
    mode: LLMMode,
}

impl Translator {
    pub fn new(mode: LLMMode) -> Self {
        Self { mode }
    }

    pub fn set_mode(&mut self, mode: LLMMode) {
        self.mode = mode;
    }

    /// 将 IR 转为给 LLM 的系统提示词
    pub fn build_system_prompt(&self) -> String {
        let base = r#"你是一个扫雷认知助手。你将收到一个JSON格式的推理中间语言(IR)，其中包含：
1. deterministic: 确定性推理证明链（必雷/必安全格的结论及其依据）
2. probabilities: 每个未知格子的雷概率和信息增益
3. regions: 连通区域的特征信息

## 数据防火墙原则
你收到的数据中绝不包含未翻开格子的真实雷藏信息。所有概率都是基于已知信息的数学推断，不是事后诸葛亮。

## 严格的坐标引用规则（违反则视为幻觉/作弊）
1. 所有坐标格式必须为 (x, y)，例如 (2, 3) 表示第3列第4行（0-indexed）
2. 你引用的每一个坐标必须在IR中存在
3. 严禁编造IR中不存在的坐标
4. 严禁使用模糊表述如"中间那个格子"、"旁边的格子"，必须给出精确坐标
5. 引用推理依据时，必须标明依赖的数字坐标，如"根据坐标(0,0)的数字3..."

## 输出格式要求
- 用中文回答
- 答案模式：使用"因为...所以..."的因果句式
- 教学模式：使用提问句式，禁止直接给答案
- 策略模式：给出坐标级建议，含概率数值"#;

        match &self.mode {
            LLMMode::Answer => format!(
                r##"{}

## 当前模式：答案模式
请将证明链转为流畅的因果推理文字。格式要求：
- 每个结论必须引用具体坐标，如「坐标(x, y)是雷」
- 必须说明依据，如「因为坐标(a, b)的数字N周围...」
- 对概率较低的格子给出精确概率数值
- 对无确定性结论的格子，给出概率排序"##,
                base
            ),
            LLMMode::Teaching => format!(
                r##"{}

## 当前模式：教学模式
请将证明链倒置，生成逐步引导问题。格式要求：
- 每步只提问一个引导性问题，如「坐标(a, b)的数字N周围还有几个未知格？」
- 严禁直接说出「坐标(x, y)是雷」或「坐标(x, y)安全」
- 引导玩家自己算出：剩余雷数 = 数字 - 已标旗数
- 最后一个问题后留出思考空间"##,
                base
            ),
            LLMMode::Strategy => format!(
                r##"{}

## 当前模式：策略模式
请基于概率和收益数据，分析各区域的利弊。格式要求：
- 每个区域必须列出其包含的坐标范围
- 比较平均雷概率和信息增益
- 给出「先处理哪个区域」的明确建议
- 建议点击的坐标必须在IR的probabilities中存在"##,
                base
            ),
        }
    }

    /// 将 IR + PlayerView 序列化为给 LLM 的用户消息
    pub fn build_user_message(&self, view: &PlayerView, ir: &InferenceIR) -> String {
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

        format!(
            "当前局面信息:\n{}\n\n已翻开数字坐标: {}\n未知格子坐标(你的建议只能在这些中选): {}\n\n推理结果 (IR):\n```json\n{}\n```\n\n请根据以上信息给出分析。记住：你引用的每个坐标必须在上述列表中。",
            view_info, revealed_coords.join(", "), valid_coords.join(", "), ir_json
        )
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
        assert!(prompt.contains("严格的坐标引用规则"));
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
        };
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
        };
        let response = "坐标(5, 5)是雷";
        let (has_bad, bad) = Translator::validate_llm_output(&ir, &view, response);
        assert!(has_bad);
        assert!(bad.contains(&"(5, 5)".to_string()));
    }
}
