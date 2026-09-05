//! Agent 核心逻辑 (Rust) — 棋盘状态机 / 文本解析 / 报告构建
//!
//! 架构约定 (AGENT.md): 任务调度、状态管理、规则判断必须由 Rust 实现。
//! 本模块承载原先散落在前端 JS 里的"语义核心", 前端仅保留渲染与交互,
//! 通过 HTTP 端点调用本模块:
//!   - POST /api/board/op    编辑操作状态机 (左键循环 / 插旗 / 清零)
//!   - POST /api/board/parse 文本棋盘解析
//!   - POST /api/export      Markdown 报告构建
//!
//! 本模块保持纯逻辑、无 IO, 可独立单元测试, 亦可脱离 axum 单独编译
//! (如后续编译为 WASM 供前端直调)。

pub mod core;

pub use core::{
    apply_board_op, BoardOp, infer_mines_for_size, parse_text_board, render_board_text,
    render_markdown_report, BoardParseError,
};
