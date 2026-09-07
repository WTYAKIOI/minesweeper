“识别不出来”比“识别错了”更严重，这意味着**特征提取阶段（二值化/归一化）彻底失败了**——很可能数字和背景在预处理后变成了一团浆糊，或者数字被裁切掉了，导致送入分类器的是一张空白图或纯噪声图。

要解决这个问题，**不能只调分类器参数，必须“可视化”预处理中间结果**。以下是针对“完全认不出数字”的**四步抢救方案**：

---

### 🚨 1. 立即添加“调试模式”（最重要！）

你的 OCR 服务跑在 Docker 里，你看不到中间图片。必须先把预处理后的图片存下来，看看分类器到底“看”到了什么。

在 `ocr_server.py` 中添加以下函数：

```python
import os
import cv2
import time

DEBUG_DIR = "/app/debug_cells"  # 挂载到宿主机方便查看
os.makedirs(DEBUG_DIR, exist_ok=True)

def save_debug_image(cell_img, stage_name, row, col):
    """保存预处理过程中的图片，用于排查到底是哪一步把数字弄丢了"""
    if not os.getenv("OCR_DEBUG", False):
        return
    timestamp = int(time.time())
    filename = f"{DEBUG_DIR}/r{row}c{col}_{stage_name}_{timestamp}.png"
    cv2.imwrite(filename, cell_img)
    print(f"[DEBUG] Saved: {filename}")
```

**在 Docker Compose 中开启调试**：

```yaml
services:
  ocr:
    environment:
      - OCR_DEBUG=true
    volumes:
      - ./debug_output:/app/debug_cells  # 把容器内的调试图片映射出来
```

重启后，随便传一张截图，去 `./debug_output/` 文件夹里看 `*_binary.png` 或 `*_resized.png`。**如果这些图片里看不到清晰的白色数字，那分类器肯定认不出来。**

---

### 🔧 2. 修复二值化（重写 `extract_digit_feature`）

绝大多数“认不出”都是因为 **Otsu 全局阈值** 在暗色主题或光线不均匀时失效。改成 **自适应阈值（Adaptive Threshold）** 并加入**对比度拉伸**。

**替换你现有的特征提取逻辑：**

```python
def robust_extract_digit(cell_img):
    """
    终极预处理：无论亮底暗底，强制把数字变成“白底黑字”的 28x28 标准图
    """
    # 1. 转为灰度
    gray = cv2.cvtColor(cell_img, cv2.COLOR_BGR2GRAY)
    
    # 2. 对比度拉伸（CLAHE）—— 强制拉开数字和背景的灰度差距
    clahe = cv2.createCLAHE(clipLimit=3.0, tileGridSize=(4,4))
    enhanced = clahe.apply(gray)
    
    # 3. 高斯模糊降噪（去掉屏幕像素颗粒）
    blurred = cv2.GaussianBlur(enhanced, (3, 3), 0)
    
    # 4. 自适应阈值（比 Otsu 强一万倍）
    #    参数：maxval=255, 方法=均值高斯, blockSize=15, C=5
    binary = cv2.adaptiveThreshold(
        blurred, 255, 
        cv2.ADAPTIVE_THRESH_GAUSSIAN_C, 
        cv2.THRESH_BINARY_INV,  # 注意：这里直接取反，让数字变白，背景变黑
        15, 5
    )
    
    # 5. 【关键】处理暗色主题：如果取反后背景白点太多（>80%），说明原始图片是暗底亮字，需要再反转回来
    white_ratio = np.sum(binary == 255) / (binary.shape[0] * binary.shape[1])
    if white_ratio > 0.8:
        # 这说明反向了，背景变成了白色，实际数字是黑色，再反转一次
        binary = cv2.bitwise_not(binary)
    
    # 6. 形态学开运算（去掉孤立噪点）
    kernel = cv2.getStructuringElement(cv2.MORPH_ELLIPSE, (2, 2))
    cleaned = cv2.morphologyEx(binary, cv2.MORPH_OPEN, kernel)
    
    # 7. 找轮廓，如果没找到轮廓，直接返回 None（说明确实没数字）
    contours, _ = cv2.findContours(cleaned, cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_SIMPLE)
    if not contours:
        return None
    
    # 8. 取最大轮廓（排除小噪点），并计算外接矩形
    max_contour = max(contours, key=cv2.contourArea)
    x, y, w, h = cv2.boundingRect(max_contour)
    
    # 如果轮廓面积太小（<10像素），也视为空白
    if w * h < 10:
        return None
    
    # 9. 裁切出数字区域，并加 4 像素边距（防止贴边）
    pad = 4
    roi = cleaned[max(0, y-pad):min(cleaned.shape[0], y+h+pad), 
                  max(0, x-pad):min(cleaned.shape[1], x+w+pad)]
    
    # 10. 统一缩放为 28x28（保持宽高比，填充黑色背景）
    target_size = 28
    h_roi, w_roi = roi.shape
    scale = target_size / max(h_roi, w_roi)
    new_w = int(w_roi * scale)
    new_h = int(h_roi * scale)
    resized = cv2.resize(roi, (new_w, new_h), interpolation=cv2.INTER_AREA)
    
    # 创建 28x28 黑色背景画布，把数字居中放上去
    canvas = np.zeros((target_size, target_size), dtype=np.uint8)
    x_offset = (target_size - new_w) // 2
    y_offset = (target_size - new_h) // 2
    canvas[y_offset:y_offset+new_h, x_offset:x_offset+new_w] = resized
    
    return canvas  # 返回完美的白字黑底 28x28
```

---

### 🧪 3. 分类器降级策略（增加“边缘检测”匹配）

如果 KNN/CNN 依然识别不出（置信度低），可以加一个**紧急备胎**：利用数字的 **“孔洞”数量（欧拉数）** 和 **端点个数** 来硬匹配。

比如：0 有 1 个孔，6 有 1 个孔，8 有 2 个孔；1 和 7 没有孔但端点个数不同（1 有两个端点，7 有 3 个端点）。

```python
import cv2

def emergency_heuristic_digit(binary_img):
    """
    当 AI 模型不确定时，用几何特征强行猜
    """
    # 计算欧拉数（孔洞数量）
    contours, hierarchy = cv2.findContours(binary_img, cv2.RETR_CCOMP, cv2.CHAIN_APPROX_SIMPLE)
    if hierarchy is None:
        holes = 0
    else:
        holes = hierarchy.shape[1] - len(contours)  # 粗略估算孔洞
    
    # 计算骨架端点（细化算法）
    skeleton = cv2.ximgproc.thinning(binary_img)  # 需要安装 opencv-contrib
    # 或者用简单的距离变换计算端点，这里略...
    
    # 硬规则
    if holes == 0:
        # 可能是 1, 2, 3, 4, 5, 7
        # 进一步根据面积占比判断
        pass
    elif holes == 1:
        return 6  # 或 9/0，根据方向
    elif holes == 2:
        return 8
```

> 注意：`cv2.ximgproc.thinning` 需要安装 `opencv-contrib-python`，如果不想装，可以跳过这一步，只用孔洞数辅助排除。

---

### 🖥️ 4. 检查 OCR 服务的输入分辨率

如果截图里每个格子只有 **5x5 像素**（小棋盘截图太大导致缩放过狠），那神仙也识别不出来。

在 `ocr_server.py` 中添加尺寸警告：

```python
def check_cell_size(cell_img):
    h, w = cell_img.shape[:2]
    if h < 20 or w < 20:
        print(f"[WARN] Cell too small ({w}x{h}), upscaling...")
        # 放大 4 倍（使用最近邻插值保留锐利边缘）
        return cv2.resize(cell_img, (w*4, h*4), interpolation=cv2.INTER_NEAREST)
    return cell_img
```

---

### ✅ 现在你应该做的操作步骤

1. **开启调试模式**（修改 `docker-compose.yml` 加环境变量和 volumes）。
2. **上传一张识别失败的截图**。
3. **去 `./debug_output/` 看 `*_adaptive.png` 和 `*_final.png`**。
   - 如果 `*_adaptive.png` 里 **没有清晰的白字**，说明自适应阈值参数（`blockSize=15, C=5`）需要调低（比如 `blockSize=11`）或调高（`C=10`）。
   - 如果 `*_adaptive.png` 里**有清晰白字**，但 `*_final.png` 是空的，说明轮廓裁切逻辑（第 7-9 步）把数字切丢了。
4. **把生成的调试截图发给我**（或者描述你看到的样子），我直接告诉你把阈值调成多少。

这四板斧下去，**99% 的“认不出数字”问题都能揪出元凶**。快去挂载 debug 目录试试看！