### 1.1 现状

项目已完成 Phase 1 的 OCR 识别模块（基于 OpenCV + Tesseract），在当前测试的扫雷软件上识别效果好。

### 1.2 问题

在更换扫雷软件后（如暗色主题、像素风格、高分辨率版本），识别准确率基本为 0，表现为：

- 数字完全无法识别（输出为空）
- 数字被误识别为其他数字（如 1 ↔ 7、3 ↔ 8、6 ↔ 8）
- 未翻开的格子被误判为数字或旗帜
- 旗帜颜色/形状变化后无法检测

### 1.3 根本原因

当前 OCR 设计存在**严重过拟合**：

| 问题 | 说明 |
|------|------|
| **颜色硬编码** | 预设数字颜色（蓝色=1、绿色=2、红色=3）仅匹配经典主题 |
| **阈值固定** | Otsu 全局阈值在暗色主题下失效，数字和背景融为一体 |
| **字体依赖** | KNN 像素匹配对字体粗细/大小极其敏感 |
| **尺寸死板** | 固定网格分割假设所有软件格子像素一致 |
| **旗帜匹配单一** | 仅匹配红色实心旗，对带标志/彩色旗帜无效 |


## 2. 改进目标

| 目标 | 指标 |
|------|------|
| 经典主题准确率 | ≥ 98%（保持） |
| 暗色主题准确率 | ≥ 90%（从 30% 提升） |
| 扁平/像素主题准确率 | ≥ 85%（从 20% 提升） |
| 灰度图准确率 | ≥ 80%（从 0% 提升） |
| 新软件适配成本 | 无需修改代码，自动适配 |


## 3. 改进方案

### 3.1 整体架构（新增/修改模块）

```
┌─────────────────────────────────────────────────────────────────┐
│                      OCR 识别 Pipeline                         │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────────┐  │
│  │ 1. 棋盘定位   │ → │ 2. 网格分割   │ → │ 3. 皮肤检测      │  │
│  │ (边缘检测)    │    │ (动态投影)    │    │ (自适应参数)     │  │
│  └──────────────┘    └──────────────┘    └──────────────────┘  │
│                                                         │       │
│                                                         ▼       │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────────┐  │
│  │ 6. 分类器     │ ← │ 5. 特征提取   │ ← │ 4. 预处理        │  │
│  │ (HOG + KNN   │    │ (形状/结构)   │    │ (自适应阈值)     │  │
│  │  或 CNN)     │    └──────────────┘    └──────────────────┘  │
│  └──────────────┘                                              │
│         │                                                       │
│         ▼                                                       │
│  ┌──────────────┐    ┌──────────────┐                          │
│  │ 7. 规则后验   │ → │ 8. 输出       │                          │
│  │ (扫雷约束)    │    │ (数字/旗帜)   │                          │
│  └──────────────┘    └──────────────┘                          │
└─────────────────────────────────────────────────────────────────┘
```

### 3.2 关键技术改进

#### 改进 1：自适应预处理（替代固定阈值）

**问题**：Otsu 全局阈值在暗色主题下将数字和背景同时二值化，导致数字丢失。

**方案**：使用 **CLAHE + 自适应阈值** 组合，并增加**亮度反转检测**。

```python
def adaptive_preprocess(cell_img):
    # 1. CLAHE 增强对比度（任何主题都适用）
    clahe = cv2.createCLAHE(clipLimit=2.0, tileGridSize=(8,8))
    enhanced = clahe.apply(cv2.cvtColor(cell_img, cv2.COLOR_BGR2GRAY))
    
    # 2. 自适应阈值（不依赖全局亮度）
    binary = cv2.adaptiveThreshold(
        enhanced, 255,
        cv2.ADAPTIVE_THRESH_GAUSSIAN_C,
        cv2.THRESH_BINARY_INV,
        15, 5
    )
    
    # 3. 自动检测并修正暗色主题（亮底黑字 vs 暗底白字）
    white_ratio = np.sum(binary == 255) / binary.size
    if white_ratio > 0.7:
        binary = cv2.bitwise_not(binary)
    
    return binary
```

#### 改进 2：HOG 特征 + 多分类器集成（替代原始像素 KNN）

**问题**：像素匹配对字体缩放、粗细、抗锯齿极其敏感。

**方案**：提取 **方向梯度直方图（HOG）** 特征，并用 **三分类器集成投票**。

```python
class DigitClassifier:
    def __init__(self):
        self.hog_knn = train_hog_knn()      # HOG + KNN
        self.pixel_knn = train_pixel_knn()  # 像素 KNN（保留作为备选）
        self.cnn = load_onnx_cnn()          # 轻量级 CNN（可选）
    
    def predict(self, cell_img):
        # 多分类器投票
        preprocessed = adaptive_preprocess(cell_img)
        hog_feat = extract_hog(preprocessed)
        pixel_feat = preprocessed.flatten()
        
        pred1 = self.hog_knn.predict(hog_feat)
        pred2 = self.pixel_knn.predict(pixel_feat)
        
        # 如果 HOG 置信度高，直接采用
        if pred1.confidence > 0.85:
            return pred1.digit
        # 否则集成投票
        return majority_vote(pred1, pred2)
```

#### 改进 3：皮肤检测与动态参数（自动适配新软件）

**问题**：每种扫雷软件的格子大小、字体粗细、颜色方案都不同，无法统一配置。

**方案**：在首次处理时自动检测皮肤特征，并缓存配置。

```python
def detect_skin(cell_samples):
    """检测当前扫雷软件的皮肤特征"""
    skin = {}
    
    # 1. 检测明暗主题
    avg_brightness = np.mean([np.mean(cell) for cell in cell_samples])
    skin['dark_mode'] = avg_brightness < 100
    
    # 2. 检测格子大小
    skin['cell_size'] = cell_samples[0].shape[0]
    
    # 3. 检测字体粗细（通过对数字图像的笔画宽度分析）
    skin['stroke_width'] = estimate_stroke_width(cell_samples)
    
    # 4. 检测是否有抗锯齿
    skin['has_antialiasing'] = detect_antialiasing(cell_samples)
    
    # 5. 根据皮肤特征调整自适应阈值参数
    skin['block_size'] = adjust_block_size(skin)
    skin['cliplimit'] = adjust_cliplimit(skin)
    
    return skin
```

#### 改进 4：用户反馈学习（渐进式提升）

**问题**：新皮肤出现时，需要用户手动修正识别错误。

**方案**：用户修正后，自动将正确样本加入模板库，下次遇到同样皮肤直接匹配。

```python
class TemplateLearner:
    def __init__(self):
        self.templates = load_templates()  # 从磁盘加载
    
    def learn(self, cell_img, confirmed_digit, skin_id):
        """用户确认后自动学习"""
        preprocessed = adaptive_preprocess(cell_img)
        hog_feat = extract_hog(preprocessed)
        self.templates[skin_id][confirmed_digit].append(hog_feat)
        # 增量更新 KNN
        self.rebuild_knn(skin_id)
        # 保存到磁盘
        save_templates(self.templates)
```

#### 改进 5：扫雷规则后验校验

**问题**：识别出数字后可能不符合扫雷约束。

**方案**：利用扫雷的数学规则自动修正明显错误。

```python
def validate_with_minesweeper_rules(ocr_grid, flagged):
    """用扫雷约束校验并修正 OCR 结果"""
    for r, row in enumerate(ocr_grid):
        for c, digit in enumerate(row):
            if digit is None or digit == 'FLAG':
                continue
            # 统计周围旗帜和未知格数
            flags = count_nearby(flagged, r, c)
            unknowns = count_nearby(ocr_grid, r, c, is_unknown=True)
            # 规则：digit <= flags + unknowns
            if digit > flags + unknowns:
                ocr_grid[r][c] = flags + unknowns
    return ocr_grid
```


## 4. 实现计划

| 阶段 | 内容 | 优先级 | 预计耗时 |
|------|------|--------|----------|
| **P0** | 自适应阈值替换 Otsu + 亮度反转检测 |
| **P0** | HOG 特征提取 + 简单 KNN 重训练 |
| **P1** | 皮肤检测（亮/暗/尺寸自适应） |
| **P1** | 多分类器集成（HOG + 像素 KNN） |
| **P2** | 扫雷规则后验校验 |
| **P2** | 用户反馈学习机制 |
| **P3** | CNN ONNX 模型替换（终极） |


## 5. 测试验证

### 5.1 测试数据集

| 软件名称 | 主题类型 | 是否用于训练 |
|----------|----------|--------------|
| Windows 11 扫雷 | 亮色经典 | ✅ 训练集 |
| http://www.minesweeper.cn/ | 亮色扁平 | ✅ 训练集 |
| Google 扫雷 | 暗色扁平 | ❌ 测试集 |
| 自定义暗色主题 | 暗色像素 | ❌ 测试集 |
| 灰度截图 | 无彩色 | ❌ 测试集 |

### 5.2 评估指标

| 指标 | 当前（改进前） | 目标（改进后） |
|------|---------------|---------------|
| 经典主题准确率 | 95% | ≥ 98% |
| 暗色主题准确率 | 30% | ≥ 90% |
| 扁平/像素主题 | 20% | ≥ 85% |
| 灰度图准确率 | 0% | ≥ 80% |
| 新软件首次适配 | 手工调参 | 自动检测 + 动态适配 |


## 6. 技术选型

| 组件 | 技术 | 理由 |
|------|------|------|
| 自适应阈值 | OpenCV `adaptiveThreshold` | 内置，无需额外依赖 |
| 对比度增强 | OpenCV `CLAHE` | 处理暗色/低对比度图片 |
| 特征提取 | OpenCV `HOGDescriptor` | 形状特征，抗字体变化 |
| 分类器 | `cv2.ml.KNearest` | 轻量级，无需重训练 |
| 皮肤检测 | 亮度直方图 + 边缘分析 | 规则驱动，无需训练 |
| 用户反馈 | JSON 本地存储 | 简单持久化 |
| 可选 CNN | ONNX Runtime | 未来可替换 |


## 7. 已知限制与未来方向

### 7.1 当前限制

- 极高分辨率截图（每个格子 > 100px）需要缩放处理
- 极端艺术字体（如手写体扫雷）难以识别
- 带有自定义皮肤纹理（如木纹背景）可能影响阈值效果

### 7.2 未来方向

1. **数据增强**：使用合成数据生成大量变体，训练端到端的 CNN
2. **多帧融合**：利用多张截图（同一棋盘不同角度）融合提高识别率
3. **主动学习**：自动识别低置信度样本并请求用户确认
4. **在线更新**：构建社区共享的皮肤模板库


## 8. 附录：代码结构变更

```
ocr_service/
├── preprocess.py          # 【新增】自适应预处理 (CLAHE + 自适应阈值)
├── feature.py             # 【新增】HOG 特征提取
├── classifier.py          # 【修改】多分类器集成 (HOG + 像素 KNN)
├── skin_detector.py       # 【新增】皮肤检测与动态参数
├── template_learner.py    # 【新增】用户反馈学习机制
├── validator.py           # 【新增】扫雷规则后验校验
├── ocr_server.py          # 【修改】集成以上模块
└── templates/             # 【新增】皮肤模板库存储
    ├── skin_001/
    │   ├── digit_1_001.npy
    │   └── ...
    └── config.json        # 皮肤特征配置
```

---