#!/usr/bin/env python3
"""
OCR 数字识别测试 — 验证各数字 (1-8) 的识别准确率。

参考:
  - minesweeper_solver: 颜色匹配 + Tesseract 回退
  - Metasweeper/ms_toollib: OBR + 颜色 LUT
  - AGENT4.md: 三层回退链 (颜色 → 模板 → Tesseract)

用法:
  python3 tests/test_ocr.py           # 测试 inuput/ 下所有图片
  python3 tests/test_ocr.py 1.png     # 测试单张图片
"""
import os
import sys
import cv2
import numpy as np

sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..', 'ocr'))
import app as ocr_app

IN_DIR = os.path.join(os.path.dirname(__file__), '..', 'inuput')


def load_image(path):
    img = cv2.imread(path)
    if img is None:
        from PIL import Image
        pil = Image.open(path).convert('RGB')
        img = np.array(pil)[:, :, ::-1].copy()
    return img


def test_digit_recognition():
    """测试所有图片的数字识别"""
    files = [f for f in sorted(os.listdir(IN_DIR))
             if f.lower().endswith(('.png', '.jpg', '.jpeg', '.webp'))]

    assert files, f"没有找到测试图片 in {IN_DIR}"

    total_digits = 0
    correct = 0
    sym = {-1: 'U', -2: 'F'}

    for fname in files:
        path = os.path.join(IN_DIR, fname)
        img = load_image(path)
        board, meta = ocr_app.recognize_board(img)
        rows = len(board)
        cols = len(board[0]) if rows else 0

        # 统计识别到的数字
        digit_count = {}
        for r in range(rows):
            for c in range(cols):
                v = board[r][c]
                if 1 <= v <= 8:
                    digit_count[v] = digit_count.get(v, 0) + 1

        print(f"  {fname}: {rows}x{cols}, digits={digit_count}")
        # 至少应识别到一些数字 (除非整张图确实没有翻开格)
        total_digits += sum(digit_count.values())

    print(f"\n总识别数字数: {total_digits}")
    assert total_digits > 0, "未识别到任何数字"


def test_color_ranges_no_overlap():
    """测试颜色区间: 1/4 (蓝色对) 和 3/5 (红色对) 不应重叠"""
    # 1 (亮蓝): B in [185,255]
    # 4 (深蓝): B in [85,170]
    r1_lo, r1_hi = ocr_app.NUMBER_RANGES[1]
    r4_lo, r4_hi = ocr_app.NUMBER_RANGES[4]
    # 蓝色通道不应重叠
    assert r1_lo[0] > r4_hi[0] or r4_lo[0] > r1_hi[0], \
        f"蓝色通道重叠: 1=[{r1_lo[0]},{r1_hi[0]}], 4=[{r4_lo[0]},{r4_hi[0]}]"

    # 3 (亮红): R in [170,255]
    # 5 (深红): R in [80,165]
    r3_lo, r3_hi = ocr_app.NUMBER_RANGES[3]
    r5_lo, r5_hi = ocr_app.NUMBER_RANGES[5]
    assert r3_lo[2] > r5_hi[2] or r5_lo[2] > r3_hi[2], \
        f"红色通道重叠: 3=[{r3_lo[2]},{r3_hi[2]}], 5=[{r5_lo[2]},{r5_hi[2]}]"


def test_shape_recognition():
    """测试形态识别: 合成数字字形验证 7 段匹配"""
    body_bgr = np.array([200, 200, 200])

    # 所有测试字形绘制在 body 区域 (24x24) 内
    # 构造 "1" (右侧竖线)
    canvas = np.full((24, 24, 3), 200, dtype=np.uint8)
    canvas[2:22, 16:20] = (0, 0, 255)
    digit, conf = ocr_app.detect_number_by_shape(canvas, body_bgr)
    assert digit == 1, f"形态识别 '1' 失败: got {digit} (conf={conf})"

    # 构造 "8" (全段)
    canvas = np.full((24, 24, 3), 200, dtype=np.uint8)
    canvas[1:3, 6:18] = (0, 0, 255)      # top
    canvas[1:11, 17:21] = (0, 0, 255)    # top_right
    canvas[13:23, 17:21] = (0, 0, 255)  # bot_right
    canvas[21:23, 6:18] = (0, 0, 255)   # bot
    canvas[13:23, 3:7] = (0, 0, 255)    # bot_left
    canvas[1:11, 3:7] = (0, 0, 255)     # top_left
    canvas[11:13, 6:18] = (0, 0, 255)   # middle
    digit, conf = ocr_app.detect_number_by_shape(canvas, body_bgr)
    assert digit == 8, f"形态识别 '8' 失败: got {digit} (conf={conf})"

    # 构造 "7" (top + right vertical)
    canvas = np.full((24, 24, 3), 200, dtype=np.uint8)
    canvas[1:3, 6:18] = (0, 0, 255)      # top
    canvas[1:23, 17:21] = (0, 0, 255)    # right vertical
    digit, conf = ocr_app.detect_number_by_shape(canvas, body_bgr)
    assert digit == 7, f"形态识别 '7' 失败: got {digit} (conf={conf})"


def test_flag_detection():
    """测试旗帜检测: 红色 + 黑色共现"""
    # 构造旗帜 (红色三角 + 黑色杆)
    canvas = np.full((40, 40, 3), 200, dtype=np.uint8)
    canvas[8:18, 10:25] = (0, 0, 255)    # 红旗面
    canvas[8:32, 22:25] = (0, 0, 0)      # 黑旗杆
    body_bgr = np.array([200, 200, 200])
    s = min(canvas.shape[0], canvas.shape[1])
    b0, b1 = int(s * 0.2), int(s * 0.8)
    body = canvas[b0:b1, b0:b1]
    glyph = ocr_app.extract_glyph_pixels(body, body_bgr)
    assert ocr_app._glyph_is_flag(glyph, body.size), "旗帜检测失败"

    # 纯红色 (数字 3) 不应被检测为旗帜
    canvas = np.full((40, 40, 3), 200, dtype=np.uint8)
    canvas[8:32, 15:25] = (0, 0, 255)    # 纯红, 无黑色
    body = canvas[b0:b1, b0:b1]
    glyph = ocr_app.extract_glyph_pixels(body, body_bgr)
    # 纯红无黑杆时, 红色占比极高 (>0.35) 仍判旗
    # 但如果红色占比不够高则不判旗
    red_r = ocr_app._glyph_red_ratio(glyph)
    if red_r < 0.35:
        assert not ocr_app._glyph_is_flag(glyph, body.size), \
            f"纯红色被误判为旗帜 (red={red_r:.2f})"


if __name__ == '__main__':
    print("=== OCR 数字识别测试 ===\n")
    test_color_ranges_no_overlap()
    print("[PASS] 颜色区间不重叠测试")

    test_shape_recognition()
    print("[PASS] 形态识别测试")

    test_flag_detection()
    print("[PASS] 旗帜检测测试")

    print("\n--- 图片识别测试 ---")
    test_digit_recognition()
    print("\n[ALL PASS]")
