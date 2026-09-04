"""
扫雷 OCR 微服务 — 光学局面识别 (OBR)

接收扫雷截图，自动检测棋盘、分割格子、识别数字/旗帜/未翻开状态，
返回与 Rust 后端 PlayerView::from_2d 兼容的二维数组。

编码约定 (与 Rust 端一致):
  -1  → 未知 (未翻开)
  -2  → 旗帜
  0-8 → 已翻开数字

识别方案 (参考 reference/ 下两个项目):
  - minesweeper_solver: cv2.inRange 颜色范围逐像素投票判数字
    (亮蓝=1 / 绿=2 / 亮红=3 / 深蓝=4 / 深红=5 / 青=6 / 黑=7 / 灰=8 为分离区间)，
    状态按颜色匹配比例优先级判定 (空/旗/未开/数字)，失败回退 Tesseract。
  - Metasweeper (ms_toollib): 经典 Windows 配色 LUT 与 OBR 分割思路。
    本服务在其基础上做了主题自适应: 以格子主体色为基准提取字形像素，
    再对字形像素做颜色区间投票，因此亮色/暗色主题均可识别。
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


def normalize_illumination(image: np.ndarray) -> np.ndarray:
    """直方图均衡化，解决整体偏暗/偏亮的问题"""
    if len(image.shape) == 3:
        yuv = cv2.cvtColor(image, cv2.COLOR_BGR2YUV)
        yuv[:, :, 0] = cv2.equalizeHist(yuv[:, :, 0])
        return cv2.cvtColor(yuv, cv2.COLOR_YUV2BGR)
    else:
        return cv2.equalizeHist(image)


# ---------------------------------------------------------------------------
# 数字颜色分类 (参考 minesweeper_solver/src/config.py 的 NUMBER_COLORS)
#
# 经典 Windows 扫雷配色 (BGR)。关键点: 亮蓝(1)/深蓝(4)、亮红(3)/深红(5)
# 是互不重叠的亮度区间，用逐像素 inRange 投票取最大匹配数，
# 而非均值匹配 —— 均值会被抗锯齿像素抬高，导致 4→1、5→3 的误判。
# 区间在参考项目基础上做了少量展宽，以容纳抗锯齿过渡像素。
# ---------------------------------------------------------------------------

NUMBER_RANGES = {
    1: ((185, 0, 0), (255, 70, 70)),      # 亮蓝
    2: ((0, 90, 0), (60, 150, 60)),       # 绿
    3: ((0, 0, 175), (70, 70, 255)),      # 亮红
    4: ((95, 0, 0), (150, 45, 45)),       # 深蓝
    5: ((0, 0, 90), (45, 45, 145)),       # 深红 (棕)
    6: ((95, 95, 0), (150, 150, 60)),     # 青
    7: ((0, 0, 0), (70, 70, 70)),         # 黑
    8: ((95, 95, 95), (145, 145, 145)),   # 灰
}

# 旗帜: 红旗面 (亮红或深红) + 黑旗杆 (参考 minesweeper_solver: red+black → flag)
_FLAG_RED_LO = np.array([0, 0, 90])
_FLAG_RED_HI = np.array([70, 70, 255])


def extract_glyph_pixels(body: np.ndarray, body_bgr: np.ndarray, dev: int = 25):
    """提取偏离格子主体色的字形像素 (主题自适应)"""
    d = np.abs(body.reshape(-1, 3).astype(int) - body_bgr.astype(int)).max(axis=1)
    return body.reshape(-1, 3)[d > dev]


def detect_number_by_color(glyph: np.ndarray):
    """
    对字形像素做颜色区间投票 (参考 minesweolver_solver 的 _detect_number_by_color)。
    返回 (数字 1-8 或 None, 置信度)
    """
    if len(glyph) < 8:
        return None, 0.0
    votes = {}
    for digit, (lo, hi) in NUMBER_RANGES.items():
        mask = cv2.inRange(glyph.reshape(-1, 1, 3).astype(np.uint8),
                           np.array(lo), np.array(hi))
        n = int(np.count_nonzero(mask))
        if n > 0:
            votes[digit] = n
    if not votes:
        return None, 0.0
    digit = max(votes, key=votes.get)
    conf = votes[digit] / len(glyph)
    # 获胜颜色需覆盖足够比例的字形像素
    if votes[digit] < max(8, 0.12 * len(glyph)):
        return None, conf
    return digit, conf


def _glyph_is_digit_shaped(body: np.ndarray, body_bgr: np.ndarray, dev: int = 25):
    """
    字形形态校验: 数字笔画应紧凑且位于格子中央 (包围盒居中、占比适中)。
    用于排除未翻开格纹理、bevel 残留等大面积/边缘分布的假字形。
    """
    d = np.abs(body.astype(int) - body_bgr.astype(int)).max(axis=2)
    mask = d > dev
    n = int(np.count_nonzero(mask))
    total = mask.size
    if n < max(12, total * 0.02) or n > total * 0.65:
        return False
    ys, xs = np.where(mask)
    h0, h1, w0, w1 = ys.min(), ys.max(), xs.min(), xs.max()
    H, W = mask.shape
    # 包围盒尺寸合理 (数字占中央一部分)
    if not (0.2 * H <= h1 - h0 + 1 <= 0.95 * H):
        return False
    if not (0.12 * W <= w1 - w0 + 1 <= 0.95 * W):
        return False
    # 包围盒中心居中
    if abs((h0 + h1) / 2 - H / 2) > 0.3 * H or abs((w0 + w1) / 2 - W / 2) > 0.3 * W:
        return False
    return True


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


def segment_cells(board: np.ndarray, rows: int, cols: int, cell_w: float, cell_h: float,
                  ox: float = 0.0, oy: float = 0.0):
    """
    将棋盘图像分割成 rows×cols 个单元格图像 (保留全部格子)。
    支持亚像素起点/周期 (先 round 再切)。
    越界部分以 None 占位，由上层标记为未知。
    返回二维列表 cell_images[y][x] = ndarray 或 None
    """
    h, w = board.shape[:2]
    cells = []
    for r in range(rows):
        row_cells = []
        for c in range(cols):
            x1 = int(round(ox + c * cell_w))
            y1 = int(round(oy + r * cell_h))
            x2 = int(round(ox + (c + 1) * cell_w))
            y2 = int(round(oy + (r + 1) * cell_h))
            if x1 < 0 or y1 < 0 or x2 > w or y2 > h or x2 <= x1 or y2 <= y1:
                row_cells.append(None)
            else:
                row_cells.append(board[y1:y2, x1:x2])
        cells.append(row_cells)
    return cells


# ---------------------------------------------------------------------------
# 单元格特征与状态分类 (参考 minesweeper_solver/src/cell_detector.py)
# ---------------------------------------------------------------------------

def cell_features(cell: np.ndarray):
    """
    计算单格特征:
      body_bgr   主体色 (中央 20%~80%)
      body_med   主体灰度中值 (未翻开/已翻开格通常底色不同)
      strips     四条边带 (8%~22%) 的平均灰度 (bevel/边框结构, 用于
                 与本簇主体色对比后判定哪一簇是未翻开格)
      glyph_n    字形像素数 (偏离主体色的像素)
    返回 (body_bgr, body_med, strips, glyph_n) 或 None
    """
    s = min(cell.shape[0], cell.shape[1])
    b0, b1 = int(s * 0.2), int(s * 0.8)
    body = cell[b0:b1, b0:b1]
    if body.size == 0:
        return None
    body_bgr = np.median(body.reshape(-1, 3), axis=0)
    body_med = float(body_bgr.mean())

    g = cv2.cvtColor(cell, cv2.COLOR_BGR2GRAY)
    e0, e1 = int(s * 0.08), int(s * 0.22)
    strips = (
        float(g[e0:e1, e1:s - e1].mean()),          # 上边带
        float(g[s - e1:s - e0, e1:s - e1].mean()),  # 下边带
        float(g[e1:s - e1, e0:e1].mean()),          # 左边带
        float(g[e1:s - e1, s - e1:s - e0].mean()),  # 右边带
    )
    glyph_n = len(extract_glyph_pixels(body, body_bgr))
    return body_bgr, body_med, strips, glyph_n


def _otsu_threshold_1d(vals):
    """一维 Otsu 阈值; 不可分时返回 None"""
    vals = np.asarray(vals, dtype=np.float64)
    if len(vals) < 4 or vals.max() - vals.min() < 1e-6:
        return None
    hist, edges = np.histogram(vals, bins=64, range=(vals.min(), vals.max()))
    centers = (edges[:-1] + edges[1:]) / 2
    total = hist.sum()
    w0 = np.cumsum(hist).astype(float)
    w1 = total - w0
    valid = (w0 > 0) & (w1 > 0)
    if not valid.any():
        return None
    m0 = np.cumsum(hist * centers)
    mt = m0[-1]
    with np.errstate(invalid='ignore', divide='ignore'):
        m0 = m0 / w0
        m1 = (mt - np.cumsum(hist * centers)) / w1
        var_between = w0 * w1 * (m0 - m1) ** 2
    var_between[~valid] = -1
    idx = int(np.argmax(var_between))
    if var_between[idx] <= 0:
        return None
    return float(centers[idx])


def _glyph_red_ratio(glyph: np.ndarray):
    """字形中红色像素占比 (旗帜检测)"""
    if len(glyph) == 0:
        return 0.0
    g = glyph.reshape(-1, 1, 3).astype(np.uint8)
    mask = cv2.inRange(g, _FLAG_RED_LO, _FLAG_RED_HI)
    return np.count_nonzero(mask) / len(glyph)


def classify_cell_with_role(cell: np.ndarray, unopened: bool):
    """
    在已知角色 (未翻开/已翻开) 的前提下分类单格状态。
    返回: -1 (未知), -2 (旗帜), 0-8 (数字)

    参考 minesweeper_solver 的 detect_state 优先级:
      未翻开: 红色字形 → 旗帜; 否则未知
      已翻开: 无字形 → 0; 颜色投票 → 数字; 失败 → 模板 → Tesseract
    """
    feat = cell_features(cell)
    if feat is None:
        return -1
    body_bgr, body_med, strips, _ = feat
    s = min(cell.shape[0], cell.shape[1])
    b0, b1 = int(s * 0.2), int(s * 0.8)
    body = cell[b0:b1, b0:b1]
    glyph = extract_glyph_pixels(body, body_bgr)
    n_glyph = len(glyph)

    if unopened:
        if n_glyph > max(20, body.size * 0.004) and _glyph_red_ratio(glyph) > 0.3:
            return -2
        return -1

    # 已翻开
    if n_glyph < max(12, body.size * 0.003):
        return 0
    digit, conf = detect_number_by_color(glyph)
    if digit is not None and _glyph_is_digit_shaped(body, body_bgr):
        return digit

    # 回退: 模板匹配 → Tesseract
    theme = detect_theme(cell)
    digit, method, _ = ocr_with_fallback(cell, theme)
    if digit is not None and 1 <= digit <= 8:
        return digit
    return 0


def detect_theme(empty_cell: np.ndarray) -> str:
    """根据格子亮度判断主题"""
    mean_color = np.mean(empty_cell, axis=(0, 1)) if empty_cell.ndim == 3 else np.mean(empty_cell)
    brightness = np.mean(mean_color)
    if brightness > 200:
        return 'classic'
    elif brightness < 50:
        return 'dark'
    else:
        return 'flat'


def classify_board(cells):
    """
    棋盘级两遍分类:
      第一遍: 每格特征 (主体色/边带/字形像素数)
      第二遍: 主体色 Otsu 双聚类区分未翻开/已翻开格 (通常底色不同)，
              簇角色由"边带与本簇主体的相对偏差"判定 (未翻开格有
              bevel/边框结构 → 偏差大)；无差异时用字形负载判定
              (已翻开簇承载数字字形)。
    保证输出保留全部 rows×cols 格子。
    """
    rows, cols = len(cells), len(cells[0]) if cells else 0

    # 第一遍: 特征
    feats = [[cell_features(c) if c is not None else None for c in row] for row in cells]
    valid = [f for row in feats for f in row if f is not None]
    roles = [[True] * cols for _ in range(rows)]  # 默认未翻开

    meds = [f[1] for f in valid]
    th_m = _otsu_threshold_1d(meds)
    if th_m is not None:
        def cluster_role(dark_group, light_group):
            """返回 dark_is_unopened: bool 或 None (无法判定)"""
            def edge_dev(group):
                if not group:
                    return 0.0
                body = float(np.median([f[1] for f in group]))
                return float(np.median([max(abs(s - body) for s in f[2]) for f in group]))
            d_dark, d_light = edge_dev(dark_group), edge_dev(light_group)
            # bevel/边框结构明显的一簇 = 未翻开
            if d_dark > d_light + 2.0:
                return True
            if d_light > d_dark + 2.0:
                return False
            # 扁平主题: 含字形像素多的簇 = 已翻开
            gd = sum(1 for f in dark_group if f[3] > 0)
            gl = sum(1 for f in light_group if f[3] > 0)
            if gd != gl:
                return gl > gd   # 字形多的一簇不是未翻开
            return None

        dark = [f for f in valid if f[1] <= th_m]
        light = [f for f in valid if f[1] > th_m]
        dark_is_unopened = cluster_role(dark, light) if dark and light else None
        if dark_is_unopened is not None:
            roles = [[(f[1] <= th_m) == dark_is_unopened if f is not None else True
                      for f in row] for row in feats]
        # 无法判定: 保守全部按未翻开

    board = []
    for r in range(rows):
        row = []
        for c in range(cols):
            if cells[r][c] is None:
                row.append(-1)
            else:
                row.append(classify_cell_with_role(cells[r][c], roles[r][c]))
        board.append(row)
    return board


# ---------------------------------------------------------------------------
# 模板匹配多主题支持
# ---------------------------------------------------------------------------

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
# 用户反馈学习机制
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
# 回退链: 模板匹配 → Tesseract (颜色投票已在 classify_cell_with_role 完成)
# ---------------------------------------------------------------------------

def ocr_with_fallback(cell_image: np.ndarray, theme: str = 'classic'):
    """
    识别数字的回退链 (颜色投票已在 classify_cell_with_role 完成)。
    返回: (数字或None, 使用的方法名, 置信度)
    """
    h, w = cell_image.shape[:2]
    pad = max(2, min(h, w) // 8)
    inner = cell_image[pad:h - pad, pad:w - pad]
    if inner.size == 0:
        return None, 'failed', 0.0

    gray = cv2.cvtColor(inner, cv2.COLOR_BGR2GRAY)

    # 1. 模板匹配
    result, conf = template_match_digit(gray, theme)
    if result is not None and 1 <= result <= 8 and conf > 0.7:
        return result, 'template_match', conf

    # 2. Tesseract OCR (参考 minesweeper_solver 的预处理: CLAHE + 连通域清理)
    try:
        denoised = cv2.bilateralFilter(gray, 5, 75, 75)
        clahe = cv2.createCLAHE(clipLimit=2.0, tileGridSize=(3, 3))
        contrast = clahe.apply(denoised)
        blurred = cv2.GaussianBlur(contrast, (3, 3), 0)
        _, binary = cv2.threshold(blurred, 0, 255, cv2.THRESH_BINARY + cv2.THRESH_OTSU)
        # 清理触碰边框的连通域 (边框残留)
        num_labels, labels, stats, _ = cv2.connectedComponentsWithStats(
            255 - binary, connectivity=8)
        cleaned = np.ones_like(binary) * 255
        for label in range(1, num_labels):
            comp = labels == label
            if not (np.any(comp[0, :]) or np.any(comp[-1, :]) or
                    np.any(comp[:, 0]) or np.any(comp[:, -1])):
                cleaned[comp] = 0
        import pytesseract
        for config in ["--psm 10 --oem 3 -c tessedit_char_whitelist=12345678",
                       "--psm 8 --oem 3 -c tessedit_char_whitelist=12345678"]:
            for img in [cleaned, cv2.bitwise_not(cleaned)]:
                text = pytesseract.image_to_string(img, config=config).strip()
                if text and text.isdigit():
                    n = int(text)
                    if 1 <= n <= 8:
                        return n, 'tesseract', 0.8
    except Exception:
        pass

    return None, 'failed', 0.0


# ---------------------------------------------------------------------------
# 网格定位: Sobel 梯度投影 + 等差网格能量拟合
# (参考 ms_toollib obr.rs: 梯度投影找格线, 霍夫式等差拟合; 比彩色像素
#   偏移搜索稳健 —— 后者无法区分半格错位)
# ---------------------------------------------------------------------------

def _sobel_projections(image: np.ndarray):
    """返回 (垂直边缘列投影, 水平边缘行投影)"""
    gray = cv2.cvtColor(image, cv2.COLOR_BGR2GRAY)
    gx = cv2.Sobel(gray, cv2.CV_32F, 1, 0, ksize=3)
    gy = cv2.Sobel(gray, cv2.CV_32F, 0, 1, ksize=3)
    return np.abs(gx).sum(axis=0), np.abs(gy).sum(axis=1)


def _autocorr_period(proj, lo=16, hi=140):
    """自相关估计投影周期 (格子尺寸)"""
    p = proj - proj.mean()
    ac = np.correlate(p, p, 'full')[len(p) - 1:]
    if ac[0] == 0:
        return 0
    ac = ac / ac[0]
    lag = int(np.argmax(ac[lo:hi])) + lo
    return lag if ac[lag] > 0.05 else 0


def _autocorr_candidates(proj, lo=11, hi=110, topk=4):
    """
    自相关候选周期列表 (含倍频/半频变体)。
    单一最强峰不可靠: 小格子图像中格子内纹理会产生半频峰,
    大格子图像中隔行错位会产生倍频峰。
    """
    p = proj - proj.mean()
    ac = np.correlate(p, p, 'full')[len(p) - 1:]
    out = []
    if ac[0] == 0:
        return out
    ac = ac / ac[0]
    peaks = []
    for lag in range(lo, min(hi, len(ac) - 1)):
        if ac[lag] > ac[lag - 1] and ac[lag] >= ac[lag + 1] and ac[lag] > 0.06:
            peaks.append((float(ac[lag]), lag))
    peaks.sort(reverse=True)
    cands = [lag for _, lag in peaks[:topk]]
    for lag in cands[:2]:
        for m in (0.5, 2.0):
            v = lag * m
            if lo <= v <= hi and abs(v - round(v)) < 0.01 and int(round(v)) not in cands:
                cands.append(int(round(v)))
    return cands[:6]


def _fit_arith_grid(proj, period_hint):
    """
    在投影上拟合等差网格线 x0 + k*period, 最大化线上投影能量 (霍夫式)。
    返回 (x0, period, lines, score) 或 None。score 为平均线能量。
    """
    N = len(proj)
    best_score, best = -1.0, None
    for p in np.arange(max(8.0, period_hint - 1.0), period_hint + 1.01, 0.25):
        for x0 in np.arange(-p / 2, p / 2, 1.0):
            lines = x0 + p * np.arange(int((N - x0) / p))
            if len(lines) < 5:
                continue
            idx = np.round(lines).astype(int)
            idx = idx[(idx >= 0) & (idx < N)]
            score = float(proj[idx].sum())
            if score > best_score:
                best_score, best = score, (float(x0), float(p))

    if best is None:
        return None
    x0, p = best
    raw = x0 + p * np.arange(int((N - x0) / p))
    idx = np.clip(np.round(raw).astype(int), 0, N - 1)
    energies = np.array([float(proj[max(0, i - 1):i + 2].max()) for i in idx])
    # 裁剪两端低能量线 (页面空白区延伸出的假想线)
    if len(energies):
        cut = max(1.0, 0.25 * np.median(energies))
        lo, hi = 0, len(raw)
        while lo < hi and energies[lo] < cut:
            lo += 1
        while hi > lo and energies[hi - 1] < cut:
            hi -= 1
        raw = raw[lo:hi]
    if len(raw) < 5:
        return None
    avg_energy = float(np.mean([proj[max(0, i - 1):i + 2].max()
                                for i in np.clip(np.round(raw).astype(int), 0, N - 1)]))
    return raw[0], p, raw, avg_energy


def _grid_start_from_lines(proj, lines, p):
    """
    由拟合线回推格子真实起点。
    - 若拟合线是格内左/上 bevel 峰 (A: 主体→高光)，其后 ~0.5p..0.9p 处有
      阴影峰 (B)，格起点 = L - 0.13p；格数 = 线数 (每格一条 A 峰，含贴边格)。
    - 若是 B 峰 (前面有 A)，格起点 = L - 0.83p。
    - 若前后均无峰，视为真实网格线，格起点 = L；格数 = 线数 - 1。
    返回 (start, cells_count, is_boundary)
    """
    def peak_in(lo_frac, hi_frac):
        a = int(round(lines[0] + lo_frac * p))
        b = int(round(lines[0] + hi_frac * p))
        a, b = max(0, min(a, b)), min(len(proj), max(a, b))
        return float(proj[a:b].max()) if b > a else 0.0

    L0 = int(round(float(lines[0])))
    own = float(proj[max(0, L0 - 3):L0 + 4].max()) if L0 + 4 > 0 else 0.0
    fwd = peak_in(0.5, 0.9)    # 同格阴影峰 B
    back = peak_in(-0.9, -0.5)  # 前一格阴影峰
    if own > 0 and max(fwd, back) < 0.25 * own:
        return float(lines[0]), len(lines) - 1, True
    if fwd >= back:
        return float(lines[0]) - 0.13 * p, len(lines), False
    return float(lines[0]) - 0.83 * p, len(lines), False


def detect_grid_candidates(image: np.ndarray, max_candidates: int = 3):
    """
    基于梯度投影的多候选网格检测 (参考 ms_toollib OBR)。
    扫描行列的多个自相关候选周期, 施加方形格子约束 (|pc-pr| 小),
    返回按拟合能量排序的 [(rows, cols, cw, ch, ox, oy), ...]。
    """
    v_proj, h_proj = _sobel_projections(image)
    pcs = _autocorr_candidates(v_proj)
    prs = _autocorr_candidates(h_proj)
    if not pcs or not prs:
        return []

    fit_cache = {}
    results = []
    for pc in pcs:
        fit_v = _fit_arith_grid(v_proj, pc)
        if fit_v is None:
            continue
        fit_cache[('v', pc)] = fit_v
        for pr in prs:
            # 方形格子约束
            if abs(pc - pr) > 0.22 * max(pc, pr):
                continue
            fit_h = fit_cache.get(('h', pr))
            if fit_h is None:
                fit_h = _fit_arith_grid(h_proj, pr)
                if fit_h is None:
                    fit_cache[('h', pr)] = False
                    continue
                fit_cache[('h', pr)] = fit_h
            if fit_h is False:
                continue
            x0v, pf_v, lines_v, score_v = fit_v
            x0h, pf_h, lines_h, score_h = fit_h
            if not lines_v.size or not lines_h.size:
                continue
            x_start, cols, _ = _grid_start_from_lines(v_proj, lines_v, pf_v)
            y_start, rows, _ = _grid_start_from_lines(h_proj, lines_h, pf_h)
            x_start = min(max(x_start, 0.0), max(0.0, len(v_proj) - pf_v))
            y_start = min(max(y_start, 0.0), max(0.0, len(h_proj) - pf_h))
            if not (5 <= rows <= 40 and 5 <= cols <= 50):
                continue
            energy = (score_v / pf_v + score_h / pf_h) / 2.0
            results.append((energy, rows, cols, pf_v, pf_h, float(x_start), float(y_start)))

    # 去重 (相同行列/周期只留能量最高) 并排序
    results.sort(key=lambda t: -t[0])
    seen, out = set(), []
    for energy, rows, cols, cw, ch, ox, oy in results:
        key = (rows, cols, round(cw), round(ch))
        if key in seen:
            continue
        seen.add(key)
        out.append((rows, cols, cw, ch, ox, oy))
        if len(out) >= max_candidates:
            break
    return out


def board_quality_score(board):
    """
    棋盘质量评分 (用于多候选网格择优): 越大越好。
    - 合法性强约束: 0 格旁不能有未翻开 (flood-fill 性质)、数字约束必须可满足
    - 已翻开内容作为正奖励 (避免"全未翻开"空网格作弊)
    """
    R = len(board)
    C = len(board[0]) if R else 0
    if R == 0 or C == 0:
        return -1e9
    zeros_bad = unsat = 0
    revealed = 0
    for r in range(R):
        for c in range(C):
            v = board[r][c]
            if 0 <= v <= 8:
                revealed += 1
            if v == 0:
                for dr in (-1, 0, 1):
                    for dc in (-1, 0, 1):
                        rr, cc = r + dr, c + dc
                        if 0 <= rr < R and 0 <= cc < C and board[rr][cc] in (-1, -2):
                            zeros_bad += 1
            elif 1 <= v <= 8:
                flags = unk = 0
                for dr in (-1, 0, 1):
                    for dc in (-1, 0, 1):
                        rr, cc = r + dr, c + dc
                        if 0 <= rr < R and 0 <= cc < C:
                            if board[rr][cc] == -2:
                                flags += 1
                            elif board[rr][cc] == -1:
                                unk += 1
                if flags > v or (v - flags) > unk:
                    unsat += 1
    return revealed - 20.0 * zeros_bad - 10.0 * unsat


def _cell_color_score(image, x1, y1, cw, ch):
    """采样一个格子中心区域的彩色像素数 (对齐质量评分)"""
    h, w = image.shape[:2]
    x2, y2 = x1 + cw, y1 + ch
    if x1 < 0 or y1 < 0 or x2 > w or y2 > h:
        return -1
    pad = int(min(cw, ch) * 0.25)
    inner = image[y1 + pad:y2 - pad, x1 + pad:x2 - pad]
    if inner.size == 0:
        return 0
    b = inner[:, :, 0].astype(int)
    g = inner[:, :, 1].astype(int)
    r = inner[:, :, 2].astype(int)
    is_gray = (abs(r - g) < 25) & (abs(g - b) < 25) & (abs(r - b) < 25)
    dark = (r < 60) & (g < 60) & (b < 60)  # 黑色数字 7 也是字形
    return int(np.count_nonzero(~is_gray | dark))


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

    r_step = max(1, rows // 8)
    c_step = max(1, cols // 8)
    for oy in range(0, max_oy, step_y):
        for ox in range(0, max_ox, step_x):
            score = 0
            for r in range(0, rows, r_step):
                for c in range(0, cols, c_step):
                    s = _cell_color_score(image, ox + c * cell_w, oy + r * cell_h,
                                          cell_w, cell_h)
                    if s < 0:
                        continue
                    score += s
            if score > best_score:
                best_score = score
                best_ox, best_oy = ox, oy
    return best_ox, best_oy, best_score


def _refine_offset(image, cell_w, cell_h, rows, cols, ox, oy):
    """在粗对齐附近做细搜索 (步长 2px，含 ±2px 格宽微调)"""
    best = (-1, ox, oy, cell_w)
    h, w = image.shape[:2]
    for cs in range(max(12, cell_w - 2), cell_w + 3):
        if cols * cs > w or rows * cs > h:
            continue
        span = max(4, cs // 4)
        for dy in range(-span, span + 1, 2):
            for dx in range(-span, span + 1, 2):
                x0, y0 = ox + dx, oy + dy
                if x0 < 0 or y0 < 0 or x0 + cols * cs > w or y0 + rows * cs > h:
                    continue
                score = 0
                for r in range(0, rows, max(1, rows // 6)):
                    for c in range(0, cols, max(1, cols // 6)):
                        score += max(0, _cell_color_score(image, x0 + c * cs,
                                                          y0 + r * cs, cs, cs))
                if score > best[0]:
                    best = (score, x0, y0, cs)
    if best[0] < 0:
        return ox, oy, cell_w
    return best[1], best[2], best[3]


def recognize_board(image: np.ndarray):
    """
    完整的棋盘识别流程 (保留全部格子)。
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

    # 主策略: 梯度投影多候选网格 + 棋盘合法性择优 (参考 ms_toollib OBR)
    candidates = detect_grid_candidates(normalized)
    if candidates:
        best_board, best_meta, best_q = None, None, None
        for rows, cols, cw, ch, ox, oy in candidates:
            cells = segment_cells(normalized, rows, cols, cw, ch, ox, oy)
            board = classify_board(cells)
            q = board_quality_score(board)
            if best_q is None or q > best_q:
                best_board, best_meta, best_q = board, {
                    "rows": rows, "cols": cols,
                    "cell_w": round(cw, 2), "cell_h": round(ch, 2),
                    "offset_x": round(ox, 2), "offset_y": round(oy, 2),
                }, q
        if best_board is not None:
            return best_board, best_meta

    # 回退策略: 标准棋盘尺寸 + 彩色像素偏移搜索
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
        # 细化对齐，确保数字完整落入格内
        ox, oy, cs = _refine_offset(normalized, cs, cs, rows, cols, ox, oy)
    else:
        board_img = detect_board(normalized)
        rows, cols, cell_w, cell_h = detect_grid_lines(board_img)
        ox, oy = 0, 0
        cs = max(cell_w, cell_h)
        normalized = board_img

    # 分割 (全部 rows×cols 格) 并分类
    cells = segment_cells(normalized, rows, cols, cs, cs, ox, oy)
    board = classify_board(cells)

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
