//! 远程多模态 LLM 棋盘识别 — 通用扫雷截图解析
//!
//! 设计目标: 不假设棋盘尺寸, 让 LLM 自行检测行列数并输出结构化 JSON,
//! Rust 侧做"格式校验 + 尺寸修正 + 扫雷规则校验", 全部通过才采用,
//! 否则由调用方回退本地 OCR。适配任意扫雷 APP, 不受 16×30 限制。
//!
//! 编排 (见 server::routes::llm_ocr):
//!   截图 base64 → LLMClient::chat_with_image → parse_llm_board_response
//!   → validate_llm_output → to_board → validate_minesweeper_rules → 采用

use serde::{Deserialize, Serialize};

use crate::model::{CellState, PlayerView};

/// 单边最大维度 (超过视为 LLM 幻觉, 拒绝)
pub const MAX_DIM: usize = 100;

/// System Prompt (固定): 定义角色与硬性输出格式
pub const BOARD_RECOGNITION_SYSTEM_PROMPT: &str = r#"你是一个扫雷截图解析引擎，不是对话助手。你的唯一任务是从截图中提取棋盘状态。

【硬性约束】
1. 你**必须**先检测棋盘的行数（rows）和列数（cols），再提取每个格子的状态。
2. 输出必须是合法的 JSON 对象，格式如下：
   {
     "rows": <整数>,
     "cols": <整数>,
     "matrix": [
       ["U", "U", "1", ...],
       ["F", "0", "2", ...],
       ...
     ]
   }
3. 每个格子的值只能是以下字符串之一：
   - "U"（未翻开 / Unknown）
   - "0" 到 "8"（已翻开的数字）
   - "F"（玩家标记的旗帜 / Flag）
   - "M"（踩雷爆炸，如果截图中有）
4. 如果某个格子无法确定，输出 "U"。
5. 不要输出任何解释、前言或结束语。只输出 JSON。"#;

/// User Prompt (固定文本部分, 图片由 API 消息结构附带)
pub const BOARD_RECOGNITION_USER_PROMPT: &str = r#"请识别这张扫雷截图，提取完整的棋盘状态。

【任务】
1. 找出截图中的扫雷棋盘区域。
2. 数清棋盘有多少行（垂直方向）和多少列（水平方向）。
3. 逐格识别每个格子的状态。

【识别指引】
- 未翻开的灰色/凸起方块 → "U"
- 翻开后空白的格子 → "0"
- 翻开后有数字的格子 → "1" 到 "8"（按实际数字）
- 红色旗帜标记 → "F"
- 红色地雷/爆炸标记（若可见）→ "M"

【额外指引】
扫雷棋盘通常由规则的方形网格组成，网格内有灰色方块或数字。
请忽略截图中的窗口标题、菜单栏、计时器、地雷计数器等 UI 元素。
只提取规则网格区域。
如果截图中有多个类似棋盘的网格，选择最大最完整的那一个。

【输出格式】
只输出一个 JSON 对象，包含 rows、cols 和 matrix。
matrix 必须是二维数组，第一维长度 = rows，第二维长度 = cols。
确保每行的列数相等。

【示例格式】（不要复制示例数值）
{
  "rows": 9,
  "cols": 9,
  "matrix": [
    ["U","U","U","U","U","U","U","U","U"],
    ["U","U","1","2","U","U","U","U","U"]
  ]
}"#;

/// LLM 输出的棋盘结构 (rows/cols 缺失时由 normalized() 从 matrix 推断)
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct LLMBoardOutput {
    #[serde(default)]
    pub rows: usize,
    #[serde(default)]
    pub cols: usize,
    #[serde(default)]
    pub matrix: Vec<Vec<String>>,
}

impl LLMBoardOutput {
    /// 归一化: 丢弃空行 → 统一大小写/空白 → 校正 rows/cols 与参差行。
    ///
    /// LLM 常见毛病: 声明的 rows×cols 与实际矩阵不符、某行少/多一格、
    /// 输出小写 "u"/"f"。此处尽力修复, 修不了才报错。
    pub fn normalized(mut self) -> Result<Self, String> {
        self.matrix.retain(|r| !r.is_empty());
        if self.matrix.is_empty() {
            return Err("matrix 为空或全为空行".to_string());
        }
        for row in &mut self.matrix {
            for cell in row.iter_mut() {
                let c = cell.trim().to_uppercase();
                *cell = if c.is_empty() { "U".to_string() } else { c };
            }
        }

        // 声明尺寸与实际矩阵一致 → 直接通过
        let declared_ok = self.rows > 0
            && self.cols > 0
            && self.matrix.len() == self.rows
            && self.matrix.iter().all(|r| r.len() == self.cols);
        if declared_ok {
            return Ok(self);
        }

        // 声明不符 → 从实际矩阵重新推断尺寸 (多数行宽), 参差行补 "U" / 截断
        let rows = self.matrix.len();
        let cols = choose_cols(&self.matrix, self.cols).ok_or_else(|| {
            format!(
                "矩阵尺寸不自洽, 无法推断 (共 {} 行, 行宽分别为 {:?})",
                self.matrix.len(),
                self.matrix.iter().map(|r| r.len()).collect::<Vec<_>>()
            )
        })?;
        for row in &mut self.matrix {
            match row.len().cmp(&cols) {
                std::cmp::Ordering::Less => {
                    row.extend(std::iter::repeat_n("U".to_string(), cols - row.len()))
                }
                std::cmp::Ordering::Greater => row.truncate(cols),
                std::cmp::Ordering::Equal => {}
            }
        }
        self.rows = rows;
        self.cols = cols;
        Ok(self)
    }

    /// 转换为项目内部棋盘表示: -1 未知, -2 旗帜 (含爆炸雷 M), 0-8 数字
    pub fn to_board(&self) -> Vec<Vec<i32>> {
        self.matrix
            .iter()
            .map(|row| {
                row.iter()
                    .map(|cell| match cell.as_str() {
                        "U" | "?" => -1,
                        "F" | "M" => -2,
                        d => d.parse::<i32>().unwrap_or(-1),
                    })
                    .collect()
            })
            .collect()
    }
}

/// 单格取值是否合法
fn is_valid_cell(cell: &str) -> bool {
    matches!(
        cell,
        "U" | "F" | "M" | "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8"
    )
}

/// 格式校验: 维度合理 + 行列一致 + 合法字符
pub fn validate_llm_output(output: &LLMBoardOutput) -> bool {
    if output.rows == 0 || output.cols == 0 {
        return false;
    }
    if output.rows > MAX_DIM || output.cols > MAX_DIM {
        return false;
    }
    if output.matrix.len() != output.rows {
        return false;
    }
    if output
        .matrix
        .iter()
        .any(|row| row.len() != output.cols)
    {
        return false;
    }
    output
        .matrix
        .iter()
        .flatten()
        .all(|cell| is_valid_cell(cell))
}

/// 从实际矩阵推断 (rows, cols):
/// - 所有行等长 → 直接采用
/// - 行宽参差 → choose_cols 多数表决 (无声明尺寸参考时按 60% 阈值)
pub fn infer_dimensions(matrix: &[Vec<String>]) -> Option<(usize, usize)> {
    let rows = matrix.len();
    if rows == 0 {
        return None;
    }
    let w0 = matrix[0].len();
    if w0 > 0 && matrix.iter().all(|r| r.len() == w0) {
        return Some((rows, w0));
    }
    let cols = choose_cols(matrix, 0)?;
    Some((rows, cols))
}

/// 参差行宽的多数表决: 取出现最多的行宽 (平票取更宽 — 补 "U" 比截断更安全)。
/// 接受条件: 多数占比 ≥ 60%, 或与声明的 cols 一吻 (LLM 声明的列数可作参考)。
fn choose_cols(matrix: &[Vec<String>], declared_cols: usize) -> Option<usize> {
    let mut counts: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    for r in matrix {
        if !r.is_empty() {
            *counts.entry(r.len()).or_insert(0) += 1;
        }
    }
    if counts.is_empty() {
        return None;
    }
    let (&best_w, &best_n) = counts
        .iter()
        .max_by(|a, b| a.1.cmp(b.1).then(a.0.cmp(b.0)))?;
    if best_w == 0 {
        return None;
    }
    let rows = matrix.len();
    if best_n * 10 >= rows * 6 || best_w == declared_cols {
        Some(best_w)
    } else {
        None
    }
}

/// 从自由文本中提取 JSON (兼容 ```json 代码块 / 前后缀废话)
pub fn extract_json_from_text(text: &str) -> Option<&str> {
    // ```json ... ``` 代码块
    if let Some(start) = text.find("```json") {
        let rest = &text[start + 7..];
        if let Some(end) = rest.find("```") {
            let block = rest[..end].trim();
            if !block.is_empty() {
                return Some(block);
            }
        }
    }
    // 任意 ``` 代码块 (可能无语言标记)
    if let Some(start) = text.find("```") {
        let rest = &text[start + 3..];
        let rest = match rest.find('\n') {
            Some(nl) => &rest[nl + 1..],
            None => rest,
        };
        if let Some(end) = rest.find("```") {
            let block = rest[..end].trim();
            if block.starts_with('{') {
                return Some(block);
            }
        }
    }
    // 首个 '{' 到末个 '}'
    let s = text.find('{')?;
    let e = text.rfind('}')?;
    if s < e {
        Some(&text[s..=e])
    } else {
        None
    }
}

/// 解析 LLM 响应文本为棋盘结构 (含归一化与尺寸修正)
pub fn parse_llm_board_response(text: &str) -> Result<LLMBoardOutput, String> {
    let t = text.trim();
    if let Ok(out) = serde_json::from_str::<LLMBoardOutput>(t) {
        return out.normalized();
    }
    if let Some(json_str) = extract_json_from_text(t) {
        if let Ok(out) = serde_json::from_str::<LLMBoardOutput>(json_str) {
            return out.normalized();
        }
        return Err(format!(
            "LLM 输出包含 JSON 但字段不合法: {}",
            json_str.chars().take(200).collect::<String>()
        ));
    }
    Err("LLM 输出中未找到 JSON 对象".to_string())
}

/// 扫雷规则校验 (识别结果可信度二次确认):
/// 1. 每个数字周围的旗数 (含爆炸雷 M) 不超过该数字
/// 2. 数字 0 周围不存在未翻开格 (经典规则会自动展开空区)
pub fn validate_minesweeper_rules(board: &[Vec<i32>]) -> bool {
    let view = PlayerView::from_2d(board, 0);
    if !view.validate() {
        return false;
    }
    let state = view.to_state_map();
    for cell in &view.revealed {
        if cell.number == 0 {
            let bad = cell
                .coord
                .neighbors(view.width, view.height)
                .iter()
                .any(|n| matches!(state.get(n), Some(CellState::Unknown)));
            if bad {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_JSON: &str = r#"{
        "rows": 3, "cols": 3,
        "matrix": [
            ["U","U","1"],
            ["F","0","2"],
            ["U","M","U"]
        ]
    }"#;

    #[test]
    fn test_parse_direct_json() {
        let out = parse_llm_board_response(VALID_JSON).unwrap();
        assert_eq!((out.rows, out.cols), (3, 3));
        assert_eq!(out.matrix[1][0], "F");
        assert!(validate_llm_output(&out));
        let board = out.to_board();
        assert_eq!(board, vec![vec![-1, -1, 1], vec![-2, 0, 2], vec![-1, -2, -1]]);
    }

    #[test]
    fn test_parse_markdown_fenced() {
        let text = format!("好的，识别结果如下：\n```json\n{}\n```\n希望对你有帮助！", VALID_JSON);
        let out = parse_llm_board_response(&text).unwrap();
        assert_eq!((out.rows, out.cols), (3, 3));
    }

    #[test]
    fn test_parse_bare_code_block() {
        let text = format!("```\n{}\n```", VALID_JSON);
        let out = parse_llm_board_response(&text).unwrap();
        assert_eq!(out.rows, 3);
    }

    #[test]
    fn test_parse_no_json() {
        assert!(parse_llm_board_response("抱歉，我无法识别这张图片。").is_err());
    }

    #[test]
    fn test_normalized_missing_dims() {
        // rows/cols 缺失 → 从 matrix 推断
        let out: LLMBoardOutput = serde_json::from_str(
            r#"{"matrix": [["U","1"],["2","U"]]}"#,
        )
        .unwrap();
        let out = out.normalized().unwrap();
        assert_eq!((out.rows, out.cols), (2, 2));
        assert!(validate_llm_output(&out));
    }

    #[test]
    fn test_normalized_jagged_rows() {
        // 第 2 行少 1 格 → 补 "U"; 声明尺寸不符 → 重新推断
        let out: LLMBoardOutput = serde_json::from_str(
            r#"{"rows": 2, "cols": 3, "matrix": [["U","1","U"], ["2","U"]]}"#,
        )
        .unwrap();
        let out = out.normalized().unwrap();
        assert_eq!((out.rows, out.cols), (2, 3));
        assert_eq!(out.matrix[1], vec!["2", "U", "U"]);
        assert!(validate_llm_output(&out));
    }

    #[test]
    fn test_normalized_wrong_declared_dims() {
        // 声明 9×9 实际 2×2 → 修正为实际尺寸
        let out: LLMBoardOutput = serde_json::from_str(
            r#"{"rows": 9, "cols": 9, "matrix": [["U","1"],["2","U"]]}"#,
        )
        .unwrap();
        let out = out.normalized().unwrap();
        assert_eq!((out.rows, out.cols), (2, 2));
    }

    #[test]
    fn test_normalized_lowercase_and_blank() {
        let out: LLMBoardOutput = serde_json::from_str(
            r#"{"rows": 1, "cols": 3, "matrix": [["u", " f ", ""]]}"#,
        )
        .unwrap();
        let out = out.normalized().unwrap();
        assert_eq!(out.matrix[0], vec!["U", "F", "U"]);
    }

    #[test]
    fn test_validate_rejects() {
        // 非法字符
        let bad: LLMBoardOutput =
            serde_json::from_str(r#"{"rows":1,"cols":2,"matrix":[["U","9"]]}"#).unwrap();
        assert!(!validate_llm_output(&bad));
        // 维度超限 (矩阵与声明自洽, 仍应被拒)
        let huge = LLMBoardOutput {
            rows: MAX_DIM + 1,
            cols: 2,
            matrix: vec![vec!["U".to_string(), "1".to_string()]; MAX_DIM + 1],
        };
        assert!(!validate_llm_output(&huge));
        // 行长不齐 (normalized 已修复的场景之外的原始校验)
        let jagged = LLMBoardOutput {
            rows: 2,
            cols: 2,
            matrix: vec![vec!["U".into(), "1".into()], vec!["U".into()]],
        };
        assert!(!validate_llm_output(&jagged));
    }

    #[test]
    fn test_infer_dimensions() {
        let u = || "U".to_string();
        assert_eq!(
            infer_dimensions(&[vec![u(); 3], vec![u(); 3]]),
            Some((2, 3))
        );
        // 多数行宽 3 (3/5 行) → 推断成功
        let jagged: Vec<Vec<String>> = vec![
            vec![u(), u(), u()],
            vec![u(), u(), u()],
            vec![u(), u(), u()],
            vec![u(), u()],
            vec![u()],
        ];
        assert_eq!(infer_dimensions(&jagged), Some((5, 3)));
        // 行宽完全分散 → 拒绝
        let chaos: Vec<Vec<String>> =
            vec![vec![u()], vec![u(), u()], vec![u(); 3], vec![u(); 4]];
        assert_eq!(infer_dimensions(&chaos), None);
        assert_eq!(infer_dimensions(&[]), None);
    }

    #[test]
    fn test_extract_json_from_text() {
        assert_eq!(
            extract_json_from_text("前置说明 ```json\n{\"a\":1}\n``` 后缀"),
            Some("{\"a\":1}")
        );
        assert_eq!(extract_json_from_text("废话 {\"a\":1} 结尾"), Some("{\"a\":1}"));
        assert_eq!(extract_json_from_text("没有 json"), None);
    }

    #[test]
    fn test_rules_valid_board() {
        // 全展开区域: 0 的邻居均为已翻开格, 数字约束自洽, 左上保留一个未知格
        let board = vec![vec![-1, 1, 0], vec![1, 1, 0], vec![0, 0, 0]];
        assert!(validate_minesweeper_rules(&board));
    }

    #[test]
    fn test_rules_flag_exceeds_number() {
        // 数字 1 周围有 2 面旗 → 矛盾
        let board = vec![vec![1, -2], vec![-2, -1]];
        assert!(!validate_minesweeper_rules(&board));
    }

    #[test]
    fn test_rules_zero_with_unknown_neighbor() {
        // 0 旁边有未翻开格 → 经典规则下不可能, 判为识别错误
        let board = vec![vec![0, -1], vec![0, 0]];
        assert!(!validate_minesweeper_rules(&board));
    }

    #[test]
    fn test_rules_all_unknown() {
        // 开局全未知 → 合法
        let board = vec![vec![-1, -1], vec![-1, -1]];
        assert!(validate_minesweeper_rules(&board));
    }
}
