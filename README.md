# 扫雷认知助手 (Minesweeper Cognitive Agent)

Rust 推理引擎 + LLM 翻译层 + 前端棋盘编辑器 + OCR 截图识别

## 架构

```
┌─────────────────────────────────────────────────────────┐
│              用户交互层 (Frontend - Web)                   │
│  截图上传 (OCR) / 棋盘手动编辑器 / 推理高亮展示 & 对话      │
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

## 核心特性

### 双引擎推理

- **确定性约束传播**：类似数独的约束传播 + 子集推理规则，找出所有必雷/必安全格，生成完整推理证明链
- **蒙特卡洛概率模拟**：对未知格子进行随机雷分配统计，输出每格的雷概率和期望信息增益
  - **动态迭代次数**：未知格 <50 → 50 000 次；50~200 → 10 000 次；>200 → 1 000 次
  - 30×30 大棋盘 **< 150ms** (release build)

### LLM 转译层

三种模式将推理 IR 翻译为人类可理解的语言：

| 模式 | 说明 |
|------|------|
| **答案模式** | 将证明链转为"因为...所以..."的因果推理文字 |
| **教学模式** | 将证明链倒置，生成逐步引导问题，禁止直接给答案 |
| **策略模式** | 基于概率和收益数据，分析各区域利弊，给出坐标级建议 |

**数据防火墙**：所有传给 LLM 的信息均不包含未翻开格子的真实雷藏，杜绝马后炮。坐标幻觉检测器验证 LLM 输出只引用 IR 中存在的坐标。

### OCR 截图识别

Python 微服务 (Flask + OpenCV + Tesseract)：
1. 从截图中自动检测棋盘区域
2. 检测网格线，分割为行列单元格
3. 颜色分析识别旗帜/未翻开状态
4. Tesseract OCR + 模板匹配识别数字 0-8

## 快速开始

### 方式一：Docker Compose (推荐)

```bash
docker compose up
```

打开 http://localhost:8080 即可使用。OCR 服务运行在端口 5001。

### 方式二：本地运行

**1. 启动 Rust 后端**

```bash
cargo run --release -- serve
```

服务启动在 http://localhost:8080，蒙特卡洛为动态自适应模式。

**2. (可选) 启动 OCR 微服务**

```bash
cd ocr
pip install -r requirements.txt
python app.py
```

**3. 使用 LLM (可选)**

```bash
export OPENAI_API_KEY=sk-xxx
export OPENAI_BASE_URL=https://api.openai.com/v1  # 或 DeepSeek 等
export OPENAI_MODEL=gpt-4o-mini
cargo run --release -- serve
```

## CLI 命令

```bash
# 启动 Web 服务
minesweeper-agent serve --port 8080 --mc-iterations 0  # 0=动态自适应

# 从 JSON 文件分析局面
minesweeper-agent analyze -i tests/sample_board.json --mode answer

# 从截图识别棋盘 (需 OCR 服务)
minesweeper-agent ocr -i screenshot.png

# 导入 JSON 并输出 PlayerView
minesweeper-agent import -i tests/sample_board.json
```

## API

### POST /api/analyze

分析扫雷棋盘，返回推理结果和自然语言分析。

```json
{
  "board": [[1,-1,-1], [-1,2,-1], [0,-1,-1]],
  "remaining_mines": 3,
  "mode": "answer",
  "use_llm": false
}
```

响应：

```json
{
  "success": true,
  "view": { "width": 3, "height": 3, ... },
  "ir": { "deterministic": [...], "probabilities": [...], "regions": [...] },
  "analysis": "【必雷格子】\n- 坐标(1, 0) 是雷...",
  "used_llm": false
}
```

### POST /api/ocr

截图识别代理（转发给 OCR 微服务）。

```json
{ "image": "<base64编码的图像>" }
```

### GET /api/health

健康检查。

## 棋盘编码

| 值 | 含义 |
|----|------|
| -1 | 未知 (未翻开) |
| -2 | 旗帜 |
| 0-8 | 已翻开数字 |

## 技术栈

| 模块 | 技术 |
|------|------|
| 推理核心 | Rust (serde, rand, rayon, axum) |
| OCR | Python (Flask, OpenCV, Tesseract) |
| LLM | OpenAI 兼容 API (reqwest) |
| 前端 | HTML/JS Canvas |
| 部署 | Docker Compose |

## 项目结构

```
minesweeper/
├── src/
│   ├── engine/           # 推理引擎
│   │   ├── deterministic.rs  # 确定性约束传播 + 子集推理
│   │   ├── probabilistic.rs  # 蒙特卡洛模拟 (动态迭代)
│   │   └── region.rs         # 连通区域分析 (并查集)
│   ├── model/            # 数据模型
│   │   ├── coord.rs          # 坐标
│   │   ├── player_view.rs    # 玩家视角局面
│   │   └── inference_ir.rs   # 推理中间语言 (IR)
│   ├── llm/              # LLM 转译层
│   │   ├── translator.rs     # IR→自然语言 (3模式)
│   │   └── client.rs         # LLM API 客户端
│   ├── server/           # Web 服务 (axum)
│   └── cli.rs            # CLI 命令 (clap)
├── ocr/                 # OCR 微服务
│   ├── app.py               # Flask + OpenCV + Tesseract
│   ├── requirements.txt
│   └── Dockerfile
├── static/              # 前端
│   └── index.html
├── tests/               # 集成测试
├── Dockerfile
├── docker-compose.yml
└── Cargo.toml
```

## License

MIT
