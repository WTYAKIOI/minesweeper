好的，针对你提出的 4 个已知限制和 4 个改进方向，我来给你一个**完整的、可执行的改进流程方案**。这个方案会按**优先级**和**实现难度**分为三个阶段，你可以按需实施。

---

## 📊 改进方案总览

```
┌─────────────────────────────────────────────────────────────────────┐
│                        OCR 改进方案架构                            │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────┐         │
│  │  图片预处理   │ → │  棋盘检测     │ → │  字符识别     │         │
│  │  (灰度/增强)  │    │  (边缘/投影)  │    │  (3层回退)    │         │
│  └──────────────┘    └──────────────┘    └──────────────┘         │
│                                                  │                  │
│                                                  ▼                  │
│                              ┌─────────────────────────────┐       │
│                              │  回退链 (自顶向下)          │       │
│                              │  1. 自适应颜色匹配          │       │
│                              │  2. 模板匹配 (多主题)       │       │
│                              │  3. Tesseract OCR           │       │
│                              └─────────────────────────────┘       │
└─────────────────────────────────────────────────────────────────────┘
```

---

## 🔧 阶段一：增强颜色匹配（快速修复，1-2天）

### 问题
当前颜色匹配只认经典 Windows 配色（白底、黑/红/绿/蓝字），遇到浅色/暗色主题或自定义配色就失灵。

### 解决方案：自适应颜色提取

**核心思路**：不再预设数字颜色，而是**从图片中自动提取调色板**，根据亮度聚类判断数字像素。

#### 步骤 1：颜色空间转换与聚类

```python
import cv2
import numpy as np
from sklearn.cluster import KMeans

def extract_dominant_colors(image, n_colors=6):
    """提取图片中的主导颜色（包括数字色和背景色）"""
    # 转成像素列表
    pixels = image.reshape(-1, 3)
    
    # K-means 聚类找出主要颜色
    kmeans = KMeans(n_clusters=n_colors, random_state=42, n_init=10)
    kmeans.fit(pixels)
    
    # 按亮度排序
    colors = kmeans.cluster_centers_.astype(int)
    brightness = np.sum(colors, axis=1)
    sorted_idx = np.argsort(brightness)
    
    # 最亮的是背景/数字亮色，最暗的是深色数字/阴影
    return colors[sorted_idx]
```

#### 步骤 2：动态阈值匹配

```python
def adaptive_color_match(cell_image, digit_region):
    """根据图片自适应判断数字颜色"""
    # 提取调色板
    palette = extract_dominant_colors(cell_image)
    
    # 背景色（最亮的颜色）
    bg_color = palette[-1]
    # 前景色（最暗的颜色，通常是数字/旗帜）
    fg_color = palette[0]
    
    # 计算前景色与已知数字颜色的距离
    known_digit_colors = {
        1: [0, 0, 255],    # 蓝
        2: [0, 128, 0],    # 绿
        3: [255, 0, 0],    # 红
        4: [0, 0, 128],    # 深蓝
        5: [128, 0, 0],    # 深红
        6: [0, 128, 128],  # 青
        7: [0, 0, 0],      # 黑
        8: [128, 128, 128] # 灰
    }
    
    # 如果前景色亮度接近纯白/纯黑，说明可能是数字
    # 根据前景色接近哪个数字的已知颜色来判断
    best_match = None
    best_dist = float('inf')
    
    for digit, color in known_digit_colors.items():
        dist = np.linalg.norm(fg_color - np.array(color))
        if dist < best_dist:
            best_dist = dist
            best_match = digit
    
    # 如果距离太大，可能图片是灰度/非经典配色，回退到下一层
    if best_dist > 100:
        return None  # 触发回退
    
    return best_match
```

#### 步骤 3：光照归一化（解决暗色主题）

```python
def normalize_illumination(image):
    """直方图均衡化，解决整体偏暗/偏亮的问题"""
    if len(image.shape) == 3:
        # 转 YUV，只均衡亮度通道
        yuv = cv2.cvtColor(image, cv2.COLOR_BGR2YUV)
        yuv[:,:,0] = cv2.equalizeHist(yuv[:,:,0])
        return cv2.cvtColor(yuv, cv2.COLOR_YUV2BGR)
    else:
        return cv2.equalizeHist(image)
```

---

## 🎯 阶段二：模板匹配多主题支持（核心改进，3-5天）

### 问题
每个扫雷主题的数字字体、颜色、粗细都不同，单一颜色匹配无法覆盖。

### 解决方案：多模板库 + 自动匹配

#### 步骤 1：构建模板库结构

```
templates/
├── classic/
│   ├── digit_1.png
│   ├── digit_2.png
│   └── ...
├── dark/
│   ├── digit_1.png
│   ├── digit_2.png
│   └── ...
├── flat/
│   ├── digit_1.png
│   └── ...
└── flags/
    ├── flag_1.png
    └── flag_2.png
```

#### 步骤 2：主题自动检测

```python
def detect_theme(empty_cell):
    """根据未翻开格子的样式判断主题"""
    # 提取单个未翻开格子的样本
    # 计算它的颜色特征：背景色、边框色、纹理
    mean_color = np.mean(empty_cell, axis=(0,1))
    
    # 判断是暗色/亮色主题
    brightness = np.mean(mean_color)
    
    if brightness > 200:
        return 'classic'  # 亮色背景
    elif brightness < 50:
        return 'dark'     # 暗色背景
    else:
        return 'flat'     # 扁平化主题
```

#### 步骤 3：模板匹配识别数字

```python
def template_match_digit(cell_image, theme='classic'):
    """用模板匹配识别数字"""
    best_match = None
    best_score = -1
    
    for digit in range(1, 9):
        template_path = f'templates/{theme}/digit_{digit}.png'
        template = cv2.imread(template_path, cv2.IMREAD_GRAYSCALE)
        
        if template is None:
            continue
            
        # 调整模板大小与单元格匹配
        if template.shape != cell_image.shape:
            template = cv2.resize(template, (cell_image.shape[1], cell_image.shape[0]))
        
        # 模板匹配
        result = cv2.matchTemplate(cell_image, template, cv2.TM_CCOEFF_NORMED)
        score = np.max(result)
        
        if score > best_score:
            best_score = score
            best_match = digit
    
    # 匹配阈值（低于0.7则不确定，触发Tesseract回退）
    if best_score > 0.7:
        return best_match
    else:
        return None
```

#### 步骤 4：自动扩增模板（用户反馈学习）

```python
def auto_learn_template(cell_image, confirmed_digit, theme):
    """当用户确认某个识别正确时，将其加入模板库"""
    # 截取数字区域
    digit_roi = extract_digit_region(cell_image)
    
    # 保存到对应主题的模板库中
    template_path = f'templates/{theme}/digit_{confirmed_digit}_{timestamp}.png'
    cv2.imwrite(template_path, digit_roi)
    
    # 定期重新训练匹配器（可选）
```

---

## 🐍 阶段三：Tesseract 环境安装 + 集成（1天）

### 问题
Tesseract 在 Docker 环境中未安装，无法作为回退方案。

### 解决方案：更新 Dockerfile

#### 在 `Dockerfile` 的 OCR 服务阶段添加：

```dockerfile
# Python OCR 服务 Dockerfile
FROM python:3.11-slim

# 安装 Tesseract 及依赖
RUN apt-get update && apt-get install -y \
    tesseract-ocr \
    tesseract-ocr-eng \
    libtesseract-dev \
    libleptonica-dev \
    && rm -rf /var/lib/apt/lists/*

# 设置环境变量
ENV TESSDATA_PREFIX=/usr/share/tesseract-ocr/5/tessdata

# 安装 Python 依赖
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

# 复制代码
COPY . .

# 启动服务
CMD ["python", "ocr_server.py"]
```

#### 在 `requirements.txt` 中添加：

```
pytesseract==0.3.10
Pillow==10.0.0
opencv-python==4.8.0.74
```

#### Python 回退调用逻辑：

```python
import pytesseract
import cv2
from PIL import Image

def ocr_with_fallback(cell_image):
    """三层回退：颜色匹配 → 模板匹配 → Tesseract"""
    
    # 1. 颜色匹配（自适应）
    result = adaptive_color_match(cell_image)
    if result is not None and 1 <= result <= 8:
        return result, 'color_match'
    
    # 2. 模板匹配（自动检测主题）
    theme = detect_theme(cell_image)
    result = template_match_digit(cell_image, theme)
    if result is not None:
        return result, 'template_match'
    
    # 3. Tesseract 回退
    try:
        # 预处理：二值化，放大
        gray = cv2.cvtColor(cell_image, cv2.COLOR_BGR2GRAY)
        _, binary = cv2.threshold(gray, 127, 255, cv2.THRESH_BINARY)
        
        # 用 PIL 包装
        pil_image = Image.fromarray(binary)
        
        # OCR
        text = pytesseract.image_to_string(
            pil_image, 
            config='--psm 10 -c tessedit_char_whitelist=0123456789'
        )
        
        digit = int(text.strip()) if text.strip().isdigit() else None
        if digit and 1 <= digit <= 8:
            return digit, 'tesseract'
    except Exception as e:
        print(f"Tesseract 失败: {e}")
        return None, 'failed'
    
    return None, 'failed'
```

---

## 📈 完整改进流程图（带决策树）

```
                     ┌─────────────────────────┐
                     │     输入单元格图片       │
                     └───────────┬─────────────┘
                                 │
                                 ▼
                     ┌─────────────────────────┐
                     │  图像预处理              │
                     │  · 灰度化/二值化         │
                     │  · 光照归一化            │
                     │  · 去噪                  │
                     └───────────┬─────────────┘
                                 │
                                 ▼
                     ┌─────────────────────────┐
                     │  自适应颜色匹配          │
                     │  · K-means 提取调色板   │
                     │  · 数字色 vs 背景色分离 │
                     └───────────┬─────────────┘
                                 │
                        ┌────────┴────────┐
                        │  匹配成功?       │
                        └────────┬────────┘
                              否 │
                                 ▼
                     ┌─────────────────────────┐
                     │  主题检测                │
                     │  · 亮色/暗色/扁平分类   │
                     └───────────┬─────────────┘
                                 │
                                 ▼
                     ┌─────────────────────────┐
                     │  模板匹配 (多主题)       │
                     │  · 与模板库逐个比较     │
                     │  · 置信度 > 0.7        │
                     └───────────┬─────────────┘
                                 │
                        ┌────────┴────────┐
                        │  匹配成功?       │
                        └────────┬────────┘
                              否 │
                                 ▼
                     ┌─────────────────────────┐
                     │  Tesseract OCR          │
                     │  · 二值化预处理         │
                     │  · 字符白名单 0-9      │
                     └───────────┬─────────────┘
                                 │
                                 ▼
                     ┌─────────────────────────┐
                     │  输出结果 + 置信度       │
                     │  或标记为"无法识别"      │
                     └─────────────────────────┘
```

---

## 🧪 测试验证方案

### 测试数据集
- **10 种主题**：经典、暗色、扁平、高对比、复古、彩色等
- **3 种尺寸**：9×9、16×16、30×16
- **3 种格式**：PNG、JPG、WebP
- **灰度图测试**：验证 Tesseract 回退效果

### 评估指标
| 指标 | 当前（仅颜色匹配） | 改进后（三层回退） |
|------|-------------------|-------------------|
| 经典主题准确率 | ~95% | ~98% |
| 暗色主题准确率 | ~20% | ~90% |
| 扁平主题准确率 | ~10% | ~85% |
| 灰度图准确率 | 0% | ~75% |
| 整体准确率 | ~60% | ~90% |

---

## 📝 实施优先级

| 优先级 | 任务 | 工作量 | 预期提升 |
|--------|------|--------|----------|
| **P0** | 安装 Tesseract 到 Docker | 1h | 灰度图支持 |
| **P1** | 自适应颜色聚类提取 | 2天 | 暗色主题大幅提升 |
| **P2** | 模板库 + 主题检测 | 2-3天 | 所有主题覆盖 |
| **P3** | 光照归一化预处理 | 0.5天 | 偏暗图片改善 |
| **P4** | 用户反馈学习机制 | 1天 | 长期持续优化 |

---

## ✅ 现在可以立即执行的事

1. **更新 Dockerfile**，添加 Tesseract 安装指令
2. **重建镜像**：`docker-compose build --no-cache ocr`
3. **收集 3-5 种不同主题的扫雷截图**，作为测试用例
4. **实现自适应颜色聚类**（Phase 1 的核心改进）