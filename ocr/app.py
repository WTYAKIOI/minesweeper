"""
扫雷 OCR 微服务 — 光学局面识别 (OBR)

接收扫雷截图，自动检测棋盘、分割格子、识别数字/旗帜/未翻开状态，
返回与 Rust 后端 PlayerView::from_2d 兼容的二维数组。

编码约定 (与 Rust 端一致):
  -1  → 未知 (未翻开)
  -2  → 旗帜
  0-8 → 已翻开数字
"""

import io
import os
import time
import base64
import numpy as np
import cv2
from flask import Flask, request, jsonify

app = Flask(__name__)

TEMPLATE_DIR = os.path.join(os.path.dirname(os.path.abspath(__file__)), "templates")
for sub in ["classic", "dark", "flat", "flags", "learned"]:
    os.makedirs(os.path.join(TEMPLATE_DIR, sub), exist_ok=True)


# ---------------------------------------------------------------------------
# 图像预处理
# ---------------------------------------------------------------------------

def decode_image(data: bytes) -> np.ndarray:
    """将字节流解码为 BGR 格式的 OpenCV 图像"""
    arr = np.frombuffer(data, np.uint8)
    img = cv2.imdecode(arr, cv2.IMREAD_COLOR)
    if img is None:
        raise ValueError("无法解码图像")
    return img


# ---------------------------------------------------------------------------
# 阶段一：自适应颜色匹配（K-means 聚类 + 动态阈值 + 光照归一化）
# ---------------------------------------------------------------------------

def normalize_illumination(image: np.ndarray) -> np.ndarray:
    """直方图均衡化，解决整体偏暗/偏亮的问题"""
    if len(image.shape) == 3:
        yuv = cv2.cvtColor(image, cv2.COLOR_BGR2YUV)
        yuv[:, :, 0] = cv2.equalizeHist(yuv[:, :, 0])
        return cv2.cvtColor(yuv, cv2.COLOR_YUV2BGR)
    else:
        return cv2.equalizeHist(image)


def extract_dominant_colors(image: np.ndarray, n_colors: int = 6) -> np.ndarray:
    """用 K-means 聚类提取图片中的主导颜色，按亮度排序"""
    pixels = image.reshape(-1, 3).astype(np.float32)
    if len(pixels) > 5000:
        idx = np.random.choice(len(pixels), 5000, replace=False)
        pixels = pixels[idx]
    try:
        from sklearn.cluster import KMeans
        kmeans = KMeans(n_clusters=min(n_colors, len(np.unique(pixels, axis=0))),
                        random_state=42, n_init=10)
        kmeans.fit(pixels)
        colors = kmeans.cluster_centers_.astype(int)
    except Exception:
        colors = np.unique(pixels.reshape(-1, 3), axis=0)[:n_colors].astype(int)
    brightness = np.sum(colors, axis=1)
    sorted_idx = np.argsort(brightness)
    return colors[sorted_idx]


def adaptive_color_match(cell_image: np.ndarray):
    """根据图片自适应判断数字颜色，返回 (数字, 置信度)"""
    b, g, r = cell_image[:, :, 0].astype(int), cell_image[:, :, 1].astype(int), cell_image[:, :, 2].astype(int)
    is_gray = (abs(r - g) < 25) & (abs(g - b) < 25) & (abs(r - b) < 25)
    colored = ~is_gray

    if not colored.any():
        return None, 0.0

    # 只对彩色像素做聚类，避免边框/背景干扰
    cb, cg, cr = b[colored], g[colored], r[colored]
    colored_pixels = np.stack([cb, cg, cr], axis=1).astype(np.float32)

    if len(colored_pixels) > 5000:
        idx = np.random.choice(len(colored_pixels), 5000, replace=False)
        colored_pixels = colored_pixels[idx]

    try:
        from sklearn.cluster import KMeans
        n_clusters = min(3, len(np.unique(colored_pixels, axis=0)))
        if n_clusters < 1:
            return None, 0.0
        kmeans = KMeans(n_clusters=n_clusters, random_state=42, n_init=10)
        kmeans.fit(colored_pixels)
        colors = kmeans.cluster_centers_.astype(int)
    except Exception:
        return None, 0.0

    # 取最大的聚类中心作为数字颜色
    labels = kmeans.labels_
    counts = np.bincount(labels)
    main_idx = np.argmax(counts)
    fg_color = colors[main_idx]

    known_digit_colors = {
        1: np.array([0, 0, 255]),
        2: np.array([0, 128, 0]),
        3: np.array([255, 0, 0]),
        4: np.array([0, 0, 128]),
        5: np.array([128, 0, 0]),
        6: np.array([0, 128, 128]),
        7: np.array([0, 0, 0]),
        8: np.array([128, 128, 128]),
    }

    best_match = None
    best_dist = float('inf')
    for digit, color in known_digit_colors.items():
        dist = np.linalg.norm(fg_color - color)
        if dist < best_dist:
            best_dist = dist
            best_match = digit

    if best_dist > 100:
        return None, 0.0
    confidence = max(0.0, 1.0 - best_dist / 100.0)
    return best_match, confidence


# ---------------------------------------------------------------------------
# 棋盘检测
# ---------------------------------------------------------------------------

def detect_board(image: np.ndarray):
    """
    检测棋盘区域，返回裁剪后的棋盘图像。
    策略一：寻找最大的矩形轮廓。
    策略二（回退）：用彩色像素包围盒定位棋盘。
    """
    gray = cv2.cvtColor(image, cv2.COLOR_BGR2GRAY)
    blurred = cv2.GaussianBlur(gray, (5, 5), 0)
    edges = cv2.Canny(blurred, 30, 120)
    kernel = cv2.getStructuringElement(cv2.MORPH_RECT, (5, 5))
    closed = cv2.morphologyEx(edges, cv2.MORPH_CLOSE, kernel)
    contours, _ = cv2.findContours(closed, cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_SIMPLE)

    img_area = image.shape[0] * image.shape[1]
    best_rect = None
    best_score = 0

    for cnt in contours:
        area = cv2.contourArea(cnt)
        if area < img_area * 0.05:
            continue
        x, y, w, h = cv2.boundingRect(cnt)
        aspect = max(w, h) / max(1, min(w, h))
        if aspect > 5:
            continue
        fill_ratio = area / (w * h)
        score = area * fill_ratio
        if score > best_score:
            best_score = score
            best_rect = (x, y, w, h)

    if best_rect is not None:
        x, y, w, h = best_rect
        pad = max(2, int(min(w, h) * 0.02))
        x = max(0, x - pad)
        y = max(0, y - pad)
        w = min(image.shape[1] - x, w + 2 * pad)
        h = min(image.shape[0] - y, h + 2 * pad)
        return image[y:y + h, x:x + w]

    # 回退：用彩色像素包围盒定位棋盘
    b_ch = image[:, :, 0].astype(int)
    g_ch = image[:, :, 1].astype(int)
    r_ch = image[:, :, 2].astype(int)
    is_gray_pix = (abs(r_ch - g_ch) < 25) & (abs(g_ch - b_ch) < 25) & (abs(r_ch - b_ch) < 25)
    colored_pix = ~is_gray_pix
    if colored_pix.any():
        rows, cols = np.where(colored_pix)
        y1, y2 = max(0, rows.min() - 5), min(image.shape[0], rows.max() + 6)
        x1, x2 = max(0, cols.min() - 5), min(image.shape[1], cols.max() + 6)
        return image[y1:y2, x1:x2]

    return image


# ---------------------------------------------------------------------------
# 网格分割
# ---------------------------------------------------------------------------

def detect_grid_lines(board: np.ndarray):
    """
    检测水平线和垂直线，推断行列数和单元格大小。
    返回 (rows, cols, cell_w, cell_h)
    策略一：形态学线检测
    策略二（回退）：自相关分析 + 标准棋盘尺寸匹配
    """
    gray = cv2.cvtColor(board, cv2.COLOR_BGR2GRAY)
    h, w = gray.shape[:2]

    # --- 策略一：形态学线检测 ---
    thresh = cv2.adaptiveThreshold(
        gray, 255, cv2.ADAPTIVE_THRESH_GAUSSIAN_C,
        cv2.THRESH_BINARY_INV, 15, 2
    )
    h_kernel_len = max(3, w // 20)
    h_kernel = cv2.getStructuringElement(cv2.MORPH_RECT, (h_kernel_len, 1))
    h_lines = cv2.morphologyEx(thresh, cv2.MORPH_OPEN, h_kernel)
    v_kernel_len = max(3, h // 20)
    v_kernel = cv2.getStructuringElement(cv2.MORPH_RECT, (1, v_kernel_len))
    v_lines = cv2.morphologyEx(thresh, cv2.MORPH_OPEN, v_kernel)

    h_proj = np.sum(h_lines, axis=1)
    h_peaks = _find_peaks(h_proj, threshold=h_proj.max() * 0.3)
    v_proj = np.sum(v_lines, axis=0)
    v_peaks = _find_peaks(v_proj, threshold=v_proj.max() * 0.3)

    if len(h_peaks) >= 2 and len(v_peaks) >= 2:
        rows = len(h_peaks) - 1
        cols = len(v_peaks) - 1
        h_diffs = np.diff(h_peaks)
        v_diffs = np.diff(v_peaks)
        cell_h = int(np.median(h_diffs)) if len(h_diffs) > 0 else h // rows
        cell_w = int(np.median(v_diffs)) if len(v_diffs) > 0 else w // cols
        # 验证：行列数必须是标准尺寸
        if (rows, cols) in [(9, 9), (16, 16), (30, 16)] or (5 <= rows <= 30 and 5 <= cols <= 30):
            return rows, cols, cell_w, cell_h

    # --- 策略二：自相关分析 ---
    def _autocorr_period(profile, min_p=15, max_p=100):
        prof = profile - np.mean(profile)
        ac = np.correlate(prof, prof, 'full')
        ac = ac[len(ac) // 2:]
        if ac[0] == 0:
            return 0, 0
        ac = ac / ac[0]
        best_p, best_s = 0, 0
        for p in range(min_p, min(max_p, len(ac))):
            if ac[p] > best_s:
                best_s = ac[p]
                best_p = p
        return best_p, best_s

    row_prof = np.mean(gray, axis=1)
    col_prof = np.mean(gray, axis=0)
    rp, rs = _autocorr_period(row_prof)
    cp, cs = _autocorr_period(col_prof)

    if rp > 0 and cp > 0:
        est_rows = round(h / rp)
        est_cols = round(w / cp)
        # 验证是否为标准棋盘尺寸
        for (cols, rows) in [(30, 16), (16, 16), (9, 9)]:
            if abs(rows - est_rows) <= 2 and abs(cols - est_cols) <= 2:
                return rows, cols, cp, rp

    # --- 策略三：标准棋盘尺寸匹配 ---
    aspect = w / max(1, h)
    for (cols, rows) in [(30, 16), (16, 16), (9, 9)]:
        expected_aspect = cols / rows
        if abs(aspect - expected_aspect) < 0.3:
            return rows, cols, w // cols, h // rows

    # 最终回退
    if aspect > 1.5:
        cols, rows = 30, 16
    elif aspect < 1.1:
        cols, rows = 16, 16
        if min(w, h) < 200:
            cols, rows = 9, 9
    else:
        cols, rows = 16, 16
    return rows, cols, w // cols, h // rows


def _find_peaks(proj, threshold=0):
    """找到投影数组中的峰值位置"""
    peaks = []
    in_peak = False
    start = 0
    for i, v in enumerate(proj):
        if v > threshold and not in_peak:
            in_peak = True
            start = i
        elif v <= threshold and in_peak:
            in_peak = False
            peaks.append((start + i) // 2)
    if in_peak:
        peaks.append((start + len(proj) - 1) // 2)
    return peaks


def segment_cells(board: np.ndarray, rows: int, cols: int, cell_w: int, cell_h: int):
    """
    将棋盘图像分割成 rows×cols 个单元格图像。
    返回二维列表 cell_images[y][x] = ndarray
    """
    cells = []
    for r in range(rows):
        row_cells = []
        for c in range(cols):
            x1 = c * cell_w
            y1 = r * cell_h
            x2 = x1 + cell_w
            y2 = y1 + cell_h
            cell_img = board[y1:y2, x1:x2]
            row_cells.append(cell_img)
        cells.append(row_cells)
    return cells


# ---------------------------------------------------------------------------
# 单元格分类
# ---------------------------------------------------------------------------

def classify_cell(cell: np.ndarray) -> int:
    """
    分类单元格状态。
    返回: -1 (未知), -2 (旗帜), 0-8 (数字)

    回退链：经典颜色匹配 → 自适应颜色(K-means) → 模板匹配 → Tesseract
    """
    h, w = cell.shape[:2]
    pad = max(2, min(h, w) // 8)
    inner = cell[pad:h - pad, pad:w - pad]
    if inner.size == 0:
        return -1

    b, g, r = inner[:, :, 0].astype(int), inner[:, :, 1].astype(int), inner[:, :, 2].astype(int)
    total = inner.shape[0] * inner.shape[1]
    gray = cv2.cvtColor(inner, cv2.COLOR_BGR2GRAY)

    is_gray = (abs(r - g) < 25) & (abs(g - b) < 25) & (abs(r - b) < 25)
    colored_count = np.count_nonzero(~is_gray)

    vals, counts = np.unique(gray, return_counts=True)
    mode_val = int(vals[np.argmax(counts)])

    # 无彩色像素：区分未翻开 vs 已翻开空格
    if colored_count < total * 0.005:
        if mode_val < 175:
            return -1
        else:
            return 0

    # 有彩色像素：旗帜或数字
    cb, cg, cr = b[~is_gray], g[~is_gray], r[~is_gray]
    mean_b, mean_g, mean_r = np.mean(cb), np.mean(cg), np.mean(cr)
    red_mask = (r > 120) & (g < 80) & (b < 80)
    red_ratio = np.count_nonzero(red_mask) / total
    bg_is_revealed = mode_val > 175

    # 红色 + 未翻开背景 → 旗帜
    if mean_r > 120 and mean_b < 80 and mean_g < 80:
        if not bg_is_revealed and (colored_count > total * 0.1 or red_ratio > 0.03):
            return -2
        elif bg_is_revealed:
            return 3

    # 第一层：经典颜色匹配
    result = _color_to_number(mean_b, mean_g, mean_r, is_gray, total)
    if result > 0:
        return result

    # 第二层：三层回退 (自适应颜色 → 模板 → Tesseract)
    theme = detect_theme(inner)
    result, method, conf = ocr_with_fallback(cell, theme)
    if result is not None and 1 <= result <= 8:
        return result

    return 0


def _color_to_number(mean_b, mean_g, mean_r, is_gray, total):
    """根据彩色像素的平均 BGR 值识别数字 1-8"""
    if mean_b > 120 and mean_r < 100 and mean_g < 100:
        return 1  # 蓝
    if mean_g > 80 and mean_g < 200 and mean_r < 100 and mean_b < 100:
        return 2  # 绿
    if mean_r > 150 and mean_b < 100 and mean_g < 100:
        return 3  # 红 (已翻开背景上的)
    if mean_b > 80 and mean_b < 180 and mean_r < 50 and mean_g < 50:
        return 4  # 深蓝
    if mean_r > 80 and mean_r < 180 and mean_b < 50 and mean_g < 50:
        return 5  # 暗红
    if mean_b > 80 and mean_b < 180 and mean_g > 80 and mean_g < 180 and mean_r < 50:
        return 6  # 青
    if mean_r < 50 and mean_g < 50 and mean_b < 50:
        return 7  # 黑
    if 100 < mean_b < 170 and 100 < mean_g < 170 and 100 < mean_r < 170:
        return 8  # 灰

    # 扩展匹配 (兼容不同配色方案)
    if mean_b > mean_r + 20 and mean_b > mean_g + 20:
        return 1 if mean_b > 150 else 4
    if mean_g > mean_r + 20 and mean_g > mean_b + 20:
        return 2
    if mean_r > mean_b + 20 and mean_r > mean_g + 20:
        return 3 if mean_r > 150 else 5
    if mean_b > 80 and mean_g > 80 and mean_r < 50:
        return 6

    return 0


def recognize_number(cell: np.ndarray) -> int:
    """
    识别单元格中的数字 (0-8)。
    优先使用颜色匹配，回退到 Tesseract OCR。
    """
    h, w = cell.shape[:2]
    b, g, r = cell[:, :, 0].astype(int), cell[:, :, 1].astype(int), cell[:, :, 2].astype(int)
    total = cell.shape[0] * cell.shape[1]
    is_gray = (abs(r - g) < 25) & (abs(g - b) < 25) & (abs(r - b) < 25)
    colored = ~is_gray

    if np.any(colored):
        cb, cg, cr = b[colored], g[colored], r[colored]
        mb, mg, mr = np.mean(cb), np.mean(cg), np.mean(cr)
        result = _color_to_number(mb, mg, mr, is_gray, total)
        if result > 0:
            return result

    # 回退：Tesseract OCR
    gray = cv2.cvtColor(cell, cv2.COLOR_BGR2GRAY)
    scale = max(2, 32 // min(h, w) + 1)
    big = cv2.resize(gray, (w * scale, h * scale), interpolation=cv2.INTER_CUBIC)
    _, binary = cv2.threshold(big, 0, 255, cv2.THRESH_BINARY + cv2.THRESH_OTSU)
    binary_inv = cv2.bitwise_not(binary)

    for config in ["--psm 10 -c tessedit_char_whitelist=012345678", "--psm 8 -c tessedit_char_whitelist=012345678"]:
        for img in [binary, binary_inv]:
            try:
                import pytesseract
                text = pytesseract.image_to_string(img, config=config).strip()
                if text and text.isdigit():
                    n = int(text)
                    if 0 <= n <= 8:
                        return n
            except Exception:
                pass

    return _template_match(gray)


# 颜色模板 (数字在典型扫雷游戏中的颜色)
NUMBER_COLORS = {
    1: [(0, 0, 255), (0, 0, 200)],      # 蓝色
    2: [(0, 128, 0), (0, 100, 0)],       # 绿色
    3: [(0, 0, 255), (0, 0, 180)],       # 红色 (某些主题)
    4: [(128, 0, 128), (100, 0, 100)],   # 紫色
    5: [(0, 0, 128), (0, 0, 100)],       # 深红
    6: [(0, 128, 128), (0, 100, 100)],   # 青色
    7: [(0, 0, 0), (30, 30, 30)],        # 黑色
    8: [(128, 128, 128), (100, 100, 100)],# 灰色
}


def _template_match(gray: np.ndarray) -> int:
    """
    回退数字识别：基于像素密度和连通域。
    如果找不到明确数字，返回 0 (空格 = 周围无雷)。
    """
    # 二值化
    _, binary = cv2.threshold(gray, 0, 255, cv2.THRESH_BINARY + cv2.THRESH_OTSU)

    # 如果几乎全白 (已翻开但无数字)，返回 0
    white_ratio = np.count_nonzero(binary) / binary.size
    if white_ratio > 0.95:
        return 0

    # 如果几乎全黑 (可能是未翻开)，返回 -1
    if white_ratio < 0.05:
        return -1

    # 统计连通域数量来粗略判断数字
    num_labels, _ = cv2.connectedComponents(binary)
    # 1 个背景 + 1 个数字 = 2 个连通域 → 单个数字
    if num_labels <= 2:
        # 难以确定具体数字，回退到 0
        return 0

    return 0


# ---------------------------------------------------------------------------
# 阶段二：模板匹配多主题支持
# ---------------------------------------------------------------------------

def detect_theme(empty_cell: np.ndarray) -> str:
    """根据未翻开格子的样式判断主题"""
    mean_color = np.mean(empty_cell, axis=(0, 1)) if empty_cell.ndim == 3 else np.mean(empty_cell)
    brightness = np.mean(mean_color)
    if brightness > 200:
        return 'classic'
    elif brightness < 50:
        return 'dark'
    else:
        return 'flat'


def template_match_digit(cell_gray: np.ndarray, theme: str = 'classic'):
    """用模板匹配识别数字，返回 (数字, 置信度)"""
    h, w = cell_gray.shape[:2]
    best_match = None
    best_score = 0.0
    theme_dir = os.path.join(TEMPLATE_DIR, theme)
    learned_dir = os.path.join(TEMPLATE_DIR, "learned")

    for search_dir in [theme_dir, learned_dir]:
        if not os.path.isdir(search_dir):
            continue
        for fname in os.listdir(search_dir):
            if not fname.startswith("digit_") or not fname.endswith(".png"):
                continue
            try:
                digit = int(fname.replace("digit_", "").split("_")[0].split(".")[0])
            except ValueError:
                continue
            if not (1 <= digit <= 8):
                continue
            tpath = os.path.join(search_dir, fname)
            template = cv2.imread(tpath, cv2.IMREAD_GRAYSCALE)
            if template is None:
                continue
            if template.shape[0] != h or template.shape[1] != w:
                template = cv2.resize(template, (w, h))
            result = cv2.matchTemplate(cell_gray, template, cv2.TM_CCOEFF_NORMED)
            score = float(np.max(result))
            if score > best_score:
                best_score = score
                best_match = digit

    if best_score > 0.7:
        return best_match, best_score
    return None, best_score


# ---------------------------------------------------------------------------
# 阶段四：用户反馈学习机制
# ---------------------------------------------------------------------------

def auto_learn_template(cell_image: np.ndarray, confirmed_digit: int, theme: str = 'learned'):
    """当用户确认某个识别正确时，将其加入模板库"""
    if cell_image.ndim == 3:
        gray = cv2.cvtColor(cell_image, cv2.COLOR_BGR2GRAY)
    else:
        gray = cell_image
    ts = int(time.time() * 1000)
    save_dir = os.path.join(TEMPLATE_DIR, theme)
    os.makedirs(save_dir, exist_ok=True)
    template_path = os.path.join(save_dir, f"digit_{confirmed_digit}_{ts}.png")
    cv2.imwrite(template_path, gray)
    return template_path


# ---------------------------------------------------------------------------
# 阶段三：三层回退链 (颜色匹配 → 模板匹配 → Tesseract)
# ---------------------------------------------------------------------------

def ocr_with_fallback(cell_image: np.ndarray, theme: str = 'classic'):
    """
    三层回退识别数字。
    返回: (数字或None, 使用的方法名, 置信度)
    """
    h, w = cell_image.shape[:2]
    pad = max(2, min(h, w) // 8)
    inner = cell_image[pad:h - pad, pad:w - pad]
    if inner.size == 0:
        return None, 'failed', 0.0

    gray = cv2.cvtColor(inner, cv2.COLOR_BGR2GRAY)

    # 1. 自适应颜色匹配
    result, conf = adaptive_color_match(inner)
    if result is not None and 1 <= result <= 8 and conf > 0.3:
        return result, 'color_match', conf

    # 2. 模板匹配
    result, conf = template_match_digit(gray, theme)
    if result is not None and 1 <= result <= 8 and conf > 0.7:
        return result, 'template_match', conf

    # 3. Tesseract OCR
    try:
        scale = max(2, 32 // min(inner.shape[:2]) + 1)
        big = cv2.resize(gray, (gray.shape[1] * scale, gray.shape[0] * scale),
                         interpolation=cv2.INTER_CUBIC)
        _, binary = cv2.threshold(big, 0, 255, cv2.THRESH_BINARY + cv2.THRESH_OTSU)
        binary_inv = cv2.bitwise_not(binary)
        import pytesseract
        for config in ["--psm 10 -c tessedit_char_whitelist=012345678",
                        "--psm 8 -c tessedit_char_whitelist=012345678"]:
            for img in [binary, binary_inv]:
                text = pytesseract.image_to_string(img, config=config).strip()
                if text and text.isdigit():
                    n = int(text)
                    if 1 <= n <= 8:
                        return n, 'tesseract', 0.8
    except Exception:
        pass

    return None, 'failed', 0.0

def _find_best_offset(image, cell_w, cell_h, rows, cols):
    """在整图上搜索最佳网格偏移 (最大化彩色像素)，采样加速"""
    h, w = image.shape[:2]
    if cols * cell_w > w or rows * cell_h > h:
        return 0, 0, 0

    best_score = 0
    best_ox, best_oy = 0, 0

    step_x = max(1, cell_w // 4)
    step_y = max(1, cell_h // 4)
    max_ox = min(cell_w, w - cols * cell_w + 1)
    max_oy = min(cell_h, h - rows * cell_h + 1)

    for oy in range(0, max_oy, step_y):
        for ox in range(0, max_ox, step_x):
            score = 0
            r_step = max(1, rows // 8)
            c_step = max(1, cols // 8)
            for r in range(0, rows, r_step):
                for c in range(0, cols, c_step):
                    y1 = oy + r * cell_h
                    x1 = ox + c * cell_w
                    if y1 + cell_h > h or x1 + cell_w > w:
                        continue
                    cell = image[y1:y1 + cell_h, x1:x1 + cell_w]
                    pad = max(2, min(cell_h, cell_w) // 8)
                    inner = cell[pad:cell_h - pad, pad:cell_w - pad]
                    if inner.size == 0:
                        continue
                    b, g, r2 = inner[:, :, 0].astype(int), inner[:, :, 1].astype(int), inner[:, :, 2].astype(int)
                    is_gray = (abs(r2 - g) < 25) & (abs(g - b) < 25) & (abs(r2 - b) < 25)
                    score += np.count_nonzero(~is_gray)
            # 归一化：除以采样单元格数，再乘以总单元格数的比例因子
            n_sampled = ((rows + r_step - 1) // r_step) * ((cols + c_step - 1) // c_step)
            if n_sampled > 0:
                score = score * (rows * cols) / (n_sampled * cell_w * cell_h)
            if score > best_score:
                best_score = score
                best_ox, best_oy = ox, oy
    return best_ox, best_oy, best_score


def recognize_board(image: np.ndarray):
    """
    完整的棋盘识别流程。
    返回:
      board: 二维 int 列表 (board[y][x])
      meta: dict (rows, cols, cell_w, cell_h, offset_x, offset_y)
    """
    h, w = image.shape[:2]

    # 光照归一化预处理 (仅对偏暗/偏亮图片)
    gray_img = cv2.cvtColor(image, cv2.COLOR_BGR2GRAY)
    mean_brightness = np.mean(gray_img)
    if mean_brightness < 80 or mean_brightness > 220:
        normalized = normalize_illumination(image)
    else:
        normalized = image

    # 尝试标准棋盘尺寸 (9x9, 16x16, 30x16) 和一系列单元格大小
    standard_sizes = [(30, 16), (16, 16), (9, 9)]
    candidates = []

    for cols, rows in standard_sizes:
        min_cw = max(16, w // (cols + 2))
        max_cw = min(100, w // max(1, cols - 2))
        min_ch = max(16, h // (rows + 2))
        max_ch = min(100, h // max(1, rows - 2))

        for cs in range(min(min_cw, min_ch), min(max_cw, max_ch) + 1, 2):
            if cols * cs > w or rows * cs > h:
                continue
            ox, oy, score = _find_best_offset(normalized, cs, cs, rows, cols)
            if score > 0:
                candidates.append((score, cs, rows, cols, ox, oy))

    candidates.sort(key=lambda x: -x[0])

    if candidates:
        score, cs, rows, cols, ox, oy = candidates[0]
    else:
        board_img = detect_board(normalized)
        rows, cols, cell_w, cell_h = detect_grid_lines(board_img)
        ox, oy = 0, 0
        cs = max(cell_w, cell_h)
        normalized = board_img

    # 分割并分类单元格
    board = []
    for r in range(rows):
        row = []
        for c in range(cols):
            y1 = oy + r * cs
            x1 = ox + c * cs
            if y1 + cs > normalized.shape[0] or x1 + cs > normalized.shape[1]:
                row.append(-1)
                continue
            cell = normalized[y1:y1 + cs, x1:x1 + cs]
            val = classify_cell(cell)
            row.append(val)
        board.append(row)

    meta = {"rows": rows, "cols": cols, "cell_w": cs, "cell_h": cs,
            "offset_x": ox, "offset_y": oy}
    return board, meta


# ---------------------------------------------------------------------------
# Flask 路由
# ---------------------------------------------------------------------------

@app.route("/api/ocr", methods=["POST"])
def ocr():
    """接收截图，返回识别后的棋盘"""
    try:
        if "image" in request.files:
            data = request.files["image"].read()
        elif "image" in request.json:
            # base64 编码
            b64 = request.json["image"]
            if "," in b64:
                b64 = b64.split(",", 1)[1]
            data = base64.b64decode(b64)
        else:
            data = request.get_data()

        if not data:
            return jsonify({"error": "未收到图像数据"}), 400

        image = decode_image(data)
        board, meta = recognize_board(image)

        return jsonify({
            "success": True,
            "board": board,
            "remaining_mines": None,  # 需要用户手动输入或从雷数显示识别
            "meta": meta,
        })
    except Exception as e:
        return jsonify({"success": False, "error": str(e)}), 500


@app.route("/api/health", methods=["GET"])
def health():
    return "ok"


@app.route("/api/learn", methods=["POST"])
def learn():
    """用户反馈学习：接收确认的单元格图片和数字，加入模板库"""
    try:
        data = request.json
        if not data or "image" not in data or "digit" not in data:
            return jsonify({"success": False, "error": "缺少 image 或 digit 参数"}), 400

        b64 = data["image"]
        if "," in b64:
            b64 = b64.split(",", 1)[1]
        img_data = base64.b64decode(b64)
        digit = int(data["digit"])
        if not (1 <= digit <= 8):
            return jsonify({"success": False, "error": "digit 必须在 1-8 之间"}), 400

        theme = data.get("theme", "learned")
        img = decode_image(img_data)
        path = auto_learn_template(img, digit, theme)
        return jsonify({"success": True, "template_path": path})
    except Exception as e:
        return jsonify({"success": False, "error": str(e)}), 500


if __name__ == "__main__":
    app.run(host="0.0.0.0", port=5001, debug=True)
