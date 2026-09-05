use std::collections::{HashMap, HashSet};
use crate::model::{PlayerView, Coord, CellProb, CellState};

/// 概率引擎 — 基于约束的解析概率计算
///
/// 参考 minesweeper_solver/src/solver.py 的三步法:
///   1. 直接约束: 每个数字格 → remaining_mines / unknown_neighbors_count
///   2. 重叠约束细化: 多个数字约束同一格时取上下界
///   3. 剩余格均分: 未被约束的格子按剩余雷数均分
///
/// 额外增强 (参考 ms_toollib 的精确枚举):
///   - 对小规模约束连通组 (≤16 未知格) 做全枚举, 得到精确概率
///   - 对大规模组使用解析近似 + 子集约束迭代细化
///   - 蒙特卡洛作为最终回退 (仅在小棋盘时有效)
pub struct ProbabilityEngine;

/// 约束: (未知邻居坐标列表, 剩余雷数)
type Constraint = (Vec<Coord>, i32);

/// 蒙特卡洛引擎 (保留接口兼容, 内部委托给 ProbabilityEngine)
#[derive(Default)]
pub struct MonteCarloEngine {
    pub iterations: usize,
}

impl MonteCarloEngine {
    pub fn new(iterations: usize) -> Self {
        Self { iterations }
    }

    pub fn dynamic_iterations(unknown_count: usize) -> usize {
        if unknown_count < 50 { 50_000 }
        else if unknown_count <= 200 { 10_000 }
        else { 1_000 }
    }

    pub fn simulate(&self, view: &PlayerView) -> Vec<CellProb> {
        ProbabilityEngine::compute(view, &HashSet::new(), &HashSet::new())
    }

    pub fn simulate_with_deductions(
        &self,
        view: &PlayerView,
        known_mines: &HashSet<Coord>,
        known_safe: &HashSet<Coord>,
    ) -> Vec<CellProb> {
        ProbabilityEngine::compute(view, known_mines, known_safe)
    }
}

impl ProbabilityEngine {
    /// 主入口: 计算每个未知格的雷概率
    pub fn compute(
        view: &PlayerView,
        known_mines: &HashSet<Coord>,
        known_safe: &HashSet<Coord>,
    ) -> Vec<CellProb> {
        let state = view.to_state_map();

        // 有效未知格: 排除已知雷/安全
        let unknowns: Vec<Coord> = view.unknown.iter()
            .filter(|c| !known_mines.contains(c) && !known_safe.contains(c))
            .copied()
            .collect();
        if unknowns.is_empty() {
            return Vec::new();
        }

        let unknown_set: HashSet<Coord> = unknowns.iter().copied().collect();

        // 1. 构建约束 (参考 minesweeper_solver._apply_direct_constraints)
        let constraints = Self::build_constraints(view, &state, known_mines, known_safe);

        // 2. 初始化概率 (参考 minesweeper_solver._initialize_probabilities)
        let mut probs: HashMap<Coord, f64> = HashMap::new();
        for c in &unknowns {
            probs.insert(*c, -1.0); // -1 = 未计算
        }

        // 3. 直接约束概率 (参考 minesweeper_solver._apply_direct_constraints)
        for (cells, rem) in &constraints {
            if cells.is_empty() || *rem < 0 {
                continue;
            }
            let p = *rem as f64 / cells.len() as f64;
            for c in cells {
                let cur = probs.get(c).copied().unwrap_or(-1.0);
                if cur < 0.0 {
                    probs.insert(*c, p);
                } else {
                    // 取最小值 (最严格约束)
                    probs.insert(*c, cur.min(p));
                }
            }
        }

        // 4. 精确枚举小约束组 (参考 ms_toollib: 全枚举精确概率)
        Self::exact_enumeration(&constraints, &unknown_set, &mut probs);

        // 5. 重叠约束细化 (参考 minesweeper_solver._refine_overlapping_constraints)
        Self::refine_overlapping(&constraints, &mut probs);

        // 7. 未计算格均分剩余雷 (参考 minesweeper_solver._handle_remaining_cells)
        //
        // 语义约定: PlayerView.remaining_mines 是"用户输入/推断的剩余雷数",
        // 即 总雷数 - 已标旗数 —— 已标旗已从该值中扣除。
        // 因此这里不能再用 known_mines 全部计数: known_mines 中含大量"旗帜证明"
        // (坐标正是已标旗格), 若再次扣除会把剩余雷数错误压到 0,
        // 使不与任何数字相邻的未知格全部得到 0% 而误判安全。
        // 正确口径: 只扣 known_mines 中"非旗帜格" (它们将额外消耗剩余雷数)。
        let flagged_set: HashSet<Coord> = view.flagged.iter().copied().collect();
        let known_nonflag_mines = known_mines
            .iter()
            .filter(|c| !flagged_set.contains(c))
            .count();
        let remaining_mines =
            (view.remaining_mines as i64 - known_nonflag_mines as i64).max(0) as f64;

        let assigned_mines: f64 = probs.values()
            .filter(|&&p| p > 0.0)
            .map(|&p| p)
            .sum();

        let uncalculated: Vec<Coord> = probs.iter()
            .filter(|(_, &v)| v < 0.0)
            .map(|(c, _)| *c)
            .collect();

        if !uncalculated.is_empty() {
            let remaining = remaining_mines - assigned_mines;
            let base_p = remaining / uncalculated.len() as f64;
            for c in &uncalculated {
                probs.insert(*c, base_p);
            }
        }

        // 7. 裁剪到 [0, 1]
        unknowns.iter().map(|c| {
            let p = probs.get(c).copied().unwrap_or(0.0).clamp(0.0, 1.0);
            CellProb {
                coord: *c,
                mine_probability: p,
                info_gain: 0.0,
            }
        }).collect()
    }

    /// 构建约束列表: (未知邻居, 剩余雷数)
    /// 注意: 已知安全格 (known_safe) 不得作为变量参与约束 —— 否则采样空间会把
    /// 已被证明必安全的格当"可放雷"处理, 破坏链式结论 (如 3 的 1/2 分组瓜分后
    /// 相邻格本应恒安全, 却因组内混入已知安全格而算出非零概率)。
    fn build_constraints(
        view: &PlayerView,
        state: &HashMap<Coord, CellState>,
        known_mines: &HashSet<Coord>,
        known_safe: &HashSet<Coord>,
    ) -> Vec<Constraint> {
        view.revealed.iter().filter_map(|cell| {
            let neighbors = cell.coord.neighbors(view.width, view.height);
            let unknowns: Vec<Coord> = neighbors.iter()
                .filter(|n| {
                    matches!(state.get(n), Some(CellState::Unknown))
                        && !known_mines.contains(n)
                        && !known_safe.contains(n)
                })
                .copied()
                .collect();
            if unknowns.is_empty() {
                return None;
            }
            let flagged_count = neighbors.iter()
                .filter(|n| {
                    matches!(state.get(n), Some(CellState::Flagged))
                        || known_mines.contains(n)
                })
                .count() as i32;
            let remaining = cell.number as i32 - flagged_count;
            Some((unknowns, remaining))
        }).collect()
    }

    /// 精确枚举小约束组 (≤16 未知格): 全枚举所有合法雷布局, 统计频率
    /// 参考 ms_toollib 的精确枚举方法
    fn exact_enumeration(
        constraints: &[Constraint],
        unknown_set: &HashSet<Coord>,
        probs: &mut HashMap<Coord, f64>,
    ) {
        // 找到约束的连通组 (共享未知格的约束为一组)
        let groups = Self::find_constraint_groups(constraints, unknown_set);
        for group in &groups {
            let group_cells: HashSet<Coord> = group.iter()
                .flat_map(|(cells, _)| cells.iter().copied())
                .collect();
            let cell_count = group_cells.len();
            // 只对小组做全枚举 (2^16 = 65536, 可接受)
            if cell_count > 0 && cell_count <= 16 {
                Self::enumerate_group(group, &group_cells, probs);
            }
        }
    }

    /// 找到约束的连通组 (共享未知格的约束归为一组)
    fn find_constraint_groups(
        constraints: &[Constraint],
        _unknown_set: &HashSet<Coord>,
    ) -> Vec<Vec<Constraint>> {
        if constraints.is_empty() {
            return Vec::new();
        }
        let n = constraints.len();
        let mut visited = vec![false; n];
        let mut groups = Vec::new();

        for i in 0..n {
            if visited[i] {
                continue;
            }
            let mut group = Vec::new();
            let mut queue = vec![i];
            visited[i] = true;
            let mut group_cells: HashSet<Coord> = HashSet::new();
            while let Some(idx) = queue.pop() {
                let (cells, _) = &constraints[idx];
                for c in cells {
                    group_cells.insert(*c);
                }
                group.push(constraints[idx].clone());
                // 找共享格子的约束
                for j in 0..n {
                    if visited[j] {
                        continue;
                    }
                    let (jcells, _) = &constraints[j];
                    if jcells.iter().any(|c| group_cells.contains(c)) {
                        visited[j] = true;
                        queue.push(j);
                    }
                }
            }
            groups.push(group);
        }
        groups
    }

    /// 全枚举一个小约束组的所有合法布局, 更新精确概率
    fn enumerate_group(
        group: &[Constraint],
        group_cells: &HashSet<Coord>,
        probs: &mut HashMap<Coord, f64>,
    ) {
        let cells: Vec<Coord> = group_cells.iter().copied().collect();
        let n = cells.len();
        if n == 0 || n > 16 {
            return;
        }

        // 坐标到位索引
        let idx_map: HashMap<Coord, usize> = cells.iter()
            .enumerate().map(|(i, c)| (*c, i)).collect();

        // 将约束转为位掩码
        let mut masks: Vec<(u32, i32)> = Vec::new();
        for (constraint_cells, rem) in group {
            let mut mask = 0u32;
            for c in constraint_cells {
                if let Some(&i) = idx_map.get(c) {
                    mask |= 1 << i;
                }
            }
            masks.push((mask, *rem));
        }

        // 枚举所有 2^n 子集
        let total = 1u64 << n;
        let mut valid_count = 0u64;
        let mut mine_counts = vec![0u64; n];

        for subset in 0..total {
            let bits = subset as u32;
            // 检查所有约束
            let mut ok = true;
            for &(mask, rem) in &masks {
                let count = (bits & mask).count_ones() as i32;
                if count != rem {
                    ok = false;
                    break;
                }
            }
            if ok {
                valid_count += 1;
                for i in 0..n {
                    if bits & (1 << i) != 0 {
                        mine_counts[i] += 1;
                    }
                }
            }
        }

        if valid_count == 0 {
            return;
        }

        // 更新概率为精确值
        for (i, c) in cells.iter().enumerate() {
            let exact_p = mine_counts[i] as f64 / valid_count as f64;
            probs.insert(*c, exact_p);
        }
    }

    /// 重叠约束细化 (参考 minesweeper_solver._refine_overlapping_constraints)
    fn refine_overlapping(
        constraints: &[Constraint],
        probs: &mut HashMap<Coord, f64>,
    ) {
        // 多次迭代直到收敛
        for _iteration in 0..5 {
            let mut changed = false;
            for i in 0..constraints.len() {
                let (cells_i, rem_i) = &constraints[i];
                if cells_i.is_empty() {
                    continue;
                }
                let set_i: HashSet<Coord> = cells_i.iter().copied().collect();

                for j in 0..constraints.len() {
                    if i == j {
                        continue;
                    }
                    let (cells_j, rem_j) = &constraints[j];
                    let set_j: HashSet<Coord> = cells_j.iter().copied().collect();

                    // A ⊂ B: A 的未知格是 B 的子集
                    if set_i.is_subset(&set_j) && set_i.len() < set_j.len() {
                        let diff: Vec<Coord> = set_j.difference(&set_i).copied().collect();
                        let rem_diff = rem_j - rem_i;

                        if rem_diff == 0 && !diff.is_empty() {
                            // 差集中的格全部安全
                            for c in &diff {
                            let cur = probs.get(c).copied().unwrap_or(-1.0);
                                if cur != 0.0 {
                                    probs.insert(*c, 0.0);
                                    changed = true;
                                }
                            }
                        } else if rem_diff == diff.len() as i32 && rem_diff > 0 {
                            // 差集中的格全部是雷
                            for c in &diff {
                                let cur = probs.get(c).copied().unwrap_or(-1.0);
                                if cur != 1.0 {
                                    probs.insert(*c, 1.0);
                                    changed = true;
                                }
                            }
                        } else if !diff.is_empty() && rem_diff > 0 {
                            // 差集中有 rem_diff 个雷
                            let p = rem_diff as f64 / diff.len() as f64;
                            for c in &diff {
                                let cur = probs.get(c).copied().unwrap_or(-1.0);
                                if cur < 0.0 || p < cur {
                                    if cur < 0.0 || (p - cur).abs() > 0.001 {
                                        probs.insert(*c, p);
                                        changed = true;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// known_safe 格不得作为约束变量泄漏: 若泄漏, 其余候选格概率会被稀释。
    #[test]
    fn test_known_safe_not_leaked_into_constraints() {
        // 两个 1 与两个未知格构成 2 选 1 约束组
        // 若 (1,0) 被证明必安全, (0,1) 应成为唯一候选 → 概率 1.0
        let board = vec![vec![1, -1], vec![-1, 1]];
        let view = PlayerView::from_2d(&board, 1);
        let safe: HashSet<Coord> = HashSet::from([Coord::new(1, 0)]);
        let probs = ProbabilityEngine::compute(&view, &HashSet::new(), &safe);
        let p01 = probs.iter().find(|p| p.coord == Coord::new(0, 1)).map(|p| p.mine_probability);
        assert!(
            probs.iter().all(|p| p.coord != Coord::new(1, 0)),
            "known_safe 格不应出现在结果中"
        );
        let expected = p01.unwrap_or(0.0);
        assert!((expected - 1.0).abs() < 1e-6, "唯一候选应为必雷, got {}", expected);
    }

    /// remaining_mines 语义 = 总雷数 - 已标旗数; known_mines 含"旗帜证明"时
    /// 不得二次扣减, 否则未约束未知格会被错误归零 (extra2 全安全 bug)。
    #[test]
    fn test_remaining_mines_counts_flags_once() {
        // 2 面旗 + 4 个未知格, 剩余 1 雷, 无数值约束 (不与任何数字相邻)
        let board = vec![vec![-2, -1, -1], vec![-2, -1, -1]];
        let view = PlayerView::from_2d(&board, 1);
        let known_mines: HashSet<Coord> = view.flagged.iter().copied().collect();
        let probs = ProbabilityEngine::compute(&view, &known_mines, &HashSet::new());
        assert_eq!(probs.len(), 4);
        for p in &probs {
            assert!(
                p.mine_probability > 0.0 && p.mine_probability <= 1.0,
                "未知格概率被错误归零: {:?} = {}",
                p.coord,
                p.mine_probability
            );
            assert!((p.mine_probability - 0.25).abs() < 1e-6, "got {}", p.mine_probability);
        }
    }
}
