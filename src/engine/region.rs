use std::collections::{HashMap, HashSet};
use crate::model::{PlayerView, Coord, Region, RegionFeature, CellState};

/// 并查集
struct UnionFind {
    parent: HashMap<Coord, Coord>,
    size: HashMap<Coord, usize>,
}

impl UnionFind {
    fn new() -> Self {
        Self {
            parent: HashMap::new(),
            size: HashMap::new(),
        }
    }

    fn add(&mut self, c: Coord) {
        self.parent.insert(c, c);
        self.size.insert(c, 1);
    }

    fn find(&mut self, c: Coord) -> Coord {
        let mut root = c;
        while let Some(&p) = self.parent.get(&root) {
            if p == root {
                break;
            }
            root = p;
        }
        // Path compression
        let mut current = c;
        while let Some(&p) = self.parent.get(&current) {
            if p == root {
                break;
            }
            self.parent.insert(current, root);
            current = p;
        }
        root
    }

    fn union(&mut self, a: Coord, b: Coord) {
        let root_a = self.find(a);
        let root_b = self.find(b);
        if root_a == root_b {
            return;
        }
        let size_a = *self.size.get(&root_a).unwrap_or(&0);
        let size_b = *self.size.get(&root_b).unwrap_or(&0);
        if size_a >= size_b {
            self.parent.insert(root_b, root_a);
            self.size.insert(root_a, size_a + size_b);
        } else {
            self.parent.insert(root_a, root_b);
            self.size.insert(root_b, size_a + size_b);
        }
    }
}

/// 连通区域分析器
///
/// 使用并查集将相邻的未知格子分组为连通区域，
/// 计算每个区域的特征 (未知格数、估计雷数、边界长度、平均雷概率)
pub struct RegionAnalyzer;

impl RegionAnalyzer {
    /// 分析连通区域
    pub fn analyze(
        view: &PlayerView,
        probabilities: &[crate::model::CellProb],
    ) -> Vec<Region> {
        let state = view.to_state_map();
        let unknown_set: HashSet<Coord> = view.unknown.iter().copied().collect();
        let prob_map: HashMap<Coord, f64> = probabilities
            .iter()
            .map(|p| (p.coord, p.mine_probability))
            .collect();

        // 初始化并查集
        let mut uf = UnionFind::new();
        for c in &view.unknown {
            uf.add(*c);
        }

        // 合并相邻的未知格
        for c in &view.unknown {
            let neighbors = c.neighbors(view.width, view.height);
            for n in &neighbors {
                if unknown_set.contains(n) {
                    uf.union(*c, *n);
                }
            }
        }

        // 收集每个区域的格子
        let mut groups: HashMap<Coord, Vec<Coord>> = HashMap::new();
        for c in &view.unknown {
            let root = uf.find(*c);
            groups.entry(root).or_default().push(*c);
        }

        // 构建区域信息
        groups
            .into_values()
            .map(|cells| {
                let unknown_count = cells.len();
                let estimated_mines: f64 = cells
                    .iter()
                    .filter_map(|c| prob_map.get(c))
                    .sum::<f64>()
                    .abs(); // avoid -0.0
                let avg_mine_prob = if unknown_count > 0 {
                    (estimated_mines / unknown_count as f64).abs()
                } else {
                    0.0
                };
                let min_mine_prob = cells
                    .iter()
                    .filter_map(|c| prob_map.get(c))
                    .cloned()
                    .fold(f64::MAX, f64::min);
                let min_mine_prob = if min_mine_prob == f64::MAX { 0.0 } else { min_mine_prob };

                // 边界长度: 与已翻开数字相邻的未知格子数
                let boundary_length = cells
                    .iter()
                    .filter(|c| {
                        c.neighbors(view.width, view.height)
                            .iter()
                            .any(|n| matches!(state.get(n), Some(CellState::Revealed(_))))
                    })
                    .count();

                Region {
                    cells,
                    features: RegionFeature {
                        unknown_count,
                        estimated_mines,
                        boundary_length,
                        avg_mine_prob,
                        min_mine_prob,
                    },
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::CellProb;

    #[test]
    fn test_region_analysis_single() {
        let board = vec![
            vec![1, -1, -1],
            vec![-1, -1, -1],
            vec![1, -1, -1],
        ];
        let pv = PlayerView::from_2d(&board, 10);
        let probs = vec![
            CellProb { coord: Coord::new(1, 0), mine_probability: 0.3, info_gain: 0.0 },
            CellProb { coord: Coord::new(2, 0), mine_probability: 0.2, info_gain: 0.0 },
            CellProb { coord: Coord::new(0, 1), mine_probability: 0.3, info_gain: 0.0 },
            CellProb { coord: Coord::new(1, 1), mine_probability: 0.3, info_gain: 0.0 },
            CellProb { coord: Coord::new(2, 1), mine_probability: 0.2, info_gain: 0.0 },
        ];
        let regions = RegionAnalyzer::analyze(&pv, &probs);
        // All unknowns are connected, so 1 region
        assert_eq!(regions.len(), 1);
        // 7 unknown cells: (1,0),(2,0),(0,1),(1,1),(2,1),(1,2),(2,2)
        assert_eq!(regions[0].features.unknown_count, 7);
    }

    #[test]
    fn test_region_analysis_multiple() {
        let board = vec![
            vec![1, -1, 0, -1, 1],
            vec![-1, -1, 0, -1, -1],
        ];
        let pv = PlayerView::from_2d(&board, 10);
        // The zeros split the unknowns into separate groups
        let regions = RegionAnalyzer::analyze(&pv, &[]);
        // (0,1),(1,0),(1,1) connected; (3,0),(3,1),(4,0),(4,1) connected → 2 regions
        assert_eq!(regions.len(), 2);
    }
}
