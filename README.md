# 🧠 扫雷认知助手 (Minesweeper Cognitive Agent)

Rust 推理引擎 + LLM 翻译层 + Web 棋盘编辑器 + OCR 截图识别

## 快速开始

### 方式一：Docker 一条命令（推荐，含 OCR）

```bash
docker compose up
```

打开 http://localhost:8080 即可使用，无需安装 Rust / Python / OpenCV。

> **自定义端口**：8080 被占用时可通过环境变量覆盖宿主机端口：
> ```bash
> PORT=8081 docker compose up -d
> ```
> 不传 `PORT` 时默认 8080。

### 方式二：Cargo 一条命令（已有 Rust 环境）

```bash
cargo run --release
```

打开 http://localhost:8080 。推理引擎 + 前端开箱即用（LLM 与 OCR 为可选增强）。

<details>
<summary>可选：启用 LLM / OCR</summary>

```bash
# LLM (OpenAI 兼容 API)
export OPENAI_API_KEY=sk-xxx
export OPENAI_BASE_URL=https://api.openai.com/v1   # 可省略
export OPENAI_MODEL=gpt-4o-mini                    # 可省略
cargo run --release

# OCR 截图识别 (Python 微服务)
cd ocr && pip install -r requirements.txt && python app.py   # 端口 5001
```
</details>

### 方式三：安装为全局命令

```bash
cargo install --path .
minesweeper-agent serve        # 或直接运行 minesweeper-agent
```

## 配置 LLM（可选）

两种方式，界面配置优先于环境变量。两者都未配置时自动回退到本地分析引擎。

| 方式 | 说明 |
|------|------|
| **界面配置** | 打开 http://localhost:8080 后，点击导航栏「🤖 LLM」→「⚙️ API 配置」，从预设选择提供商（OpenAI / DeepSeek / Kimi / Ollama / 通义千问 / 自定义），填入 Base URL / 模型名 / API Key，点击「测试连接」验证后保存 |
| **环境变量** | `OPENAI_API_KEY=sk-xxx OPENAI_BASE_URL=... OPENAI_MODEL=... cargo run --release` |

> 🔒 API Key 仅保存在页面内存中，刷新即清除。

## 核心功能

- **确定性推理**：约束传播 + 子集规则，输出带完整推理链的必雷/必安全证明
- **概率引擎**：动态迭代蒙特卡洛模拟，30×30 大棋盘 < 150ms
- **旗帜验证与容错**：推理前逐面验证旗帜（单旗约束 → 必要性 → 容错重推 → 局部闭包枚举 → 概率佐证），矛盾旗自动降级为未知格继续推理，不污染后续结论；前端紫框高亮并支持 🧹 一键移除
- **旗帜问题特判**：检测到误标旗时跳过全盘分析，只取第一面问题旗（矛盾旗优先）交给 LLM 专项解释「这面旗为什么有问题」，分析结果以「第 1 步 / 第 2 步 …」推理链逐步呈现（点击步骤高亮棋盘）；LLM 不可用时回退引擎素材（含「保留旗 vs 撤旗」约束对照）
- **ReAct Agent 循环**：LLM 逐轮决策调用工具（标旗/取证/知识库），Rust 校验执行并回写状态，带步数/超时/费用护栏
- **LLM 三种模式**：答案模式（直接给出推理链）、教学模式（引导提问）、策略模式（全局比较）
- **OCR 截图识别**：三层回退（颜色投票 → 形态匹配 → Tesseract），支持经典/暗色等多主题
- **数据防火墙**：传给 LLM 的信息绝不包含未翻开格子的真实雷藏

## 使用

1. 打开 Web 界面，三种方式录入局面：
   - **粘贴**：对话输入框按 `Ctrl+V` / `Cmd+V`，截图自动 OCR、文本棋盘（`? F 0-8`）自动解析
   - **手动编辑**：左键递增数字（1→8），右键插旗 🚩，`Shift+左键` 清零，`Ctrl+左键` 变为未知
   - **加载示例**：下拉菜单载入经典残局
2. 点击 **⚡ 分析局面**，棋盘上绿色 = 必安全、红色 = 必雷、蓝色百分比 = 雷概率
3. 点击推理链步骤，棋盘高亮该步涉及的格子；悬停格子查看概率与依据
4. **旗帜问题**：存在误标旗时，本次分析自动聚焦第一面问题旗（LLM 专项解释成因，推理链逐步呈现），配合「🧹 移除矛盾旗并重新分析」逐面处理后再做全盘推理
5. **🎓 教学模式**：不给答案，逐步引导提问；**📤 导出结果**：下载 Markdown 报告

## API

| 端点 | 说明 |
|------|------|
| `POST /api/analyze` | 分析局面（支持 `mode=answer/teaching/strategy`，可选 LLM） |
| `POST /api/ocr` | 截图识别代理 |
| `POST /api/llm/test` | 测试 LLM 连接 |
| `GET /api/llm/status` | 查询环境变量 LLM 配置状态 |
| `GET /api/usage/stats` | Token 用量统计 |
| `GET /api/usage/logs` | Token 用量明细日志 |
| `GET /api/health` | 健康检查 |

完整 API 文档见项目源码 `src/server/routes.rs`。

## CLI

```bash
minesweeper-agent serve [--port 8080] [--mc-iterations 0]   # Web 服务, 0=动态自适应
minesweeper-agent analyze -i tests/sample_board.json --mode answer
minesweeper-agent ocr -i screenshot.png                      # 需 OCR 服务
minesweeper-agent import -i tests/sample_board.json
```

## 架构概览

```
┌─────────────────────────────────────────────────────────┐
│              用户交互层 (Web 前端)                        │
│  截图粘贴 (OCR) / 手动编辑 / 推理高亮 & 对话              │
└─────────────────────────┬───────────────────────────────┘
                          │ JSON over HTTP
                          ▼
┌──────────────────┐  ┌──────────────────────────────────────┐
│  OCR 微服务      │  │     Rust 核心 + Web 服务              │
│  (Python/Flask)  │  │  ├─ 确定性约束传播 (子集推理)           │
│  OpenCV+Tesseract│◄─┤  ├─ 蒙特卡洛概率模拟                   │
│  截图→棋盘数组    │  │  ├─ ReAct Agent 循环（工具调用）       │
└──────────────────┘  │  ├─ LLM 转译 (3种模式)                │
                      │  └─ Token 用量统计                     │
                      └──────────────────────────────────────┘
```

## 项目结构

```
minesweeper/
├── src/
│   ├── engine/           # 推理引擎 (确定性/概率/区域)
│   ├── agent/            # Agent 核心 (工具/循环/状态机)
│   ├── model/            # PlayerView / Coord / InferenceIR
│   ├── llm/              # LLM 转译 + 客户端 + 用量统计
│   ├── server/           # Web 服务 (axum)
│   └── cli.rs            # CLI (clap)
├── static/index.html     # 前端 (Canvas 渲染)
├── ocr/                  # OCR 微服务 (Flask + OpenCV + Tesseract)
├── tests/                # 集成测试
├── Dockerfile / docker-compose.yml
└── Cargo.toml
```

## 开发

```bash
cargo test            # 单元 + 集成测试
cargo build --release # 构建
```

## 引用的开源库

| 库 | 用途 |
|----|------|
| [axum](https://github.com/tokio-rs/axum) / [tokio](https://github.com/tokio-rs/tokio) | Web 服务与异步运行时 |
| [serde](https://serde.rs) / [serde_json](https://github.com/serde-rs/json) | 序列化与 JSON |
| [rand](https://github.com/rust-random/rand) | 蒙特卡洛随机模拟 |
| [rayon](https://github.com/rayon-rs/rayon) | 数据并行 |
| [reqwest](https://github.com/seanmonstar/reqwest) | LLM API / OCR HTTP 客户端 |
| [clap](https://github.com/clap-rs/clap) | 命令行参数 |
| [Flask](https://flask.palletsprojects.com) / [OpenCV](https://opencv.org) / [Tesseract](https://github.com/tesseract-ocr/tesseract) | OCR 微服务 |

## License

MIT

## 致谢

感谢 GPT 5.6 系列、GLM 5.3 系列、DeepSeek V4 系列模型对此项目的贡献。

感谢清华大学计算机科学与技术系提供的平台支持。