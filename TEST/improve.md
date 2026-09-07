# AGENTS.md – 从“翻译器”升级为“操作器”：ReAct 循环与工具调用架构

## 1. 核心转变：架构升级

当前架构是 **“单向流水线”**（Rust推 → LLM译 → 输出），而目标是 **“双向循环代理”**（LLM决策 → 执行工具 → 观察结果 → 再决策）。

| 维度 | 当前（翻译器模式） | 目标（操作器模式） |
|------|-------------------|-------------------|
| 流程 | 固定：IR → 翻译 → 结束 | 动态：决策 → 执行 → 反思 → 再决策 |
| LLM 角色 | 文本转译器 | 推理引擎 + 行动计划制定者 |
| 工具调用 | ❌ 无 | ✅ 可调用 Rust 函数（点击、标旗、分析） |
| 状态管理 | 无状态（每次独立） | 有状态（记忆已执行的操作和假设） |
| 知识库 | ❌ 无 | ✅ 扫雷模式库（1-2-1、1-2-2-1 等） |
| 迭代能力 | 单轮 | 多轮（直到解决或用户中断） |

---

## 2. 改进方案概述

```
┌─────────────────────────────────────────────────────────────────┐
│                    Agent 执行循环 (ReAct)                       │
│                                                                 │
│  ┌─────────┐    ┌─────────┐    ┌─────────┐    ┌─────────┐    │
│  │ 用户输入 │ → │  LLM     │ → │ 工具调用 │ → │ 观察结果 │    │
│  │ (问题)   │    │ 推理+决策│    │ (Rust)   │    │ (新状态) │    │
│  └─────────┘    └────┬────┘    └─────────┘    └────┬────┘    │
│                      │                               │         │
│                      └────────── 循环 ───────────────┘         │
│                              ↓ 直到完成                        │
│                      ┌─────────────┐                         │
│                      │ 最终响应     │                         │
│                      └─────────────┘                         │
└─────────────────────────────────────────────────────────────────┘
```

---

## 3. 具体实现方式

### 3.1 工具定义（LLM 可调用的 Rust 函数）

在 Rust 后端定义一组工具（Tools），暴露给 LLM 作为函数调用选项。

```rust
// src/tools/mod.rs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentTool {
    /// 分析当前棋盘，返回 IR（确定性结论 + 概率）
    AnalyzeBoard,
    /// 在指定坐标标旗（如果逻辑允许）
    FlagCell { x: usize, y: usize },
    /// 在指定坐标点击（如果逻辑允许）
    ClickCell { x: usize, y: usize },
    /// 获取某个数字的约束详情
    GetConstraints { x: usize, y: usize },
    /// 验证某个旗帜是否正确
    VerifyFlag { x: usize, y: usize },
    /// 获取某个连通区域的概要
    GetRegionSummary { region_id: usize },
    /// 应用一个扫雷模式（如 1-2-1）
    ApplyPattern { pattern: String, anchor_x: usize, anchor_y: usize },
}
```

**工具调用 JSON Schema（供 LLM 参考）**：
```json
{
  "tools": [
    {
      "name": "analyze_board",
      "description": "执行完整的确定性推理和概率模拟，返回 IR 数据",
      "parameters": {}
    },
    {
      "name": "flag_cell",
      "description": "在 (x,y) 处标旗，返回操作结果和更新后的棋盘状态",
      "parameters": {
        "x": { "type": "integer", "description": "列坐标" },
        "y": { "type": "integer", "description": "行坐标" }
      }
    },
    {
      "name": "click_cell",
      "description": "在 (x,y) 处点击，返回安全/雷结果和更新后的棋盘",
      "parameters": {
        "x": { "type": "integer", "description": "列坐标" },
        "y": { "type": "integer", "description": "行坐标" }
      }
    }
  ]
}
```

### 3.2 Agent 循环实现（Rust 状态机）

在 `src/agent/loop.rs` 中实现 ReAct 循环：

```rust
use std::collections::VecDeque;

pub struct AgentState {
    pub board: PlayerView,
    pub history: VecDeque<Action>,      // 已执行的操作记录
    pub hypotheses: Vec<String>,        // LLM 提出的假设
    pub iteration: u32,
    pub max_iterations: u32,
}

pub enum Action {
    ToolCall(AgentTool),
    FinalAnswer(String),
}

pub async fn run_agent_loop(
    initial_board: PlayerView,
    llm_client: &LLMClient,
    tool_executor: &ToolExecutor,
    max_steps: u32,
) -> Result<String, AgentError> {
    let mut state = AgentState {
        board: initial_board,
        history: VecDeque::new(),
        hypotheses: Vec::new(),
        iteration: 0,
        max_iterations: max_steps,
    };

    loop {
        state.iteration += 1;
        if state.iteration > state.max_iterations {
            break;
        }

        // 1. 调用 LLM，传入当前状态 + 工具定义
        let response = llm_client
            .request(&state.board, &state.history, &get_tool_definitions())
            .await?;

        // 2. 解析 LLM 的响应
        match parse_action(&response) {
            Action::ToolCall(tool) => {
                // 3. 执行工具
                let result = tool_executor.execute(tool, &mut state.board).await?;
                // 4. 记录结果
                state.history.push_back(Action::ToolCall(tool));
                state.history.push_back(Action::Observation(result));
                // 5. 继续循环
            }
            Action::FinalAnswer(answer) => {
                return Ok(answer);
            }
        }
    }

    Ok("达到最大迭代次数，未完成推理".to_string())
}
```

### 3.3 LLM 提示词（带工具调用）

System Prompt 中新增工具调用指引：

```
你是一个扫雷求解代理。你被赋予了以下工具来控制游戏：

1. analyze_board() — 分析当前棋盘，返回推理结果。
2. flag_cell(x, y) — 在 (x,y) 标旗。只有确定是雷时才调用。
3. click_cell(x, y) — 在 (x,y) 点击。只有确定安全时才调用。
4. verify_flag(x, y) — 验证 (x,y) 的旗帜是否正确。
5. get_constraints(x, y) — 获取数字 (x,y) 周围的约束详情。
6. apply_pattern(pattern, x, y) — 应用扫雷模式（如 "121" 在 (x,y)）。

【工作流程】
1. 分析局面 → 2. 形成假设 → 3. 用工具验证 → 4. 执行操作 → 5. 观察新状态 → 6. 重复，直到解决

【输出格式】
如果你认为需要执行工具，输出 JSON：
{"action": "tool", "name": "flag_cell", "args": {"x": 5, "y": 0}}

如果你认为已解决或需要给出建议，输出：
{"action": "final", "answer": "建议翻开 (29,14)..."}

【约束】
- 每次只调用一个工具
- 调用工具前必须说明推理依据（不超过30字）
- 禁止在没有确定性证据时调用 flag_cell
- 点击/标旗前必须确保不超过剩余雷数
```

### 3.4 知识库集成（扫雷模式库）

在 Rust 中预置常见扫雷模式，供 LLM 引用：

```rust
// src/agent/patterns.rs
pub struct Pattern {
    pub name: String,          // "1-2-1"
    pub description: String,   // "边缘模式：数字1-2-1连续排列时，两端为雷"
    pub condition: Vec<u8>,    // 数字序列 [1, 2, 1]
    pub result: Vec<Action>,   // 推导结果（标旗/点击坐标）
}

pub fn get_pattern_library() -> Vec<Pattern> {
    vec![
        Pattern {
            name: "1-2-1".to_string(),
            description: "三格连续数字1-2-1时，两端必雷，中间安全".to_string(),
            condition: vec![1, 2, 1],
            result: vec![
                Action::Flag { offset: (0, -1) },
                Action::Flag { offset: (0, 1) },
                Action::Click { offset: (0, 0) },
            ],
        },
        Pattern {
            name: "1-2-2-1".to_string(),
            description: "四格连续数字1-2-2-1时，两端雷，中间两个安全".to_string(),
            condition: vec![1, 2, 2, 1],
            result: vec![
                Action::Flag { offset: (0, -1) },
                Action::Flag { offset: (0, 1) },
                Action::Click { offset: (0, -2) },
                Action::Click { offset: (0, 2) },
            ],
        },
        // ... 更多模式
    ]
}
```

LLM 可以通过 `apply_pattern("1-2-1", 5, 10)` 调用这些模式，Rust 自动完成推导和操作。

### 3.5 状态记忆（存储已执行操作）

用户可随时查看 Agent 的“思维链”：

```rust
#[derive(Serialize)]
pub struct AgentTrace {
    pub step: u32,
    pub thought: String,      // LLM 的推理
    pub action: String,       // 执行的操作
    pub result: String,       // 操作结果
    pub board_snapshot: String, // 棋盘摘要
}
```

前端可展示为时间线：

```
步骤 1: 🤔 观察到 (28,10) 与 (29,10) 构成 50/50
步骤 2: 🔍 调用 get_constraints(27,11) → 发现 3 个约束
步骤 3: 💡 推理出 (28,12) 必安全
步骤 4: 🖱️ 调用 click_cell(28,12) → 安全，翻开数字 2
步骤 5: 🔄 重新分析... (继续)
```

---

## 4. 实施路线图

| 阶段 | 任务 | 产出 | 预计耗时 |
|------|------|------|----------|
| **阶段 1** | 定义工具 API（Rust 结构体 + JSON Schema） | `src/tools/mod.rs` | 2 小时 |
| **阶段 2** | 实现工具执行器（`ToolExecutor`） | 每个工具对应的 Rust 函数 | 2 小时 |
| **阶段 3** | 实现 ReAct 循环（状态机） | `src/agent/loop.rs` | 3 小时 |
| **阶段 4** | 修改 LLM 调用接口，支持工具调用 | `src/llm/client.rs` 升级 | 2 小时 |
| **阶段 5** | 集成扫雷模式库 | `src/agent/patterns.rs` | 1 小时 |
| **阶段 6** | 前端展示“思维链” | 对话区新增时间线视图 | 2 小时 |
| **阶段 7** | 集成测试 + 端到端验证 | 测试用例 | 2 小时 |

---

## 5. 代码示例：工具调用落地

### 5.1 Rust 端请求 LLM（带工具定义）

```rust
let request = json!({
    "model": model,
    "messages": messages,
    "tools": get_tool_definitions(),  // ← 关键
    "tool_choice": "auto",
    "max_tokens": 2048,
    "temperature": 0.1,
});
```

### 5.2 LLM 返回的工具调用

```json
{
  "choices": [{
    "message": {
      "role": "assistant",
      "tool_calls": [{
        "id": "call_abc123",
        "type": "function",
        "function": {
          "name": "click_cell",
          "arguments": "{\"x\": 28, \"y\": 12}"
        }
      }]
    }
  }]
}
```

### 5.3 Rust 执行并返回结果

```rust
match tool_call.function.name.as_str() {
    "click_cell" => {
        let args: ClickArgs = serde_json::from_str(&tool_call.function.arguments)?;
        let result = board.click(args.x, args.y);
        // 将结果作为新的 tool_message 返回给 LLM
        let tool_result = json!({
            "role": "tool",
            "tool_call_id": tool_call.id,
            "content": format!("点击 ({},{}) 返回: {}", args.x, args.y, result)
        });
        messages.push(tool_result);
        // 继续循环...
    }
    // ...
}
```

---

## 6. 对作业的价值（展示重点）

| 展示点 | 说明 |
|--------|------|
| **工具调用（Function Calling）** | 体现 LLM 不仅“说话”，还能“操作”本地程序 |
| **ReAct 循环** | 展示 Agent 的自主决策和迭代推理能力 |
| **状态管理** | 展示 Agent 不是无状态的，能记住已执行的操作 |
| **知识库集成** | 扫雷模式库体现专家经验植入 |
| **思维链可视化** | 展示“黑盒”背后的推理过程，满足作业 R5（上下文历史管理） |

---

## 7. 总结

| 维度 | 旧架构（翻译器） | 新架构（操作器） |
|------|----------------|----------------|
| 交互方式 | 单向 | 双向循环 |
| LLM 作用 | 翻译文本 | 决策 + 规划 |
| 程序操作 | 无 | 点击、标旗、分析 |
| 状态持久化 | 无 | 有（操作历史 + 假设） |
| 知识库 | 无 | 扫雷模式库 |
| 展示效果 | 静态报告 | 动态推理过程 |

这套架构让你的 Agent 真正成为一个**可交互的解题助手**，而不仅仅是一个“高级翻译器”。