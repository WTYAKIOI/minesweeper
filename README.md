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

**方式 A — 界面配置（推荐）**：打开 http://localhost:8080 后，点击导航栏「🤖 LLM」展开「⚙️ API 配置」面板，从快速预设选择提供商（OpenAI / DeepSeek / Kimi / OpenRouter / Ollama 本地 / 通义千问 / 自定义），填入 Base URL / 模型名 / API Key，点击「🔗 测试连接」验证后「💾 保存配置」。

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

# 方式 B: 界面配置 — 打开 http://localhost:8080 后点击「🤖 LLM」进入 API 配置面板填写

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
| `POST /api/analyze` | 分析局面：`{ "board": [[1,-1,-1],...], "remaining_mines": 3, "mode": "answer\|teaching\|strategy", "use_llm": false, "language": "zh\|en", "question": "可选提问", "llm_config": {"api_key":"sk-...","base_url":"...","model":"...","max_tokens":4096,"temperature":0.3} }` |
| `POST /api/ocr` | 截图识别代理：`{ "image": "<base64>" }` → `{ "board": [[...]], "remaining_mines": n }` |
| `POST /api/llm/test` | 测试 LLM 连接：`{ "llm_config": {...} }` → `{ "success": true, "model": "openai/gpt-4o-mini" }` |
| `POST /api/llm/models` | 从网关获取模型列表（后端代理 `/models`，避免浏览器 CORS） |
| `GET /api/llm/status` | 查询环境变量 LLM 配置状态（不泄露 key） |
| `GET /api/usage/stats` | Token 用量统计（今日/近7日/本月/累计 + 模型分布 + 趋势） |
| `GET /api/usage/logs` | Token 用量明细日志（`?limit=&offset=`） |
| `POST /api/debug` | OCR 调试：返回每格的颜色/形态分析细节，便于排查误识别 |
| `POST /api/learn` | 用户反馈学习：`{ "image": "<base64>", "digit": n }` → 加入模板库 |
| `GET /api/health` | 健康检查 |

**棋盘编码**：`-1` = 未知，`-2` = 旗帜，`0-8` = 已翻开数字（`board[行][列]`）。

## 🤖 LLM 大模型集成

LLM 负责将 Rust 推理引擎的计算结果（IR）翻译为人类可读的策略语言，支持**答案 / 教学 / 策略**三种模式。
同时自动记录每次调用的 token 用量（见下方「Token 用量统计」）。

### 配置方式

| 方式 | 说明 | 优先级 |
|------|------|--------|
| **界面配置** | 导航栏「🤖 LLM」→「⚙️ API 配置」面板：预设 / Base URL / 模型 / API Key / 最大 Token / 温度，点击「测试连接」验证 | 高 |
| **环境变量** | `OPENAI_API_KEY` / `OPENAI_BASE_URL` / `OPENAI_MODEL` | 低 |
| **均未配置** | 自动回退到本地分析引擎（纯 Rust 规则转译） | — |

> 🔒 **API Key 安全**：界面填写的 Key 仅保存在页面内存中，刷新即清除，不写入任何持久化存储；Base URL / 模型 / 参数等非敏感配置保存在浏览器 localStorage。

### 支持的 API 提供商（任何 OpenAI 兼容接口）

| 提供商 | Base URL | 推荐模型 |
|--------|----------|----------|
| OpenAI | `https://api.openai.com/v1` | `gpt-4o-mini` |
| DeepSeek | `https://api.deepseek.com/v1` | `deepseek-chat` |
| Kimi / Moonshot | `https://api.moonshot.cn/v1` | `moonshot-v1-8k` |
| OpenRouter | `https://openrouter.ai/api/v1` | `openai/gpt-4o-mini` |
| Ollama 本地 | `http://localhost:11434/v1` | `qwen2.5:7b`（免 Key） |
| 通义千问 | `https://dashscope.aliyuncs.com/compatible-mode/v1` | `qwen-plus` |
| 自定义中转网关 | 网关提供的地址 | 网关已配置的模型名 |

界面面板提供一键预设按钮，点击自动填入 Base URL 和模型名。

### 自定义 / 中转网关（OpenRouter 协议、清华 Sub2API 等）

选择「✨ 自定义」进入自由编辑模式，支持：

- **从网关获取模型列表**：点击「🔍 获取模型」，后端代理请求 `{base_url}/models`（Ollama 自动回退 `/api/tags`），下拉框中列出网关真实可用的模型 id —— 彻底避免手写模型名不可用的问题。
- **模型下拉 + 手动输入组合框**：可从获取的列表中选择，也可手动输入。
- **厂商前缀**（可选）：对要求 `vendor/model` 的网关，在高级选项填前缀后，模型名无前缀且请求失败时自动重试 `前缀/模型`：
  - `auto` → 按模型名首段自动猜（`openai` / `anthropic` / `deepseek` / `thu-ai` 等）
  - 具体前缀如 `thu-ai` → 固定使用该前缀重试
  - 留空 → 不补前缀（OpenRouter 类网关除外，仍自动猜测）
- **我的预设**：将 Base URL / 模型 / 参数（不含 Key）保存到浏览器 localStorage，下次一键载入。

示例（清华 Sub2API 风格网关）：

```
Base URL: https://<你的网关>/v1      模型: glm-5     厂商前缀: thu-ai
→ 后端自动重试 thu-ai/glm-5 并成功
```

### 常见问题排查

**连接失败提示 `No proxy configuration found for requested model`**（HTTP 500 类错误）：

- 这是 **OpenRouter / 中转网关** 的典型报错：模型名不带厂商前缀，或该模型在此网关没有可用通道。
- **修复**：模型名需带厂商前缀，如 `gpt-4o-mini` → `openai/gpt-4o-mini`、`glm-5` → `thu-ai/glm-5`。选择「OpenRouter」预设会自动填入正确格式；也可在高级选项填「厂商前缀」（`auto` 或具体前缀如 `thu-ai`）。
- **推荐**：点击「🔍 获取模型」从网关拉取真实模型列表并选择，可彻底避免手写模型名不可用。
- 后端已内置**自动补全重试**：模型名无前缀且首次请求失败时，按厂商前缀配置（OpenRouter 类网关自动猜测 `openai/`、`anthropic/`、`deepseek/`、`thu-ai/` 等）重试一次。
- 若为其他中转/网关（one-api / new-api 等），需填该网关「已配置的模型别名」，并检查渠道是否启用。

**其他常见错误**：

| 提示 | 含义 |
|------|------|
| `Insufficient Balance` / `quota` | 账户余额不足或额度用完 |
| `Invalid API Key` / 401 | Key 错误或已过期 |
| `rate limit` / 429 | 请求频率超限，稍后重试 |
| 网络错误 | Base URL 不可达（本地 Ollama 未启动、代理未开等） |

### 对话语言与生成参数

| 参数 | 默认 | 说明 |
|------|------|------|
| **语言** | 中文 | 界面「分析模式」旁可选 中文 / English；随请求传入后端并注入 System Prompt（`## 语言要求`），教学模式有对应的中/英结构化模板（`## 引导问题` / `## Guiding Questions`） |
| **max_tokens** | **4096** | 单次响应最大生成 token 数。默认 4096（足够教学模式 3~5 问与答案模式长文）；范围 16–32768，可在「API 配置 → 高级选项」调整，或通过 `LLMConfig.max_tokens` / 请求体传入 |
| **temperature** | **0.3** | 采样温度，影响输出的随机性/创造力。**建议**：扫雷推理属于逻辑任务，保持 **0 ~ 0.5**（确定性更高、坐标幻觉更少）；如需更活泼的措辞可调到 0.7 以上，但不建议超过 1.0，否则容易编造坐标。可在「API 配置 → 高级选项」调整 |

### 教学模式：智能引导与完整性保障

- **模式选择规则**：只有显式开启「🎓 教学模式」时提问才进入教学模式；未开启时一律使用界面下拉所选模式（答案/策略）直接回答，**不做关键词隐式跳转**（不再出现"没开教学却收到引导问题"）。
- **智能自适应提示词**（teachmod2.md）：开启教学后 LLM 根据你的提问切换策略，而不是"复读机式"只给问题：
  - 问「为什么 / 原因 / 解释」→ 先给 **1-3 句直接解释**（`## 直接原因`），再附 **1 个思考引导**（`## 思考引导`）
  - 问「怎么做 / 下一步 / 该点哪里」或问题不明确 → 输出 **3-5 个引导性问题**（`## 引导问题`，每题 ≤30 字/词、含精确坐标）
  - 说「我懂了 / 继续 / 下一个」→ 进入更深推理，给 **1-2 个进阶问题**（`## 进阶引导`）
- **输出完整性检测 + 自动重试**：若 LLM 返回为空、或仅输出 `##` 标题而无正文（"输出被吃"），后端自动重试（附带重试提醒），**最多 3 次，不会死循环**；重试耗尽仍不完整则回退到本地引导并附警告。网络/API 错误不触发重试，直接回退本地分析。
- **用户提问透传**：对话区输入的问题（`question` 字段）会随每次分析请求发给 LLM，教学模式围绕你的提问展开引导，不再答非所问。
- **思维链模型兼容**：GLM / DeepSeek-reasoner 等把正文放在 `reasoning_content` 的模型，`content` 为空时自动回退使用思维链文本。
- **调试**：容器/进程加环境变量 `LLM_DEBUG=1` 时打印每次上游调用的完整原始响应（`[LLM Raw Response]`），便于排查"输出被吃"。

### Token 用量统计（v0.2）

每次 LLM 调用后，后端自动从响应 `usage` 字段提取 token 消耗并记录：

- **存储**：本机 `data/usage.jsonl`（JSONL 追加，无原生依赖；可用环境变量 `USAGE_DB_PATH` 覆盖路径）
- **统计维度**：今日 / 近 7 日 / 本月 / 累计 tokens 与估算费用、按模型分布、近 7 天趋势
- **界面**：底部状态栏常驻显示提供商 · 本月费用 · 用量进度条；点击展开「📊 Token 消耗统计」弹窗（支持导出 CSV）
- **费用估算**：内置常见模型单价表（gpt-4o-mini / deepseek-chat / moonshot / qwen 等，每 1M tokens 输入/输出单价），未知模型与本地模型按 $0 计，仅本地估算不上传
- **月度限额**：环境变量 `LLM_MONTHLY_TOKEN_LIMIT`（如 `100000`）设置后，本月用量超限将自动拒绝 LLM 调用并回退本地分析；进度条 70% 变橙、90% 变红提示
- **时区**：统计按日/月分界默认使用 UTC+8，可用 `USAGE_TZ_OFFSET_HOURS` 调整

```bash
LLM_MONTHLY_TOKEN_LIMIT=100000 USAGE_DB_PATH=/data/usage.jsonl cargo run --release
```

| 端点 | 说明 |
|------|------|
| `GET /api/usage/stats` | 用量统计（今日/近7日/本月/累计 + 模型分布 + 趋势 + 限额状态） |
| `GET /api/usage/logs?limit=100&offset=0` | 用量明细（按时间倒序） |

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
│   ├── llm/              # LLM 转译 (translator + client + usage 用量统计)
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
