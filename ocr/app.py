"""
扫雷 OCR 微服务 — 光学局面识别 (OBR)

接收扫雷截图，自动检测棋盘、分割格子、识别数字/旗帜/未翻开状态，
返回与 Rust 后端 PlayerView::from_2d 兼容的二维数组。

编码约定 (与 Rust 端一致):
  -1  → 未知 (未翻开)
  -2  → 旗帜
  0-8 → 已翻开数字

识别方案 (v3, 参考 inuput/OCR-suggestion.md / OCR-suggestion2.md /
gptsolve.md 与 reference/ 下项目):

  网格检测:
    - 结构化检测 (首选): 彩色字形 (数字/旗帜) 质量投影 → band 中心,
      由 "相位一致性" 推周期 (正确周期下所有 band 同相位, 偏差周期
      会漂移), 相位环形聚类推起点; 支持标准尺寸与自定义尺寸,
      并要求网格覆盖全部相位一致 band (防截断)。
    - 回退: 方差峰值网格线 / 梯度投影等差拟合 / 标准尺寸+彩色偏移,
      全部候选按约束质量 + bevel 双峰度 + 颜色解释度 + 结构覆盖率择优。

  单元格分类 (参考 OCR-suggestion.md "排中律"):
    ① cell_props: 环带背景色 (6%~20%) + bevel 符号 (上-下 / 左-右)
       + 中心区彩色像素。未翻开格与旗帜格共享凸起 bevel 背景,
       已翻开格为平坦底色 —— 据此先分"未翻开底色 / 已翻开底色"两簇
       (全盘聚类, bevel 更强的一簇为未翻开)。
    ② 未翻开底色格: 中心有彩色/旗杆签名 → 旗帜 -2, 否则未知 -1。
    ③ 已翻开底色格: 无字形 → 0; 彩色字形 → HSV 颜色判别 1-8
       (亮蓝1/绿2/亮红3/深蓝4/深红5/青6/黑7/灰8, 同色系用 V 仲裁,
       参考 gptsolve.md 颜色编码); 失败回退 7 段形态匹配;
       仍失败 → 排中律 → 旗帜。
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
    支持亚像素起点/周期 (先 round 再切); 轻微越界的窗口收缩到图像内,
    仅当有效区域不足半格时才以 None 占位 (由上层标记为未知)。
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
            cx1, cy1, cx2, cy2 = max(0, x1), max(0, y1), min(w, x2), min(h, y2)
            if cx2 <= cx1 or cy2 <= cy1 or (cx2 - cx1) < cell_w * 0.5 or (cy2 - cy1) < cell_h * 0.5:
                row_cells.append(None)
            else:
                row_cells.append(board[cy1:cy2, cx1:cx2])
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
# 结构化单元格特征 (v3)
#
# 关键观察 (extra.png / extra2.png 主题):
#   - 未翻开格与旗帜格共享"凸起 bevel"背景 (左上高光 ~204 / 右下阴影 ~153,
#     主体 ~179), 且未翻开格中心无彩色像素
#   - 已翻开格为平坦背景 (主体 ~166, 各边带等亮), 数字为彩色字形
#   - 因此判别顺序: 先用"环带背景色 + bevel 符号"区分未翻开/已翻开,
#     再在已翻开格中区分 数字/空白, 在未翻开格中用彩色内容区分 旗帜/未知
#     (即 OCR-suggestion.md 的"排中律": 已翻开格中非数字非空白 → 旗帜;
#      未翻开格中非空白 → 旗帜)
# ---------------------------------------------------------------------------

def cell_props(cell: np.ndarray):
    """
    计算格子的结构化特征 (与主题无关):

      ring_bgr   环带背景色 (6%~20% 边框带的中位数; 已翻开/未翻开底色不同)
      ring_med   环带灰度中值
      bevel_v    垂直 bevel: 上边带灰度 - 下边带灰度 (未翻开格显著为正)
      bevel_h    水平 bevel: 左边带 - 右边带
      body_med   中心区域灰度中值
      glyph_n    中心区字形像素数 (偏离环带背景色 > 40)
      colored_n  中心区彩色像素数 (饱和度 > 60)
      colored_px 中心区彩色像素 (BGR, 用于 HSV 数字判别)
      glyph_px   中心区字形像素 (BGR)
      dark_n     中心区深色像素数 (V < 80)
      white_n    中心区高亮像素数 (V > 235)
    """
    h, w = cell.shape[:2]
    s = min(h, w)
    if s < 8:
        return None
    g = cv2.cvtColor(cell, cv2.COLOR_BGR2GRAY)

    r0, r1 = int(round(s * 0.06)), int(round(s * 0.20))
    ring_mask = np.zeros((h, w), dtype=bool)
    ring_mask[r0:h - r0, r0:w - r0] = True
    ring_mask[r1:h - r1, r1:w - r1] = False
    if not ring_mask.any():
        return None
    ring_bgr = np.median(cell[ring_mask].reshape(-1, 3), axis=0)
    ring_med = float(np.median(g[ring_mask]))

    # bevel 边带: 8%~16% vs 84%~92%, 取中轴 25%~75% 避开角落
    t0, t1 = int(round(s * 0.08)), int(round(s * 0.16))
    m0, m1 = int(round(s * 0.25)), int(round(s * 0.75))
    b1_, b0_ = int(round(s * 0.84)), int(round(s * 0.92))
    bevel_v = float(g[t0:t1, m0:m1].mean() - g[b1_:b0_, m0:m1].mean())
    bevel_h = float(g[m0:m1, t0:t1].mean() - g[m0:m1, w - b0_:w - b1_].mean())

    # 浮雕度量 (去平面照度后): 未翻开 3D 凸起 → 上/左残差正、下/右负;
    # 已翻开 0 格平面化后残差≈0; 页面大梯度被平面拟合吸收, 不再诱发误判
    cy_, cx_ = np.mgrid[0:h, 0:w].astype(np.float64)
    gv = g.astype(np.float64)
    A = np.stack([np.ones_like(cx_), cx_, cy_], axis=-1).reshape(-1, 3)
    sol, *_ = np.linalg.lstsq(A, gv.reshape(-1), rcond=None)
    resid = gv - (sol[0] + sol[1] * cx_ + sol[2] * cy_)
    relief_v = float(resid[t0:t1, m0:m1].mean() - resid[b1_:b0_, m0:m1].mean())
    relief_h = float(resid[m0:m1, t0:t1].mean() - resid[m0:m1, w - b0_:w - b1_].mean())

    # 中心区 (20%~80%)
    c0, c1 = int(round(s * 0.20)), int(round(s * 0.80))
    center = cell[c0:c1, c0:c1]
    if center.size == 0:
        return None
    center_g = g[c0:c1, c0:c1]
    body_med = float(np.median(center_g))

    diff = np.abs(center.astype(int) - ring_bgr.astype(int)).max(axis=2)
    glyph_mask = diff > 40
    glyph_px = center[glyph_mask]

    mx = center.max(axis=2).astype(int)
    mn = center.min(axis=2).astype(int)
    sat = mx - mn
    colored_mask = sat > 60
    colored_px = center[colored_mask]

    return {
        "ring_bgr": ring_bgr,
        "ring_med": ring_med,
        "bevel_v": bevel_v,
        "bevel_h": bevel_h,
        "relief_v": relief_v,
        "relief_h": relief_h,
        "body_med": body_med,
        "glyph_n": int(glyph_mask.sum()),
        "glyph_px": glyph_px,
        "colored_n": int(colored_mask.sum()),
        "colored_px": colored_px,
        "dark_n": int(np.count_nonzero(mx < 80)),
        "white_n": int(np.count_nonzero(mn > 235)),
    }


def detect_digit_by_hsv(colored_px: np.ndarray):
    """
    HSV 颜色判别数字 1-8 (参考 gptsolve.md 的颜色编码思路):
      亮蓝→1 / 绿→2 / 亮红→3 / 深蓝→4 / 深红(棕)→5 / 青→6 / 黑→7 / 灰→8

    同色系对 (蓝 1/4, 红 3/5) 用 HSV 亮度 V 仲裁; 灰/黑需低饱和度.
    返回 (数字 或 None, 置信度)
    """
    if len(colored_px) < 12:
        return None, 0.0
    px = colored_px.reshape(-1, 1, 3).astype(np.uint8)
    hsv = cv2.cvtColor(px, cv2.COLOR_BGR2HSV).reshape(-1, 3)
    h, s, v = hsv[:, 0].astype(float), hsv[:, 1].astype(float), hsv[:, 2].astype(float)

    # 优先用高饱和度像素 (字形核心, 排除抗锯齿过渡)
    strong = s > 80
    if np.count_nonzero(strong) >= max(6, len(hsv) * 0.15):
        h, s, v = h[strong], s[strong], v[strong]

    h_med = float(np.median(h))
    s_med = float(np.median(s))
    v_med = float(np.median(v))

    if s_med < 50:
        # 无彩色: 黑(7) / 灰(8)
        if v_med < 80:
            return 7, 0.8
        if v_med < 210:
            return 8, 0.8
        return None, 0.0
    if 95 <= h_med <= 135:
        return (1 if v_med >= 180 else 4), 0.9
    if 35 <= h_med <= 85:
        return 2, 0.9
    if 86 <= h_med <= 94:
        return 6, 0.8
    if h_med < 10 or h_med > 170:
        return (3 if v_med >= 170 else 5), 0.9
    return None, 0.0


# ---------------------------------------------------------------------------
# 棋盘级分类 (v3)
#
# 流水线 (融合 OCR-suggestion.md 的"排中律"与 gptsolve.md 的颜色编码):
#   ① 全盘环带背景色聚类 → 已翻开底色 / 未翻开底色
#      (未翻开簇 = bevel 更明显的一簇; 两簇亮度差不足时用 bevel 兜底)
#   ② 未翻开底色格:
#        中心有彩色/旗杆签名 → 旗帜 -2
#        否则                → 未知 -1
#   ③ 已翻开底色格:
#        无字形              → 空白 0
#        彩色字形            → HSV 颜色判别 1-8 (失败回退 7 段形态匹配)
#        灰色字形            → 7 段形态匹配 (7/8), 低置信度 → 0
#        形态亦失败          → 排中律 → 旗帜 -2
# ---------------------------------------------------------------------------


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


_GRAY_TEMPLATES_CACHE = None


def _load_gray_templates():
    """加载 ocr/templates/gray 下的灰度数字模板 (弱色主题专用)"""
    global _GRAY_TEMPLATES_CACHE
    if _GRAY_TEMPLATES_CACHE is not None:
        return _GRAY_TEMPLATES_CACHE
    d = os.path.join(TEMPLATE_DIR, "gray")
    out = {}
    if os.path.isdir(d):
        for fn in os.listdir(d):
            if not (fn.startswith("digit_") and fn.endswith(".png")):
                continue
            try:
                parts = fn.replace("digit_", "").split("_")
                digit = int(parts[0])
            except (ValueError, IndexError):
                continue
            if not (1 <= digit <= 8):
                continue
            t = cv2.imread(os.path.join(d, fn), cv2.IMREAD_GRAYSCALE)
            if t is not None:
                out.setdefault(digit, []).append(t.astype(np.float32))
    _GRAY_TEMPLATES_CACHE = out
    return out


def _gray_glyph_feature(cell_gray, side=24):
    """弱色(近灰度)主题: 提取亮笔画字形 ROI (居中 24x24), 返回 (roi, cx_frac, cy_frac)"""
    c = cell_gray.astype(np.float32)
    mask = c >= 224
    if mask.sum() < 20:
        mask = c >= 208
    ys, xs = np.where(mask)
    if len(xs) < 15:
        return None
    h0, h1, w0, w1 = ys.min(), ys.max(), xs.min(), xs.max()
    cx = (w0 + w1) / 2.0 / max(1.0, cell_gray.shape[1])
    cy = (h0 + h1) / 2.0 / max(1.0, cell_gray.shape[0])
    bh, bw = h1 - h0 + 1, w1 - w0 + 1
    if bw < 4 or bh < 4 or bw > 0.9 * cell_gray.shape[1] or bh > 0.9 * cell_gray.shape[0]:
        return None
    side_sq = max(bh, bw)
    canvas = np.full((side_sq, side_sq), 120.0, dtype=np.float32)
    sy, sx = (side_sq - bh) // 2, (side_sq - bw) // 2
    canvas[sy:sy + bh, sx:sx + bw] = c[h0:h1 + 1, w0:w1 + 1]
    return cv2.resize(canvas, (side, side)), cx, cy


def _match_gray_digit(cell):
    """灰度模板匹配: 返回 (digit, score) 或 (None, 0)"""
    tmpl = _load_gray_templates()
    if not tmpl:
        return None, 0.0
    g = cv2.cvtColor(cell, cv2.COLOR_BGR2GRAY)
    feat = _gray_glyph_feature(g)
    if feat is None:
        return None, 0.0
    roi, cx, cy = feat
    # 笔画需大致居中 (排除 3D 边框/遮罩高光)
    if abs(cx - 0.5) > 0.24 or abs(cy - 0.5) > 0.24:
        return None, 0.0
    best_d, best_s = None, -1.0
    for digit, samples in tmpl.items():
        for t in samples:
            r = cv2.matchTemplate(roi, t, cv2.TM_CCOEFF_NORMED)
            s = float(np.max(r))
            if s > best_s:
                best_s, best_d = s, digit
    return (best_d, best_s) if best_d is not None else (None, 0.0)


def _overlay_mark_smooth(cells, board):
    """顶部 UI 白光带/大梯度的模糊格 (遮罩污染) 不直接定 U,
    而是作为候选标记, 用 8 邻域中"可靠格"(有数字/旗/低梯度)的多数
    决定其应为已翻开空白(0)还是未翻开(U)。
    这样 cn 顶部真 U 保留, 而 5/6/7/8 中大面积 0 区不再被误标成 U。"""
    R, C = len(board), len(board[0]) if board else 0
    if R == 0:
        return
    mark = [[False] * C for _ in range(R)]
    for r in range(R):
        for c in range(C):
            cl = cells[r][c]
            if cl is None:
                continue
            p = cell_props(cl)
            if p is None:
                continue
            ambiguous = False
            if p["colored_n"] < 10 and p["dark_n"] < 40:
                if p["white_n"] >= 90 and p["glyph_n"] >= 40:
                    ambiguous = True          # 白光带噪声
                elif p["glyph_n"] < 5 and abs(p["bevel_v"]) >= 35.0:
                    ambiguous = True          # 大梯度平坦格
            mark[r][c] = ambiguous

    def reliable(r, c):
        v = board[r][c]
        if v == -1 or v == -2 or v == 0:
            return False
        return 1 <= v <= 8

    # 迭代平滑 (3 轮, 允许已修正格参与后续投票)
    for _ in range(3):
        updates = []
        for r in range(R):
            for c in range(C):
                if not mark[r][c]:
                    continue
                opened = unknown = 0
                for dr in (-1, 0, 1):
                    for dc in (-1, 0, 1):
                        if dr == 0 and dc == 0:
                            continue
                        rr, cc = r + dr, c + dc
                        if not (0 <= rr < R and 0 <= cc < C):
                            continue
                        v = board[rr][cc]
                        if v == -1:
                            unknown += 1
                        elif 0 <= v <= 8:
                            opened += 1
                if opened == 0 and unknown == 0:
                    continue
                if opened >= 4:
                    updates.append((r, c, 0))
                elif unknown >= 5 and opened <= 1:
                    updates.append((r, c, -1))
        for (r, c, v) in updates:
            board[r][c] = v
            mark[r][c] = False


def _gray_relax(cells, board):
    """弱色主题后处理: 对低彩色但含明显居中字形、当前被判为
    U/F/0/错误数字 的格子, 用灰度模板匹配纠为正确数字。"""
    total = sum(row.count(-1) + row.count(-2) + sum(1 for v in row if 0 <= v <= 8)
                for row in board)
    if total == 0:
        return
    colored_sum = 0
    area_sum = 0
    flat = []
    for row in cells:
        for c in row:
            if c is None:
                continue
            s = min(c.shape[0], c.shape[1])
            area_sum += s * s
            p = cell_props(c)
            if p is not None:
                colored_sum += p["colored_n"]
                flat.append((c, p))
    if area_sum == 0:
        return
    # 近灰度主题: 彩色像素占比极低
    if colored_sum / area_sum > 0.018:
        return
    for r in range(len(board)):
        for c in range(len(board[r])):
            v = board[r][c]
            if v != -1 and v != -2 and v != 0:
                continue
            cl = cells[r][c]
            if cl is None:
                continue
            p = cell_props(cl)
            if p is None or p["colored_n"] >= 60 or p["glyph_n"] < 60:
                continue
            digit, score = _match_gray_digit(cl)
            if digit is not None and score >= 0.62:
                board[r][c] = digit


def classify_board(cells, return_props=False):
    """
    棋盘级分类 (v3, 结构化两遍流水线, 见上方说明)。
    保证输出保留全部 rows×cols 格子。
    return_props=True 时同时返回 props 矩阵 (供候选评分使用)。
    """
    rows, cols = len(cells), len(cells[0]) if cells else 0

    # 第一遍: 结构特征
    props = [[cell_props(c) if c is not None else None for c in row] for row in cells]
    valid = [p for row in props for p in row if p is not None]
    if not valid:
        board = [[-1] * cols for _ in range(rows)]
        return (board, props) if return_props else board

    # --- ① 环带背景色聚类: 找已翻开底色 / 未翻开底色 ---
    med_counts = {}
    for p in valid:
        key = int(round(p["ring_med"]))
        med_counts[key] = med_counts.get(key, 0) + 1

    # 合并相近灰度 (±3) 的桶, 取计数最多的两个代表值
    rep_vals = sorted(med_counts, key=lambda k: -med_counts[k])
    clusters = []  # (代表灰度, 计数)
    for v in rep_vals:
        for i, (cv_, cn) in enumerate(clusters):
            if abs(cv_ - v) <= 3:
                clusters[i] = ((cv_ * cn + v * med_counts[v]) / (cn + med_counts[v]),
                               cn + med_counts[v])
                break
        else:
            clusters.append((float(v), med_counts[v]))
        if len(clusters) >= 3:
            break
    clusters.sort(key=lambda t: -t[1])

    opened_bg = unopened_bg = None
    if len(clusters) >= 2 and abs(clusters[0][0] - clusters[1][0]) >= 6:
        # bevel 更明显的一簇 = 未翻开 (bevel/体色分离对经典与灰度主题均适用)
        def cluster_bevel(val):
            bv = [max(0.0, p["bevel_v"]) + max(0.0, p["bevel_h"])
                  for p in valid if abs(p["ring_med"] - val) <= 4]
            return float(np.mean(bv)) if bv else 0.0
        b0c, b1c = cluster_bevel(clusters[0][0]), cluster_bevel(clusters[1][0])
        if b0c >= b1c:
            unopened_bg, opened_bg = clusters[0][0], clusters[1][0]
        else:
            unopened_bg, opened_bg = clusters[1][0], clusters[0][0]
    else:
        # 单一底色: 用绝对 bevel 强度判定整体状态
        mean_bevel = float(np.mean([max(0.0, p["bevel_v"]) for p in valid]))
        if mean_bevel > 12:
            unopened_bg = clusters[0][0]
        else:
            opened_bg = clusters[0][0]

    # --- ② 逐格分类 ---
    board = []
    for r in range(rows):
        row = []
        for c in range(cols):
            p = props[r][c]
            if p is None:
                row.append(-1)
                continue
            bevel = max(0.0, p["bevel_v"]) + max(0.0, p["bevel_h"])
            # 校准模式 (OCR_RELIEF_TAU>0): U/0 完全按"去除平面照度后的凸起"判别
            relief_tau = float(os.getenv("OCR_RELIEF_TAU", "-1"))
            relief_mode = relief_tau > 0.0
            relief_score = p["relief_v"] + p["relief_h"]
            # 大量彩色字形 (≥350) 只可能是已翻开的大号数字 —— 无论 bevel 假象如何
            # (UI 渐变遮罩/压花底纹都会让"凸起感"失效), 旗帜字形 ≤ ~250。
            strong_opened = p["colored_n"] >= 350
            if relief_mode and not strong_opened:
                unopened_like = relief_score > relief_tau
            elif unopened_bg is not None:
                dist_u = abs(p["ring_med"] - unopened_bg)
                dist_o = abs(p["ring_med"] - opened_bg) if opened_bg is not None else 1e9
                unopened_like = ((dist_u <= dist_o and dist_u < 10) or bevel > 20) \
                    and not strong_opened
            else:
                unopened_like = bevel > 20 and not strong_opened

            # answer 监督 U/0 判别 (cn/1-8 全样本逻辑回归, 原尺度权重):
            # 无彩色弱字形且环带≥185 (cn 主题) 才启用, 其余主题不受影响
            # 强洗白字形 (white≥80 & 大量字形 & 无暗色): 洗白带中的未翻开格 → U
            if (p["colored_n"] < 10 and p["dark_n"] < 40 and p["white_n"] >= 80
                    and p["glyph_n"] >= 250 and p["ring_med"] >= 190.0):
                row.append(-1)
                continue
            # 浅色无彩字形但凸起感强 (relief≥8): 洗白带中的未翻开格 → U
            if (p["colored_n"] < 10 and p["dark_n"] < 40 and p["white_n"] >= 35
                    and p["glyph_n"] >= 130 and p["ring_med"] >= 190.0
                    and p["relief_v"] >= 8.0):
                row.append(-1)
                continue
            # 右侧洗白柱 (white 30~90 + 弱字形 + 平坦): 0
            if (p["colored_n"] < 10 and p["dark_n"] < 40 and 30 <= p["white_n"] <= 90
                    and 30 <= p["glyph_n"] <= 115 and 185.0 <= p["ring_med"] <= 191.0
                    and abs(p["relief_v"]) <= 12.0):
                row.append(0)
                continue
            if (p["colored_n"] < 40 and p["glyph_n"] < 130 and p["dark_n"] < 60
                    and p["ring_med"] >= 185.0):
                score = (0.4116 * p["ring_med"] + 0.0311 * p["relief_v"]
                         + 0.0491 * p["bevel_v"] + 5.6348 * (p["white_n"] / 300.0)
                         + 7.2735 * (p["glyph_n"] / 200.0))
                row.append(-1 if score > float(os.getenv("OCR_LOGIT_TAU", "80.0")) else 0)
                continue

            # 旗帜签名: 红色旗头 + 大量暗像素旗杆 + 低环带(无翻开背景) → 旗帜
            if (p["colored_n"] >= 100 and p["dark_n"] >= 120 and p["glyph_n"] >= 450
                    and p["ring_med"] < 185.0):
                cp = p["colored_px"]
                hsv_px = cv2.cvtColor(cp.reshape(-1, 1, 3), cv2.COLOR_BGR2HSV).reshape(-1, 3)
                hm = float(np.median(hsv_px[:, 0]))
                if hm < 12 or hm > 170:
                    row.append(-2)
                    continue

            if unopened_like:
                # 未翻开底色: 有内容 → 旗帜 (排中律), 否则未知
                has_content = (p["colored_n"] >= 40 or
                               (p["colored_n"] >= 20 and p["dark_n"] >= 25) or
                               (p["dark_n"] >= 40 and p["white_n"] >= 40))
                row.append(-2 if has_content else -1)
                continue

            # 已翻开底色
            # 遮罩/阴影带内的"低色凸起格": 无彩色字形却有凸起 bevel (top 亮于 bottom)
            # 且带少量高光条纹 → 实为被 UI 渐变伪装成已翻开的未翻开格
            if (p["colored_n"] < 40 and 10 <= p["glyph_n"] < 120
                    and p["bevel_v"] > 4.0 and p["dark_n"] < 40):
                row.append(-1)
                continue
            # 亮白洗白带 / 大梯度启发 (relief_mode 时关闭, 交由浮雕阈值裁决)
            if not relief_mode:
                if (p["colored_n"] < 10 and p["dark_n"] < 40 and p["white_n"] >= 90
                        and p["glyph_n"] >= 40):
                    row.append(-1)
                    continue
                if (p["colored_n"] < 10 and p["glyph_n"] < 5
                        and abs(p["bevel_v"]) >= 35.0):
                    row.append(-1)
                    continue
            if p["glyph_n"] < 25 and p["colored_n"] < 25:
                row.append(0)  # 空白格
                continue

            # 彩色字形 → HSV 颜色判别
            hsv_digit = None
            if p["colored_n"] >= 25:
                hsv_digit, _conf = detect_digit_by_hsv(p["colored_px"])
                if hsv_digit is not None:
                    row.append(hsv_digit)
                    continue

            # 灰色字形 / 颜色判别失败 → 7 段形态匹配 (主题无关)
            s = min(cells[r][c].shape[0], cells[r][c].shape[1])
            cc0, cc1 = int(round(s * 0.20)), int(round(s * 0.80))
            body = cells[r][c][cc0:cc1, cc0:cc1]
            shape_digit, shape_conf = detect_number_by_shape(body, p["ring_bgr"])
            if shape_digit is not None and shape_conf >= 0.75:
                row.append(shape_digit)
            elif p["colored_n"] >= 60:
                # 颜色不在已知范围但有大量彩色像素: 取 HSV 判别的原始结果
                row.append(hsv_digit if hsv_digit is not None else -2)
            else:
                # 低色噪点 (遮罩亮纹/抗锯齿) → 空白, 避免误判为旗
                if p["colored_n"] < 25 and p["glyph_n"] < 100 and p["dark_n"] < 40:
                    row.append(0)
                else:
                    # 排中律兜底: 已翻开格中非数字非空白 → 旗帜
                    row.append(-2)
        board.append(row)
    try:
        _gray_relax(cells, board)
    except Exception:
        pass
    if return_props:
        return board, props
    return board


def color_explanation_score(board, props):
    """
    颜色内容解释度: 数字/旗帜格应包含彩色字形, 空白/未知格不应有。
    错位或退化的网格会把彩色像素"落"在 0/U 格上 → 负分。
    """
    s = 0.0
    for r, row in enumerate(board):
        for c, v in enumerate(row):
            p = props[r][c]
            if p is None:
                continue
            cn = p["colored_n"]
            if 1 <= v <= 8 or v == -2:
                s += min(cn, 250)
            else:
                s -= min(cn, 120)
    return s


def gradient_energy_projections(image: np.ndarray):
    """预计算 |Sobel| 投影 (列/行), 供网格覆盖率评分复用"""
    gray = cv2.cvtColor(image, cv2.COLOR_BGR2GRAY)
    e = np.abs(cv2.Sobel(gray, cv2.CV_32F, 1, 0, 3))
    col_e = np.clip(e.sum(axis=0), 0, None)
    row_e = np.clip(e.sum(axis=1), 0, None)
    k = 31
    kernel = np.ones(k, dtype=np.float32) / k
    col_e = np.convolve(col_e, kernel, mode='same')
    row_e = np.convolve(row_e, kernel, mode='same')
    return col_e, row_e


def grid_coverage_score(col_e, row_e, rows, cols, cw, ch, ox, oy):
    """
    网格对图像结构的覆盖率: 棋盘 (含未翻开格的 bevel 边缘) 是梯度能量的主体,
    截断式小网格 (如把 16×30 的数字区截成 16×16) 会漏掉大片梯度能量 → 低分。
    返回 [0, ~1]。
    """
    def _cov(proj, start, span):
        L = len(proj)
        a = int(max(0, round(start)))
        b = int(min(L, round(start + span)))
        total = float(proj.sum())
        if total <= 0 or b <= a:
            return 0.0
        return float(proj[a:b].sum()) / total

    return 0.5 * (_cov(col_e, ox, cols * cw) + _cov(row_e, oy, rows * ch))


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
# 回退链: 模板匹配 → Tesseract (颜色投票已在 classify_board 完成)
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

    利用扫雷 reveal 语义的硬约束 (对正确网格成立, 错位网格大量违反):
      1. flood-fill 性质: 已翻开空格 (0) 的 8 邻域必然全部已翻开
         (0-8) —— 0 旁出现未知/旗帜即违规 (强惩罚)
      2. 数字约束: flags ≤ v ≤ flags + unknown
         (flags 多于 v, 或剩余未知不足以凑满 v, 即违规)
    """
    R = len(board)
    C = len(board[0]) if R else 0
    if R == 0 or C == 0:
        return -1e9
    zeros_ok = zeros_bad = unsat = sat = 0
    revealed = 0
    for r in range(R):
        for c in range(C):
            v = board[r][c]
            if v == 0:
                revealed += 1
                bad = False
                for dr in (-1, 0, 1):
                    for dc in (-1, 0, 1):
                        rr, cc = r + dr, c + dc
                        if 0 <= rr < R and 0 <= cc < C and board[rr][cc] in (-1, -2):
                            bad = True
                if bad:
                    zeros_bad += 1
                else:
                    zeros_ok += 1
            elif 1 <= v <= 8:
                revealed += 1
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
                else:
                    sat += 1
    return float(zeros_ok) + float(sat) - 3.0 * zeros_bad - 3.0 * unsat + 0.02 * revealed


def grid_bevel_score(cells):
    """
    网格对齐评分 (与分类无关): 正确网格下, 格子 bevel_v 呈双峰分布
    (未翻开格 ≈ +40, 已翻开格 ≈ 0); 错位窗口跨格采样 → 连续分布。
    返回 [0,1]。
    """
    vals = []
    total = len(cells) * (len(cells[0]) if cells else 0)
    step = max(1, total // 300)
    flat = [c for row in cells for c in row if c is not None][::step]
    for c in flat:
        p = cell_props(c)
        if p:
            vals.append(p["bevel_v"])
    if len(vals) < 20:
        return 0.0
    vals = np.array(vals)
    hist, edges = np.histogram(vals, bins=24, range=(-20, 60))
    idx = np.argsort(hist)[::-1]
    i1 = idx[0]
    c1 = (edges[i1] + edges[i1 + 1]) / 2
    c2 = None
    for i in idx[1:]:
        if abs(edges[i] - edges[i1]) >= 3 * (edges[1] - edges[0]):
            c2 = (edges[i] + edges[i + 1]) / 2
            break
    centers = np.array([c1] + ([c2] if c2 is not None else []))
    d = np.min(np.abs(vals[:, None] - centers[None, :]), axis=1)
    return float(np.count_nonzero(d <= 8) / len(vals))


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


def _band_mids(mass: np.ndarray, thr):
    """一维质量投影的连续band中心"""
    mids = []
    in_band = False
    start = 0
    for i, v in enumerate(mass):
        if v > thr and not in_band:
            in_band = True
            start = i
        elif v <= thr and in_band:
            in_band = False
            mids.append((start + i - 1) / 2.0)
    if in_band:
        mids.append((start + len(mass) - 1) / 2.0)
    return mids


def _axis_period_candidates(mids):
    """
    由 band 中心间距推断周期候选 (整数 32..95)。
    周期评分 = 主相位对齐的 band 数 (正确周期下所有 band 同相位;
    偏差 1px 的错误周期会随距离漂移 → 相位分散)。
    """
    diffs = [b - a for a, b in zip(mids, mids[1:]) if 15 < b - a < 130]
    if not diffs:
        return []
    counts = {}
    for d in diffs:
        counts[int(round(d))] = counts.get(int(round(d)), 0) + 1
    scored = []
    for p in counts:
        if not (32 <= p <= 95):
            continue
        # 对齐数 (含 2p 缺格和谐波修正)
        align = _phase_align_count(mids, p)
        score = align + 0.25 * counts.get(p, 0)
        scored.append((score, p))
    scored.sort(reverse=True)
    out = []
    for _, p in scored[:2]:
        for dp in (-1, 0, 1):
            if 32 <= p + dp <= 95 and (p + dp) not in out:
                out.append(p + dp)
    return out


def _phase_align_count(mids, period, tol=4.0):
    """mids mod period 后与主相位一致的 band 数 (环形距离)"""
    if not mids:
        return 0
    phases = np.mod(np.asarray(mids, dtype=float), period)
    best = 0
    for ph in phases:
        d = np.abs(phases - ph)
        d = np.minimum(d, period - d)
        best = max(best, int(np.count_nonzero(d <= tol)))
    return best


def _dominant_phase(mids, period, tol=4.0):
    """主相位 (环形聚类均值), 无 mids 返回 None"""
    if not mids:
        return None
    phases = np.mod(np.asarray(mids, dtype=float), period)
    best_n, best_ph = 0, None
    for ph in phases:
        d = np.abs(phases - ph)
        d = np.minimum(d, period - d)
        n = int(np.count_nonzero(d <= tol))
        if n > best_n:
            best_n = n
            best_ph = ph
    if best_ph is None:
        return None
    d = np.abs(phases - best_ph)
    d = np.minimum(d, period - d)
    members = phases[d <= tol]
    # 环形均值 (以 best_ph 为参考解缠绕)
    off = members - best_ph
    off = (off + period / 2) % period - period / 2
    return (best_ph + float(np.mean(off))) % period


def _axis_align_score(mass, mids, start, period, n_cells):
    """给定轴起点/周期/格数: 打分 = 各格中心窗口的投影质量 - 未覆盖 band 惩罚"""
    L = len(mass)
    centers = start + period * (np.arange(n_cells) + 0.5)
    if centers[-1] >= L + period * 0.49:
        return -1.0
    idx = np.clip(np.round(centers).astype(int), 0, L - 1)
    half = max(2, int(round(period * 0.18)))
    lo = np.clip(idx - half, 0, L)
    hi = np.clip(idx + half + 1, 0, L)
    score = 0.0
    for a, b in zip(lo, hi):
        if b > a:
            score += float(mass[a:b].max())
    # 命中率加权: 预测中心须落在观测 band 上
    matched = 0
    for m in mids:
        k = round((m - start - period * 0.5) / period)
        if 0 <= k < n_cells and abs(start + period * (k + 0.5) - m) <= period * 0.2:
            matched += 1
    score = (score / max(1, n_cells)) * (0.5 + 0.5 * matched / max(1, len(mids)))
    return score


def _axis_best_starts(mass, mids, period, n_cells, axis_size):
    """主相位 (环形聚类均值) × 平移枚举, 返回按分数排序的起点列表"""
    phase = _dominant_phase(mids, period)
    if phase is None:
        return []
    base = phase - period * 0.5
    if base < -period * 0.49:
        base += period * int(np.ceil((-base - period * 0.49) / period))
    starts = []
    j = 0
    while base + j * period <= axis_size + period * 0.49:
        s = float(base + j * period)
        if s + n_cells * period <= axis_size + period * 0.49:
            starts.append(s)
        j += 1
        if j > 32:
            break
    scored = [(_axis_align_score(mass, mids, s, period, n_cells), s) for s in starts]
    scored.sort(key=lambda t: -t[0])
    return [s for _, s in scored[:3]]


def _phase_consistent_mids(mids, period, tol=6.0):
    """仅保留与主相位一致的 band (排除棋盘外的彩色干扰, 如 LED 计数器)"""
    phase = _dominant_phase(mids, period)
    if phase is None:
        return list(mids)
    out = []
    for m in mids:
        d = abs((m - phase) % period)
        d = min(d, period - d)
        if d <= tol:
            out.append(m)
    return out


def _covers_extent(mids, grid_span, period, axis_size, tol_frac=0.2):
    """网格 [start, start+span] 须能容纳全部(相位一致)band 中心 (含半格边距)"""
    mids = _phase_consistent_mids(mids, period)
    if not mids:
        return True
    lo = min(mids) - 0.5 * period
    hi = max(mids) + 0.5 * period
    tol = tol_frac * period
    return (hi - lo) <= grid_span + 2 * tol


def detect_grid_by_structure(image: np.ndarray, std_sizes=None):
    """
    结构化网格检测 (针对数字/旗帜对齐良好的截图):
      - 彩色字形 (数字/旗帜) 质量投影的 band 中心与格子中心对齐
      - 由 band 间距推周期, 相位直方图推起点, 标准棋盘尺寸枚举
    返回 [(rows, cols, cw, ch, ox, oy), ...] 按对齐分数排序
    """
    if std_sizes is None:
        std_sizes = [(30, 16), (16, 16), (9, 9)]
    h, w = image.shape[:2]
    mx = image.max(axis=2).astype(int)
    mn = image.min(axis=2).astype(int)
    colored = (mx - mn) > 60
    row_mass = colored.sum(axis=1).astype(float)
    col_mass = colored.sum(axis=0).astype(float)

    r_mids = _band_mids(row_mass, 3)
    c_mids = _band_mids(col_mass, 3)
    if len(r_mids) < 2 or len(c_mids) < 2:
        return []

    r_periods = _axis_period_candidates(r_mids)
    c_periods = _axis_period_candidates(c_mids)
    if not r_periods or not c_periods:
        return []

    results = []

    # 候选尺寸族: 标准尺寸 + 由 band 范围推断的自定义尺寸
    size_families = set(std_sizes)
    for rp in r_periods:
        r_cons = _phase_consistent_mids(r_mids, rp)
        if len(r_cons) < 4:
            continue
        rows_b = int(round((max(r_cons) - min(r_cons)) / rp)) + 1
        for cp in c_periods:
            c_cons = _phase_consistent_mids(c_mids, cp)
            if len(c_cons) < 4:
                continue
            cols_b = int(round((max(c_cons) - min(c_cons)) / cp)) + 1
            for dr, dc in ((0, 0), (1, 0), (0, 1), (1, 1)):
                size_families.add((cols_b + dc, rows_b + dr))

    for cols, rows in sorted(size_families):
        for rp in r_periods:
            if not (rows * rp <= h + rp * 0.49 and rows * rp >= h * 0.45):
                continue
            # 网格必须覆盖相位一致 band 的完整范围 (防止把数字区域截断成小棋盘)
            if not _covers_extent(r_mids, rows * rp, rp, h):
                continue
            r_starts = _axis_best_starts(row_mass, r_mids, rp, rows, h)
            for cp in c_periods:
                if not (cols * cp <= w + cp * 0.49 and cols * cp >= w * 0.45):
                    continue
                if not _covers_extent(c_mids, cols * cp, cp, w):
                    continue
                c_starts = _axis_best_starts(col_mass, c_mids, cp, cols, w)
                for sy in r_starts[:2]:
                    for sx in c_starts[:2]:
                        sc = (_axis_align_score(row_mass, r_mids, sy, rp, rows) +
                              _axis_align_score(col_mass, c_mids, sx, cp, cols))
                        results.append((sc, rows, cols, float(cp), float(rp), sx, sy))

    results.sort(key=lambda t: -t[0])
    out = []
    seen = set()
    per_family = {}
    for sc, rows, cols, cp, rp, sx, sy in results:
        key = (rows, cols, round(cp), round(rp), round(sx), round(sy))
        if key in seen:
            continue
        seen.add(key)
        # 每个标准尺寸族至少保留最优候选 (防止小尺寸截断棋盘霸榜)
        fam = (rows, cols)
        if per_family.get(fam, 0) >= 3:
            continue
        per_family[fam] = per_family.get(fam, 0) + 1
        out.append((rows, cols, cp, rp, sx, sy, float(sc)))
        if len(out) >= max(14, 3 * len(size_families)):
            break
    return out


def constraint_repair(board):
    """
    扫雷规则后验校验/修复 (trans-app-ocr.md P2)。
    保证输出棋盘满足基本合法性:
      - 任一已翻开数字 d 周围: 已标旗数 f ≤ d ≤ f + 未知数 u
      - 修复顺序: 先处理 "f > d" (多余旗帜, 移除对邻域破坏最小的旗);
                再处理 "d > f+u" (数字不可能满足: f+u==0 视为误分类还原为未知,
                否则数字修正为 f+u)。
    已合法的棋盘保持不变。
    """
    b = [row[:] for row in board]
    R, C = len(b), len(b[0]) if b else 0
    if R == 0 or C == 0:
        return b

    def neighbors_around(r, c):
        out = []
        for dr in (-1, 0, 1):
            for dc in (-1, 0, 1):
                rr, cc = r + dr, c + dc
                if 0 <= rr < R and 0 <= cc < C and not (dr == 0 and dc == 0):
                    out.append((rr, cc))
        return out

    for _ in range(60):
        changed = False
        # 1) f > d: 移除多余旗帜 (优先移除"移除后不会使相邻饱和数字欠雷"的旗)
        for r in range(R):
            for c in range(C):
                v = b[r][c]
                if not (0 <= v <= 8):
                    continue
                flags = [n for n in neighbors_around(r, c) if b[n[0]][n[1]] == -2]
                while len(flags) > v and flags:
                    best = None
                    best_pen = None
                    for (fr, fc) in flags:
                        pen = 0
                        for (nr, nc) in neighbors_around(fr, fc):
                            nv = b[nr][nc]
                            if not (0 <= nv <= 8):
                                continue
                            ff = sum(1 for (r2, c2) in neighbors_around(nr, nc) if b[r2][c2] == -2)
                            uu = sum(1 for (r2, c2) in neighbors_around(nr, nc) if b[r2][c2] == -1)
                            if (ff - 1) + uu < nv:  # 移除该旗后此数字将无法满足
                                pen += 1
                        if best_pen is None or pen < best_pen:
                            best_pen, best = pen, (fr, fc)
                    b[best[0]][best[1]] = -1
                    flags.remove(best)
                    changed = True
        # 2) d > f + u
        for r in range(R):
            for c in range(C):
                v = b[r][c]
                if not (0 <= v <= 8):
                    continue
                f = sum(1 for n in neighbors_around(r, c) if b[n[0]][n[1]] == -2)
                u = sum(1 for n in neighbors_around(r, c) if b[n[0]][n[1]] == -1)
                if v > f + u:
                    if f + u == 0:
                        b[r][c] = -1  # 无旗无候选 → 误分类, 还原为未知
                    else:
                        b[r][c] = f + u
                    changed = True
        if not changed:
            break
    return b


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

    # 收集所有候选网格: 结构化检测 + 方差线检测 + 梯度投影 + 标准尺寸 + 彩色像素偏移
    all_candidates = []
    struct_size_set = set()
    standard_size_set = {(9, 9), (16, 16), (30, 16), (16, 30), (9, 30), (30, 9)}

    # 0. 结构化检测 (数字/旗帜相位 + band 间距): 对齐良好截图的最可靠来源
    #    (可以是自定义尺寸, 如 25×16)。若成功, 直接采用其几何最优候选,
    #    避免 "先分类后打分" 的反馈噪声; 仅在无候选时走候选集评分回退。
    try:
        struct_candidates = detect_grid_by_structure(normalized)
        struct_size_set = {(c[0], c[1]) for c in struct_candidates}
    except Exception:
        struct_candidates = []

    if struct_candidates:
        col_e, row_e = gradient_energy_projections(normalized)
        max_sc = max(abs(c[6]) for c in struct_candidates) or 1.0

        def _geo(c):
            rows_, cols_, cw_, ch_, ox_, oy_, sc_ = c
            cov = grid_coverage_score(col_e, row_e, rows_, cols_, cw_, ch_, ox_, oy_)
            exc = (max(0.0, -ox_) + max(0.0, ox_ + cols_ * cw_ - w) +
                   max(0.0, -oy_) + max(0.0, oy_ + rows_ * ch_ - h))
            prior = 15.0 if (rows_, cols_) in standard_size_set else 0.0
            return 200.0 * cov + 20.0 * (sc_ / max_sc) - 1.5 * exc + prior

        rows, cols, cw, ch, ox, oy, _ = max(struct_candidates, key=_geo)
        cells = segment_cells(normalized, rows, cols, cw, ch, ox, oy)
        board = constraint_repair(classify_board(cells))
        meta = {
            "rows": rows, "cols": cols,
            "cell_w": round(cw, 2), "cell_h": round(ch, 2),
            "offset_x": round(ox, 2), "offset_y": round(oy, 2),
        }
        return board, meta

    # 1. 方差峰值网格线检测 (参考 GridLineDetector — 直接定位网格线)
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

    # 去重 + 过滤非标准尺寸 (梯度检测可能返回 22×40 等非标准尺寸;
    # 结构化检测出的自定义尺寸除外)
    seen = set()
    unique_candidates = []
    for c in all_candidates:
        key = (c[0], c[1], round(c[2]), round(c[3]))
        if key in seen:
            continue
        # 非标准尺寸: 过滤 (避免 22×40 等误检尺寸靠 0 格刷分)
        if (c[0], c[1]) not in standard_size_set and (c[0], c[1]) not in struct_size_set:
            continue
        seen.add(key)
        unique_candidates.append(c)

    # 对所有候选评分择优 (约束质量 + bevel 双峰对齐度 + 颜色解释度 + 结构覆盖率)
    col_e, row_e = gradient_energy_projections(normalized)
    best_board, best_meta, best_q = None, None, None
    for rows, cols, cw, ch, ox, oy in unique_candidates:
        cells = segment_cells(normalized, rows, cols, cw, ch, ox, oy)
        board, props = classify_board(cells, return_props=True)
        # 越界惩罚: 网格超出图像边缘越多越不可信 (截断窗口会破坏 bevel/字形)
        excess = (max(0.0, -ox) + max(0.0, ox + cols * cw - w) +
                  max(0.0, -oy) + max(0.0, oy + rows * ch - h))
        q = (board_quality_score(board)
             + 10.0 * grid_bevel_score(cells)
             + 0.05 * color_explanation_score(board, props)
             + 200.0 * grid_coverage_score(col_e, row_e, rows, cols, cw, ch, ox, oy)
             - 1.5 * excess)
        # 标准尺寸先验 (9×9/16×16/30×16 远比自定义尺寸常见)
        if (rows, cols) in standard_size_set:
            q += 15.0
        if best_q is None or q > best_q:
            best_board = board
            best_meta = {
                "rows": rows, "cols": cols,
                "cell_w": round(cw, 2), "cell_h": round(ch, 2),
                "offset_x": round(ox, 2), "offset_y": round(oy, 2),
            }
            best_q = q

    if best_board is not None:
        return constraint_repair(best_board), best_meta

    # 最终回退: detect_board + detect_grid_lines
    board_img = detect_board(normalized)
    rows, cols, cell_w, cell_h = detect_grid_lines(board_img)
    ox, oy = 0, 0
    cs = max(cell_w, cell_h)
    cells = segment_cells(board_img, rows, cols, cs, cs, ox, oy)
    board = classify_board(cells)
    board = constraint_repair(board)
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
        elif request.is_json and "image" in (request.get_json(silent=True) or {}):
            # base64 编码
            b64 = request.get_json(silent=True)["image"]
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
        elif request.is_json and "image" in (request.get_json(silent=True) or {}):
            b64 = request.get_json(silent=True)["image"]
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
        data = request.get_json(silent=True)
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
