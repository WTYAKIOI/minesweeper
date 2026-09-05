//! 旗帜正确性判定模块
//!
//! 背景: 旧实现对用户标注的旗帜"全盘信任", 一旦误标, 整个推理基于错误前提。
//! 本模块在确定性推理/概率模拟**之前**验证每面旗帜与当前数字约束是否自洽:
//!
//!   1. 单旗约束检查: 相邻数字的已标旗数 > 数字值 → 矛盾;
//!      相邻数字为 0 → 矛盾 (0 的邻居必安全)。
//!   2. 必要性检查: 若某数字的邻居中去掉该旗后, 即使所有未知格都算雷仍
//!      无法凑够数字值 → 该旗是"必要雷" → Verified。
//!   3. 容错重推: 把矛盾旗降级为"未知"构造容错视图, 跑确定性推理:
//!      - 证明该格必安全 → 该旗确系误标 (Contradicted, 强证据);
//!      - 证明该格必为雷 → 原旗反而正确 (Verified)。
//!
//! 容错视图 (矛盾旗降级为未知) 会返回给推理管道, 使后续推理不再信任矛盾旗。

use std::collections::{HashMap, HashSet};

use crate::engine::DeterministicEngine;
use crate::model::{
    Conclusion, Coord, FlagStatus, FlagVerificationResult, FlagVerifyStatus, PlayerView, Proof,
    RevealedCell,
};

/// 验证入口 (旧签名包装, 忽略证明链)
pub fn verify_board(
    board: &[Vec<i32>],
    remaining_mines: u32,
) -> (PlayerView, FlagVerificationResult) {
    let (view, fvr, _proofs) = verify_board_full(board, remaining_mines);
    (view, fvr)
}

/// 验证结果: (容错视图, 旗帜判定结果, 形式化证明链)
/// 容错视图: 矛盾旗已被降级为未知格, 供确定性推理/概率模拟使用。
/// 证明链: 由确定性推理/局部组合枚举得出的"恒为安全/恒为雷"结论, 可并入主推理链,
/// 并作为 known_safe/known_mines 排除于概率采样 (避免与旗帜判定矛盾的百分比)。
pub fn verify_board_full(
    board: &[Vec<i32>],
    remaining_mines: u32,
) -> (PlayerView, FlagVerificationResult, Vec<Proof>) {
    let orig = PlayerView::from_2d(board, remaining_mines);
    if orig.flagged.is_empty() {
        return (
            orig,
            FlagVerificationResult {
                flags: Vec::new(),
                summary: "棋盘上没有旗帜, 无需验证".to_string(),
                has_contradiction: false,
            },
            Vec::new(),
        );
    }

    let (w, h) = (orig.width, orig.height);
    let flagged_set: HashSet<Coord> = orig.flagged.iter().copied().collect();
    let revealed = orig.revealed.clone();

    // ---------------- 第 1 步: 单旗约束检查 ----------------
    // flag -> 矛盾/理由; 先收集"超限"与"贴0"两类硬矛盾
    let mut statuses: HashMap<Coord, FlagVerifyStatus> = HashMap::new();
    let mut reasons: HashMap<Coord, Vec<String>> = HashMap::new();
    // 形式化证明集合: 只有这些格能给出"恒为安全/恒为雷"的确定性结论,
    // 可安全并入主推理链与 known_safe/known_mines (供概率引擎排除)
    let mut proven_safe: HashSet<Coord> = HashSet::new();
    let mut proven_mine: HashSet<Coord> = HashSet::new();

    for rv in &revealed {
        let nb = rv.coord.neighbors(w, h);
        let adjacent_flags: Vec<Coord> =
            nb.iter().copied().filter(|c| flagged_set.contains(c)).collect();
        if adjacent_flags.is_empty() {
            continue;
        }
        if rv.number == 0 {
            // 数字 0 的邻居必安全 → 旗必错 (形式证明: 该格恒为安全)
            for f in adjacent_flags {
                statuses.insert(f, FlagVerifyStatus::Contradicted);
                proven_safe.insert(f);
                reasons.entry(f).or_default().push(format!(
                    "相邻数字0位于{}, 其周围不可能有雷, 该旗必为误标",
                    rv.coord
                ));
            }
        } else if adjacent_flags.len() > rv.number as usize {
            // 旗数 > 数字 → 至少一面误标 (超限的部分旗中必含错旗, 全部标出供用户排查)
            for f in &adjacent_flags {
                statuses.insert(*f, FlagVerifyStatus::Contradicted);
                reasons.entry(*f).or_default().push(format!(
                    "数字{}位于{}周围已有{}面旗, 超出限制({}/{})",
                    rv.number,
                    rv.coord,
                    adjacent_flags.len(),
                    adjacent_flags.len(),
                    rv.number
                ));
            }
        }
    }

    // ---------------- 第 2 步: 必要性检查 (Verified) ----------------
    // 对未矛盾旗: 若某相邻数字去掉该旗后将无法满足 → 该旗必为雷
    for f in &orig.flagged {
        if statuses.get(f) == Some(&FlagVerifyStatus::Contradicted) {
            continue;
        }
        for rv in &revealed {
            if !rv.coord.neighbors(w, h).contains(f) {
                continue;
            }
            let nb = rv.coord.neighbors(w, h);
            let flags_total = nb.iter().filter(|c| flagged_set.contains(c)).count() as i32;
            let unknown_total = nb
                .iter()
                .filter(|c| !flagged_set.contains(c) && !is_revealed(&revealed, **c))
                .count() as i32;
            // 去掉该旗后, 可用候选 = (旗数-1) + 未知数; 若 < 数字值 → 该旗必要
            if (flags_total - 1) + unknown_total < rv.number as i32 {
                reasons.entry(*f).or_default().push(format!(
                    "该旗为数字{}位于{}的必要雷: 若移除则周围可用格不足满足{}",
                    rv.number, rv.coord, rv.number
                ));
                if statuses.get(f).is_none() {
                    statuses.insert(*f, FlagVerifyStatus::Verified);
                    proven_mine.insert(*f);
                }
            }
        }
    }

    // ---------------- 第 3 步: 容错视图 + 确定性推理 ----------------
    // 矛盾旗降级为未知, 重新推理: 得到必安全/必雷结论可升级判定证据
    let demoted: HashSet<Coord> = statuses
        .iter()
        .filter(|(_, s)| **s == FlagVerifyStatus::Contradicted)
        .map(|(c, _)| *c)
        .collect();
    let tolerant_board = demote_flags(board, &demoted);
    let tolerant_view = PlayerView::from_2d(&tolerant_board, remaining_mines);
    let proofs = DeterministicEngine::solve_with_subset_rule(&tolerant_view);
    let verdict_of: HashMap<Coord, bool> = proofs
        .iter()
        .map(|p| (p.conclusion.coord, p.conclusion.is_mine))
        .collect();

    for f in &demoted {
        match verdict_of.get(f) {
            // 推理证明该格必安全 → 确系误标
            Some(false) => {
                statuses.insert(*f, FlagVerifyStatus::Contradicted);
                proven_safe.insert(*f);
                reasons.entry(*f).or_default().push(
                    "确定性推理证明该格必安全, 旗帜与结论矛盾, 确系误标".to_string(),
                );
            }
            // 推理证明该格必为雷 → 原旗实际正确 (虽然曾因邻居超限被怀疑)
            Some(true) => {
                statuses.insert(*f, FlagVerifyStatus::Verified);
                proven_mine.insert(*f);
                reasons
                    .entry(*f)
                    .or_default()
                    .push("容错重推后证明该格必为雷, 旗帜判定为正确".to_string());
            }
            None => { /* 保持第 1 步的"矛盾"结论 */ }
        }
    }

    // ---------------- 第 4 步: 联合容错重推 (改进: 一次性降级全部"未验证旗") ----------------
    // 逐面探测的局限: 只移除一面时其余旗仍被当作固定雷, 跨数字子集矛盾
    // (如 3 周围的 1/2 分组约束) 无法显现。把全部 Suspected 旗一起降级为未知,
    // 在"自洽空间"内重推: 确定性证明 → 直接判定; 概率求解给出可信边际。
    // 第 1~3 步中未被标记的旗 = 未判定 (Suspected): 全部进入联合重推
    let suspects: Vec<Coord> = orig
        .flagged
        .iter()
        .filter(|f| !statuses.contains_key(f))
        .copied()
        .collect();
    if !suspects.is_empty() {
        let joint_demoted: HashSet<Coord> = demoted.union(&suspects.iter().copied().collect()).copied().collect();
        let joint_board = demote_flags(board, &joint_demoted);
        let joint_view = PlayerView::from_2d(&joint_board, remaining_mines);
        let joint_proofs = DeterministicEngine::solve_with_subset_rule(&joint_view);
        let j_verdict: HashMap<Coord, bool> = joint_proofs
            .iter()
            .map(|p| (p.conclusion.coord, p.conclusion.is_mine))
            .collect();

        // 引擎结论 → 判定 (先应用: 确定性证明的必安全/必雷)
        for f in &suspects {
            match j_verdict.get(f) {
                Some(false) => {
                    statuses.insert(*f, FlagVerifyStatus::Contradicted);
                    proven_safe.insert(*f);
                    reasons.entry(*f).or_default().push(
                        "联合容错重推: 确定性推理证明该格必安全, 旗帜确系误标".to_string(),
                    );
                }
                Some(true) => {
                    statuses.insert(*f, FlagVerifyStatus::Verified);
                    proven_mine.insert(*f);
                    reasons
                        .entry(*f)
                        .or_default()
                        .push("联合容错重推后证明该格必为雷, 旗帜判定为正确".to_string());
                }
                None => {}
            }
        }

        // ---------------- 第 5 步: 局部闭包枚举 (链式子集矛盾) ----------------
        // 对仍未判定的旗, 逐一取其相邻数字的未知邻居 S 作约束闭包:
        // 仅纳入"未知邻居完全 ⊆ S"的数字 (外部雷不会进入 S, 松弛空间 ⊇ 原空间,
        // 故"松弛下亦无解/全解"的结论对原问题保持 sound)。
        // 覆盖如 "3 周围的两组 1/2 约束把雷瓜分 → 相邻旗格必非雷" 的跨数字链。
        let seeds: Vec<Coord> = suspects
            .iter()
            .filter(|s| !statuses.contains_key(s))
            .copied()
            .collect();
        for f in seeds {
            if statuses.contains_key(&f) {
                continue;
            }
            if let Some(is_mine) = local_neighborhood_verdict(&joint_view, f) {
                if is_mine {
                    statuses.insert(f, FlagVerifyStatus::Verified);
                    proven_mine.insert(f);
                    reasons.entry(f).or_default().push(
                        "局部组合枚举: 该旗相邻数字的所有可行雷方案中该格恒为雷, 旗帜判定为正确".to_string(),
                    );
                } else {
                    statuses.insert(f, FlagVerifyStatus::Contradicted);
                    proven_safe.insert(f);
                    reasons.entry(f).or_default().push(format!(
                        "局部组合枚举(含 {} 等约束链): 该格若为雷将使相邻数字超限, 所有可行方案中恒为安全, 旗帜确系误标",
                        f
                    ));
                }
            }
        }

        // ---------------- 概率边际 (仅对局部枚举仍无法判定的旗) ----------------
        // 已知集合: 一切已被证明必雷/必安全的格 (与 joint 视图未知格取交集),
        // 让蒙特卡洛在"剔除确定格"的自洽空间采样, 避免残存百分比与证明矛盾。
        let undecided: Vec<Coord> = suspects
            .iter()
            .filter(|s| !statuses.contains_key(s))
            .copied()
            .collect();
        if !undecided.is_empty() {
            let unknown_set: HashSet<Coord> = joint_view.unknown.iter().copied().collect();
            let known_mines: HashSet<Coord> = proven_mine.intersection(&unknown_set).copied().collect();
            let known_safe: HashSet<Coord> = proven_safe.intersection(&unknown_set).copied().collect();
            let j_probs = crate::engine::ProbabilityEngine::compute(&joint_view, &known_mines, &known_safe);
            for f in &undecided {
                let p = j_probs.iter().find(|p| p.coord == *f).map(|p| p.mine_probability);
                if let Some(p) = p {
                    reasons.entry(*f).or_default().push(format!(
                        "联合重推蒙特卡洛: 自洽空间中该格雷概率约 {:.0}%{}",
                        p * 100.0,
                        if p < 0.06 {
                            ", 极可能为误标"
                        } else if p > 0.94 {
                            ", 极可能正确"
                        } else {
                            ""
                        }
                    ));
                }
            }
        }
    }

    // ---------------- 汇总 ----------------
    let mut flags: Vec<FlagStatus> = Vec::with_capacity(orig.flagged.len());
    for f in &orig.flagged {
        let status = statuses.get(f).copied().unwrap_or(FlagVerifyStatus::Suspected);
        let reason = match status {
            FlagVerifyStatus::Verified => reasons
                .get(f)
                .map(|r| r.join("; "))
                .unwrap_or_else(|| "经约束检查确认与相邻数字自洽".to_string()),
            FlagVerifyStatus::Contradicted => reasons
                .get(f)
                .map(|r| r.join("; "))
                .unwrap_or_else(|| "与相邻数字约束矛盾".to_string()),
            FlagVerifyStatus::Suspected => {
                let has_number_neighbor = revealed.iter().any(|rv| rv.coord.neighbors(w, h).contains(f));
                let base = if has_number_neighbor {
                    "与相邻数字约束无矛盾, 但推理无法进一步确认该旗必为雷".to_string()
                } else {
                    "该旗不与任何已翻开数字相邻, 无法用数字约束验证".to_string()
                };
                // 联合重推阶段可能追加了概率佐证 (reason map)
                match reasons.get(f) {
                    Some(extra) => format!("{}; {}", base, extra.join("; ")),
                    None => base,
                }
            }
        };
        flags.push(FlagStatus { coord: *f, status, reason });
    }
    flags.sort_by_key(|f| (f.coord.y, f.coord.x));

    let (mut n_ok, mut n_sus, mut n_bad) = (0usize, 0usize, 0usize);
    for f in &flags {
        match f.status {
            FlagVerifyStatus::Verified => n_ok += 1,
            FlagVerifyStatus::Suspected => n_sus += 1,
            FlagVerifyStatus::Contradicted => n_bad += 1,
        }
    }
    let summary = if n_bad == 0 {
        format!("已检查 {} 面旗: 全部与当前棋盘自洽{}", orig.flagged.len(),
            if n_ok > 0 { format!(" (其中 {} 面经推理确认必为雷)", n_ok) } else { String::new() })
    } else {
        format!(
            "已检查 {} 面旗: 发现 {} 面可能标错, {} 面经推理确认正确, {} 面无明显矛盾",
            orig.flagged.len(),
            n_bad,
            n_ok,
            n_sus
        )
    };

    // 形式化证明链: proven_safe → 必安全; proven_mine → 必雷
    let mut proofs: Vec<Proof> = Vec::new();
    for f in &orig.flagged {
        let is_safe = proven_safe.contains(f);
        let is_mine = proven_mine.contains(f) && !is_safe;

        if !is_mine && !is_safe {
            continue;
        }
        let depends_on: Vec<Coord> = revealed
            .iter()
            .filter(|rv| rv.coord.neighbors(w, h).contains(f))
            .map(|rv| rv.coord)
            .take(8)
            .collect();
        let detail = reasons.get(f).map(|r| r.join("; ")).unwrap_or_default();
        let rule = if is_mine {
            format!("旗帜验证: 该旗经推理/组合枚举确认必为雷。{}", detail)
        } else {
            format!("旗帜验证: 该旗确系误标 (该格必安全)。{}", detail)
        };
        proofs.push(Proof {
            conclusion: Conclusion { coord: *f, is_mine },
            depends_on,
            rule,
        });
    }
    proofs.sort_by_key(|p| (p.conclusion.coord.y, p.conclusion.coord.x));

    (
        tolerant_view,
        FlagVerificationResult { flags, summary, has_contradiction: n_bad > 0 },
        proofs,
    )
}

/// 局部组合枚举: 对未判定的旗格 f, 构造其相邻数字的未知邻居 S, 并纳入所有
/// "未知邻居 ⊆ S" 的数字约束, 在 |S| ≤ 10 时枚举全部可行雷方案。
/// 返回该格在全部可行方案中恒为雷(true)/恒为安全(false)的判定, 无法判定返回 None。
///
/// Soundness: 被纳入的约束在其变量集 S 内封闭; 未纳入数字仅会扩大可行空间,
/// 因此"松弛问题中恒真/恒假"的结论对原问题同样成立 (跨数字子集链由此可证)。
fn local_neighborhood_verdict(view: &PlayerView, f: Coord) -> Option<bool> {
    let flagged: HashSet<Coord> = view.flagged.iter().copied().collect();
    let state = view.to_state_map();
    // S = f 相邻数字的未知邻居并集 (含 f 自身)
    let mut s: HashSet<Coord> = HashSet::new();
    let mut seed_numbers: Vec<Coord> = Vec::new();
    for rv in &view.revealed {
        if rv.coord.neighbors(view.width, view.height).contains(&f) {
            seed_numbers.push(rv.coord);
        }
    }
    if seed_numbers.is_empty() {
        return None; // 不与任何数字相邻
    }
    for n in &seed_numbers {
        for c in n.neighbors(view.width, view.height) {
            if matches!(state.get(&c), Some(crate::model::CellState::Unknown)) {
                s.insert(c);
            }
        }
    }
    if s.is_empty() || s.len() > 10 {
        return None;
    }
    // 数字约束闭包: 只纳入未知邻居 ⊆ S 的数字
    let mut constraints: Vec<(Vec<Coord>, i32)> = Vec::new();
    let mut any_with_f = false;
    for rv in &view.revealed {
        let nb = rv.coord.neighbors(view.width, view.height);
        let mut unknown_nb: Vec<Coord> = Vec::new();
        let mut flags_n = 0i32;
        for c in &nb {
            if flagged.contains(c) {
                flags_n += 1;
            } else if matches!(state.get(c), Some(crate::model::CellState::Unknown)) {
                unknown_nb.push(*c);
            }
        }
        if unknown_nb.is_empty() {
            continue;
        }
        // 必须完全落在 S 内 (含 S 未含则跳过该约束, 保持 sound)
        if !unknown_nb.iter().all(|c| s.contains(c)) {
            continue;
        }
        let need = rv.number as i32 - flags_n;
        if need < 0 {
            continue; // 固定旗已超限, 由更早阶段处理
        }
        if rv.coord.neighbors(view.width, view.height).contains(&f) {
            any_with_f = true;
        }
        constraints.push((unknown_nb, need));
    }
    if !any_with_f {
        return None;
    }
    // f 是否在某个约束的变量中
    let f_in = constraints.iter().any(|(cells, _)| cells.contains(&f));
    if !f_in {
        return None;
    }
    let vars: Vec<Coord> = s.iter().copied().collect();
    let k = vars.len();
    let idx: HashMap<Coord, usize> = vars.iter().enumerate().map(|(i, c)| (*c, i)).collect();
    let total: u64 = 1u64 << k;
    let mut valid_with_f = 0u64;
    let mut valid_without_f = 0u64;
    for mask in 0..total {
        let has_f = (mask >> idx[&f]) & 1 == 1;
        let mut ok = true;
        for (cells, need) in &constraints {
            let mut c = 0i32;
            for cell in cells {
                if (mask >> idx[cell]) & 1 == 1 {
                    c += 1;
                }
            }
            if c != *need {
                ok = false;
                break;
            }
        }
        if ok {
            if has_f {
                valid_with_f += 1;
            } else {
                valid_without_f += 1;
            }
        }
    }
    if valid_with_f > 0 && valid_without_f == 0 {
        Some(true) // 恒为雷
    } else if valid_with_f == 0 && valid_without_f > 0 {
        Some(false) // 恒为安全 → 旗必错
    } else {
        None
    }
}

/// 对棋盘上每个未知格做"局部闭包组合枚举", 把恒为安全/恒为雷的格导出为
/// 确定性 Proof (供主推理链与概率引擎的 known_safe/known_mines 消费)。
/// 解决大连通组 (>16 格) 时概率求解器退化为近似、把可证明格算成非零概率的问题。
pub fn derive_local_forced_proofs(view: &PlayerView) -> Vec<Proof> {
    let mut out: Vec<Proof> = Vec::new();
    for c in &view.unknown {
        if let Some(is_mine) = local_neighborhood_verdict(view, *c) {
            let depends_on: Vec<Coord> = view
                .revealed
                .iter()
                .filter(|rv| rv.coord.neighbors(view.width, view.height).contains(c))
                .map(|rv| rv.coord)
                .take(8)
                .collect();
            let rule = if is_mine {
                "确定性组合推断(局部闭包枚举): 该格在所有可行雷方案中恒为雷".to_string()
            } else {
                "确定性组合推断(局部闭包枚举): 该格在所有可行雷方案中恒为安全 (若为雷将使相邻数字超限)".to_string()
            };
            out.push(Proof {
                conclusion: Conclusion { coord: *c, is_mine },
                depends_on,
                rule,
            });
        }
    }
    out
}

/// 将矛盾旗坐标在棋盘副本中降级为未知 (-1)
fn demote_flags(board: &[Vec<i32>], demoted: &HashSet<Coord>) -> Vec<Vec<i32>> {
    let mut out: Vec<Vec<i32>> = board.to_vec();
    for c in demoted {
        let (x, y) = (c.x as usize, c.y as usize);
        if y < out.len() && x < out[y].len() && out[y][x] == -2 {
            out[y][x] = -1;
        }
    }
    out
}

fn is_revealed(revealed: &[RevealedCell], c: Coord) -> bool {
    revealed.iter().any(|r| r.coord == c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::FlagVerifyStatus;

    fn status_of(res: &FlagVerificationResult, x: u32, y: u32) -> Option<FlagVerifyStatus> {
        res.flags
            .iter()
            .find(|f| f.coord == Coord::new(x, y))
            .map(|f| f.status)
    }

    #[test]
    fn test_no_flags() {
        let board = vec![vec![0, 1, -1], vec![-1, -1, -1]];
        let (_view, res) = verify_board(&board, 2);
        assert!(res.flags.is_empty());
        assert!(!res.has_contradiction);
        assert!(res.summary.contains("没有旗帜"));
    }

    #[test]
    fn test_clean_flags_all_self_consistent() {
        // 单行棋盘: 数字1 (0,0) 的邻居只有旗 (1,0) → 移除即无法满足 → Verified
        let board = vec![vec![1, -2, -1]];
        let (_view, res) = verify_board(&board, 10);
        assert!(!res.has_contradiction);
        assert_eq!(status_of(&res, 1, 0), Some(FlagVerifyStatus::Verified));
        assert!(res.flags[0].reason.contains("必要雷"));
        assert!(res.summary.contains("自洽"));
    }

    #[test]
    fn test_overcount_flag_contradiction() {
        // 数字1 (0,0) 周围两面旗 (1,0) (0,1) → 2>1 超限;
        // 容错重推: (0,1) 相邻数字0 被证明必安全 → 矛盾旗; (1,0) 成为唯一候选 → Verified
        let board = vec![
            vec![1, -2, -1],
            vec![-2, 2, -1],
            vec![0, -1, -1],
        ];
        let (_view, res) = verify_board(&board, 10);
        assert!(res.has_contradiction);
        assert_eq!(status_of(&res, 0, 1), Some(FlagVerifyStatus::Contradicted));
        assert_eq!(status_of(&res, 1, 0), Some(FlagVerifyStatus::Verified));
        assert!(res.flags.iter().any(|f| f.reason.contains("超出限制")));
    }

    #[test]
    fn test_flag_next_to_zero() {
        // 数字0 (0,2) 邻居 (1,2) 是旗 → 必错
        let board = vec![
            vec![-1, -1, 0],
            vec![-1, -2, -1],
            vec![-1, -1, -1],
        ];
        let (_view, res) = verify_board(&board, 10);
        assert!(res.has_contradiction);
        assert_eq!(status_of(&res, 1, 1), Some(FlagVerifyStatus::Contradicted));
        assert!(res.flags.iter().any(|f| f.reason.contains("数字0")));
    }

    #[test]
    fn test_tolerant_view_demotes_contradicted() {
        // 超限的两面旗先被降级为未知参与重推 (两者都不再作为已知雷),
        // 最终判定: 仅被证明必安全的 (0,1) 保持矛盾
        let board = vec![
            vec![1, -2, -1],
            vec![-2, 2, -1],
            vec![0, -1, -1],
        ];
        let (view, res) = verify_board(&board, 10);
        assert!(res.has_contradiction);
        let bad: HashSet<Coord> = res.contradicted_coords().into_iter().collect();
        assert_eq!(bad, HashSet::from([Coord::new(0, 1)]));
        // 两面原旗在容错视图中都已被降级为未知 (推理期间不受信任)
        for c in [Coord::new(0, 1), Coord::new(1, 0)] {
            assert!(view.unknown.contains(&c));
            assert!(!view.flagged.contains(&c));
        }
    }

    #[test]
    fn test_suspected_flag_when_ambiguous() {
        // 数字2 (0,0) 一面旗 (1,0), 另有多个未知邻居 → 旗未确认也非矛盾
        let board = vec![
            vec![2, -2, -1],
            vec![-1, -1, -1],
            vec![-1, -1, -1],
        ];
        let (_view, res) = verify_board(&board, 10);
        assert!(!res.has_contradiction);
        assert_eq!(status_of(&res, 1, 0), Some(FlagVerifyStatus::Suspected));
    }
}
