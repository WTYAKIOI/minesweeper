# 🧠 扫雷认知助手 (Minesweeper Cognitive Agent)

Rust 推理引擎 + LLM 翻译层 + Web 棋盘编辑器 + OCR 截图识别

- **确定性推理**：约束传播 + 子集规则，输出带完整依赖链的必雷 / 必安全证明
- **概率引擎**：动态迭代蒙特卡洛模拟，30×30 大棋盘 < 150ms
- **LLM 转译**：答案 / 教学 / 策略三种模式，把推理 IR 翻译成人类语言（含坐标幻觉检测）
- **OCR 截图识别**：三层回退（颜色投票 → 7 段形态匹配 → Tesseract），支持经典 / 暗色 / 紫色等多主题
- **数据防火墙**：传给 LLM 的信息绝不包含未翻开格子的真实雷藏

## 🚀 快速开始（选一种即可）

### 方式一：Docker 一条命令（推荐，含 OCR）

```bash
docker compose up
```

打开 http://localhost:8080 即可使用，无需安装 Rust / Python / OpenCV。

<details>
<summary>启用 LLM（可选，两种方式）</summary>

**方式 A — 界面配置（推荐）**：打开 http://localhost:8080 后，展开左侧「🤖 LLM 设置」面板，填入 API Key / Base URL / 模型名，点击「测试连接」验证后保存。配置存储在浏览器 localStorage，下次自动加载。

**方式 B — 环境变量**：

```bash
OPENAI_API_KEY=sk-xxx OPENAI_BASE_URL=https://api.openai.com/v1 OPENAI_MODEL=gpt-4o-mini docker compose up
```
或在 `docker-compose.yml` 的 `app.environment` 中填写后重启。

> 界面配置优先于环境变量。两者都未配置时自动回退到本地分析引擎。

</details>

### 方式二：Cargo 一条命令（已有 Rust 环境）

```bash
cargo run --release
```

打开 http://localhost:8080 。推理引擎 + 前端开箱即用（LLM 与 OCR 为可选增强）。

<details>
<summary>可选：启用 LLM / OCR</summary>

```bash
# LLM (OpenAI 兼容 API: OpenAI / DeepSeek / Kimi 等)
# 方式 A: 环境变量
export OPENAI_API_KEY=sk-xxx
export OPENAI_BASE_URL=https://api.openai.com/v1   # 可省略
export OPENAI_MODEL=gpt-4o-mini                    # 可省略
cargo run --release

# 方式 B: 界面配置 — 打开 http://localhost:8080 后在「LLM 设置」面板填写

# OCR 截图识别 (Python 微服务)
cd ocr && pip install -r requirements.txt && python app.py   # 端口 5001
```

</details>

### 方式三：安装为全局命令

```bash
cargo install --path .
minesweeper-agent serve        # 或直接运行 minesweeper-agent
```

## 📖 使用

1. 打开 Web 界面，三种方式录入局面：
   - **粘贴（推荐）**：对话输入框按 `Ctrl+V` / `Cmd+V`，截图自动 OCR、文本棋盘（`? F 0-8`）自动解析
   - **手动编辑**：左键递增数字（1→8），右键插旗 🚩，`Shift+左键` 清零
   - **加载示例**：下拉菜单载入经典残局
2. 点击 **⚡ 分析局面**，棋盘上绿色 = 必安全、红色 = 必雷、蓝色百分比 = 雷概率
3. 点击推理链步骤，棋盘橙色高亮该步涉及的格子；悬停格子查看概率与依据
4. **🎓 教学模式**：不给答案，逐步引导提问；**📤 导出结果**：下载 Markdown 报告

## CLI

```bash
minesweeper-agent serve [--port 8080] [--mc-iterations 0]   # Web 服务, 0=动态自适应(推荐)
minesweeper-agent analyze -i tests/sample_board.json --mode answer
minesweeper-agent ocr -i screenshot.png                      # 需 OCR 服务
minesweeper-agent import -i tests/sample_board.json
```

## API

| 端点 | 说明 |
|------|------|
| `POST /api/analyze` | 分析局面：`{ "board": [[1,-1,-1],...], "remaining_mines": 3, "mode": "answer\|teaching\|strategy", "use_llm": false, "llm_config": {"api_key":"sk-...","base_url":"...","model":"..."} }` |
| `POST /api/ocr` | 截图识别代理：`{ "image": "<base64>" }` → `{ "board": [[...]], "remaining_mines": n }` |
| `POST /api/llm/test` | 测试 LLM 连接：`{ "llm_config": {"api_key":"...","base_url":"...","model":"..."} }` → `{ "success": true, "model": "gpt-4o-mini" }` |
| `GET /api/llm/status` | 查询环境变量 LLM 配置状态（不泄露 key） |
| `POST /api/debug` | OCR 调试：返回每格的颜色/形态分析细节，便于排查误识别 |
| `POST /api/learn` | 用户反馈学习：`{ "image": "<base64>", "digit": n }` → 加入模板库 |
| `GET /api/health` | 健康检查 |

**棋盘编码**：`-1` = 未知，`-2` = 旗帜，`0-8` = 已翻开数字（`board[行][列]`）。

## 🤖 LLM 大模型集成

LLM 负责将 Rust 推理引擎的计算结果（IR）翻译为人类可读的策略语言，支持**答案 / 教学 / 策略**三种模式。

### 配置方式

| 方式 | 说明 | 优先级 |
|------|------|--------|
| **界面配置** | 左侧「LLM 设置」面板填写 API Key / Base URL / 模型，保存到 localStorage | 高 |
| **环境变量** | `OPENAI_API_KEY` / `OPENAI_BASE_URL` / `OPENAI_MODEL` | 低 |
| **均未配置** | 自动回退到本地分析引擎（纯 Rust 规则转译） | — |

### 支持的 API 提供商（任何 OpenAI 兼容接口）

| 提供商 | Base URL | 推荐模型 |
|--------|----------|----------|
| OpenAI | `https://api.openai.com/v1` | `gpt-4o-mini` |
| DeepSeek | `https://api.deepseek.com/v1` | `deepseek-chat` |
| Kimi / Moonshot | `https://api.moonshot.cn/v1` | `moonshot-v1-8k` |
| Ollama 本地 | `http://localhost:11434/v1` | `qwen2.5:7b` |
| 通义千问 | `https://dashscope.aliyuncs.com/compatible-mode/v1` | `qwen-plus` |

界面面板提供一键预设按钮，点击自动填入 Base URL 和模型名。

### Rust ↔ LLM 交互流程

```
PlayerView (棋盘) → Rust 推理引擎 → InferenceIR (JSON)
                                          ↓
                        Translator.build_system_prompt()  → system prompt
                        Translator.build_user_message()   → user message (含 IR + 合法坐标)
                                          ↓
                        LLMClient.chat() → OpenAI 兼容 API
                                          ↓
                        LLM 响应 → validate_llm_output() 坐标幻觉检测
                                          ↓
                        前端展示分析文本
```

**数据防火墙**：传给 LLM 的信息绝不包含未翻开格子的真实雷藏。所有概率均为数学推断，非事后诸葛亮。LLM 输出经过坐标幻觉检测，引用不存在坐标时会被标记。

## 架构

```
┌─────────────────────────────────────────────────────────┐
│              用户交互层 (Frontend - Web)                   │
│  截图粘贴 (OCR) / 棋盘手动编辑器 / 推理高亮展示 & 对话      │
└─────────────────────────┬───────────────────────────────┘
                          │ JSON over HTTP
                          ▼
┌──────────────────┐  ┌──────────────────────────────────────┐
│  OCR 微服务      │  │     Rust 推理核心 + Web 服务           │
│  (Python/Flask)  │  │  ├─ 确定性约束传播 (子集推理)           │
│  OpenCV+Tesseract│◄─┤  ├─ 蒙特卡洛概率模拟 (动态迭代)        │
│  截图→棋盘数组    │  │  ├─ 连通区域分析 (并查集)              │
└──────────────────┘  │  └─ LLM 战略转译 (3种模式)             │
                      └──────────────────────────────────────┘
                                   │
                                   ▼
                        推理 IR (JSON) → 自然语言
```

蒙特卡洛动态迭代：未知格 `< 50` → 50 000 次；`50~200` → 10 000 次；`> 200` → 1 000 次。

## 项目结构

```
minesweeper/
├── src/
│   ├── engine/           # 推理引擎
│   │   ├── deterministic.rs  # 确定性约束传播 + 子集推理
│   │   ├── probabilistic.rs  # 蒙特卡洛模拟 (动态迭代)
│   │   └── region.rs         # 连通区域分析 (并查集)
│   ├── model/            # PlayerView / Coord / InferenceIR
│   ├── llm/              # LLM 转译 (translator + client)
│   ├── server/           # Web 服务 (axum)
│   └── cli.rs            # CLI (clap)
├── static/index.html     # 前端 (Canvas 棋盘 + 对话引导区)
├── ocr/                  # OCR 微服务 (Flask + OpenCV + Tesseract)
│   └── app.py            # 三层回退: 颜色投票 → 7段形态匹配 → Tesseract
├── tests/                # 集成测试
│   ├── integration_test.rs  # Rust 推理引擎测试
│   └── test_ocr.py          # OCR 数字识别测试
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
| [Metasweeper](https://github.com/Green-Cat-Games/Metasweeper) | OBR（光学局面识别）思路参考 |

## License

MIT
