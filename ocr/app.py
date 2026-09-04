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
from typing import Optional
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
# 数字颜色分类 (参考 minesweeper_solver/src/config.py 的 NUMBER_COLORS +
#                Metasweeper/ms_toollib 经典配色 LUT)
#
# 经典 Windows 扫雷配色 (BGR)。关键点: 亮蓝(1)/深蓝(4)、亮红(3)/深红(5)
# 是互不重叠的亮度区间，用逐像素 inRange 投票取最大匹配数，
# 而非均值匹配 —— 均值会被抗锯齿像素抬高，导致 4→1、5→3 的误判。
#
# 改进 (v2):
#   1. 区间在参考项目基础上做了展宽，以容纳抗锯齿过渡像素
#   2. 8(灰) 增加饱和度约束 (HSV S < 30) —— 防止紫色等彩色字形的
#      抗锯齿边缘落入灰范围而被误判为 8
#   3. 7(黑) 收紧上限 (V < 60) —— 防止深色抗锯齿像素被误判
#   4. 1/4 和 3/5 使用 HSV 亮度 (V) 做二次仲裁: 同为蓝/红色调时,
#      V > 180 → 亮色 (1/3), V < 170 → 暗色 (4/5)
# ---------------------------------------------------------------------------

NUMBER_RANGES = {
    1: ((185, 0, 0), (255, 80, 80)),       # 亮蓝
    2: ((0, 80, 0), (70, 160, 70)),        # 绿
    3: ((0, 0, 170), (80, 80, 255)),       # 亮红
    4: ((85, 0, 0), (170, 50, 50)),        # 深蓝
    5: ((0, 0, 80), (55, 55, 165)),        # 深红 (棕)
    6: ((85, 85, 0), (160, 160, 70)),      # 青
    7: ((0, 0, 0), (60, 60, 60)),          # 黑
    8: ((80, 80, 80), (160, 160, 160)),    # 灰
}

# HSV 仲裁: 同色系 (蓝 1/4, 红 3/5) 用亮度 V 分段
#   V >= _BRIGHT_V → 亮色 (1, 3);  V <= _DARK_V → 暗色 (4, 5)
_BRIGHT_V = 180
_DARK_V = 170

# 灰色 (8) 的饱和度上限: 真灰 S < 25, 抗锯齿彩色 S 通常 > 40
_GRAY_MAX_S = 30

# 蓝色 H 范围 (OpenCV H: 0-180, 蓝色 ≈ 100-130)
_BLUE_H_LO, _BLUE_H_HI = 95, 135
# 红色 H 范围 (OpenCV H: 红色跨 0, 取 0-10 和 170-180)
_RED_H_RANGES = [(0, 10), (170, 180)]

# 旗帜: 红旗面 (亮红或深红) + 黑旗杆 (参考 minesweeper_solver: red+black → flag)
_FLAG_RED_LO = np.array([0, 0, 90])
_FLAG_RED_HI = np.array([70, 70, 255])
# 旗杆黑色范围
_FLAG_BLACK_LO = np.array([0, 0, 0])
_FLAG_BLACK_HI = np.array([80, 80, 80])


def extract_glyph_pixels(body: np.ndarray, body_bgr: np.ndarray, dev: int = 25):
    """提取偏离格子主体色的字形像素 (主题自适应)"""
    d = np.abs(body.reshape(-1, 3).astype(int) - body_bgr.astype(int)).max(axis=1)
    return body.reshape(-1, 3)[d > dev]


def _bgr_to_hsv_pixels(pixels: np.ndarray) -> np.ndarray:
    """BGR 像素数组 → HSV 像素数组 (形状 N×1×3)"""
    if len(pixels) == 0:
        return np.empty((0, 1, 3), dtype=np.uint8)
    return cv2.cvtColor(pixels.reshape(-1, 1, 3).astype(np.uint8), cv2.COLOR_BGR2HSV)


def _is_gray_pixel(hsv_pixel) -> bool:
    """判断 HSV 像素是否为真灰色 (低饱和度)"""
    s, v = int(hsv_pixel[0, 1]), int(hsv_pixel[0, 2])
    return s < _GRAY_MAX_S and 60 <= v <= 170


def detect_number_by_color(glyph: np.ndarray):
    """
    对字形像素做颜色区间投票 (参考 minesweeper_solver 的 _detect_number_by_color)。

    改进 (v2):
      - 8(灰) 票需通过 HSV 饱和度校验, 排除彩色字形的抗锯齿边缘
      - 1/4 (蓝色对) 和 3/5 (红色对) 用 HSV 亮度 V 仲裁:
        若 BGR 投票中 1 和 4 都有票, 按字形像素中位 V 分流
      - 7(黑) 收紧 V 上限, 避免深色抗锯齿误匹配

    返回 (数字 1-8 或 None, 置信度)
    """
    if len(glyph) < 8:
        return None, 0.0

    glyph_px = glyph.reshape(-1, 1, 3).astype(np.uint8)
    glyph_hsv = _bgr_to_hsv_pixels(glyph)

    votes = {}
    for digit, (lo, hi) in NUMBER_RANGES.items():
        mask = cv2.inRange(glyph_px, np.array(lo), np.array(hi))
        n = int(np.count_nonzero(mask))
        if n > 0:
            # 对 8 (灰) 施加饱和度约束: 只计低饱和度像素
            if digit == 8:
                gray_mask = np.array([
                    _is_gray_pixel(glyph_hsv[i]) for i in range(len(glyph))
                ])
                n = int(np.count_nonzero(mask & gray_mask))
            # 对 7 (黑) 施加 HSV V 约束
            if digit == 7:
                v_vals = glyph_hsv[:, 0, 2]
                n = int(np.count_nonzero(mask & (v_vals <= 60)))
            if n > 0:
                votes[digit] = n

    if not votes:
        return None, 0.0

    digit = max(votes, key=votes.get)
    conf = votes[digit] / len(glyph)
    # 获胜颜色需覆盖足够比例的字形像素
    if votes[digit] < max(8, 0.12 * len(glyph)):
        return None, conf

    # HSV 亮度仲裁: 蓝色对 (1,4) 和 红色对 (3,5)
    if digit in (1, 4) and (1 in votes and 4 in votes):
        med_v = int(np.median(glyph_hsv[:, 0, 2]))
        if med_v >= _BRIGHT_V:
            digit = 1
        elif med_v <= _DARK_V:
            digit = 4
    elif digit in (3, 5) and (3 in votes and 5 in votes):
        med_v = int(np.median(glyph_hsv[:, 0, 2]))
        if med_v >= _BRIGHT_V:
            digit = 3
        elif med_v <= _DARK_V:
            digit = 5

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
# 形态数字识别 (参考 minesweeper_solver/src/seven_segment_ocr.py)
#
# 将字形二值图按 7 段数码管区域切分, 计算每段像素密度, 与数字 1-8 的
# 段码模板做匹配。此方法**不依赖颜色**, 仅依赖字形空间分布, 因此对
# 非经典主题 (紫色/暗色/扁平) 同样有效 —— 是颜色投票失败时的关键回退。
# ---------------------------------------------------------------------------

# 7 段排列: [top, top_right, bot_right, bot, bot_left, top_left, middle]
DIGIT_SEGMENTS = {
    1: [0, 1, 1, 0, 0, 0, 0],
    2: [1, 1, 0, 1, 1, 0, 1],
    3: [1, 1, 1, 1, 0, 0, 1],
    4: [0, 1, 1, 0, 0, 1, 1],
    5: [1, 0, 1, 1, 0, 1, 1],
    6: [1, 0, 1, 1, 1, 1, 1],
    7: [1, 1, 1, 0, 0, 0, 0],
    8: [1, 1, 1, 1, 1, 1, 1],
}


def _extract_segments(binary_mask: np.ndarray) -> list:
    """
    从二值字形图中提取 7 段密度 (参考 seven_segment_ocr.extract_segments_binary)。
    binary_mask: 2D uint8, 255=字形像素
    返回 7 个 0.0-1.0 的密度值
    """
    h, w = binary_mask.shape[:2]
    if h < 4 or w < 4:
        return [0.0] * 7

    # 7 段区域定义 (与 seven_segment_ocr 一致, 略微展宽以适应不同字体)
    regions = [
        (slice(h // 16, 3 * h // 16), slice(3 * w // 8, 5 * w // 8)),        # top (中心带, 不与左右重叠)
        (slice(1 * h // 16, 7 * h // 16), slice(11 * w // 16, 15 * w // 16)),  # top_right
        (slice(9 * h // 16, 15 * h // 16), slice(11 * w // 16, 15 * w // 16)), # bot_right
        (slice(13 * h // 16, 15 * h // 16), slice(3 * w // 8, 5 * w // 8)),    # bot
        (slice(9 * h // 16, 15 * h // 16), slice(w // 16, 5 * w // 16)),      # bot_left
        (slice(1 * h // 16, 7 * h // 16), slice(w // 16, 5 * w // 16)),        # top_left
        (slice(7 * h // 16, 9 * h // 16), slice(3 * w // 8, 5 * w // 8)),     # middle
    ]
    segs = []
    for r_slice, c_slice in regions:
        roi = binary_mask[r_slice, c_slice]
        if roi.size == 0:
            segs.append(0.0)
        else:
            density = min(float(np.count_nonzero(roi)) / (roi.size * 0.2), 1.0)
            segs.append(density)
    return segs


def detect_number_by_shape(body: np.ndarray, body_bgr: np.ndarray, dev: int = 25):
    """
    形态数字识别 (主题无关): 对字形二值图做 7 段密度匹配。

    改进 (v2): 使用加权评分 —— 模板期望 ON 的段必须实际 ON,
    否则施加重罚; 模板期望 OFF 的段如果实际 ON 也罚分。
    避免 8 (全段 ON) 成为万能匹配。

    返回 (数字 1-8 或 None, 置信度)
    """
    d = np.abs(body.astype(int) - body_bgr.astype(int)).max(axis=2)
    mask = (d > dev).astype(np.uint8) * 255

    n = int(np.count_nonzero(mask))
    total = mask.size
    if n < max(12, total * 0.02) or n > total * 0.65:
        return None, 0.0

    # 使用完整 body 区域做 7 段切分 (不裁剪, 避免段定位偏移)
    if body.shape[0] < 4 or body.shape[1] < 4:
        return None, 0.0

    segs = _extract_segments(mask)

    best_digit, best_score = None, -1.0
    for digit, template in DIGIT_SEGMENTS.items():
        # 加权评分: ON 段必须实际 ON (权重 1.5), OFF 段不应 ON (权重 1.0)
        score = 0.0
        for seg, t in zip(segs, template):
            if t == 1:
                # 模板期望 ON: seg 越高越好, 低于 0.3 重罚
                score += seg * 1.5 if seg >= 0.3 else seg * 0.2
            else:
                # 模板期望 OFF: seg 越低越好
                score += (1 - seg)
        score = score / (sum(1.5 if t == 1 else 1.0 for t in template))

        if score > best_score:
            best_score = score
            best_digit = digit

    # 8 需要所有段都足够亮, 否则不选 8
    if best_digit == 8:
        min_seg = min(segs)
        if min_seg < 0.35:
            # 某段太暗, 不是 8, 选次优
            scores = []
            for digit, template in DIGIT_SEGMENTS.items():
                if digit == 8:
                    continue
                s = 0.0
                for seg, t in zip(segs, template):
                    if t == 1:
                        s += seg * 1.5 if seg >= 0.3 else seg * 0.2
                    else:
                        s += (1 - seg)
                s = s / (sum(1.5 if t == 1 else 1.0 for t in template))
                scores.append((s, digit))
            scores.sort(reverse=True)
            if scores and scores[0][0] > 0.55:
                best_digit = scores[0][1]
                best_score = scores[0][0]
            else:
                return None, best_score

    threshold = 0.65  # 65% 置信度 (加权评分更严格)
    if best_score >= threshold:
        return best_digit, best_score
    return None, best_score


def _glyph_black_ratio(glyph: np.ndarray):
    """字形中黑色像素占比 (旗帜杆检测)"""
    if len(glyph) == 0:
        return 0.0
    g = glyph.reshape(-1, 1, 3).astype(np.uint8)
    mask = cv2.inRange(g, _FLAG_BLACK_LO, _FLAG_BLACK_HI)
    return np.count_nonzero(mask) / len(glyph)


def _glyph_is_flag(glyph: np.ndarray, body_size: int):
    """
    旗帜检测 (改进): 红色像素 + 黑色像素共现判定。
    参考 minesweeper_solver: red>1% AND black>5% → flag
    降低红色阈值 (0.15) 并增加黑色校验, 避免旗帜与数字 3 混淆。
    """
    if len(glyph) < max(15, body_size * 0.003):
        return False
    red_r = _glyph_red_ratio(glyph)
    black_r = _glyph_black_ratio(glyph)
    # 红旗面 + 黑旗杆共现
    if red_r > 0.15 and black_r > 0.03:
        return True
    # 红色占比极高 (旗帜主体), 即使黑少也判旗
    if red_r > 0.35:
        return True
    return False


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


def _count_colored_glyph_pixels(glyph: np.ndarray) -> int:
    """
    统计字形像素中真正有颜色的像素数 (排除灰色/黑白)。
    参考 cell_detector.py: 空白格的字形"噪点"全部是灰色 (S≈0),
    而真实数字的字形像素有高饱和度 (S>40)。
    """
    if len(glyph) < 2:
        return 0
    hsv = cv2.cvtColor(glyph.reshape(-1, 1, 3).astype(np.uint8), cv2.COLOR_BGR2HSV)
    return int(np.count_nonzero(hsv[:, 0, 1] > 40))


# ---------------------------------------------------------------------------
# 新识别流水线 (参考 inuput/OCR-suggestion.md)
#
# 核心策略调整: "排中律" — 在已翻开格中, 不是数字也不是空白, 那一定是旗帜
#
# 流水线:
#   ① 纹理/方差分析 → 区分"未翻开"与"已翻开"
#   ② 高置信度数字识别 (Otsu二值化 + 7段形态匹配) → 数字 1-8
#   ③ 空白格检测 (方差 < 30) → 空格 0
#   ④ 最终兜底 → 旗帜 -2
# ---------------------------------------------------------------------------

def extract_digit_feature(cell_img: np.ndarray):
    """
    提取数字的核心特征, 无视背景色 (亮底/暗底自动适应)。
    参考 OCR-suggestion.md §1.

    返回 20x20 二值图, 或 None (空白格/无字形)
    """
    gray = cv2.cvtColor(cell_img, cv2.COLOR_BGR2GRAY)

    # 1. Otsu 二值化 (自动区分前景数字和背景)
    _, binary = cv2.threshold(gray, 0, 255, cv2.THRESH_BINARY_INV + cv2.THRESH_OTSU)

    # 2. 判断是否需要反转 (处理暗色主题)
    white_pixels = np.sum(binary == 255)
    black_pixels = np.sum(binary == 0)
    if white_pixels > black_pixels:
        binary = cv2.bitwise_not(binary)

    # 3. 裁切边缘 (只留下数字笔画)
    coords = cv2.findNonZero(binary)
    if coords is None:
        return None  # 全黑/全白 → 空白格

    x, y, w, h = cv2.boundingRect(coords)
    if w < 3 or h < 5:
        return None  # 太小, 不是有效数字

    digit_roi = binary[y:y + h, x:x + w]
    return cv2.resize(digit_roi, (20, 20), interpolation=cv2.INTER_AREA)


def _render_digit_template(digit: int, size: int = 20) -> np.ndarray:
    """
    用 7 段数码管模板渲染一个 20x20 的二值数字图像 (用于模板匹配)。
    参考 DIGIT_SEGMENTS 定义。
    """
    templates = DIGIT_SEGMENTS  # {1: [0,1,1,0,0,0,0], ...}
    if digit not in templates:
        return np.zeros((size, size), dtype=np.uint8)

    segs = templates[digit]
    img = np.zeros((size, size), dtype=np.uint8)

    # 7 段区域定义 (20x20 坐标)
    h, w = size, size
    regions = [
        (slice(h // 16, 3 * h // 16), slice(3 * w // 8, 5 * w // 8)),         # top
        (slice(1 * h // 16, 7 * h // 16), slice(11 * w // 16, 15 * w // 16)),  # top_right
        (slice(9 * h // 16, 15 * h // 16), slice(11 * w // 16, 15 * w // 16)),# bot_right
        (slice(13 * h // 16, 15 * h // 16), slice(3 * w // 8, 5 * w // 8)),   # bot
        (slice(9 * h // 16, 15 * h // 16), slice(w // 16, 5 * w // 16)),      # bot_left
        (slice(1 * h // 16, 7 * h // 16), slice(w // 16, 5 * w // 16)),       # top_left
        (slice(7 * h // 16, 9 * h // 16), slice(3 * w // 8, 5 * w // 8)),      # middle
    ]

    for seg_val, (r_slice, c_slice) in zip(segs, regions):
        if seg_val == 1:
            img[r_slice, c_slice] = 255
    return img


# 预渲染所有数字模板 (20x20)
_DIGIT_TEMPLATES_CACHE: dict = {}


def _get_digit_templates() -> dict:
    global _DIGIT_TEMPLATES_CACHE
    if not _DIGIT_TEMPLATES_CACHE:
        for d in range(1, 9):
            _DIGIT_TEMPLATES_CACHE[d] = _render_digit_template(d, 20)
    return _DIGIT_TEMPLATES_CACHE


def predict_digit_safe(cell_img: np.ndarray):
    """
    数字识别: Otsu 二值化 + 模板匹配 + 置信度评估。
    参考 OCR-suggestion.md §2.

    返回 (数字 1-8 或 None, 置信度)
    """
    feature = extract_digit_feature(cell_img)
    if feature is None:
        return None, 0.0

    templates = _get_digit_templates()
    best_digit, best_score = None, -1.0

    for digit, template in templates.items():
        # 归一化互相关 (NCC)
        result = cv2.matchTemplate(feature, template, cv2.TM_CCOEFF_NORMED)
        score = float(np.max(result))
        if score > best_score:
            best_score = score
            best_digit = digit

    # 置信度阈值 0.75 (参考 suggestion)
    if best_score >= 0.75:
        return best_digit, best_score
    return None, best_score


def classify_cell_with_role(cell: np.ndarray, unopened: bool):
    """
    新识别流水线 (参考 inuput/OCR-suggestion.md):

    核心策略 — "排中律":
      在已翻开格中, 不是数字也不是空白, 那一定是旗帜

    流水线:
      ① 未翻开 → HIDDEN (-1)
      ② 已翻开: 彩色数字识别 (颜色投票 1-6) → DIGIT
      ③ 已翻开: 空白格检测 (彩色像素少 + 灰色占比高) → BLANK (0)
      ④ 已翻开: 最终兜底 → FLAG (-2)

    关键阈值:
      - colored_n >= 12: 有足够彩色字形像素 → 可能是彩色数字
      - colored_n < 12 且 gray_ratio > 0.65: 纯灰色背景 → 空白格
      - 其他: 旗帜 (排中律)
    """
    feat = cell_features(cell)
    if feat is None:
        return -1

    if unopened:
        return -1

    # --- 已翻开格 ---

    body_bgr, body_med, strips, _ = feat
    s = min(cell.shape[0], cell.shape[1])
    b0, b1 = int(s * 0.2), int(s * 0.8)
    body = cell[b0:b1, b0:b1]
    glyph = extract_glyph_pixels(body, body_bgr)
    n_glyph = len(glyph)

    # 统计彩色字形像素 (排除灰色噪点)
    colored_n = _count_colored_glyph_pixels(glyph)

    # ② 彩色数字识别 (颜色投票 1-6, 参考 cell_detector._detect_number_by_color)
    if colored_n >= 12:
        color_digit, color_conf = detect_number_by_color(glyph)
        if color_digit is not None and color_digit <= 6:
            # 有足够彩色像素且颜色投票返回 1-6 → 数字
            return color_digit
        # 彩色像素多但颜色不在 1-6 范围 → 可能是旗帜 (红色旗帜/黑色杆)
        # 不返回数字, 继续到空白/旗帜判定

    # ③ 空白格检测 (参考 suggestion: 方差/纯色检测)
    #    纯空白格: 彩色像素少 (< 12) 且主体灰色占比高 (> 65%)
    gray_cell = cv2.cvtColor(cell, cv2.COLOR_BGR2GRAY)
    body_gray = gray_cell[b0:b1, b0:b1]
    gray_ratio = np.count_nonzero((body_gray >= 160) & (body_gray <= 210)) / max(1, body_gray.size)

    if colored_n < 12 and (gray_ratio > 0.65 or n_glyph < 20):
        return 0  # BLANK

    # 也有可能是深色数字 7 (黑色, 无彩色)
    # 检查是否有大量低亮度像素 (黑色字形)
    black_ratio = np.count_nonzero(body_gray < 60) / max(1, body_gray.size)
    if black_ratio > 0.05 and n_glyph > 30:
        # 有黑色字形 → 可能是数字 7
        shape_digit, shape_conf = detect_number_by_shape(body, body_bgr)
        if shape_digit is not None and shape_conf > 0.80:
            return shape_digit

    # ④ 最终兜底: FLAG (排中律 — 不是数字、不是空白 → 旗帜)
    return -2


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

        # 后处理: 被归为"未翻开"但含大量彩色字形像素的格 → 实为已翻开数字格
        # (Otsu 阈值可能将数字格 (med 偏低) 误分到未翻开簇)
        for r in range(rows):
            for c in range(cols):
                if not roles[r][c] or feats[r][c] is None:
                    continue
                f = feats[r][c]
                body_bgr, _, _, _ = f
                cell = cells[r][c]
                s = min(cell.shape[0], cell.shape[1])
                b0, b1 = int(s * 0.2), int(s * 0.8)
                body = cell[b0:b1, b0:b1]
                glyph = extract_glyph_pixels(body, body_bgr)
                colored_n = _count_colored_glyph_pixels(glyph)
                if colored_n >= max(15, len(glyph) * 0.15):
                    roles[r][c] = False  # 修正为已翻开

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
    识别数字的回退链:
      1. 形态匹配 (主题无关, 7 段数码管模板)
      2. 模板匹配 (用户反馈学习库)
      3. Tesseract OCR (参考 minesweeper_solver 的预处理)

    返回: (数字或None, 使用的方法名, 置信度)
    """
    h, w = cell_image.shape[:2]
    pad = max(2, min(h, w) // 8)
    inner = cell_image[pad:h - pad, pad:w - pad]
    if inner.size == 0:
        return None, 'failed', 0.0

    # 0. 形态匹配 (主题无关, 首选回退)
    inner_bgr = np.median(inner.reshape(-1, 3), axis=0)
    shape_digit, shape_conf = detect_number_by_shape(inner, inner_bgr)
    if shape_digit is not None and shape_conf > 0.70:
        return shape_digit, 'shape_match', shape_conf

    gray = cv2.cvtColor(inner, cv2.COLOR_BGR2GRAY)

    # 1. 模板匹配
    result, conf = template_match_digit(gray, theme)
    if result is not None and 1 <= result <= 8 and conf > 0.7:
        return result, 'template_match', conf

    # 2. Tesseract OCR (参考 minesweeper_solver 的预处理: CLAHE + 连通域清理)
    try:
        # 上采样 (小格子识别率提升)
        scale = 2
        gray_up = cv2.resize(gray, (w * scale, h * scale), interpolation=cv2.INTER_CUBIC)
        denoised = cv2.bilateralFilter(gray_up, 5, 75, 75)
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
        # 只尝试单次 PSM 10 (单字符模式), 减少延迟
        text = pytesseract.image_to_string(
            cleaned,
            config='--psm 10 --oem 3 -c tessedit_char_whitelist=12345678'
        ).strip()
        if text and text.isdigit():
            n = int(text)
            if 1 <= n <= 8:
                return n, 'tesseract', 0.8
    except Exception:
        pass

    # 3. 形态匹配低置信度也作为最后手段
    if shape_digit is not None and shape_conf > 0.60:
        return shape_digit, 'shape_match_low', shape_conf

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
    参考 cell_detector: 正确对齐的棋盘应满足数字约束,
    且 0 格旁一般不应直接接未翻开 (flood-fill 性质)。

    改进 (v2):
      - 不再以 revealed 原始计数为主奖励 (会鼓励误分类)
      - 以约束满足率为主奖励: 满足约束的数字越多越好
      - unsat (约束违反) 重罚: 50x (强信号, 对齐错误时大量违反)
      - zeros_bad (0 旁有未知) 轻罚: 5x (flood-fill 边界正常)
      - 加底分: 有一定已翻开格才有意义 (避免全未翻开空网格)
    """
    R = len(board)
    C = len(board[0]) if R else 0
    if R == 0 or C == 0:
        return -1e9
    zeros_bad = unsat = 0
    revealed = 0
    nums = 0
    zero_count = 0
    for r in range(R):
        for c in range(C):
            v = board[r][c]
            if 0 <= v <= 8:
                revealed += 1
            if v == 0:
                zero_count += 1
                for dr in (-1, 0, 1):
                    for dc in (-1, 0, 1):
                        rr, cc = r + dr, c + dc
                        if 0 <= rr < R and 0 <= cc < C and board[rr][cc] in (-1, -2):
                            zeros_bad += 1
            elif 1 <= v <= 8:
                nums += 1
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
    # 主信号: 0 格 flood-fill 连片性 (正确对齐时 0 格多且占比高;
    # 误对齐时空格被误判为数字, 0 格少且占比低)
    zero_ratio = zero_count / max(1, revealed)
    zero_bonus = zero_count * zero_ratio
    # 辅助: 有一定满足约束的数字 + 总已翻开格 (封顶)
    nums_bonus = 0.1 * min(nums - unsat, 80)
    revealed_bonus = 0.05 * min(revealed, 400)
    # 轻罚 unsat: 分类误差导致 (0 旁有未知在 flood-fill 边界是正常的, 不罚)
    return float(zero_bonus) + nums_bonus + revealed_bonus - 0.5 * unsat


def _detect_grid_by_variance(image: np.ndarray, std_sizes=None):
    """
    基于行列方差峰值的网格线检测 (参考 GridLineDetector.detect)。
    找到等间距的高方差行/列 (网格线), 直接推算 offset 和 cell size。
    返回 [(rows, cols, cw, ch, ox, oy), ...] 或空列表。
    """
    if std_sizes is None:
        std_sizes = [(30, 16), (16, 16), (9, 9)]
    h, w = image.shape[:2]
    gray = cv2.cvtColor(image, cv2.COLOR_BGR2GRAY) if image.ndim == 3 else image

    def find_lines(proj, axis_size, std_count):
        """在投影中找等间距峰值, 返回 (offset, period, count) 或 None"""
        threshold = np.median(proj) * 3.0
        peaks = []
        in_peak = False
        start = 0
        for i in range(len(proj)):
            if proj[i] > threshold and not in_peak:
                in_peak = True; start = i
            elif proj[i] <= threshold and in_peak:
                in_peak = False
                peaks.append((start + i - 1) // 2)
        if in_peak:
            peaks.append((start + len(proj) - 1) // 2)
        if len(peaks) < std_count:
            return None
        # Search candidate spacings (grid may not fill entire image)
        best = None
        # Try matching std_count lines (std_count-1 rows) or std_count+1 lines (std_count rows)
        for need in (std_count + 1, std_count):
            for sp in range(35, 85, 2):
                for start_peak in peaks:
                    expected = [start_peak + sp * k for k in range(need)]
                    if expected[-1] > axis_size:
                        break
                    matched = []
                    for exp_pos in expected:
                        nearest = min(peaks, key=lambda p: abs(p - exp_pos))
                        if abs(nearest - exp_pos) <= sp * 0.25:
                            matched.append(nearest)
                        else:
                            break
                    if len(matched) == need:
                        spacings = np.diff(matched)
                        med_sp = float(np.median(spacings))
                        max_dev = float(np.max(np.abs(spacings - med_sp)))
                        if max_dev <= med_sp * 0.25:
                            score = -max_dev / med_sp + (need - std_count) * 0.1  # prefer more lines
                            if best is None or score > best[0]:
                                actual_rows = need - 1
                                best = (score, float(matched[0]), med_sp, actual_rows)
        if best is not None:
            return (best[1], best[2], best[3])
        return None

    row_proj = np.var(gray, axis=1)
    col_proj = np.var(gray, axis=0)
    candidates = []
    for cols, rows in std_sizes:
        h_fit = find_lines(row_proj, h, rows)
        w_fit = find_lines(col_proj, w, cols)
        if h_fit and w_fit:
            oy, ch, _ = h_fit
            ox, cw, _ = w_fit
            # 候选必须覆盖图像 50% 以上 (排除子集误报)
            if rows * ch < h * 0.5 or cols * cw < w * 0.5:
                continue
            # 网格应从图像左侧/上侧开始 (排除棋盘子集)
            if ox > w * 0.15 or oy > h * 0.3:
                continue
            if abs(cw - ch) / max(cw, ch) < 0.15:
                candidates.append((rows, cols, cw, ch, ox, oy))
        elif h_fit and not w_fit:
            # 水平线检测成功但垂直线失败 — 用水平偏移 + 标准宽度构造混合候选
            oy, ch, actual_rows = h_fit
            if actual_rows >= rows - 1 and rows * ch >= h * 0.5:
                # 用标准宽度 (image_width / cols) 和 offset_x=0
                cw_est = w / cols
                if cw_est > 0 and abs(cw_est - ch) / max(cw_est, ch) < 0.2:
                    candidates.append((rows, cols, ch, ch, 0.0, oy))
        elif w_fit and not h_fit:
            ox, cw, actual_cols = w_fit
            if actual_cols >= cols - 1 and cols * cw >= w * 0.5:
                ch_est = h / rows
                if ch_est > 0 and abs(ch_est - cw) / max(ch_est, cw) < 0.2:
                    candidates.append((rows, cols, cw, cw, ox, 0.0))
    return candidates


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


def _find_offset_from_colored_pixels(image: np.ndarray, cell_w: float, cell_h: float,
                                     rows: int, cols: int):
    """
    用彩色像素 (非灰) 聚类中心反推网格偏移。
    数字/旗帜是有颜色的, 背景是灰色的; 找到聚类中心,
    反推 cell 0,0 的左上角偏移。

    返回 (ox, oy, score) 或 (0, 0, 0)
    """
    h, w = image.shape[:2]
    if cols * cell_w > w or rows * cell_h > h:
        return 0, 0, 0

    b = image[:, :, 0].astype(int)
    g = image[:, :, 1].astype(int)
    r = image[:, :, 2].astype(int)
    is_gray = (abs(r - g) < 15) & (abs(g - b) < 15) & (abs(r - b) < 15)
    colored = ~is_gray

    total_colored = int(np.count_nonzero(colored))
    if total_colored < 50:
        return 0, 0, 0

    row_proj = np.sum(colored, axis=1)
    col_proj = np.sum(colored, axis=0)

    row_centers = _find_cluster_centers(row_proj, cell_h, min_count=3)
    col_centers = _find_cluster_centers(col_proj, cell_w, min_count=3)

    if not row_centers or not col_centers:
        return 0, 0, 0

    # 合并间距过近的聚类中心 (同格 bevel+字形分裂)
    row_centers = _merge_nearby_centers(row_centers, cell_h * 0.4)
    col_centers = _merge_nearby_centers(col_centers, cell_w * 0.4)

    best_score = 0
    best_ox, best_oy = 0, 0

    # 遍历所有可能的 (oy, ox) 组合
    for rc in row_centers:
        for row_idx in range(min(6, rows)):
            oy = rc - row_idx * cell_h - cell_h / 2
            if oy < -cell_h or oy + rows * cell_h > h + cell_h:
                continue
            for cc in col_centers:
                for col_idx in range(min(6, cols)):
                    ox = cc - col_idx * cell_w - cell_w / 2
                    if ox < -cell_w or ox + cols * cell_w > w + cell_w:
                        continue
                    # 验证: 检查聚类中心是否落在 cell 中心上
                    # cell r 的中心 = oy + r*cell_h + cell_h/2
                    # 所以 r = (center - oy - cell_h/2) / cell_h
                    score = 0
                    for rc2 in row_centers:
                        local_r = (rc2 - oy - cell_h / 2) / cell_h
                        nearest = round(local_r)
                        if abs(local_r - nearest) < 0.2 and 0 <= nearest < rows:
                            score += 1
                    for cc2 in col_centers:
                        local_c = (cc2 - ox - cell_w / 2) / cell_w
                        nearest = round(local_c)
                        if abs(local_c - nearest) < 0.2 and 0 <= nearest < cols:
                            score += 1
                    if score > best_score:
                        best_score = score
                        best_ox = max(0, ox)
                        best_oy = max(0, oy)

    return best_ox, best_oy, best_score


def _merge_nearby_centers(centers: list, max_gap: float):
    """合并间距小于 max_gap 的聚类中心"""
    if not centers:
        return []
    merged = [centers[0]]
    for c in centers[1:]:
        if c - merged[-1] < max_gap:
            merged[-1] = (merged[-1] + c) / 2.0
        else:
            merged.append(c)
    return merged


def _find_cluster_centers(proj: np.ndarray, period: float, min_count: int = 3):
    """在投影中找到聚类中心 (峰值), 间距应约为 period"""
    threshold = max(min_count, np.median(proj[proj > 0]) * 0.3) if np.any(proj > 0) else min_count
    centers = []
    in_peak = False
    start = 0
    for i in range(len(proj)):
        if proj[i] > threshold and not in_peak:
            in_peak = True
            start = i
        elif proj[i] <= threshold and in_peak:
            in_peak = False
            center = (start + i - 1) / 2.0
            centers.append(center)
    if in_peak:
        centers.append((start + len(proj) - 1) / 2.0)
    return centers


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

    # 收集所有候选网格: 方差线检测 + 梯度投影 + 标准尺寸 + 彩色像素偏移
    all_candidates = []

    # 0. 方差峰值网格线检测 (参考 GridLineDetector — 直接定位网格线)
    #    当棋盘有明显的网格线时, 此方法最可靠, 不受误分类影响
    var_candidates = _detect_grid_by_variance(normalized)
    all_candidates.extend(var_candidates)

    # 1. 梯度投影多候选网格 (参考 ms_toollib OBR)
    grad_candidates = detect_grid_candidates(normalized)
    all_candidates.extend(grad_candidates)

    # 2. 标准棋盘尺寸 + 彩色像素偏移搜索 (补充梯度投影的遗漏)
    standard_sizes = [(30, 16), (16, 16), (9, 9)]
    for cols, rows in standard_sizes:
        # 直接用标准尺寸反算 cell 大小
        cw = w // cols
        ch = h // rows
        # 尝试几种 cell 大小
        for delta in range(-4, 5):
            cs = cw + delta
            if cs < 12 or cols * cs > w or rows * cs > h:
                continue
            # 彩色像素偏移
            ox, oy, score = _find_offset_from_colored_pixels(normalized, cs, cs, rows, cols)
            if score > 0:
                all_candidates.append((rows, cols, float(cs), float(cs), float(ox), float(oy)))
            # 也用 _find_best_offset 作为备选
            ox2, oy2, score2 = _find_best_offset(normalized, cs, cs, rows, cols)
            if score2 > 0:
                all_candidates.append((rows, cols, float(cs), float(cs), float(ox2), float(oy2)))

    # 去重 + 过滤非标准尺寸 (梯度检测可能返回 22×40 等非标准尺寸)
    standard_size_set = {(9, 9), (16, 16), (30, 16), (16, 30), (9, 30), (30, 9)}
    seen = set()
    unique_candidates = []
    for c in all_candidates:
        key = (c[0], c[1], round(c[2]), round(c[3]))
        if key in seen:
            continue
        # 非标准尺寸: 过滤 (避免 22×40 等误检尺寸靠 0 格刷分)
        if (c[1], c[0]) not in standard_size_set:
            continue
        seen.add(key)
        unique_candidates.append(c)

    # 对所有候选评分择优
    best_board, best_meta, best_q = None, None, None
    for rows, cols, cw, ch, ox, oy in unique_candidates:
        cells = segment_cells(normalized, rows, cols, cw, ch, ox, oy)
        board = classify_board(cells)
        q = board_quality_score(board)
        if best_q is None or q > best_q:
            best_board = board
            best_meta = {
                "rows": rows, "cols": cols,
                "cell_w": round(cw, 2), "cell_h": round(ch, 2),
                "offset_x": round(ox, 2), "offset_y": round(oy, 2),
            }
            best_q = q

    if best_board is not None:
        return best_board, best_meta

    # 最终回退: detect_board + detect_grid_lines
    board_img = detect_board(normalized)
    rows, cols, cell_w, cell_h = detect_grid_lines(board_img)
    ox, oy = 0, 0
    cs = max(cell_w, cell_h)
    cells = segment_cells(board_img, rows, cols, cs, cs, ox, oy)
    board = classify_board(cells)
    meta = {"rows": rows, "cols": cols, "cell_w": cs, "cell_h": cs,
            "offset_x": ox, "offset_y": oy}
    return board, meta


# ---------------------------------------------------------------------------
# 雷数计数器识别 (参考 reference/minesweeper_solver-main/src/bomb_counter.py)
#
# 扫雷窗口左上角的红色 LED 数码管显示剩余雷数。识别流程:
#   1. 取图像上方区域 (棋盘之上的标题栏), 定位红色像素包围盒
#   2. 对红色 LED 掩码做 7 段数码管 OCR, 读取 3 位数字
#   3. 失败则回退到按棋盘尺寸推断 (9×9=10, 16×16=40, 30×16=99) 减去已标旗数
# ---------------------------------------------------------------------------

# 7 段排列: [top, top_right, bot_right, bot, bot_left, top_left, middle]
# 雷数计数器需要 0-9 (棋盘数字只需 1-8)
BOMB_COUNTER_SEGMENTS = {
    0: [1, 1, 1, 1, 1, 1, 0],
    1: [0, 1, 1, 0, 0, 0, 0],
    2: [1, 1, 0, 1, 1, 0, 1],
    3: [1, 1, 1, 1, 0, 0, 1],
    4: [0, 1, 1, 0, 0, 1, 1],
    5: [1, 0, 1, 1, 0, 1, 1],
    6: [1, 0, 1, 1, 1, 1, 1],
    7: [1, 1, 1, 0, 0, 0, 0],
    8: [1, 1, 1, 1, 1, 1, 1],
    9: [1, 1, 1, 1, 0, 1, 1],
}

# 标准棋盘尺寸 → 总雷数 (参考 minesweeper_solver/src/config.py)
STANDARD_MINES = {
    (9, 9): 10,
    (16, 16): 40,
    (30, 16): 99,
}


def _recognize_counter_digit(digit_img: np.ndarray) -> Optional[int]:
    """对单个数码管数字图 (白底黑字灰度图) 做 7 段匹配 (参考 seven_segment_ocr.recognize_digit)"""
    if digit_img.size == 0:
        return None
    segs = _extract_segments(digit_img)
    best_digit, best_score = None, -1.0
    for digit, template in BOMB_COUNTER_SEGMENTS.items():
        score = sum((seg * t) + ((1 - seg) * (1 - t))
                    for seg, t in zip(segs, template))
        if score > best_score:
            best_score = score
            best_digit = digit
    # 80% 置信度阈值 (参考 seven_segment_ocr: 7 * 0.8)
    if best_score >= 7 * 0.8:
        return best_digit
    return None


def detect_bomb_counter(image: np.ndarray) -> Optional[int]:
    """
    从截图中识别雷数计数器 (参考 reference/bomb_counter.py)。

    策略: 在图像上半部分搜索红色 LED 数码管区域,
    提取 3 位数字并转为整数。失败返回 None。
    """
    h, w = image.shape[:2]
    # 雷数计数器通常在窗口左上角 (棋盘上方标题栏区域)
    # 取上方 1/3 区域, 左侧 2/3 区域搜索
    top_region = image[:h // 3, :2 * w // 3]
    if top_region.size == 0:
        return None

    hsv = cv2.cvtColor(top_region, cv2.COLOR_BGR2HSV)
    # 红色 HSV 范围 (参考 bomb_counter._create_led_mask)
    lower_red1, upper_red1 = np.array([0, 100, 100]), np.array([10, 255, 255])
    lower_red2, upper_red2 = np.array([160, 100, 100]), np.array([180, 255, 255])
    mask1 = cv2.inRange(hsv, lower_red1, upper_red1)
    mask2 = cv2.inRange(hsv, lower_red2, upper_red2)
    red_mask = cv2.bitwise_or(mask1, mask2)

    # 形态学清理噪点
    kernel = cv2.getStructuringElement(cv2.MORPH_RECT, (3, 3))
    red_mask = cv2.morphologyEx(red_mask, cv2.MORPH_OPEN, kernel)

    contours, _ = cv2.findContours(red_mask, cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_SIMPLE)
    if not contours:
        return None

    # 取所有有效轮廓的包围盒 (参考 bomb_counter._get_led_bounds)
    min_x, min_y = float('inf'), float('inf')
    max_x, max_y = 0, 0
    found = False
    for cnt in contours:
        x, y, cw, ch = cv2.boundingRect(cnt)
        if cw * ch > 20:
            found = True
            min_x, max_x = min(min_x, x), max(max_x, x + cw)
            min_y, max_y = min(min_y, y), max(max_y, y + ch)
    if not found:
        return None

    # 提取计数器区域
    counter_region = top_region[int(min_y):int(max_y), int(min_x):int(max_x)]
    led_mask = red_mask[int(min_y):int(max_y), int(min_x):int(max_x)]
    if counter_region.size == 0:
        return None

    # 转为白底黑字 (参考 bomb_counter._process_counter_image)
    result = np.full(counter_region.shape[:2], 255, dtype=np.uint8)
    result[led_mask > 0] = 0

    # 分割成 3 位数字并识别
    rh, rw = result.shape[:2]
    digit_w = rw // 3
    digits = []
    for i in range(3):
        start_x = i * digit_w
        end_x = (i + 1) * digit_w if i < 2 else rw
        digit_img = result[:, start_x:end_x]
        # 裁剪到有效像素区域
        cols = np.any(digit_img == 0, axis=0)
        rows = np.any(digit_img == 0, axis=1)
        if np.any(cols) and np.any(rows):
            ymin, ymax = np.where(rows)[0][[0, -1]]
            xmin, xmax = np.where(cols)[0][[0, -1]]
            digit_img = digit_img[ymin:ymax + 1, xmin:xmax + 1]
        d = _recognize_counter_digit(digit_img)
        digits.append(d)

    # 解析 3 位数字
    valid = [d for d in digits if d is not None]
    if not valid:
        return None
    # 至少识别出 2 位才可信 (1 位极易误读)
    if len(valid) < 2:
        return None
    # 未识别位按 0 处理
    val = 0
    for d in digits:
        val = val * 10 + (d if d is not None else 0)
    # 合理性检查: 1-999 (排除 0 —— 0 雷时 LED 通常显示空白或灭灯,
    # 识别到 0 多为红色旗帜/数字3像素被误读为 LED 段)
    if 1 <= val <= 999:
        return val
    return None


def infer_mines_from_board(board: list) -> Optional[int]:
    """
    根据棋盘尺寸推断剩余雷数 (回退方案)。
    标准: 9×9=10, 16×16=40, 30×16=99。
    剩余雷数 = 标准总雷数 - 已标旗数。
    """
    if not board or not board[0]:
        return None
    rows = len(board)
    cols = len(board[0])
    # 匹配标准尺寸 (宽, 高)
    key = (cols, rows)
    if key not in STANDARD_MINES:
        # 也尝试 (行, 列) 因为可能有转置
        key2 = (rows, cols)
        if key2 not in STANDARD_MINES:
            return None
        key = key2
    total = STANDARD_MINES[key]
    flag_count = sum(1 for row in board for v in row if v == -2)
    remaining = total - flag_count
    return max(0, remaining)


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

        # 识别剩余雷数:
        #   优先按棋盘尺寸推断 (标准棋盘最可靠, 等价于 LED 计数器显示值)
        #   仅当棋盘尺寸非标准时, 才回退到 LED 计数器识别
        #   (LED 识别易受棋盘内红色旗帜/数字3像素干扰, 可能误读为 0)
        remaining = infer_mines_from_board(board)
        if remaining is None:
            remaining = detect_bomb_counter(image)

        return jsonify({
            "success": True,
            "board": board,
            "remaining_mines": remaining,
            "meta": meta,
        })
    except Exception as e:
        return jsonify({"success": False, "error": str(e)}), 500


@app.route("/api/health", methods=["GET"])
def health():
    return "ok"


@app.route("/api/debug", methods=["POST"])
def debug():
    """
    调试端点: 返回每格的颜色/形态分析细节, 便于排查误识别。
    返回 JSON: { board, meta, cells: [{r,c,value,method,glyph_n,glyph_bgr,...}] }
    """
    try:
        if "image" in request.files:
            data = request.files["image"].read()
        elif "image" in request.json:
            b64 = request.json["image"]
            if "," in b64:
                b64 = b64.split(",", 1)[1]
            data = base64.b64decode(b64)
        else:
            data = request.get_data()
        if not data:
            return jsonify({"error": "未收到图像数据"}), 400

        image = decode_image(data)
        # 光照归一化
        gray_img = cv2.cvtColor(image, cv2.COLOR_BGR2GRAY)
        mean_brightness = np.mean(gray_img)
        if mean_brightness < 80 or mean_brightness > 220:
            normalized = normalize_illumination(image)
        else:
            normalized = image

        board, meta = recognize_board(image)
        cells = segment_cells(normalized, meta["rows"], meta["cols"],
                              meta["cell_w"], meta["cell_h"],
                              meta["offset_x"], meta["offset_y"])

        cell_debug = []
        for r in range(meta["rows"]):
            for c in range(meta["cols"]):
                cell_img = cells[r][c]
                v = board[r][c]
                info = {"r": r, "c": c, "value": v}
                if cell_img is None:
                    info["status"] = "out_of_bounds"
                    cell_debug.append(info)
                    continue
                feat = cell_features(cell_img)
                if feat is None:
                    info["status"] = "no_features"
                    cell_debug.append(info)
                    continue
                body_bgr, body_med, strips, glyph_n = feat
                s = min(cell_img.shape[0], cell_img.shape[1])
                b0, b1 = int(s * 0.2), int(s * 0.8)
                body = cell_img[b0:b1, b0:b1]
                glyph = extract_glyph_pixels(body, body_bgr)
                info["body_bgr"] = [int(body_bgr[0]), int(body_bgr[1]), int(body_bgr[2])]
                info["body_med"] = round(body_med, 1)
                info["glyph_n"] = len(glyph)
                info["red_ratio"] = round(_glyph_red_ratio(glyph), 3) if len(glyph) else 0
                info["black_ratio"] = round(_glyph_black_ratio(glyph), 3) if len(glyph) else 0

                if 1 <= v <= 8 and len(glyph) > 8:
                    med = np.median(glyph, axis=0)
                    info["glyph_median_bgr"] = [int(med[0]), int(med[1]), int(med[2])]
                    # 颜色投票
                    color_digit, color_conf = detect_number_by_color(glyph)
                    info["color_digit"] = color_digit
                    info["color_conf"] = round(color_conf, 3)
                    # 形态匹配
                    shape_digit, shape_conf = detect_number_by_shape(body, body_bgr)
                    info["shape_digit"] = shape_digit
                    info["shape_conf"] = round(shape_conf, 3)
                cell_debug.append(info)
            cell_debug.append({"separator": True})

        return jsonify({
            "success": True,
            "board": board,
            "meta": meta,
            "cells": cell_debug,
        })
    except Exception as e:
        import traceback
        return jsonify({"success": False, "error": str(e),
                        "trace": traceback.format_exc()}), 500


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
    app.run(host="0.0.0.0", port=5001, debug=False, threaded=True)
