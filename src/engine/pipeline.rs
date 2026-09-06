//! 本地推理管道 — 被 analyze / export / Agent 工具执行器共用, 保证任务编排一致。
//!
//! 编排: 旗帜正确性验证(容错) → 确定性推理(+子集+局部枚举) → 概率 → 区域。
//!
//! 容错语义: 用户可能误标旗。verify_board 先把与数字约束矛盾的旗帜降级为
//! "未知"构造容错视图, 推理不再信任矛盾旗; 判定结果随 IR 的
//! flag_verification 字段返回, 供前端高亮矛盾旗并提示纠错。

use crate::model::{Conclusion, Coord, InferenceIR, PlayerView, Proof};
use super::{DeterministicEngine, ProbabilityEngine, RegionAnalyzer};

/// 输入上限 (防御恶意/误传超大连载: 棋盘 ≤100x100, 雷数 ≤ 总格数)
pub const MAX_BOARD_DIM: usize = 100;
pub const MAX_BOARD_CELLS: usize = 10_000;

/// 输入校验: 非空 / 矩形 / 尺寸上限 / 格子取值合法 / 雷数合理。
/// 所有对外入口 (analyze/export/agent/chat/OCR) 最终都经 run_local_pipeline,
/// 在单一关卡统一校验, 防止构造异常棋盘造成 CPU/内存耗尽或歧义。
pub fn validate_board_input(
    board: &[Vec<i32>],
    remaining_mines: u32,
) -> Result<(), String> {
    if board.is_empty() || board[0].is_empty() {
        return Err("棋盘为空".to_string());
    }
    let rows = board.len();
    let cols = board[0].len();
    if rows > MAX_BOARD_DIM || cols > MAX_BOARD_DIM {
        return Err(format!(
            "棋盘尺寸 {}x{} 超过上限 {}x{}",
            rows, cols, MAX_BOARD_DIM, MAX_BOARD_DIM
        ));
    }
    let cells = rows * cols;
    if cells > MAX_BOARD_CELLS {
        return Err(format!("棋盘总格数 {} 超过上限 {}", cells, MAX_BOARD_CELLS));
    }
    for (y, row) in board.iter().enumerate() {
        if row.len() != cols {
            return Err(format!(
                "棋盘不是矩形: 第 {} 行宽度 {} 与首行 {} 不一致",
                y,
                row.len(),
                cols
            ));
        }
        for (x, v) in row.iter().enumerate() {
            if !matches!(v, -2 | -1 | 0..=8) {
                return Err(format!(
                    "非法格子值 {} 位于 ({}, {}); 只允许 -2(旗) -1(未知) 0-8(数字)",
                    v, x, y
                ));
            }
        }
    }
    if remaining_mines as usize > cells {
        return Err(format!(
            "剩余雷数 {} 大于总格数 {}",
            remaining_mines, cells
        ));
    }
    Ok(())
}

/// 对 board 执行完整本地推理管道, 返回 (PlayerView, InferenceIR)。
pub fn run_local_pipeline(
    board: &[Vec<i32>],
    remaining_mines: u32,
) -> Result<(PlayerView, InferenceIR), String> {
    validate_board_input(board, remaining_mines)?;
    // 旗帜正确性判定 (在推理之前执行) — 返回容错视图 + 形式化证明链
    let (view, flag_verification, flag_proofs) = super::verify_board_full(board, remaining_mines);

    // 运行确定性推理 (含子集规则)
    let mut deterministic = DeterministicEngine::solve_with_subset_rule(&view);

    // 合并旗帜验证与"局部闭包枚举"的形式化结论 (按坐标去重, 保证 已知雷/安全 集合一致)
    let has_coord = |c: &Coord, proofs: &[Proof]| proofs.iter().any(|p| &p.conclusion.coord == c);
    for p in flag_proofs
        .iter()
        .chain(super::derive_local_forced_proofs(&view).iter())
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
    // 不存在把雷放在该格的可行方案)。这些结论并入确定性链, 并从概率列表中移除。
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

/// 依据 (PlayerView, InferenceIR) 生成紧凑棋盘摘要 (供 Agent 提示词复用)
pub fn board_summary(view: &PlayerView, ir: &InferenceIR) -> String {
    let mines_unflagged = ir
        .deterministic
        .iter()
        .filter(|p| p.conclusion.is_mine)
        .filter(|p| view.unknown.iter().any(|c| *c == p.conclusion.coord))
        .count();
    let safe_unknown = ir
        .deterministic
        .iter()
        .filter(|p| !p.conclusion.is_mine)
        .filter(|p| view.unknown.iter().any(|c| *c == p.conclusion.coord))
        .count();
    let contrad = ir
        .flag_verification
        .flags
        .iter()
        .filter(|f| f.status == crate::model::FlagVerifyStatus::Contradicted)
        .count();
    format!(
        "棋盘 {}x{}, 剩余雷 {}, 已标旗 {}, 未翻开 {} 格; 引擎结论: 必雷未标旗 {} 格, \
         必安全未翻开 {} 格, 矛盾旗 {} 面, 待定概率格 {} 个",
        view.width,
        view.height,
        view.remaining_mines,
        view.flagged.len(),
        view.unknown.len(),
        mines_unflagged,
        safe_unknown,
        contrad,
        ir.probabilities.len()
    )
}
