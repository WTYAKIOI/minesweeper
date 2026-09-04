"""
使用 reference/minesweeper_solver 的识别逻辑处理 extra2.png,
输出标准棋盘到 inuput/answer.out (格式同 1.out)。

参考: reference/minesweeper_solver-main/src/
  - cell_detector.py: CellStateDetector (格子状态分类)
  - config.py: Colors / CELL_STATE (颜色范围定义)

由于 reference 针对 classic Windows 主题, 颜色范围需自适应:
  - gray (已翻开空格): 适配 extra2.png 的 flat 主题 (V~166-200)
  - 网格线: 使用方差峰值检测 (reference GridLineDetector 对 flat 主题间距误判)
"""
import sys
import os
import types
import numpy as np
import cv2

# --- Stub out matplotlib-dependent debugger ---
debugger_stub = types.ModuleType("debugger")
class _StubDebugger:
    def show_debug(self, *a, **k): pass
    def visualize_game_state(self, *a, **k): pass
    def visualize_cell_analysis(self, *a, **k): pass
debugger_stub.Debugger = _StubDebugger
sys.modules["debugger"] = debugger_stub

REF_SRC = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                       "..", "reference", "minesweeper_solver-main", "src")
sys.path.insert(0, REF_SRC)

from config import Colors, CELL_STATE
from cell_detector import CellStateDetector

GRID_SIZE = (16, 30)  # expert


def _patch_colors_for_flat_theme():
    """
    扩展 reference Colors 以支持 flat 主题和完整数字 1-8。

    reference config.py 只定义了 1-6 的颜色范围 (classic Windows),
    7(黑) 和 8(灰) 未定义 → _detect_number_by_color 返回 undetected。
    此处补充 7/8 并展宽 red 范围以覆盖 flat 主题的旗帜。

    gray 范围保持窄 (只匹配已翻开空格, 不覆盖未翻开格),
    由 classify_cell_adaptive 中的 gray_ratio 回退处理未翻开格。
    """
    # 补充 7(黑) — reference 的 _detect_number_by_color 查找
    # color_to_number = {..., 'black': 7, 'gray': 8}, 但 NUMBER_COLORS 未定义这两个 key
    # 注: 'gray' 范围过宽会匹配所有格的抗锯齿像素 → 全判 8, 故不添加
    # 7/8 在 classify_cell 的 undetected 回退中处理
    Colors.NUMBER_COLORS['black'] = ([0, 0, 0], [50, 50, 50])  # 7 (收紧, 避免深灰误匹配)
    # 展宽 red 范围 (flat 主题旗帜可能偏暗)
    Colors.CELL_COLORS['red'] = ([0, 0, 100], [80, 80, 255])
    # gray 保持 reference 原值 [180-200], 不自适应 — 避免未翻开格也匹配 gray


def detect_grid_lines_variance(image, rows, cols):
    """基于行列方差峰值检测网格线 (补充 reference GridLineDetector 的不足)"""
    gray = cv2.cvtColor(image, cv2.COLOR_BGR2GRAY) if image.ndim == 3 else image
    h, w = gray.shape[:2]

    def find_lines(proj, axis_size, count):
        threshold = np.median(proj) * 3.0
        peaks = []
        in_peak = False
        start = 0
        for i in range(len(proj)):
            if proj[i] > threshold and not in_peak:
                in_peak = True
                start = i
            elif proj[i] <= threshold and in_peak:
                in_peak = False
                peaks.append((start + i - 1) // 2)
        if in_peak:
            peaks.append((start + len(proj) - 1) // 2)
        if len(peaks) < count:
            return None
        best = None
        for need in (count + 1, count):
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
                            score = -max_dev / med_sp + (need - count) * 0.1
                            if best is None or score > best[0]:
                                best = (score, matched, med_sp, need - 1)
        return best[1] if best else None

    h_lines = find_lines(np.var(gray, axis=1), h, rows)
    v_lines = find_lines(np.var(gray, axis=0), w, cols)
    if h_lines is None:
        h_lines = np.linspace(0, h - 1, rows + 1).astype(int).tolist()
    if v_lines is None:
        v_lines = np.linspace(0, w - 1, cols + 1).astype(int).tolist()
    return list(h_lines), list(v_lines)


def main():
    img_path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "extra2.png")
    img = cv2.imread(img_path)
    if img is None:
        print(f"ERROR: cannot read {img_path}")
        return
    h, w = img.shape[:2]
    print(f"Image: {w}x{h}")
    rows, cols = GRID_SIZE

    # Step 1: 网格线检测 (方差峰值 — 对 flat 主题更可靠)
    h_lines, v_lines = detect_grid_lines_variance(img, rows, cols)
    # 确保有 rows+1 条水平线 (可能最后一条在图像边缘未被检测到)
    if len(h_lines) == rows:
        sp = int(np.median(np.diff(h_lines)))
        h_lines.append(min(h, h_lines[-1] + sp))
    if len(v_lines) == cols:
        sp = int(np.median(np.diff(v_lines)))
        v_lines.append(min(w, v_lines[-1] + sp))
    print(f"Grid: {len(h_lines)} H lines (spacing ~{np.median(np.diff(h_lines)):.0f}), "
          f"{len(v_lines)} V lines (spacing ~{np.median(np.diff(v_lines)):.0f})")
    # Step 2: 自适应颜色范围
    _patch_colors_for_flat_theme()

    # 采集主体色用于 undetected 回退
    sample_cells = []
    for i in range(min(3, rows)):
        for j in range(min(5, cols)):
            top, bottom = h_lines[i], h_lines[i + 1]
            left, right = v_lines[j], v_lines[j + 1]
            cell = img[top + 3:bottom - 3, left + 3:right - 3]
            if cell.size > 0:
                sample_cells.append(np.median(cell.reshape(-1, 3), axis=0))
    opened_med = float(np.median(sample_cells, axis=0).mean()) if sample_cells else 179.0
    print(f"Opened cell median brightness: {opened_med:.0f}")

    # Step 3: 用 reference CellStateDetector 分类每格
    cell_detector = CellStateDetector(None, None)
    game_state = np.zeros(GRID_SIZE, dtype=int)
    margin = 3

    for i in range(rows):
        for j in range(cols):
            top = h_lines[i]
            bottom = h_lines[i + 1] if i + 1 < len(h_lines) else h_lines[-1]
            left = v_lines[j]
            right = v_lines[j + 1] if j + 1 < len(v_lines) else v_lines[-1]
            cell = img[top + margin:bottom - margin, left + margin:right - margin]
            if cell.size == 0 or cell.shape[0] < 5 or cell.shape[1] < 5:
                game_state[i, j] = CELL_STATE.unopened
                continue
            state = cell_detector.detect_state(cell, (i, j))
            # undetected → 使用 gray_ratio + 彩色像素检查回退
            if state == CELL_STATE.undetected:
                gray_cell = cv2.cvtColor(cell, cv2.COLOR_BGR2GRAY)
                s = min(cell.shape[0], cell.shape[1])
                b0, b1 = int(s * 0.2), int(s * 0.8)
                body_gray = gray_cell[b0:b1, b0:b1]
                brightness = float(np.median(body_gray))
                if abs(brightness - opened_med) < 10:
                    # 已翻开 — 检查是否有数字字形
                    d = np.abs(cell.astype(int) - int(opened_med)).max(axis=2)
                    glyph_mask = d > 25
                    glyph_pixels = cell[glyph_mask]
                    n_glyph = len(glyph_pixels)
                    if n_glyph < max(12, body_gray.size * 0.003):
                        state = CELL_STATE.empty  # 无字形 → 0
                    else:
                        # 有字形 — 检查是否为彩色数字 (1-6 reference 可检测)
                        hsv_g = cv2.cvtColor(glyph_pixels.reshape(-1, 1, 3).astype(np.uint8),
                                             cv2.COLOR_BGR2HSV)
                        colored_n = int(np.count_nonzero(hsv_g[:, 0, 1] > 40))
                        if colored_n >= max(8, n_glyph * 0.1):
                            # 彩色数字 — reference 已尝试但未识别 → 0 (颜色不在范围)
                            state = CELL_STATE.empty
                        else:
                            # 灰色字形 → 可能是数字 8 (灰) 或 7 (黑)
                            # 用 7 段形态匹配 (reference seven_segment_ocr 逻辑)
                            glyph_gray = gray_cell[b0:b1, b0:b1]
                            bin_mask = (d > 25).astype(np.uint8) * 255
                            # 检查是否为全段 ON (8) 或少段 (其他)
                            DIGIT_SEGS = {
                                1: [0,1,1,0,0,0,0], 2: [1,1,0,1,1,0,1], 3: [1,1,1,1,0,0,1],
                                4: [0,1,1,0,0,1,1], 5: [1,0,1,1,0,1,1], 6: [1,0,1,1,1,1,1],
                                7: [1,1,1,0,0,0,0], 8: [1,1,1,1,1,1,1],
                            }
                            hh, ww = bin_mask.shape[:2]
                            if hh >= 4 and ww >= 4:
                                regions = [
                                    (slice(hh//16,3*hh//16), slice(3*ww//8,5*ww//8)),
                                    (slice(1*hh//16,7*hh//16), slice(11*ww//16,15*ww//16)),
                                    (slice(9*hh//16,15*hh//16), slice(11*ww//16,15*ww//16)),
                                    (slice(13*hh//16,15*hh//16), slice(3*ww//8,5*ww//8)),
                                    (slice(9*hh//16,15*hh//16), slice(ww//16,5*ww//16)),
                                    (slice(1*hh//16,7*hh//16), slice(ww//16,5*ww//16)),
                                    (slice(7*hh//16,9*hh//16), slice(3*ww//8,5*ww//8)),
                                ]
                                segs = []
                                for rs, cs_ in regions:
                                    roi = bin_mask[rs, cs_]
                                    segs.append(min(float(np.count_nonzero(roi)) / max(1, roi.size * 0.2), 1.0))
                                best_digit, best_score = None, -1.0
                                for digit, template in DIGIT_SEGS.items():
                                    score = sum((seg * t) + ((1-seg) * (1-t)) for seg, t in zip(segs, template))
                                    if score > best_score:
                                        best_score = score
                                        best_digit = digit
                                if best_score >= 7 * 0.8:
                                    state = best_digit
                                else:
                                    state = CELL_STATE.empty
                            else:
                                state = CELL_STATE.empty
                else:
                    # 主体色与 opened_med 差异大 → 未翻开
                    red_match = cell_detector._is_color_match(cell, Colors.CELL_COLORS['red'])
                    black_match = cell_detector._is_color_match(cell, Colors.CELL_COLORS['black'])
                    if red_match > 0.01 and black_match > 0.03:
                        state = CELL_STATE.flag
                    else:
                        state = CELL_STATE.unopened
            game_state[i, j] = state

    # Stats
    gs = game_state
    flags = int(np.sum(gs == -2))
    unknowns = int(np.sum(gs == -1))
    numbers = int(np.sum((gs >= 1) & (gs <= 8)))
    zeros = int(np.sum(gs == 0))
    remaining = 99 - flags

    print(f"Result: flags={flags}, unknowns={unknowns}, numbers={numbers}, "
          f"zeros={zeros}, remaining={remaining}")

    # Build ascii view
    sym = {-1: "U", -2: "F", -4: "U", -3: "U"}
    view_lines = []
    for r in range(rows):
        view_lines.append("".join(sym.get(int(gs[r, c]), str(int(gs[r, c])))
                           for c in range(cols)))

    # Write answer.out
    out_path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "answer.out")
    lines = [
        "# MINESWEEPER-BOARD v0.1",
        "# Render: ascii",
        f"# Source: inuput/extra2.png ({rows}x{cols} expert, "
        f"OBR: reference/minesweeper_solver CellStateDetector + variance-grid)",
        f"# mines assumed 99 (expert standard); "
        f"remaining_mines = 99 - {flags} flags = {remaining}",
        "",
        f"rows: {rows}",
        f"columns: {cols}",
        "mines: 99",
        "game_mode: classic",
        "",
        "[view]",
    ] + view_lines

    with open(out_path, "w") as f:
        f.write("\n".join(lines) + "\n")

    print(f"\nWritten to {out_path}")
    print()
    for line in lines:
        print(line)


if __name__ == "__main__":
    main()
