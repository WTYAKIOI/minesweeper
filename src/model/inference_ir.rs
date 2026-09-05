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

/// 旗帜验证状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FlagVerifyStatus {
    /// 推理确认该旗正确 (该格必为雷)
    Verified,
    /// 与数字约束无矛盾, 但推理无法进一步确认
    Suspected,
    /// 与数字约束矛盾 (必为误标), 或推理证明该格必安全
    Contradicted,
}

/// 单面旗帜的判定结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlagStatus {
    pub coord: Coord,
    pub status: FlagVerifyStatus,
    /// 判定理由 (中文, 含具体数字坐标与旗数)
    pub reason: String,
}

/// 旗帜正确性判定结果 (在推理之前执行, 随 IR 返回)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlagVerificationResult {
    /// 全部旗帜的状态列表
    pub flags: Vec<FlagStatus>,
    /// 中文摘要, 如 "已检查 83 面旗, 发现 2 面可能标错, 1 面确认正确"
    pub summary: String,
    /// 是否存在矛盾旗 (前端快捷判断, 免去遍历)
    pub has_contradiction: bool,
}

impl Default for FlagVerificationResult {
    fn default() -> Self {
        Self::empty()
    }
}

impl FlagVerificationResult {
    /// 空结果 (无旗帜 / 未执行验证时使用)
    pub fn empty() -> Self {
        Self {
            flags: Vec::new(),
            summary: "棋盘上没有旗帜, 无需验证".to_string(),
            has_contradiction: false,
        }
    }

    /// 矛盾旗坐标列表 (方便 UI 高亮与一键移除)
    pub fn contradicted_coords(&self) -> Vec<Coord> {
        self.flags
            .iter()
            .filter(|f| f.status == FlagVerifyStatus::Contradicted)
            .map(|f| f.coord)
            .collect()
    }
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
    /// 旗帜正确性判定 (推理前执行, 判定结果随 IR 返回前端)
    #[serde(default)]
    pub flag_verification: FlagVerificationResult,
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
