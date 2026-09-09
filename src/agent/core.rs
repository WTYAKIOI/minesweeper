//! 棋盘语义核心 (纯逻辑, 无 IO)
//!
//! 编码约定 (与 Rust 端 PlayerView / 前端展示一致):
//!   -1 = 未知(未翻开), -2 = 旗帜, 0-8 = 已翻开数字

use std::fmt::Write as _;

/// 棋盘行数下限 (防止误把普通文本当棋盘)
const MIN_ROWS: usize = 2;

/// 编辑操作 (前端棋盘点击语义的 Rust 唯一实现)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoardOp {
    /// 左键: 未知/旗帜 → 1; 空白0 → 1; 数字 1..7 递增; 8 → 回到未知
    LeftCycle,
    /// 右键: 旗帜 ↔ 未知
    ToggleFlag,
    /// Shift+左键: 清除为已翻开 0
    ClearToZero,
    /// Ctrl+左键: 变为未知 (-1)
    SetUnknown,
    /// 删除第 y 行 (0 基); 越界返回原棋盘
    DeleteRow,
    /// 删除第 x 列 (0 基); 越界返回原棋盘
    DeleteCol,
}

/// 文本棋盘解析错误
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardParseError {
    Empty,
    TooFewRows(usize),
    Ragged { row: usize, expect: usize, got: usize },
    BadToken { row: usize, col: usize, token: String },
}

impl std::fmt::Display for BoardParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BoardParseError::Empty => write!(f, "文本为空"),
            BoardParseError::TooFewRows(n) => write!(f, "行数过少 ({}行, 至少需 {} 行)", n, MIN_ROWS),
            BoardParseError::Ragged { row, expect, got } => {
                write!(f, "第 {} 行宽度 {} 与首行 {} 不一致", row + 1, got, expect)
            }
            BoardParseError::BadToken { row, col, token } => {
                write!(f, "第 {} 行第 {} 列无法识别的符号: \"{}\" (支持 ? F 🚩 0-8)", row + 1, col + 1, token)
            }
        }
    }
}

fn parse_token(tok: &str) -> Option<i32> {
    match tok {
        "?" | "？" | "-1" | "·" | "x" | "X" | "U" | "u" => Some(-1),
        "F" | "f" | "🚩" | "P" | "p" | "-2" => Some(-2),
        "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" => Some(tok.parse::<i32>().unwrap()),
        _ => None,
    }
}

/// 将文本棋盘 (每行空格/逗号/竖线分隔) 解析为 board[y][x]。
/// 语义与旧版前端 parseTextBoard 一致, 但现在由 Rust 作为唯一实现。
pub fn parse_text_board(text: &str) -> Result<Vec<Vec<i32>>, BoardParseError> {
    let lines: Vec<&str> = text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() {
        return Err(BoardParseError::Empty);
    }
    if lines.len() < MIN_ROWS {
        return Err(BoardParseError::TooFewRows(lines.len()));
    }

    let mut board: Vec<Vec<i32>> = Vec::with_capacity(lines.len());
    for (r, line) in lines.iter().enumerate() {
        let tokens: Vec<&str> = line
            .split(|c| c == ' ' || c == '\t' || c == ',' || c == ';' || c == '|')
            .map(|t| t.trim().trim_matches(|c| c == ',' || c == ';'))
            .filter(|t| !t.is_empty())
            .collect();
        if tokens.is_empty() {
            return Err(BoardParseError::Empty);
        }
        let mut row = Vec::with_capacity(tokens.len());
        for (c, tok) in tokens.iter().enumerate() {
            match parse_token(tok) {
                Some(v) => row.push(v),
                None => {
                    return Err(BoardParseError::BadToken {
                        row: r,
                        col: c,
                        token: (*tok).to_string(),
                    })
                }
            }
        }
        let w = row.len();
        if board.first().map(|f: &Vec<i32>| f.len()) != Some(w) && !board.is_empty() {
            return Err(BoardParseError::Ragged {
                row: r,
                expect: board[0].len(),
                got: w,
            });
        }
        board.push(row);
    }
    Ok(board)
}

/// 应用一次编辑操作 (棋盘状态机的 Rust 唯一实现)。
/// 超出边界或操作目标非法时返回原棋盘 (幂等, 由调用方决定是否报错)。
pub fn apply_board_op(board: &[Vec<i32>], x: usize, y: usize, op: BoardOp) -> Vec<Vec<i32>> {
    let mut out: Vec<Vec<i32>> = board.to_vec();
    if y >= out.len() {
        return out;
    }
    if x >= out[y].len() {
        return out;
    }
    let cur = out[y][x];
    let next = match op {
        BoardOp::ToggleFlag => match cur {
            -2 => -1,
            -1 => -2,
            v => v, // 已翻开数字格不能插旗
        },
        BoardOp::ClearToZero => 0,
        BoardOp::SetUnknown => -1,
        BoardOp::DeleteRow => {
            if y >= out.len() {
                return out; // 越界: 原样返回
            }
            out.remove(y);
            return out;
        }
        BoardOp::DeleteCol => {
            if out.is_empty() || x >= out[0].len() {
                return out; // 越界: 原样返回
            }
            for row in out.iter_mut() {
                row.remove(x);
            }
            return out;
        }
        BoardOp::LeftCycle => match cur {
            -1 | -2 => 1,            // 未知/旗帜 → 数字 1
            0 => 1,                  // 空白(已翻开0) → 数字 1 (可继续递增编辑)
            v if (1..=7).contains(&v) => v + 1,
            8 => -1,                 // 8 → 回到未知
            v => v,                  // 兜底: 其他值保持不变
        },
    };
    out[y][x] = next;
    out
}

/// 标准棋盘尺寸 → 总雷数 (参考 minesweeper_solver 约定)
fn standard_mines(width: usize, height: usize) -> Option<u32> {
    match (width, height) {
        (9, 9) => Some(10),
        (16, 16) => Some(40),
        (30, 16) => Some(99),
        (16, 30) => Some(99),
        _ => None,
    }
}

/// 依据棋盘尺寸推断剩余雷数 (OCR 未识别到雷数时的回退)。
/// 返回 None 表示非标准尺寸, 由调用方决定如何处理。
pub fn infer_mines_for_size(board: &[Vec<i32>]) -> Option<u32> {
    let height = board.len();
    let width = board.first().map(|r| r.len()).unwrap_or(0);
    let total = standard_mines(width, height)?;
    let flags = board
        .iter()
        .flat_map(|r| r.iter())
        .filter(|v| **v == -2)
        .count() as u32;
    Some(total.saturating_sub(flags))
}

/// 将 board 渲染为 ?/F/0-8 文本 (每行空格分隔), 供导出报告使用
pub fn render_board_text(board: &[Vec<i32>]) -> String {
    let mut out = String::new();
    for (i, row) in board.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        for (j, v) in row.iter().enumerate() {
            if j > 0 {
                out.push(' ');
            }
            let ch = match v {
                -1 => '?',
                -2 => 'F',
                n => char::from_digit(*n as u32, 10).unwrap_or('?'),
            };
            out.push(ch);
        }
    }
    out
}

/// Markdown 分析报告构建 (语义在 Rust, 前端只负责下载文件)。
pub fn render_markdown_report(
    board: &[Vec<i32>],
    view: &crate::model::PlayerView,
    ir: &crate::model::InferenceIR,
    analysis: &str,
) -> String {
    // 旗帜格的确定性结论 (必雷/必安全) 已反映在棋盘上, 不在"推理链"中重复罗列
    let flagged_set: std::collections::HashSet<crate::model::Coord> = board
        .iter()
        .enumerate()
        .flat_map(|(y, row)| {
            row.iter()
                .enumerate()
                .filter(|(_, v)| **v == -2)
                .map(move |(x, _)| crate::model::Coord::new(x as u32, y as u32))
        })
        .collect();
    let actionable = ir
        .deterministic
        .iter()
        .filter(|p| !flagged_set.contains(&p.conclusion.coord))
        .collect::<Vec<_>>();
    let mines: Vec<_> = actionable
        .iter()
        .filter(|p| p.conclusion.is_mine)
        .copied()
        .collect();
    let safes: Vec<_> = actionable
        .iter()
        .filter(|p| !p.conclusion.is_mine)
        .copied()
        .collect();
    let board_text = render_board_text(board);

    let mut md = String::new();
    let _ = writeln!(md, "# 扫雷认知助手 · 分析结果\n");
    let _ = writeln!(
        md,
        "- 棋盘: {}x{} · 剩余雷数: {}",
        view.width, view.height, view.remaining_mines
    );
    let _ = writeln!(md, "- 必雷: {} 格 · 必安全: {} 格\n", mines.len(), safes.len());
    let _ = writeln!(md, "## 棋盘\n```\n{}\n```\n", board_text);
    let _ = writeln!(md, "## 推理链");
    if actionable.is_empty() {
        let _ = writeln!(md, "（无确定性结论）");
    } else {
        for (i, p) in actionable.iter().enumerate() {
            let tag = if p.conclusion.is_mine { "必雷" } else { "必安全" };
            let _ = writeln!(
                md,
                "{}. {} ({}, {}) — {}",
                i + 1,
                tag,
                p.conclusion.coord.x,
                p.conclusion.coord.y,
                p.rule
            );
        }
    }
    let _ = writeln!(md, "\n## 解读\n\n{}", analysis);
    md
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(s: &str) -> Vec<Vec<i32>> {
        parse_text_board(s).unwrap_or_else(|e| panic!("parse failed: {}", e))
    }

    #[test]
    fn test_parse_simple() {
        let b = parse_ok("? ? 1\n? 2 ?");
        assert_eq!(b, vec![vec![-1, -1, 1], vec![-1, 2, -1]]);
    }

    #[test]
    fn test_parse_separators_and_forms() {
        let b = parse_ok("?,-1,F\n0|1|🚩\nx X -2");
        assert_eq!(b, vec![vec![-1, -1, -2], vec![0, 1, -2], vec![-1, -1, -2]]);
    }

    #[test]
    fn test_parse_errors() {
        assert_eq!(parse_text_board(""), Err(BoardParseError::Empty));
        assert_eq!(parse_text_board("1 2"), Err(BoardParseError::TooFewRows(1)));
        assert!(matches!(
            parse_text_board("1 2 3\n1 2"),
            Err(BoardParseError::Ragged { .. })
        ));
        assert!(matches!(
            parse_text_board("1 2\n1 Q 3"),
            Err(BoardParseError::BadToken { .. })
        ));
    }

    #[test]
    fn test_left_cycle_semantics() {
        let b = vec![vec![-1, -2, 1, 8, 0, 7]];
        let b = apply_board_op(&b, 0, 0, BoardOp::LeftCycle); // 未知→1
        assert_eq!(b[0][0], 1);
        let b = apply_board_op(&b, 1, 0, BoardOp::LeftCycle); // 旗→1
        assert_eq!(b[0][1], 1);
        let b = apply_board_op(&b, 2, 0, BoardOp::LeftCycle); // 1→2
        assert_eq!(b[0][2], 2);
        let b = apply_board_op(&b, 3, 0, BoardOp::LeftCycle); // 8→未知
        assert_eq!(b[0][3], -1);
        let b = apply_board_op(&b, 4, 0, BoardOp::LeftCycle); // 空白 0 → 数字 1 (bug 修复)
        assert_eq!(b[0][4], 1);
        let b = apply_board_op(&b, 4, 0, BoardOp::LeftCycle); // 1→2
        assert_eq!(b[0][4], 2);
        let b = apply_board_op(&b, 5, 0, BoardOp::LeftCycle); // 7→8
        assert_eq!(b[0][5], 8);
    }

    #[test]
    fn test_flag_clear_ops() {
        let b = vec![vec![-1, -2, 3]];
        let b = apply_board_op(&b, 1, 0, BoardOp::ToggleFlag); // 旗→未知
        assert_eq!(b[0][1], -1);
        let b = apply_board_op(&b, 0, 0, BoardOp::ToggleFlag); // 未知→旗
        assert_eq!(b[0][0], -2);
        let b = apply_board_op(&b, 2, 0, BoardOp::ToggleFlag); // 数字不可插旗
        assert_eq!(b[0][2], 3);
        let b = apply_board_op(&b, 2, 0, BoardOp::ClearToZero);
        assert_eq!(b[0][2], 0);
        // Ctrl+左键: 任意状态 → 未知; 已是未知则幂等
        let b = apply_board_op(&b, 0, 0, BoardOp::SetUnknown); // 旗 → 未知
        assert_eq!(b[0][0], -1);
        let b = apply_board_op(&b, 2, 0, BoardOp::SetUnknown); // 0 → 未知
        assert_eq!(b[0][2], -1);
        let b = apply_board_op(&b, 2, 0, BoardOp::SetUnknown); // 已是未知, 幂等
        assert_eq!(b[0][2], -1);
        // 越界幂等
        assert_eq!(apply_board_op(&b, 99, 99, BoardOp::LeftCycle), b);
    }

    #[test]
    fn test_delete_row_col_ops() {
        let b = vec![vec![1, 2, 3], vec![4, 5, 6], vec![7, 8, 9]];
        // 删除第 1 行 (y=1)
        let r = apply_board_op(&b, 0, 1, BoardOp::DeleteRow);
        assert_eq!(r, vec![vec![1, 2, 3], vec![7, 8, 9]]);
        // 删除第 0 列 (x=0)
        let c = apply_board_op(&b, 0, 0, BoardOp::DeleteCol);
        assert_eq!(c, vec![vec![2, 3], vec![5, 6], vec![8, 9]]);
        // 越界: 返回原棋盘
        assert_eq!(apply_board_op(&b, 0, 5, BoardOp::DeleteRow), b);
        assert_eq!(apply_board_op(&b, 5, 0, BoardOp::DeleteCol), b);
        // 删除唯一一行 → 空 (不崩溃)
        let single = vec![vec![1, 2]];
        assert!(apply_board_op(&single, 0, 0, BoardOp::DeleteRow).is_empty());
    }

    #[test]
    fn test_infer_mines() {
        let b = vec![vec![-1; 9]; 9];
        assert_eq!(infer_mines_for_size(&b), Some(10));
        // 16x30, 首行 30 旗 → 99-30=69
        let mut b30 = vec![vec![-1; 30]; 16];
        b30[0] = vec![-2; 30];
        assert_eq!(infer_mines_for_size(&b30), Some(69));
        // 非标准尺寸 → None
        let small = vec![vec![-1; 5]; 5];
        assert_eq!(infer_mines_for_size(&small), None);
    }

    #[test]
    fn test_render_report_contains_parts() {
        let b = parse_ok("? ? 1\n? 2 ?");
        let ir = crate::model::InferenceIR {
            deterministic: vec![crate::model::Proof {
                conclusion: crate::model::Conclusion {
                    coord: crate::model::Coord::new(2, 1),
                    is_mine: false,
                },
                depends_on: vec![],
                rule: "数字2周围…".into(),
            }],
            probabilities: vec![],
            regions: vec![],
        
            flag_verification: crate::model::FlagVerificationResult::empty(),};
        let view = crate::model::PlayerView {
            width: 3,
            height: 2,
            revealed: vec![],
            flagged: vec![],
            unknown: vec![],
            remaining_mines: 1,
        };
        let md = render_markdown_report(&b, &view, &ir, "解读正文");
        assert!(md.contains("3x2"));
        assert!(md.contains("必安全"));
        assert!(md.contains("? ? 1"));
        assert!(md.contains("解读正文"));
    }
}
