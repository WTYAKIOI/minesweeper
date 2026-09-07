可以。先说明一点：**前两张棋盘的识别并不是调用了某个独立的 Tesseract/PaddleOCR 服务**，而是我直接对你上传的棋盘截图进行视觉解析。因此如果你要把这个过程做成一个**可以自动扫描扫雷棋盘的程序**，更适合采用“OpenCV 棋盘定位 + 格子分类 + OCR/模板识别”的方案，而不是直接对整张图跑 OCR。

下面是一个比较可靠的实现方案。

## 1. 推荐 OCR 环境

如果目标是这种固定 UI 的扫雷游戏，我推荐：

| 组件         | 推荐环境            | 用途            |
| ---------- | --------------- | ------------- |
| Python     | 3.11+           | 主程序           |
| OpenCV     | `opencv-python` | 棋盘定位、切格子、图像处理 |
| NumPy      | 最新版             | 图像矩阵          |
| PaddleOCR  | 3.x             | 数字 OCR，作为辅助   |
| Tesseract  | 5.x             | 可选备用 OCR      |
| PIL/Pillow | 最新版             | 图片读取/处理       |

但有一个关键点：

**这种扫雷棋盘其实不应该主要依赖 OCR。**

因为你的棋盘具有非常强的规则性：

```text
F = 旗帜
0 = 灰色空白
1 = 蓝色数字
2 = 绿色数字
3 = 红色数字
4 = 深蓝色数字
U = 凸起的未打开格
```

数字甚至可以通过**颜色 + 连通区域 + 模板匹配**直接识别。

所以最稳定的架构是：

```text
截图
 ↓
OpenCV 检测棋盘区域
 ↓
确定行列数
 ↓
切成 16 × 30 = 480 个 cell
 ↓
每个 cell 独立分类
 ↓
 ┌───────────────┐
 │ 是否未打开？  │ → U
 ├───────────────┤
 │ 是否有旗帜？  │ → F
 ├───────────────┤
 │ 是否有数字？  │ → 颜色/模板/OCR
 ├───────────────┤
 │ 没有内容      │ → 0
 └───────────────┘
 ↓
输出二维数组
```

---

# 2. 第一步：确定棋盘网格

你的截图非常适合做这个。

第一张图可以观察到：

```text
行 = 16
列 = 30
```

也就是标准的：

```text
16 × 30
```

因此不需要逐个寻找方块。

如果知道棋盘左上角 `(x0, y0)`，以及单格尺寸：

```text
cell_w
cell_h
```

那么第 `r` 行、第 `c` 列：

```python
x1 = x0 + c * cell_w
y1 = y0 + r * cell_h

x2 = x1 + cell_w
y2 = y1 + cell_h
```

直接切出来。

这比让 OCR 自己寻找数字位置可靠得多。

---

# 3. 检测 U / 已打开格

这是整个识别过程中非常重要的一步。

你的截图中：

### 未打开格 `U`

具有明显的 3D 凸起效果：

```text
┌──────────┐
│ 白色高光  │
│          │
│   灰色   │
│          │
└──────────┘
```

而已经打开的格子：

```text
┌──────────┐
│          │
│   灰色   │
│          │
└──────────┘
```

因此可以在 cell 边缘检测：

```python
top_mean
left_mean
bottom_mean
right_mean
```

如果上边和左边存在明显亮色高光，同时内部比较暗，就可以判断：

```python
U
```

伪代码：

```python
if top_highlight > THRESHOLD and left_highlight > THRESHOLD:
    return "U"
```

实际实现时最好不要只看一个像素，而是取边缘区域的平均亮度。

---

# 4. 检测旗帜 F

旗帜反而比数字更容易。

你的旗帜是红色，因此可以使用 HSV：

```python
hsv = cv2.cvtColor(cell, cv2.COLOR_BGR2HSV)
```

建立红色 mask：

```python
mask1 = cv2.inRange(
    hsv,
    np.array([0, 100, 80]),
    np.array([10, 255, 255])
)

mask2 = cv2.inRange(
    hsv,
    np.array([170, 100, 80]),
    np.array([180, 255, 255])
)

red_mask = mask1 | mask2
```

然后计算：

```python
red_pixels = np.count_nonzero(red_mask)
```

如果红色像素数量超过阈值：

```python
if red_pixels > FLAG_THRESHOLD:
    return "F"
```

这样根本不需要 OCR。

---

# 5. 数字识别

数字可以进一步利用**颜色编码**。

从你截图的扫雷 UI 可以发现：

```text
1 → 蓝色
2 → 绿色
3 → 红色
4 → 深蓝色
```

所以实际上可以先根据文字颜色判断数字类别。

例如：

```python
blue → 1
green → 2
red → 3
dark_blue → 4
```

如果游戏只出现：

```text
1 2 3 4
```

那么几乎可以做到不使用 OCR。

例如：

```python
def classify_number(cell):

    hsv = cv2.cvtColor(cell, cv2.COLOR_BGR2HSV)

    # 提取高饱和度文字
    mask = cv2.inRange(
        hsv,
        np.array([0, 80, 50]),
        np.array([180, 255, 255])
    )

    pixels = cell[mask > 0]

    if len(pixels) == 0:
        return "0"

    # 根据 RGB/BGR 或 HSV 判断主色
    ...
```

---

# 6. 为什么建议 OCR 作为第二层

因为不同扫雷实现的颜色可能变化。

比如：

```text
1 = 蓝色
2 = 绿色
3 = 红色
4 = 深蓝
5 = 棕色
6 = 青色
7 = 黑色
8 = 灰色
```

如果颜色规则未知，就可以把 cell 内的数字区域送给 OCR。

例如 PaddleOCR：

```python
from paddleocr import PaddleOCR

ocr = PaddleOCR(
    lang="en"
)
```

然后：

```python
result = ocr.predict(cell)
```

不过这里有一个非常重要的优化：

**不要把整张 1536×988 截图直接扔给 OCR。**

应该：

```text
1536×988 screenshot
       ↓
16×30 grid
       ↓
单个 cell
       ↓
裁掉边框
       ↓
放大 3～5 倍
       ↓
OCR
```

这样 OCR 的准确率会高很多。

---

# 7. 最推荐的识别优先级

我会做成：

```text
             ┌── 红色大量像素 ──→ F
             │
Cell ────────┼── 3D 高光 ──────→ U
             │
             ├── 彩色文字 ─────→ 1/2/3/4...
             │
             ├── OCR ──────────→ 数字
             │
             └── 无文字 ───────→ 0
```

也就是说：

### 第一优先级：几何特征

判断：

```text
U / 非 U
```

### 第二优先级：红色

判断：

```text
F / 非 F
```

### 第三优先级：颜色

判断：

```text
1 / 2 / 3 / 4 / ...
```

### 第四优先级：OCR

处理颜色规则无法确定的情况。

---

# 8. 一个完整扫描函数

最终可以把整个过程封装成：

```python
def scan_board(image):

    board = locate_board(image)

    rows = 16
    cols = 30

    result = []

    for r in range(rows):

        row = []

        for c in range(cols):

            cell = extract_cell(
                board,
                r,
                c
            )

            if is_unopened(cell):
                value = "U"

            elif has_flag(cell):
                value = "F"

            elif has_number(cell):

                value = recognize_number(cell)

            else:
                value = "0"

            row.append(value)

        result.append(row)

    return result
```

最终：

```python
board = scan_board(image)

for row in board:
    print("".join(row))
```

输出就是你前面要求的：

```text
UUUUUUUUUUUUU2112F32F2F22222F1
UUUUUUUUUUUUU3F23F4F234F3FF211
UUUUUUUUUUUUUU22F3F322FF322100
...
```

---

# 9. 更进一步：自动确定 16×30

如果以后不保证一定是 `16×30`，可以让程序自动检测。

思路是检测棋盘上的方块边界。

例如利用灰度图：

```python
gray = cv2.cvtColor(image, cv2.COLOR_BGR2GRAY)

edges = cv2.Canny(gray, 50, 150)

lines = cv2.HoughLinesP(
    edges,
    1,
    np.pi / 180,
    threshold=50
)
```

然后统计：

```text
垂直线位置
水平线位置
```

得到：

```text
x0 x1 x2 x3 ... x30
y0 y1 y2 ... y16
```

于是：

```text
30 列
16 行
```

就自动出来了。

对于这种固定 UI，我其实更推荐**第一次人工校准棋盘坐标，以后直接固定网格参数**，可靠性会更高。

---

# 10. 实际工程结构

如果你准备真正做一个“截图 → 棋盘状态”的程序，我建议项目结构：

```text
minesweeper_ocr/
│
├── main.py
│
├── capture.py
│   └── 截屏
│
├── board.py
│   ├── locate_board()
│   ├── detect_grid()
│   └── split_cells()
│
├── classifier.py
│   ├── is_unopened()
│   ├── has_flag()
│   ├── recognize_color_number()
│   └── recognize_ocr()
│
├── templates/
│   ├── 1.png
│   ├── 2.png
│   ├── 3.png
│   └── ...
│
└── output.py
    └── 输出 16×30 字符矩阵
```

---

## 11. 对你这个具体扫雷游戏，我甚至建议不用 OCR

因为你给出的两张图具有**完全一致的 UI 风格**。

可以做一次模板采集：

```text
0.png
1.png
2.png
3.png
4.png
F.png
U.png
```

然后每个 cell：

```text
resize
 ↓
灰度化
 ↓
归一化
 ↓
template matching
 ↓
最高匹配模板
```

甚至可以达到：

```text
U → 99%+
F → 99%+
0 → 99%+
1 → 99%+
2 → 99%+
3 → 99%+
4 → 99%+
```

而且**比通用 OCR 更适合扫雷这种固定字体、固定尺寸、固定颜色的场景**。

如果你的最终目的是进一步做到 **“自动截图 → 识别棋盘 → 根据扫雷逻辑计算安全格/雷 → 输出下一步”**，那么下一步应该把识别器设计成：

```text
截图
 ↓
16×30 Grid Parser
 ↓
Board Matrix
 ↓
Minesweeper Constraint Solver
 ↓
安全格 / 必雷格
 ↓
下一步操作
```

这样就不只是 OCR，而是一套完整的扫雷棋盘识别与求解器。
