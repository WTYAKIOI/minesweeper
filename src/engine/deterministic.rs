use std::collections::HashSet;
use crate::model::{PlayerView, Coord, Proof, Conclusion, CellState};

/// 确定性约束传播引擎
///
/// 类似数独的约束传播：
/// - 若数字周围未知格数 == 剩余雷数 → 全部标雷
/// - 若数字周围剩余雷数 == 0 → 全部标安全
///
/// 循环推理直到不再有新结论
pub struct DeterministicEngine;

impl DeterministicEngine {
    /// 执行约束传播，返回推理证明链
    pub fn solve(view: &PlayerView) -> Vec<Proof> {
        let state = view.to_state_map();
        let mut proofs: Vec<Proof> = Vec::new();
        let mut known_mines: HashSet<Coord> = view.flagged.iter().copied().collect();
        let mut known_safe: HashSet<Coord> = HashSet::new();
        let mut changed = true;

        while changed {
            changed = false;

            for cell in &view.revealed {
                let neighbors = cell.coord.neighbors(view.width, view.height);

                // 统计当前已知雷数和未知格
                let mut unknown_neighbors: Vec<Coord> = Vec::new();
                let mut flagged_count = 0u32;
                for n in &neighbors {
                    match state.get(n) {
                        Some(CellState::Flagged) => flagged_count += 1,
                        Some(CellState::Unknown)
                            if !known_mines.contains(n) && !known_safe.contains(n) =>
                        {
                            unknown_neighbors.push(*n);
                        }
                        _ => {}
                    }
                }
                // 也包含已被本轮推理标为雷的格子
                let mines_from_proof: u32 = neighbors
                    .iter()
                    .filter(|n| known_mines.contains(n) && !matches!(state.get(n), Some(CellState::Flagged)))
                    .count() as u32;

                let total_known_mines = flagged_count + mines_from_proof;
                let remaining = cell.number as i32 - total_known_mines as i32;

                // 规则 1: 剩余雷数 == 未知格数 → 全部是雷
                if remaining > 0 && remaining as usize == unknown_neighbors.len() {
                    for n in &unknown_neighbors {
                        if !known_mines.contains(n) {
                            known_mines.insert(*n);
                            proofs.push(Proof {
                                conclusion: Conclusion { coord: *n, is_mine: true },
                                depends_on: vec![cell.coord],
                                rule: format!(
                                    "数字{}周围剩余{}个未知格，还需{}雷 → 全部为雷",
                                    cell.number, unknown_neighbors.len(), remaining
                                ),
                            });
                            changed = true;
                        }
                    }
                }

                // 规则 2: 剩余雷数 == 0 → 全部安全
                if remaining == 0 && !unknown_neighbors.is_empty() {
                    for n in &unknown_neighbors {
                        if !known_safe.contains(n) && !known_mines.contains(n) {
                            known_safe.insert(*n);
                            proofs.push(Proof {
                                conclusion: Conclusion { coord: *n, is_mine: false },
                                depends_on: vec![cell.coord],
                                rule: format!(
                                    "数字{}周围剩余雷数为0 → 该格安全",
                                    cell.number
                                ),
                            });
                            changed = true;
                        }
                    }
                }
            }
        }

        proofs
    }

    /// 子集推理 (高级规则)：
    /// 如果数字 A 的未知邻居集合是数字 B 的未知邻居集合的子集，
    /// 且 A 剩余雷数 == B 剩余雷数 → B 中不在 A 集合里的格子全部安全
    /// 且 A 剩余雷数 < B 剩余雷数，差值 == B\A 集合大小 → B\A 全部为雷
    pub fn solve_with_subset_rule(view: &PlayerView) -> Vec<Proof> {
        let mut proofs = Self::solve(view);

        let state = view.to_state_map();
        let mut known_mines: HashSet<Coord> = proofs
            .iter()
            .filter(|p| p.conclusion.is_mine)
            .map(|p| p.conclusion.coord)
            .collect::<HashSet<_>>()
            .union(&view.flagged.iter().copied().collect())
            .copied()
            .collect();
        let mut known_safe: HashSet<Coord> = proofs
            .iter()
            .filter(|p| !p.conclusion.is_mine)
            .map(|p| p.conclusion.coord)
            .collect();

        // 构建每个数字格子的约束信息
        let constraints: Vec<(Coord, Vec<Coord>, i32)> = view
            .revealed
            .iter()
            .map(|cell| {
                let neighbors = cell.coord.neighbors(view.width, view.height);
                let unknowns: Vec<Coord> = neighbors
                    .iter()
                    .filter(|n| {
                        matches!(state.get(n), Some(CellState::Unknown))
                            && !known_mines.contains(n)
                            && !known_safe.contains(n)
                    })
                    .copied()
                    .collect();
                let flagged_count = neighbors
                    .iter()
                    .filter(|n| matches!(state.get(n), Some(CellState::Flagged)))
                    .count() as i32;
                let proof_mines = neighbors
                    .iter()
                    .filter(|n| known_mines.contains(n) && !matches!(state.get(n), Some(CellState::Flagged)))
                    .count() as i32;
                let remaining = cell.number as i32 - flagged_count - proof_mines;
                (cell.coord, unknowns, remaining)
            })
            .filter(|(_, unknowns, _)| !unknowns.is_empty())
            .collect();

        // 比较所有约束对
        for i in 0..constraints.len() {
            for j in 0..constraints.len() {
                if i == j {
                    continue;
                }
                let (coord_a, unknowns_a, rem_a) = &constraints[i];
                let (coord_b, unknowns_b, rem_b) = &constraints[j];

                let set_a: HashSet<Coord> = unknowns_a.iter().copied().collect();
                let set_b: HashSet<Coord> = unknowns_b.iter().copied().collect();

                // A 是 B 的子集
                if set_a.is_subset(&set_b) && set_a.len() < set_b.len() {
                    let diff: Vec<Coord> = set_b.difference(&set_a).copied().collect();
                    let rem_diff = rem_b - rem_a;

                    if rem_diff == 0 && !diff.is_empty() {
                        for c in &diff {
                            if !known_safe.contains(c) && !known_mines.contains(c) {
                                known_safe.insert(*c);
                                proofs.push(Proof {
                                    conclusion: Conclusion { coord: *c, is_mine: false },
                                    depends_on: vec![*coord_a, *coord_b],
                                    rule: format!(
                                        "子集推理: 数字在{}的未知格是{}的子集，剩余雷差为0 → 该格安全",
                                        coord_b, coord_a
                                    ),
                                });
                            }
                        }
                    } else if rem_diff == diff.len() as i32 && rem_diff > 0 {
                        for c in &diff {
                            if !known_mines.contains(c) && !known_safe.contains(c) {
                                known_mines.insert(*c);
                                proofs.push(Proof {
                                    conclusion: Conclusion { coord: *c, is_mine: true },
                                    depends_on: vec![*coord_a, *coord_b],
                                    rule: format!(
                                        "子集推理: 数字在{}的未知格是{}的子集，剩余雷差={}等于差集大小 → 该格为雷",
                                        coord_b, coord_a, rem_diff
                                    ),
                                });
                            }
                        }
                    }
                }
            }
        }

        // 再次运行约束传播以利用子集推理的新结论
        let mut new_view = view.clone();
        for proof in &proofs {
            if proof.conclusion.is_mine {
                new_view.flagged.push(proof.conclusion.coord);
                new_view.unknown.retain(|c| c != &proof.conclusion.coord);
            } else {
                new_view.unknown.retain(|c| c != &proof.conclusion.coord);
            }
        }
        if new_view.validate() {
            let extra = Self::solve(&new_view);
            // 只添加之前没有的结论
            let existing: HashSet<Coord> = proofs.iter().map(|p| p.conclusion.coord).collect();
            for p in extra {
                if !existing.contains(&p.conclusion.coord) {
                    proofs.push(p);
                }
            }
        }

        proofs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_constraint_all_mines() {
        // 3 的周围有 3 个未知格，没有标旗 → 3个都是雷
        let board = vec![
            vec![3, -1, -1],
            vec![-1, -1, -1],
            vec![0, -1, -1],
        ];
        let pv = PlayerView::from_2d(&board, 10);
        let proofs = DeterministicEngine::solve(&pv);
        let mines: Vec<Coord> = proofs.iter()
            .filter(|p| p.conclusion.is_mine)
            .map(|p| p.conclusion.coord)
            .collect();
        // (1,0), (0,1), (1,1) should be mines
        assert!(mines.contains(&Coord::new(1, 0)));
        assert!(mines.contains(&Coord::new(0, 1)));
        assert!(mines.contains(&Coord::new(1, 1)));
    }

    #[test]
    fn test_basic_constraint_all_safe() {
        // 1 的周围已标1旗 → 其余未知格安全
        // Board: 1  F  ?
        //        ?  ?  ?
        let board = vec![
            vec![1, -2, -1],
            vec![-1, -1, -1],
        ];
        let pv = PlayerView::from_2d(&board, 10);
        let proofs = DeterministicEngine::solve(&pv);
        let safe: Vec<Coord> = proofs.iter()
            .filter(|p| !p.conclusion.is_mine)
            .map(|p| p.conclusion.coord)
            .collect();
        // (0,0) neighbors: (1,0)=flagged, (0,1)=unknown, (1,1)=unknown
        // remaining = 1-1 = 0 → (0,1) and (1,1) are safe
        assert!(safe.contains(&Coord::new(0, 1)));
        assert!(safe.contains(&Coord::new(1, 1)));
        // (2,0) is NOT a neighbor of (0,0), should not be in safe
        assert!(!safe.contains(&Coord::new(2, 0)));
    }

    #[test]
    fn test_subset_rule() {
        // 1 at (0,0): unknowns = {(1,1),(0,1)}, remaining = 1
        // 2 at (1,0): unknowns = {(1,1),(0,1),(2,1)}, remaining = 2
        // (0,0)'s set ⊂ (1,0)'s set → diff = {(2,1)}, rem_diff = 1 = diff size → (2,1) is mine
        // 1 at (2,0): unknowns = {(1,1),(2,1)}, remaining = 1
        // (2,0)'s set ⊂ (1,0)'s set → diff = {(0,1)}, rem_diff = 1 = diff size → (0,1) is mine
        // Then (0,0): both unknowns are mines (from above), remaining=1 → contradiction? No:
        // After finding (0,1) and (2,1) as mines, (0,0) has 1 mine in its unknowns → (1,1) is safe
        let board = vec![
            vec![1, 2, 1],
            vec![-1, -1, -1],
        ];
        let pv = PlayerView::from_2d(&board, 3);
        let proofs = DeterministicEngine::solve_with_subset_rule(&pv);
        let mines: Vec<Coord> = proofs.iter()
            .filter(|p| p.conclusion.is_mine)
            .map(|p| p.conclusion.coord)
            .collect();
        let safe: Vec<Coord> = proofs.iter()
            .filter(|p| !p.conclusion.is_mine)
            .map(|p| p.conclusion.coord)
            .collect();
        // (0,1) and (2,1) should be mines
        assert!(mines.contains(&Coord::new(0, 1)));
        assert!(mines.contains(&Coord::new(2, 1)));
        // (1,1) should be safe
        assert!(safe.contains(&Coord::new(1, 1)));
    }

    #[test]
    fn test_no_conclusions() {
        // 1 at (0,0) with 3 unknowns, 1 remaining → no deterministic conclusion
        let board = vec![
            vec![1, -1, -1],
            vec![-1, -1, -1],
            vec![-1, -1, -1],
        ];
        let pv = PlayerView::from_2d(&board, 10);
        let proofs = DeterministicEngine::solve(&pv);
        assert!(proofs.is_empty());
    }
}
