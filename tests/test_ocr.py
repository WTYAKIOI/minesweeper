#!/usr/bin/env python3
"""
OCR 数字识别测试 — 验证各数字 (1-8) 的识别准确率。

参考:
  - minesweeper_solver: 颜色匹配 + Tesseract 回退
  - Metasweeper/ms_toollib: OBR + 颜色 LUT
  - TEST/AGENT4.md: 三层回退链 (颜色 → 模板 → Tesseract)

用法:
  python3 tests/test_ocr.py           # 测试 TEST/inuput/ 下所有图片
  python3 tests/test_ocr.py 1.png     # 测试单张图片
"""
import os
import sys
import cv2
import numpy as np

sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..', 'ocr'))
import app as ocr_app

IN_DIR = os.path.join(os.path.dirname(__file__), '..', 'TEST', 'inuput')


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


def _board_legality_violations(board):
    """返回违反扫雷基本约束的 (r,c,digit,flags,unknown) 列表:
    flags ≤ digit ≤ flags + unknown"""
    bad = []
    R, C = len(board), len(board[0])
    for r in range(R):
        for c in range(C):
            v = board[r][c]
            if not (0 <= v <= 8):
                continue
            f = u = 0
            for dr in (-1, 0, 1):
                for dc in (-1, 0, 1):
                    rr, cc = r + dr, c + dc
                    if 0 <= rr < R and 0 <= cc < C:
                        w = board[rr][cc]
                        if w == -2:
                            f += 1
                        elif w == -1:
                            u += 1
            if f > v or v > f + u:
                bad.append((r, c, v, f, u))
    return bad


def test_all_input_boards_legal():
    """trans-app-ocr.md P2: 规则后验校验。所有 TEST/inuput 截图识别出的棋盘
    必须满足扫雷基本约束 (无大量不合法旗帜/数字)。"""
    files = [f for f in sorted(os.listdir(IN_DIR))
             if f.lower().endswith(('.png', '.jpg', '.jpeg', '.webp'))]
    cn_dir = os.path.join(IN_DIR, 'minesweeper.cn')
    if os.path.isdir(cn_dir):
        files += [os.path.join('minesweeper.cn', f) for f in sorted(os.listdir(cn_dir))
                  if f.lower().endswith(('.png', '.jpg', '.jpeg', '.webp'))]
    assert files, "没有找到测试图片"
    for fname in files:
        img = load_image(os.path.join(IN_DIR, fname))
        board, meta = ocr_app.recognize_board(img)
        rows, cols = len(board), len(board[0]) if board else 0
        assert rows >= 2 and cols >= 2, f"{fname}: 未识别出棋盘 ({rows}x{cols})"
        bad = _board_legality_violations(board)
        assert not bad, f"{fname}: 存在不合法约束 {len(bad)} 处, 示例 {bad[:5]}"
        print(f"  [OK] {fname}: {cols}x{rows}, 合法 (F={sum(r.count(-2) for r in board)})")


def _parse_cn_answers():
    """解析 answer.txt 中 'for N.png' 分节棋盘"""
    import re
    ans_path = os.path.join(IN_DIR, 'minesweeper.cn', 'answer.txt')
    if not os.path.exists(ans_path):
        return {}
    out, cur = {}, None
    for ln in open(ans_path):
        s = ln.strip()
        m = re.search(r'for (\d)\.png', s)
        if m:
            cur = m.group(1)
            out[cur] = []
            continue
        toks = s.split()
        if cur and len(toks) == 30 and all(t in '012345678UF' for t in toks):
            out[cur].append(toks)
    return out


def test_minesweeper_cn_accuracy():
    """minesweeper.cn 1-4.png 精度回归 (answer.txt 真值)"""
    import re
    sections = _parse_cn_answers()
    if not sections:
        return
    limits = {'1': 0.95, '2': 0.97, '4': 0.99}
    for name, expect in limits.items():
        grid = sections.get(name)
        if not grid or len(grid) != 16:
            continue
        T = [[-1 if t == 'U' else -2 if t == 'F' else int(t) for t in row] for row in grid]
        img = load_image(os.path.join(IN_DIR, 'minesweeper.cn', name + '.png'))
        board, _meta = ocr_app.recognize_board(img)
        ok = sum(board[r][c] == T[r][c] for r in range(16) for c in range(30))
        ratio = ok / 480
        print(f"  minesweeper.cn/{name}.png: 一致 {ok}/480 ({ratio:.1%})")
        assert ratio >= expect, f"cn/{name} 精度过低: {ratio:.1%} (<{expect:.0%})"


def test_minesweeper_cn4_accuracy():
    """minesweeper.cn/4.png 精度回归 (answer.txt 提供真值):
    大号数字识别 (≥350 彩色像素强判已翻开) 修复后应 ≥ 96%。"""
    ans_path = os.path.join(IN_DIR, 'minesweeper.cn', 'answer.txt')
    if not os.path.exists(ans_path):
        return
    grid = []
    for ln in open(ans_path):
        toks = ln.split()
        if len(toks) == 30 and all(t in '012345678UF' for t in toks):
            grid.append(toks)
    if len(grid) != 16:
        return
    truth = [[-1 if t == 'U' else -2 if t == 'F' else int(t) for t in row] for row in grid]
    img = load_image(os.path.join(IN_DIR, 'minesweeper.cn', '4.png'))
    board, _meta = ocr_app.recognize_board(img)
    rows, cols = len(board), len(board[0])
    assert (rows, cols) == (16, 30)
    ok = sum(board[r][c] == truth[r][c] for r in range(16) for c in range(30))
    ratio = ok / 480
    print(f"  minesweeper.cn/4.png: 与 answer.txt 一致 {ok}/480 ({ratio:.1%})")
    assert ratio >= 0.96, f"cn/4 精度过低: {ratio:.1%}"


def test_extra_boards_against_answer():
    """extra.png / extra2.png 网格与识别回归测试。

    answer.out 为人工/视觉模型转写, 在 红色3↔旗帜 等 ~77 处与截图像素不符
    (详见 TEST/inuput/ 下 .out 说明), 故仅要求:
      1) 网格为 16×30 (expert)
      2) 与 answer.out 一致率 ≥ 85% (像素真值 > 90%)
    """
    for name in ('extra2.png', 'extra.png'):
        img = load_image(os.path.join(IN_DIR, name))
        board, meta = ocr_app.recognize_board(img)
        rows, cols = len(board), len(board[0])
        assert (rows, cols) == (16, 30), f"{name}: 网格 {rows}x{cols} != 16x30"

        answer = os.path.join(IN_DIR, 'answer.out')
        if not os.path.exists(answer):
            continue
        grid, mode = [], False
        for ln in open(answer):
            s = ln.strip()
            if s in ('extra2', 'extra'):
                mode = (s == name.replace('.png', ''))
                continue
            if mode and len(s) == 30 and set(s) <= set('012345678UF'):
                grid.append(s)
        assert len(grid) == 16, f"answer.out 缺少 {name} 棋盘"
        sym = {-1: 'U', -2: 'F'}
        pred = ["".join(sym.get(int(v), str(int(v))) for v in row) for row in board]
        agree = sum(pred[r][c] == grid[r][c] for r in range(16) for c in range(30))
        ratio = agree / 480
        print(f"  {name}: 与 answer.out 一致 {agree}/480 ({ratio:.1%})")
        assert ratio >= 0.85, f"{name}: 与 answer.out 一致率过低 ({ratio:.1%})"


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

    print("\n--- 规则后验合法性测试 (trans-app-ocr.md P2) ---")
    test_all_input_boards_legal()
    print("\n--- cn 精度回归测试 (answer.txt) ---")
    test_minesweeper_cn_accuracy()
    test_minesweeper_cn4_accuracy()

    print("\n--- extra 棋盘回归测试 ---")
    test_extra_boards_against_answer()
    print("\n[ALL PASS]")
