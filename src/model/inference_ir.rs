use serde::{Deserialize, Serialize};
use super::Coord;

/// 结论：某格子是雷还是安全
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conclusion {
    pub coord: Coord,
    pub is_mine: bool,
}

/// 推理证明链
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proof {
    /// 结论
    pub conclusion: Conclusion,
    /// 推理依赖的已翻开数字坐标
    pub depends_on: Vec<Coord>,
    /// 规则简述
    pub rule: String,
}

/// 单个格子的概率信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CellProb {
    pub coord: Coord,
    /// 雷概率 0.0 ~ 1.0
    pub mine_probability: f64,
    /// 期望信息增益 (点击后连锁展开的面积期望)
    pub info_gain: f64,
}

/// 连通区域特征 (用于策略比较)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegionFeature {
    /// 区域内未知格子数
    pub unknown_count: usize,
    /// 区域内估计总雷数
    pub estimated_mines: f64,
    /// 区域边界长度 (与已翻开数字相邻的格子数)
    pub boundary_length: usize,
    /// 区域内平均雷概率
    pub avg_mine_prob: f64,
    /// 区域内最低雷概率
    pub min_mine_prob: f64,
}

/// 连通区域
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Region {
    /// 区域包含的未知格子坐标
    pub cells: Vec<Coord>,
    /// 区域特征
    pub features: RegionFeature,
}

/// 推理引擎输出中间语言 (IR)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceIR {
    /// 确定性推理证明链
    pub deterministic: Vec<Proof>,
    /// 概率模拟结果
    pub probabilities: Vec<CellProb>,
    /// 连通区域划分 (用于策略比较)
    pub regions: Vec<Region>,
}

impl InferenceIR {
    /// 从证明链中提取所有必雷坐标
    pub fn all_mines(&self) -> Vec<Coord> {
        self.deterministic
            .iter()
            .filter(|p| p.conclusion.is_mine)
            .map(|p| p.conclusion.coord)
            .collect()
    }

    /// 从证明链中提取所有必安全坐标
    pub fn all_safe(&self) -> Vec<Coord> {
        self.deterministic
            .iter()
            .filter(|p| !p.conclusion.is_mine)
            .map(|p| p.conclusion.coord)
            .collect()
    }

    /// 查找某坐标的概率信息
    pub fn prob_of(&self, coord: &Coord) -> Option<&CellProb> {
        self.probabilities.iter().find(|p| &p.coord == coord)
    }
}
