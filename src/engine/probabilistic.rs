use std::collections::{HashMap, HashSet};
use rand::seq::SliceRandom;
use rand::Rng;
use crate::model::{PlayerView, Coord, CellProb, CellState};

/// 蒙特卡洛概率模拟引擎
///
/// 对剩余未知格子进行多次随机分配雷，统计每个格子被踩中的频率。
/// 同时计算期望信息增益（模拟点击后若为0会连锁展开的面积）。
#[derive(Default)]
pub struct MonteCarloEngine {
    /// 模拟次数；设为 0 则启用动态迭代
    pub iterations: usize,
}

/// 动态迭代次数阈值
const DYNAMIC_HIGH: usize = 50_000;
const DYNAMIC_MID: usize = 10_000;
const DYNAMIC_LOW: usize = 1_000;

impl MonteCarloEngine {
    pub fn new(iterations: usize) -> Self {
        Self { iterations }
    }

    /// 根据未知格数量动态决定迭代次数
    /// - 未知格 < 50  → 50 000 次（高精度残局）
    /// - 未知格 50~200 → 10 000 次（中盘）
    /// - 未知格 > 200  →  1 000 次（开局 / 大棋盘）
    pub fn dynamic_iterations(unknown_count: usize) -> usize {
        if unknown_count < 50 {
            DYNAMIC_HIGH
        } else if unknown_count <= 200 {
            DYNAMIC_MID
        } else {
            DYNAMIC_LOW
        }
    }

    /// 返回本引擎实际应使用的迭代次数
    fn effective_iterations(&self, unknown_count: usize) -> usize {
        if self.iterations == 0 {
            Self::dynamic_iterations(unknown_count)
        } else {
            self.iterations
        }
    }

    /// 执行蒙特卡洛模拟，返回每个未知格子的 (雷概率, 信息增益)
    pub fn simulate(&self, view: &PlayerView) -> Vec<CellProb> {
        let state = view.to_state_map();
        let unknowns: Vec<Coord> = view.unknown.to_vec();
        if unknowns.is_empty() {
            return Vec::new();
        }

        // 构建约束：每个已翻开数字格子 → (未知邻居列表, 剩余雷数)
        let constraints = Self::build_constraints(view, &state);

        // 找出所有与约束相关的未知格 (边界格) 和与约束无关的未知格 (内部格)
        let constrained_set: HashSet<Coord> = constraints
            .iter()
            .flat_map(|(_, cells, _)| cells.iter().copied())
            .collect();
        let unconstrained: Vec<Coord> = unknowns
            .iter()
            .filter(|c| !constrained_set.contains(c))
            .copied()
            .collect();
        let remaining_for_unconstrained = view.remaining_mines as usize;

        // 动态迭代次数
        let iters = self.effective_iterations(unknowns.len());

        // 信息增益计算在大棋盘上代价过高 (O(迭代×未知格×flood_fill))，
        // 仅在中小棋盘时启用以保持实时响应
        let compute_info_gain = unknowns.len() <= 200;

        // 统计每个格子被标雷的次数
        let mut mine_counts: HashMap<Coord, usize> = HashMap::new();
        let mut info_gain_sum: HashMap<Coord, f64> = HashMap::new();
        for c in &unknowns {
            mine_counts.insert(*c, 0);
            info_gain_sum.insert(*c, 0.0);
        }

        let mut rng = rand::thread_rng();
        let mut valid_count = 0usize;

        for _ in 0..iters {
            // 随机尝试分配雷
            if let Some(mine_layout) = Self::random_layout(
                &constraints,
                &constrained_set,
                &unconstrained,
                unknowns.len(),
                remaining_for_unconstrained,
                &mut rng,
            ) {
                valid_count += 1;
                for coord in &mine_layout {
                    *mine_counts.get_mut(coord).unwrap() += 1;
                }
                // 信息增益：模拟点击每个安全格后连锁展开的面积
                if compute_info_gain {
                    for c in &unknowns {
                        if !mine_layout.contains(c) {
                            let gain = Self::estimate_info_gain(c, &state, &mine_layout, view);
                            *info_gain_sum.get_mut(c).unwrap() += gain;
                        }
                    }
                }
            }
        }

        if valid_count == 0 {
            // 没有有效的布局，回退到均匀概率
            let p = if remaining_for_unconstrained > 0 && !unknowns.is_empty() {
                remaining_for_unconstrained as f64 / unknowns.len() as f64
            } else {
                0.0
            };
            return unknowns
                .iter()
                .map(|c| CellProb {
                    coord: *c,
                    mine_probability: p,
                    info_gain: 0.0,
                })
                .collect();
        }

        unknowns
            .iter()
            .map(|c| {
                let mc = *mine_counts.get(c).unwrap_or(&0) as f64;
                let ig = *info_gain_sum.get(c).unwrap_or(&0.0) / valid_count as f64;
                CellProb {
                    coord: *c,
                    mine_probability: mc / valid_count as f64,
                    info_gain: ig,
                }
            })
            .collect()
    }

    /// 同时运行确定性推理和蒙特卡洛模拟
    pub fn simulate_with_deductions(
        &self,
        view: &PlayerView,
        known_mines: &HashSet<Coord>,
        known_safe: &HashSet<Coord>,
    ) -> Vec<CellProb> {
        let state = view.to_state_map();
        let unknowns: Vec<Coord> = view
            .unknown
            .iter()
            .filter(|c| !known_mines.contains(c) && !known_safe.contains(c))
            .copied()
            .collect();
        if unknowns.is_empty() {
            return Vec::new();
        }

        let constraints = Self::build_constraints_with_known(view, &state, known_mines);
        let constrained_set: HashSet<Coord> = constraints
            .iter()
            .flat_map(|(_, cells, _)| cells.iter().copied())
            .collect();
        let unconstrained: Vec<Coord> = unknowns
            .iter()
            .filter(|c| !constrained_set.contains(c))
            .copied()
            .collect();
        let total_mines = view.remaining_mines as usize;
        let known_mine_count = known_mines.len();
        let remaining = total_mines.saturating_sub(known_mine_count);

        // 动态迭代次数
        let iters = self.effective_iterations(unknowns.len());

        let mut mine_counts: HashMap<Coord, usize> = HashMap::new();
        for c in &unknowns {
            mine_counts.insert(*c, 0);
        }

        let mut rng = rand::thread_rng();
        let mut valid_count = 0usize;

        for _ in 0..iters {
            if let Some(mine_layout) = Self::random_layout(
                &constraints,
                &constrained_set,
                &unconstrained,
                unknowns.len(),
                remaining,
                &mut rng,
            ) {
                valid_count += 1;
                for coord in &mine_layout {
                    *mine_counts.get_mut(coord).unwrap_or(&mut 0) += 1;
                }
            }
        }

        if valid_count == 0 {
            let p = if remaining > 0 && !unknowns.is_empty() {
                remaining as f64 / unknowns.len() as f64
            } else {
                0.0
            };
            return unknowns
                .iter()
                .map(|c| CellProb {
                    coord: *c,
                    mine_probability: p,
                    info_gain: 0.0,
                })
                .collect();
        }

        unknowns
            .iter()
            .map(|c| {
                let mc = *mine_counts.get(c).unwrap_or(&0) as f64;
                CellProb {
                    coord: *c,
                    mine_probability: mc / valid_count as f64,
                    info_gain: 0.0,
                }
            })
            .collect()
    }

    /// 构建约束列表: (数字坐标, 未知邻居列表, 剩余雷数)
    fn build_constraints(
        view: &PlayerView,
        state: &HashMap<Coord, CellState>,
    ) -> Vec<(Coord, Vec<Coord>, i32)> {
        view.revealed
            .iter()
            .filter_map(|cell| {
                let neighbors = cell.coord.neighbors(view.width, view.height);
                let unknowns: Vec<Coord> = neighbors
                    .iter()
                    .filter(|n| matches!(state.get(n), Some(CellState::Unknown)))
                    .copied()
                    .collect();
                if unknowns.is_empty() {
                    return None;
                }
                let flagged_count = neighbors
                    .iter()
                    .filter(|n| matches!(state.get(n), Some(CellState::Flagged)))
                    .count() as i32;
                let remaining = cell.number as i32 - flagged_count;
                Some((cell.coord, unknowns, remaining))
            })
            .collect()
    }

    fn build_constraints_with_known(
        view: &PlayerView,
        state: &HashMap<Coord, CellState>,
        known_mines: &HashSet<Coord>,
    ) -> Vec<(Coord, Vec<Coord>, i32)> {
        view.revealed
            .iter()
            .filter_map(|cell| {
                let neighbors = cell.coord.neighbors(view.width, view.height);
                let unknowns: Vec<Coord> = neighbors
                    .iter()
                    .filter(|n| {
                        matches!(state.get(n), Some(CellState::Unknown))
                            && !known_mines.contains(n)
                    })
                    .copied()
                    .collect();
                if unknowns.is_empty() {
                    return None;
                }
                let flagged_count = neighbors
                    .iter()
                    .filter(|n| {
                        matches!(state.get(n), Some(CellState::Flagged))
                            || known_mines.contains(n)
                    })
                    .count() as i32;
                let remaining = cell.number as i32 - flagged_count;
                Some((cell.coord, unknowns, remaining))
            })
            .collect()
    }

    /// 随机生成一个满足约束的雷布局
    /// 返回被标为雷的坐标集合
    fn random_layout<R: Rng>(
        constraints: &[(Coord, Vec<Coord>, i32)],
        constrained_set: &HashSet<Coord>,
        unconstrained: &[Coord],
        _total_unknown: usize,
        remaining_mines: usize,
        rng: &mut R,
    ) -> Option<HashSet<Coord>> {
        let constrained_vec: Vec<Coord> = constrained_set.iter().copied().collect();

        // 尝试有限次来找到满足约束的布局
        for _ in 0..100 {
            let mut layout: HashSet<Coord> = HashSet::new();

            // 对每个约束，随机选 remaining 个格子标雷
            let mut ok = true;
            for (_, cells, rem) in constraints {
                if *rem < 0 || *rem as usize > cells.len() {
                    ok = false;
                    break;
                }
                let mut shuffled = cells.clone();
                shuffled.shuffle(rng);
                for c in shuffled.iter().take(*rem as usize) {
                    layout.insert(*c);
                }
            }

            if !ok {
                continue;
            }

            // 检查约束是否满足
            let mut valid = true;
            for (_, cells, rem) in constraints {
                let count = cells.iter().filter(|c| layout.contains(c)).count() as i32;
                if count != *rem {
                    valid = false;
                    break;
                }
            }

            if !valid {
                continue;
            }

            // 在非约束格中随机分配剩余的雷
            let mines_in_constrained = layout.len();
            let mines_needed = remaining_mines.saturating_sub(mines_in_constrained);

            if mines_needed <= unconstrained.len() {
                let mut shuffled_unc = unconstrained.to_vec();
                shuffled_unc.shuffle(rng);
                for c in shuffled_unc.iter().take(mines_needed) {
                    layout.insert(*c);
                }
            } else {
                continue;
            }

            // 检查总雷数
            if layout.len() == remaining_mines || (remaining_mines == 0 && layout.is_empty()) {
                return Some(layout);
            }
            // 如果约束格中的雷数已经超过总雷数，跳过
            if mines_in_constrained > remaining_mines {
                continue;
            }

            return Some(layout);
        }

        // 回退：简单随机分配
        let mut all_unknown: Vec<Coord> = Vec::new();
        all_unknown.extend(constrained_vec);
        all_unknown.extend_from_slice(unconstrained);
        all_unknown.shuffle(rng);
        let layout: HashSet<Coord> = all_unknown.iter().take(remaining_mines).copied().collect();
        Some(layout)
    }

    /// 估算信息增益：点击某安全格后，连锁展开的格子数
    fn estimate_info_gain(
        coord: &Coord,
        state: &HashMap<Coord, CellState>,
        mine_layout: &HashSet<Coord>,
        view: &PlayerView,
    ) -> f64 {
        // 如果该格是雷，信息增益为0
        if mine_layout.contains(coord) {
            return 0.0;
        }

        // 模拟 flood fill: 如果该格周围雷数为0，连锁展开
        let mut visited: HashSet<Coord> = HashSet::new();
        let mut queue: Vec<Coord> = vec![*coord];
        visited.insert(*coord);

        // 构建模拟数字：每个格子周围的雷数
        while let Some(c) = queue.pop() {
            let neighbors = c.neighbors(view.width, view.height);
            let mine_count = neighbors.iter().filter(|n| mine_layout.contains(n)).count();
            if mine_count == 0 {
                for n in &neighbors {
                    if !visited.contains(n) && !mine_layout.contains(n) {
                        // 只展开未知格
                        if matches!(state.get(n), Some(CellState::Unknown)) {
                            visited.insert(*n);
                            queue.push(*n);
                        }
                    }
                }
            }
        }

        visited.len() as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simulate_basic() {
        // 简单局面: 1 周围有3个未知格, 剩余1雷
        // 3个约束内未知格概率应约1/3, 非约束格概率应为0
        let board = vec![
            vec![1, -1, -1],
            vec![-1, -1, -1],
            vec![-1, -1, -1],
        ];
        let pv = PlayerView::from_2d(&board, 1);
        let mc = MonteCarloEngine::new(2000);
        let probs = mc.simulate(&pv);
        // (1,0), (0,1), (1,1) are constrained by (0,0) → ~1/3
        for p in &probs {
            if p.coord == Coord::new(1, 0) || p.coord == Coord::new(0, 1) || p.coord == Coord::new(1, 1) {
                assert!(p.mine_probability > 0.15 && p.mine_probability < 0.50,
                    "prob {} out of range for {:?}", p.mine_probability, p.coord);
            } else {
                // Unconstrained cells should have ~0 probability since only 1 mine total
                assert!(p.mine_probability < 0.15,
                    "prob {} too high for unconstrained {:?}", p.mine_probability, p.coord);
            }
        }
    }

    #[test]
    fn test_simulate_deterministic_case() {
        // 3 周围有3个未知格, 总3雷 → 100% 是雷
        let board = vec![
            vec![3, -1, -1],
            vec![-1, -1, -1],
        ];
        let pv = PlayerView::from_2d(&board, 3);
        let mc = MonteCarloEngine::new(2000);
        let probs = mc.simulate(&pv);
        // (1,0), (0,1), (1,1) should have ~100% mine probability
        for p in &probs {
            if p.coord == Coord::new(1, 0) || p.coord == Coord::new(0, 1) || p.coord == Coord::new(1, 1) {
                assert!(p.mine_probability > 0.9, "mine prob for {:?} = {}", p.coord, p.mine_probability);
            }
        }
    }

    #[test]
    fn test_simulate_no_mines() {
        let board = vec![
            vec![0, -1, -1],
            vec![-1, -1, -1],
        ];
        let pv = PlayerView::from_2d(&board, 0);
        let mc = MonteCarloEngine::new(100);
        let probs = mc.simulate(&pv);
        for p in &probs {
            assert!((p.mine_probability - 0.0).abs() < 0.1 || p.mine_probability.is_nan());
        }
    }

    #[test]
    fn test_dynamic_iterations_thresholds() {
        assert_eq!(MonteCarloEngine::dynamic_iterations(1), 50_000);
        assert_eq!(MonteCarloEngine::dynamic_iterations(49), 50_000);
        assert_eq!(MonteCarloEngine::dynamic_iterations(50), 10_000);
        assert_eq!(MonteCarloEngine::dynamic_iterations(200), 10_000);
        assert_eq!(MonteCarloEngine::dynamic_iterations(201), 1_000);
        assert_eq!(MonteCarloEngine::dynamic_iterations(900), 1_000);
    }

    #[test]
    fn test_dynamic_mode_default() {
        // 默认 iterations=0 → 动态模式
        let mc = MonteCarloEngine::default();
        assert_eq!(mc.iterations, 0);
        // 小棋盘 → 50 000 次
        let board = vec![vec![1, -1, -1], vec![-1, -1, -1]];
        let pv = PlayerView::from_2d(&board, 1);
        let probs = mc.simulate(&pv);
        assert!(!probs.is_empty());
    }

    #[test]
    fn test_30x30_large_board_fast() {
        // 30x30 大棋盘，未知格 > 200 → 动态 1 000 次
        // 验证排序稳定且耗时合理
        let mut board: Vec<Vec<i32>> = Vec::new();
        for _y in 0..30 {
            let mut row = vec![-1; 30];
            // 在棋盘边缘放几个数字
            row[0] = 1;
            board.push(row);
        }
        let pv = PlayerView::from_2d(&board, 99);
        let mc = MonteCarloEngine::default(); // 动态迭代
        let start = std::time::Instant::now();
        let probs = mc.simulate(&pv);
        let elapsed = start.elapsed();
        assert!(!probs.is_empty());
        // 30x30 with >200 unknowns → 1000 iterations → should be under 5s
        assert!(elapsed.as_secs() < 10, "took too long: {:?}", elapsed);
        // 大棋盘不计算信息增益 → info_gain 应为 0
        for p in &probs {
            assert!((p.info_gain - 0.0).abs() < 0.01, "info_gain should be 0 for large board, got {}", p.info_gain);
        }
    }
}
