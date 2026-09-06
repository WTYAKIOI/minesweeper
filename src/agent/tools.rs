//! Agent 工具层 — 模型只"提出调用请求", Rust 校验参数、执行、返回观察结果。
//!
//! 架构原则 (improve.md §1/§3.1):
//! - 模型不直接访问任何文件/API/数据; 只能请求 `{"action":"tool","name":...,"args":...}`。
//! - Rust 是唯一执行者: 校验参数合法性 → 在"工作棋盘"上执行 → 把观察结果写回对话。
//! - 诚实性: 本系统只有玩家视角 (无真实雷藏), 因此不做"模拟点击翻开"等造假工具;
//!   工具只执行可被证明的操作 (给引擎证明的必雷格标旗) 与查询 (约束/旗验证/概率/知识库)。

use serde_json::{json, Value};

use crate::engine::{board_summary, run_local_pipeline};
use crate::model::{CellState, Coord, InferenceIR, PlayerView};

/// Agent 持有的工作棋盘 (每次成功操作都会更新, 驱动下一轮决策)
#[derive(Debug, Clone)]
pub struct AgentBoard {
    pub board: Vec<Vec<i32>>,
    pub remaining_mines: u32,
}

impl AgentBoard {
    pub fn new(board: Vec<Vec<i32>>, remaining_mines: u32) -> Self {
        Self { board, remaining_mines }
    }
}

// ---------------------------------------------------------------------------
// 工具清单 (暴露给 LLM 的 JSON Schema)
// ---------------------------------------------------------------------------

/// 供 LLM 提示词使用的工具定义数组 (name / description / parameters)
pub fn tool_definitions() -> Value {
    json!([
        {
            "name": "analyze_board",
            "description": "对当前棋盘执行完整推理 (确定性+概率+区域+旗验证), 返回关键结论摘要",
            "parameters": {}
        },
        {
            "name": "flag_cell",
            "description": "在 (x,y) 处标旗。仅当引擎证明该格必为雷且尚未标旗时可执行; 执行后棋盘更新、剩余雷数减 1",
            "parameters": {
                "x": {"type": "integer", "description": "列坐标 x"},
                "y": {"type": "integer", "description": "行坐标 y"}
            }
        },
        {
            "name": "get_cell_info",
            "description": "获取某格当前状态 (已翻开数字/已标旗/未翻开) 及引擎结论 (必雷/必安全/概率)",
            "parameters": {
                "x": {"type": "integer", "description": "列坐标 x"},
                "y": {"type": "integer", "description": "行坐标 y"}
            }
        },
        {
            "name": "get_probability",
            "description": "获取某格的雷概率 (无确定性结论时用于策略判断)",
            "parameters": {
                "x": {"type": "integer", "description": "列坐标 x"},
                "y": {"type": "integer", "description": "行坐标 y"}
            }
        },
        {
            "name": "get_local_board",
            "description": "获取以 (x,y) 为中心、半径 radius(1-4, 默认2) 的局部棋盘 (?:未翻开 F:旗 数字:已翻开)",
            "parameters": {
                "x": {"type": "integer", "description": "列坐标 x"},
                "y": {"type": "integer", "description": "行坐标 y"},
                "radius": {"type": "integer", "description": "可选半径, 1-4, 默认2"}
            }
        },
        {
            "name": "check_safe",
            "description": "查询 (x,y) 是否是引擎证明的安全格(可建议点击)或必雷格, 或只有概率; 返回证据/概率",
            "parameters": {
                "x": {"type": "integer", "description": "列坐标 x"},
                "y": {"type": "integer", "description": "行坐标 y"}
            }
        },
        {
            "name": "explain_cell",
            "description": "获取 (x,y) 的完整可验证推理素材: 引擎判定 + 相关数字的局部约束表 (数字=剩余需雷数/候选格列表)。在最终回答需要给出某格的\"是雷/不是雷\"推理链前, 先调用本工具取得素材; 每一步推导都必须与本表数字一致",
            "parameters": {
                "x": {"type": "integer", "description": "列坐标 x"},
                "y": {"type": "integer", "description": "行坐标 y"}
            }
        },
        {
            "name": "verify_flag",
            "description": "验证 (x,y) 的旗帜是否正确 (确认/矛盾/未确认), 矛盾旗实际是安全格",
            "parameters": {
                "x": {"type": "integer", "description": "列坐标 x"},
                "y": {"type": "integer", "description": "行坐标 y"}
            }
        },
        {
            "name": "get_constraints",
            "description": "查看 (x,y) 及其周围数字的约束详情 (数字=旗数/剩余需雷数/未知邻居)",
            "parameters": {
                "x": {"type": "integer", "description": "列坐标 x"},
                "y": {"type": "integer", "description": "行坐标 y"}
            }
        },
        {
            "name": "list_regions",
            "description": "列出当前棋盘的分区 (每区域未知格数/估计雷数/边界/平均雷概率)",
            "parameters": {}
        },
        {
            "name": "get_knowledge",
            "description": "查询扫雷知识库 (模式库: 1-2-1 / 1-2-2-1 / 子集 / 组合枚举 / 剩余雷数规则 等)。参数 topic 可省略(返回全部)",
            "parameters": {
                "topic": {"type": "string", "description": "可选: 模式名或主题, 如 \"121\" / \"subset\" / \"mines\"", "optional": true}
            }
        }
    ])
}

// ---------------------------------------------------------------------------
// 知识库 — "知道什么": 领域模式库, 供模型引用 (不会执行, 只返回资料)
// ---------------------------------------------------------------------------

pub const KNOWLEDGE_BASE: &[(&str, &str, &str)] = &[
    (
        "剩余雷数规则",
        "mines/剩余",
        "当剩余雷数 = 某连通区域内未知格数时, 该区域所有未知格都是雷; 当剩余雷数 = 0 时所有未知格安全。",
    ),
    (
        "1-2-1",
        "121",
        "一行/一列中连续三个数字 1-2-1 且只有一侧是未知格时: 两个 1 旁的未知格都是雷, 中间的 2 下方的格安全。",
    ),
    (
        "1-2-2-1",
        "1221",
        "四个连续数字 1-2-2-1: 两端的 1 旁各 1 雷, 中间两个 2 之间的格安全。",
    ),
    (
        "单约束推导",
        "single",
        "数字剩余需雷数(=数字-周围已标旗数)为 0 → 其所有未知邻居安全; 剩余需雷数=未知邻居数 → 其未知邻居全为雷。",
    ),
    (
        "子集推理",
        "subset/子集",
        "若数字 A 的未知邻居集合 ⊆ 数字 B 的未知邻居集合, 且 B 剩余需雷 = A 剩余需雷 + k, 则 B 的专属未知格中恰有 k 颗雷; k=0 时专属格全安全。",
    ),
    (
        "组合枚举",
        "enumerate/枚举/121",
        "对局部约束联合枚举所有可行布雷方案: 若某格在所有方案中恒为雷/恒安全, 即确定性结论 (可用于判断矛盾旗: 若某旗在所有可行方案中都必须是安全格, 旗帜必为误标)。",
    ),
    (
        "待定格策略",
        "guess/概率",
        "无确定性结论时比较雷概率与信息增益: 优先点击雷概率低且信息增益高的格; 两个相邻未知格互为镜像(50/50)时没有确定性, 只能猜。",
    ),
];

/// 知识库查询: topic 为空返回全部; 否则匹配名称/标签/说明中的关键词
pub fn query_knowledge(topic: &str) -> String {
    let t = topic.trim().to_lowercase();
    let hits: Vec<&(&str, &str, &str)> = if t.is_empty() {
        KNOWLEDGE_BASE.iter().collect()
    } else {
        KNOWLEDGE_BASE
            .iter()
            .filter(|(name, tags, desc)| {
                name.to_lowercase().contains(&t)
                    || tags.to_lowercase().contains(&t)
                    || desc.to_lowercase().contains(&t)
            })
            .collect()
    };
    if hits.is_empty() {
        return format!("知识库中未找到 \"{}\"。可用主题: 剩余雷数/121/1221/single/subset/enumerate/guess", topic);
    }
    hits.iter()
        .map(|(name, _, desc)| format!("## {}\n{}", name, desc))
        .collect::<Vec<_>>()
        .join("\n\n")
}

// ---------------------------------------------------------------------------
// 工具执行器
// ---------------------------------------------------------------------------

/// 校验并执行工具调用; 成功/失败都返回观察文本 (失败以 "错误:" 开头, 模型据此修正)。
pub fn execute_tool(
    state: &mut AgentBoard,
    name: &str,
    args: &Value,
) -> Result<String, String> {
    match name {
        "analyze_board" => {
            let (view, ir) = run_local_pipeline(&state.board, state.remaining_mines)?;
            Ok(analyze_observation(&view, &ir))
        }
        "flag_cell" => {
            let (x, y) = coord_arg(args)?;
            flag_cell(state, x, y)
        }
        "check_safe" => {
            let (x, y) = coord_arg(args)?;
            check_safe(state, x, y)
        }
        "get_cell_info" => {
            let (x, y) = coord_arg(args)?;
            check_safe(state, x, y)
        }
        "get_probability" => {
            let (x, y) = coord_arg(args)?;
            probability_at(state, x, y)
        }
        "get_local_board" => {
            let (x, y) = coord_arg(args)?;
            let radius = args
                .get("radius")
                .and_then(|v| v.as_u64())
                .unwrap_or(2)
                .clamp(1, 4) as usize;
            local_board(state, x, y, radius)
        }
        "explain_cell" => {
            let (x, y) = coord_arg(args)?;
            explain_cell(state, x, y)
        }
        "verify_flag" => {
            let (x, y) = coord_arg(args)?;
            verify_flag(state, x, y)
        }
        "get_constraints" => {
            let (x, y) = coord_arg(args)?;
            constraints_at(state, x, y)
        }
        "list_regions" => {
            let (view, ir) = run_local_pipeline(&state.board, state.remaining_mines)?;
            Ok(regions_observation(&view, &ir))
        }
        "get_knowledge" => {
            let topic = args
                .get("topic")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Ok(query_knowledge(&topic))
        }
        other => Err(format!("未知工具: {} (可用: analyze_board / flag_cell / check_safe / verify_flag / get_constraints / list_regions / get_knowledge)", other)),
    }
}

fn coord_arg(args: &Value) -> Result<(usize, usize), String> {
    let x = args
        .get("x")
        .and_then(|v| v.as_u64())
        .ok_or("参数缺少整数 x")? as usize;
    let y = args
        .get("y")
        .and_then(|v| v.as_u64())
        .ok_or("参数缺少整数 y")? as usize;
    Ok((x, y))
}

/// 标旗: 只允许对"引擎证明必雷且未标旗"的格执行 (Rust 校验, 防止模型乱标)
fn flag_cell(state: &mut AgentBoard, x: usize, y: usize) -> Result<String, String> {
    let h = state.board.len();
    let w = if h > 0 { state.board[0].len() } else { 0 };
    if y >= h || x >= w {
        return Err(format!("坐标越界: ({}, {}) 超出棋盘 {}x{}", x, y, w, h));
    }
    match state.board[y][x] {
        -2 => return Err(format!("({}, {}) 已经标旗, 不要重复标旗", x, y)),
        v if (0..=8).contains(&v) => {
            return Err(format!("({}, {}) 已翻开(数字{}), 不能标旗", x, y, v))
        }
        _ => {}
    }
    // 引擎必须证明该格必雷
    let (_view, ir) = run_local_pipeline(&state.board, state.remaining_mines)?;
    let coord = crate::model::Coord::new(x as u32, y as u32);
    let proof = ir
        .deterministic
        .iter()
        .find(|p| p.conclusion.coord == coord && p.conclusion.is_mine);
    let Some(proof) = proof else {
        let prob = ir
            .prob_of(&coord)
            .map(|p| format!(" (该格雷概率 {:.1}%)", p.mine_probability * 100.0))
            .unwrap_or_default();
        return Err(format!(
            "引擎未证明 ({}, {}) 必为雷{}; 禁止无确定性证据时标旗",
            x, y, prob
        ));
    };
    if state.remaining_mines == 0 {
        return Err(format!(
            "剩余雷数已为 0, ({}, {}) 不可能是雷 (引擎结论 {} 与剩余雷数矛盾?)",
            x, y, proof.rule
        ));
    }
    state.board[y][x] = -2;
    state.remaining_mines -= 1;
    let (view2, ir2) = run_local_pipeline(&state.board, state.remaining_mines)?;
    Ok(format!(
        "已在 ({}, {}) 标旗 (依据: {}); 剩余雷数 {} → {}. 当前: {}",
        x,
        y,
        proof.rule,
        state.remaining_mines + 1,
        state.remaining_mines,
        board_summary(&view2, &ir2)
    ))
}

/// 查询某格是必雷/必安全/概率
fn check_safe(state: &AgentBoard, x: usize, y: usize) -> Result<String, String> {
    let (view, ir) = run_local_pipeline(&state.board, state.remaining_mines)?;
    let coord = crate::model::Coord::new(x as u32, y as u32);
    if y as u32 >= view.height || x as u32 >= view.width {
        return Err(format!("坐标越界: ({}, {})", x, y));
    }
    match view.to_state_map().get(&coord) {
        Some(CellState::Revealed(n)) => Ok(format!("({}, {}) 已翻开, 数字 {}", x, y, n)),
        Some(CellState::Flagged) => {
            let status = ir
                .flag_verification
                .flags
                .iter()
                .find(|f| f.coord == coord)
                .map(|f| format!("{:?}: {}", f.status, f.reason))
                .unwrap_or_else(|| "该旗状态未知".into());
            Ok(format!("({}, {}) 已标旗 ({})", x, y, status))
        }
        Some(CellState::Unknown) | None => {
            if let Some(p) = ir.deterministic.iter().find(|p| p.conclusion.coord == coord) {
                return Ok(format!(
                    "({}, {}): {} — {} (依据数字: {})",
                    x,
                    y,
                    if p.conclusion.is_mine { "必雷" } else { "必安全" },
                    p.rule,
                    p.depends_on.iter().map(|c| format!("({},{})", c.x, c.y)).collect::<Vec<_>>().join(", ")
                ));
            }
            let prob = ir
                .prob_of(&coord)
                .map(|p| format!("{:.1}%", p.mine_probability * 100.0))
                .unwrap_or_else(|| "未知".into());
            Ok(format!("({}, {}): 无确定性结论, 雷概率 {} (建议优先点击低概率高增益格)", x, y, prob))
        }
    }
}

/// 旗验证
fn verify_flag(state: &AgentBoard, x: usize, y: usize) -> Result<String, String> {
    let (view, ir) = run_local_pipeline(&state.board, state.remaining_mines)?;
    let coord = crate::model::Coord::new(x as u32, y as u32);
    if y as u32 >= view.height || x as u32 >= view.width {
        return Err(format!("坐标越界: ({}, {})", x, y));
    }
    if !matches!(view.to_state_map().get(&coord), Some(CellState::Flagged)) {
        return Err(format!("({}, {}) 不是旗帜格, 无需验证", x, y));
    }
    let f = ir
        .flag_verification
        .flags
        .iter()
        .find(|f| f.coord == coord)
        .ok_or_else(|| format!("({}, {}) 旗验证数据缺失", x, y))?;
    let verdict = match f.status {
        crate::model::FlagVerifyStatus::Verified => "该旗确认正确 (该格必为雷)",
        crate::model::FlagVerifyStatus::Contradicted => "该旗与数字约束矛盾 — 该格实际必安全, 旗帜是误标",
        crate::model::FlagVerifyStatus::Suspected => "无法确定该旗是否正确 (只能概率讨论)",
    };
    Ok(format!("({}, {}) 旗验证: {} ({})", x, y, verdict, f.reason))
}

/// 完整可验证推理素材: 目标格判定 + 相关数字局部约束表 + 邻近已知证明。
///
/// 设计动机 (AGENTS.md §3): 只给结论 LLM 只能写"联动排除"这类黑话;
/// 把"每个相关数字还剩几雷、候选格有哪些"喂给它, 它才能写出可验证的
/// "第 1 步/第 2 步…"排除链 (如 "2雷5格 → 两个1雷对 → 某格必安全")。
fn explain_cell(state: &AgentBoard, x: usize, y: usize) -> Result<String, String> {
    let (view, ir) = run_local_pipeline(&state.board, state.remaining_mines)?;
    let coord = crate::model::Coord::new(x as u32, y as u32);
    let h = view.height as usize;
    let w = view.width as usize;
    if y >= h || x >= w {
        return Err(format!("坐标越界: ({}, {})", x, y));
    }
    let st = view.to_state_map();
    let mut out = String::new();

    // 目标格是否为"矛盾旗" (需要给出"为什么错"的对照素材)
    let flag_status = ir
        .flag_verification
        .flags
        .iter()
        .find(|f| f.coord == coord)
        .map(|f| f.status);

    // 1) 目标格状态
    out.push_str(&format!(
        "目标格 ({}, {}): {}\n",
        x,
        y,
        match flag_status {
            Some(crate::model::FlagVerifyStatus::Contradicted) => "已标旗 — 矛盾旗(引擎判必安全, 见下)".to_string(),
            Some(crate::model::FlagVerifyStatus::Verified) => "已标旗 — 该旗已确认正确(必雷)".to_string(),
            Some(crate::model::FlagVerifyStatus::Suspected) => "已标旗 — 无法确认(见下概率)".to_string(),
            None => match st.get(&coord) {
                Some(CellState::Revealed(n)) => format!("已翻开数字{}", n),
                Some(CellState::Flagged) => "已标旗".to_string(),
                _ => "未翻开".to_string(),
            },
        }
    ));

    // 2) 引擎判定
    if let Some(p) = ir.deterministic.iter().find(|p| p.conclusion.coord == coord) {
        out.push_str(&format!(
            "引擎判定: {} — 摘要: {} (依据数字: {})\n",
            if p.conclusion.is_mine { "必雷" } else { "必安全" },
            p.rule,
            p.depends_on
                .iter()
                .map(|c| format!("({},{})", c.x, c.y))
                .collect::<Vec<_>>()
                .join(",")
        ));
    } else if let Some(p) = ir.prob_of(&coord) {
        out.push_str(&format!(
            "引擎判定: 无确定性结论, 雷概率 {:.1}%\n",
            p.mine_probability * 100.0
        ));
    } else {
        out.push_str("引擎判定: 该格越界或已翻开且非前沿, 无结论\n");
    }

    // 3) 相关数字的局部约束表: 目标格的 1 环未知格 + 2 环数字, 候选集含目标格或与目标 1 环未知相邻
    //    先收集"相关数字"集合: 目标格相邻数字 ∪ (目标格未知邻居的相邻数字), 去重按距离排序
    let mut related: Vec<Coord> = Vec::new();
    for n in coord.neighbors(view.width, view.height) {
        if matches!(st.get(&n), Some(CellState::Revealed(_))) && !related.contains(&n) {
            related.push(n);
        }
    }
    let ring1_unknown: Vec<Coord> = coord
        .neighbors(view.width, view.height)
        .into_iter()
        .filter(|n| matches!(st.get(n), Some(CellState::Unknown)))
        .collect();
    for u in &ring1_unknown {
        for n in u.neighbors(view.width, view.height) {
            if matches!(st.get(&n), Some(CellState::Revealed(_))) && !related.contains(&n) {
                related.push(n);
            }
        }
    }
    // 候选集包含目标格的数字也纳入 (约束可能更远, 但直接约束目标)
    for cell in &view.revealed {
        let c = cell.coord;
        if c == coord || related.contains(&c) {
            continue;
        }
        let unknowns: Vec<Coord> = c
            .neighbors(view.width, view.height)
            .into_iter()
            .filter(|n| matches!(st.get(n), Some(CellState::Unknown)))
            .collect();
        if unknowns.contains(&coord) {
            related.push(c);
        }
    }

    if !related.is_empty() {
        out.push_str(&format!(
            "\n局部约束表 (共 {} 个相关数字; 每行: 数字@坐标: 旗数 剩余需雷数 候选格):",
            related.len()
        ));
        for n in related {
            if let Some(CellState::Revealed(num)) = st.get(&n) {
                let flagged = n
                    .neighbors(view.width, view.height)
                    .iter()
                    .filter(|m| matches!(st.get(m), Some(CellState::Flagged)))
                    .count() as i32;
                let candidates: Vec<String> = n
                    .neighbors(view.width, view.height)
                    .into_iter()
                    .filter(|m| matches!(st.get(m), Some(CellState::Unknown)))
                    .map(|m| format!("({},{})", m.x, m.y))
                    .collect();
                if candidates.is_empty() {
                    continue;
                }
                let remaining = *num as i32 - flagged;
                out.push_str(&format!(
                    "\n数字{}@({},{}): 旗{} 剩{} 候选共{}格 [{}]",
                    num,
                    n.x,
                    n.y,
                    flagged,
                    remaining,
                    candidates.len(),
                    candidates.join(",")
                ));
            }
        }
    } else {
        out.push_str("\n该格周围没有可用的数字约束。");
    }

    // 矛盾旗专用: 提供"把旗当作未知 vs 当作雷(现状)"的对照行,
    // 让 LLM 能写出可验证的排除链 (只引用本表的数字与候选, 不靠"联动排除"黑话)。
    if flag_status == Some(crate::model::FlagVerifyStatus::Contradicted) {
        let mut board_as_unknown = state.board.clone();
        board_as_unknown[y][x] = -1; // 撤旗 → 未知
        if let Ok((view2, _)) = run_local_pipeline(&board_as_unknown, state.remaining_mines) {
            let st2 = view2.to_state_map();
            let mut diffs: Vec<String> = Vec::new();
            for n in coord.neighbors(view2.width, view2.height) {
                if !matches!(st2.get(&n), Some(CellState::Revealed(_))) {
                    continue;
                }
                // 当前行 (该格仍为旗)
                let cur = constraint_row(&st, n, view.width, view.height);
                // 撤旗行
                let alt = constraint_row(&st2, n, view2.width, view2.height);
                if cur != alt {
                    diffs.push(format!(
                        "  保留旗(当作雷): {}\n  撤旗(当作未知): {}",
                        cur.unwrap_or_default(),
                        alt.unwrap_or_default()
                    ));
                }
                if diffs.len() >= 4 {
                    break;
                }
            }
            if !diffs.is_empty() {
                out.push_str(&format!(
                    "\n\n矛盾旗对照 (引擎已枚举所有可行布雷方案, 证明该格必安全):\n{}",
                    diffs.join("\n")
                ));
                out.push_str(
                    "\n推导提示: 把该旗当作雷时, 上述每个相邻数字都多占用 1 雷名额; \
                     你只需用第 1/2 步逐步指出某处出现\"候选容量不足\", 即可证明该格必安全。",
                );
            }
        }
    }
    Ok(out)
}

/// 单个数字约束行 (用于 explain_cell 对照)
fn constraint_row(
    st: &std::collections::HashMap<Coord, CellState>,
    n: Coord,
    w: u32,
    h: u32,
) -> Option<String> {
    let CellState::Revealed(num) = st.get(&n)? else {
        return None;
    };
    let flagged = n
        .neighbors(w, h)
        .iter()
        .filter(|m| matches!(st.get(m), Some(CellState::Flagged)))
        .count() as i32;
    let candidates: Vec<String> = n
        .neighbors(w, h)
        .into_iter()
        .filter(|m| matches!(st.get(m), Some(CellState::Unknown)))
        .map(|m| format!("({},{})", m.x, m.y))
        .collect();
    if candidates.is_empty() {
        return None;
    }
    Some(format!(
        "数字{}@({},{}): 旗{} 剩{} 候选共{}格 [{}]",
        num,
        n.x,
        n.y,
        flagged,
        *num as i32 - flagged,
        candidates.len(),
        candidates.join(",")
    ))
}

/// 约束详情 (目标格及其邻居中的前沿数字)
fn constraints_at(state: &AgentBoard, x: usize, y: usize) -> Result<String, String> {
    let (view, _ir) = run_local_pipeline(&state.board, state.remaining_mines)?;
    let coord = crate::model::Coord::new(x as u32, y as u32);
    if y as u32 >= view.height || x as u32 >= view.width {
        return Err(format!("坐标越界: ({}, {})", x, y));
    }
    // 目标格状态
    let state_desc = match view.to_state_map().get(&coord) {
        Some(CellState::Revealed(n)) => format!("({}, {}) 已翻开数字 {}", x, y, n),
        Some(CellState::Flagged) => format!("({}, {}) 已标旗", x, y),
        _ => format!("({}, {}) 未翻开", x, y),
    };
    // 目标格周围的数字约束行
    let mut lines = vec![state_desc];
    for n in coord.neighbors(view.width, view.height) {
        if let Some(CellState::Revealed(num)) = view.to_state_map().get(&n) {
            let f = view
                .flagged_count_around()
                .get(&n)
                .copied()
                .unwrap_or(0) as i32;
            let unk = view
                .unknown_count_around()
                .get(&n)
                .copied()
                .unwrap_or(0);
            lines.push(format!(
                "数字{}@({},{}): 旗{} 剩{} 未知邻居{}个",
                num, n.x, n.y, f, *num as i32 - f, unk
            ));
        }
    }
    Ok(lines.join("\n"))
}

/// 单格雷概率 (无确定性结论时的策略依据)
fn probability_at(state: &AgentBoard, x: usize, y: usize) -> Result<String, String> {
    let (view, ir) = run_local_pipeline(&state.board, state.remaining_mines)?;
    let coord = Coord::new(x as u32, y as u32);
    if y as u32 >= view.height || x as u32 >= view.width {
        return Err(format!("坐标越界: ({}, {})", x, y));
    }
    // 确定性结论优先 (概率无意义)
    if let Some(p) = ir.deterministic.iter().find(|p| p.conclusion.coord == coord) {
        return Ok(format!(
            "({}, {}): {} (确定性结论, 无概率不确定性)",
            x,
            y,
            if p.conclusion.is_mine { "必雷" } else { "必安全" }
        ));
    }
    match ir.prob_of(&coord) {
        Some(p) => Ok(format!(
            "({}, {}): 雷概率 {:.1}% (信息增益 {:.1})",
            x,
            y,
            p.mine_probability * 100.0,
            p.info_gain
        )),
        None => Ok(format!("({}, {}) 无概率数据 (可能已翻开)", x, y)),
    }
}

/// 以 (x,y) 为中心截取局部棋盘文本 (纯展示, 不跑推理)
fn local_board(state: &AgentBoard, x: usize, y: usize, radius: usize) -> Result<String, String> {
    let h = state.board.len();
    let w = if h > 0 { state.board[0].len() } else { 0 };
    if y >= h || x >= w {
        return Err(format!("坐标越界: ({}, {})", x, y));
    }
    let (x0, x1) = (
        x.saturating_sub(radius),
        (x + radius).min(w.saturating_sub(1)),
    );
    let (y0, y1) = (
        y.saturating_sub(radius),
        (y + radius).min(h.saturating_sub(1)),
    );
    let mut out = format!(
        "以 ({}, {}) 为中心半径 {} 的局部图 ({}-{} × {}-{}) (?:未翻开 F:旗 数字:已翻开):\n",
        x, y, radius, x0, x1, y0, y1
    );
    for ry in y0..=y1 {
        let mut line = format!("y={:2} | ", ry);
        for cx in x0..=x1 {
            let ch = match state.board[ry][cx] {
                -1 => '?',
                -2 => 'F',
                n => char::from_digit(n as u32, 10).unwrap_or('?'),
            };
            line.push(ch);
            line.push(' ');
        }
        if ry == y {
            out.push_str(&format!("{}  ← 目标格 ({}, {})\n", line.trim_end(), x, y));
        } else {
            out.push_str(&format!("{}\n", line.trim_end()));
        }
    }
    out.push_str(&format!(
        "行坐标范围: x={}..={} (列), y={}..={} (行); 提示: (x, y) 中 x 为列号, y 为行号",
        x0, x1, y0, y1
    ));
    Ok(out)
}

/// 分析摘要 (analyze_board 的观察结果)。
///
/// 分级原则: 已标旗并通过验证的雷 = "已知条件" → 只出现在摘要计数里;
/// "新发现" = 未标旗的必雷格 / 未翻开的必安全格 / 矛盾旗, 才逐条列出 (限量)。
fn analyze_observation(view: &PlayerView, ir: &InferenceIR) -> String {
    let summary = board_summary(view, ir);
    let new_mines: Vec<String> = ir
        .deterministic
        .iter()
        .filter(|p| p.conclusion.is_mine)
        .filter(|p| view.unknown.contains(&p.conclusion.coord))
        .take(5)
        .map(|p| format!("({},{}):{}", p.conclusion.coord.x, p.conclusion.coord.y, p.rule))
        .collect();
    let mine_more = ir
        .deterministic
        .iter()
        .filter(|p| p.conclusion.is_mine)
        .filter(|p| view.unknown.contains(&p.conclusion.coord))
        .count()
        .saturating_sub(5);
    let new_safes: Vec<String> = ir
        .deterministic
        .iter()
        .filter(|p| !p.conclusion.is_mine)
        .filter(|p| view.unknown.contains(&p.conclusion.coord))
        .take(5)
        .map(|p| format!("({},{})", p.conclusion.coord.x, p.conclusion.coord.y))
        .collect();
    let safe_more = ir
        .deterministic
        .iter()
        .filter(|p| !p.conclusion.is_mine)
        .filter(|p| view.unknown.contains(&p.conclusion.coord))
        .count()
        .saturating_sub(5);
    let contrad: Vec<String> = ir
        .flag_verification
        .flags
        .iter()
        .filter(|f| f.status == crate::model::FlagVerifyStatus::Contradicted)
        .map(|f| format!("({},{})", f.coord.x, f.coord.y))
        .collect();
    // 参考: 按信息增益取前 5 个概率格 (供无新结论时做策略)
    let mut probs: Vec<&crate::model::CellProb> = ir.probabilities.iter().collect();
    probs.sort_by(|a, b| {
        b.info_gain
            .partial_cmp(&a.info_gain)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let top_probs: Vec<String> = probs
        .iter()
        .take(5)
        .map(|p| {
            format!(
                "({},{}) 雷{:.0}% 增益{:.1}",
                p.coord.x,
                p.coord.y,
                p.mine_probability * 100.0,
                p.info_gain
            )
        })
        .collect();

    let mut obs = format!("{}.", summary);
    if new_mines.is_empty() && new_safes.is_empty() && contrad.is_empty() {
        obs.push_str("\n本次无新增确定性结论 (已标旗验证结果视为已知条件, 不再列出)。");
    } else {
        if !new_mines.is_empty() {
            let more = if mine_more > 0 {
                format!(" (其余 {} 个略)", mine_more)
            } else {
                String::new()
            };
            obs.push_str(&format!("\n新发现·必雷未标旗 (可 flag_cell): {}{}", new_mines.join("; "), more));
        }
        if !new_safes.is_empty() {
            let more = if safe_more > 0 {
                format!(" (其余 {} 个略)", safe_more)
            } else {
                String::new()
            };
            obs.push_str(&format!("\n新发现·必安全未翻开 (可建议点击): {}{}", new_safes.join("; "), more));
        }
        if !contrad.is_empty() {
            obs.push_str(&format!("\n矛盾旗 (实际安全, 应提醒移除): {}", contrad.join("; ")));
        }
    }
    if !top_probs.is_empty() {
        obs.push_str(&format!("\n高价值参考 (信息增益前5): {}", top_probs.join("; ")));
    }
    obs
}

/// 棋盘状态指纹: 用于判断"棋盘是否发生变化" (防空转, 见 loop_.rs)
pub fn board_fingerprint(board: &[Vec<i32>], mines: u32) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    for row in board {
        for v in row {
            v.hash(&mut h);
        }
    }
    mines.hash(&mut h);
    h.finish()
}

/// 分区观察 (list_regions)
fn regions_observation(view: &PlayerView, ir: &InferenceIR) -> String {
    if ir.regions.is_empty() {
        return "当前没有待处理的连通区域 (已全部确定或棋盘已开)。".to_string();
    }
    let mut obs = format!("共 {} 个连通区域:", ir.regions.len());
    for (i, r) in ir.regions.iter().enumerate() {
        obs.push_str(&format!(
            "\n区域 {}: 未知 {} 格, 估计雷 {:.1}, 边界 {} 格, 平均雷概率 {:.1}%",
            i,
            r.features.unknown_count,
            r.features.estimated_mines,
            r.features.boundary_length,
            r.features.avg_mine_prob * 100.0
        ));
        let head: Vec<String> = r
            .cells
            .iter()
            .take(10)
            .map(|c| format!("({},{})", c.x, c.y))
            .collect();
        let more = if r.cells.len() > 10 {
            format!(" …等共{}格", r.cells.len())
        } else {
            String::new()
        };
        obs.push_str(&format!("\n  格子: {}{}", head.join(", "), more));
    }
    let _ = view;
    obs
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 3x3 手写局面: (0,0)=1, (1,0)=旗(矛盾—实际安全) → (0,1)/(1,1)? 简化用文本棋盘造"必雷未标旗":
    /// 1 1 ?  / 2 F ? / ? ? ? 之类不好造; 直接以引擎运行结果为准做黑盒断言较少。
    fn board_3x3() -> Vec<Vec<i32>> {
        vec![
            vec![1, -1, -1],
            vec![1, -1, -1],
            vec![-1, -1, -1],
        ]
    }

    #[test]
    fn test_tool_unknown_name_rejected() {
        let mut st = AgentBoard::new(board_3x3(), 1);
        let e = execute_tool(&mut st, "click_cell", &json!({"x": 0, "y": 0}));
        assert!(e.is_err());
        assert!(e.unwrap_err().contains("未知工具"));
    }

    #[test]
    fn test_flag_cell_rejects_unproven_and_mutates_nothing() {
        let mut st = AgentBoard::new(board_3x3(), 1);
        let before = st.board.clone();
        // 左上 (0,0) 已翻开数字 1 → 拒绝
        let e = execute_tool(&mut st, "flag_cell", &json!({"x": 0, "y": 0}));
        assert!(e.is_err());
        assert!(e.unwrap_err().contains("已翻开"));
        // 未证明必雷的未知格 → 拒绝
        let e2 = execute_tool(&mut st, "flag_cell", &json!({"x": 2, "y": 0}));
        assert!(e2.is_err());
        assert!(e2.unwrap_err().contains("未证明"));
        assert_eq!(st.board, before); // 棋盘未被修改
    }

    #[test]
    fn test_flag_cell_allowed_on_proven_mine() {
        // 局面: 数字 3 位于(0,0) 且 (1,0),(1,1),(0,1) 未知 → (0,0) 剩3雷3未知 → 全雷
        let mut st = AgentBoard::new(
            vec![
                vec![3, -1, -1],
                vec![-1, -1, -1],
                vec![-1, -1, -1],
            ],
            3,
        );
        let r = execute_tool(&mut st, "flag_cell", &json!({"x": 1, "y": 0}));
        assert!(r.is_ok(), "{}", r.err().unwrap_or_default());
        assert_eq!(st.board[0][1], -2);
        assert_eq!(st.remaining_mines, 2);
    }

    #[test]
    fn test_check_safe_reports_proof_or_probability() {
        let mut st = AgentBoard::new(
            vec![
                vec![3, -1, -1],
                vec![-1, -1, -1],
                vec![-1, -1, -1],
            ],
            3,
        );
        // (1,0) 必雷
        let r = execute_tool(&mut st, "check_safe", &json!({"x": 1, "y": 0})).unwrap();
        assert!(r.contains("必雷"));
        // (0,1) 必雷; (2,2) 与数字不相邻但雷概率>0 (总雷数多于约束) → 输出概率或必安全
        let r2 = execute_tool(&mut st, "check_safe", &json!({"x": 2, "y": 2})).unwrap();
        assert!(r2.contains("雷概率") || r2.contains("必安全") || r2.contains("必雷"));
    }

    #[test]
    fn test_verify_flag_and_constraints() {
        // 3 必需全雷局面, 先在 (1,0) 标旗, verify_flag → 该旗成为 Verified(?) 或 Suspected
        let mut st = AgentBoard::new(
            vec![
                vec![3, -1, -1],
                vec![-1, -1, -1],
                vec![-1, -1, -1],
            ],
            3,
        );
        execute_tool(&mut st, "flag_cell", &json!({"x": 1, "y": 0})).unwrap();
        let v = execute_tool(&mut st, "verify_flag", &json!({"x": 1, "y": 0}));
        assert!(v.is_ok());
        let c = execute_tool(&mut st, "get_constraints", &json!({"x": 1, "y": 0}));
        assert!(c.is_ok());
        assert!(c.unwrap().contains("数字3@(0,0)"));
    }

    #[test]
    fn test_knowledge_query() {
        assert!(query_knowledge("").contains("1-2-1"));
        assert!(query_knowledge("121").contains("1-2-1"));
        assert!(query_knowledge("不存在的主题").contains("未找到"));
    }
}

#[cfg(test)]
mod explain_tests {
    use super::*;

    #[test]
    fn test_explain_cell_returns_local_constraint_table() {
        // 3x3: (0,0)=3 且右列/下行未知 → (1,0) 等被证必雷; explain(1,0) 应含约束行 数字3@(0,0)
        let mut st = AgentBoard::new(
            vec![vec![3, -1, -1], vec![-1, -1, -1], vec![-1, -1, -1]],
            3,
        );
        let r = execute_tool(&mut st, "explain_cell", &json!({"x": 1, "y": 0})).unwrap();
        assert!(r.contains("引擎判定"));
        assert!(r.contains("数字3@(0,0)"));
        assert!(r.contains("候选"));
        assert!(r.contains("剩"));
        // 越界拒绝
        let e = execute_tool(&mut st, "explain_cell", &json!({"x": 9, "y": 9}));
        assert!(e.is_err());
    }
}

#[cfg(test)]
mod react_tools_tests {
    use super::*;

    #[test]
    fn test_get_probability_tool() {
        let mut st = AgentBoard::new(vec![vec![3, -1, -1], vec![-1, -1, -1], vec![-1, -1, -1]], 4);
        // (1,0) 必雷 (数字3 唯一候选必填) → 概率工具返回确定性说明
        let r = execute_tool(&mut st, "get_probability", &json!({"x": 1, "y": 0})).unwrap();
        assert!(r.contains("必雷") || r.contains("必安全"));
        // 右下 (2,2): 剩 1 雷未定 → 概率
        let r2 = execute_tool(&mut st, "get_probability", &json!({"x": 2, "y": 2})).unwrap();
        assert!(r2.contains("雷概率"));
    }

    #[test]
    fn test_get_local_board_tool() {
        let st = AgentBoard::new(
            vec![vec![1, -2, -1, 0], vec![-1, -1, 2, -1], vec![0, -1, -1, -1]],
            2,
        );
        let mut stm = st.clone();
        let r = execute_tool(&mut stm, "get_local_board", &json!({"x": 1, "y": 1, "radius": 1})).unwrap();
        assert!(r.contains("目标格 (1, 1)"));
        assert!(r.contains("F")); // 旗渲染
        assert!(r.contains("?")); // 未知渲染
        assert!(r.contains("y="));
        // 越界拒绝
        let mut stm2 = st.clone();
        assert!(execute_tool(&mut stm2, "get_local_board", &json!({"x": 9, "y": 9})).is_err());
    }

    #[test]
    fn test_get_cell_info_alias() {
        let mut st = AgentBoard::new(vec![vec![3, -1, -1], vec![-1, -1, -1], vec![-1, -1, -1]], 3);
        let r = execute_tool(&mut st, "get_cell_info", &json!({"x": 0, "y": 0})).unwrap();
        assert!(r.contains("已翻开"));
    }
}
