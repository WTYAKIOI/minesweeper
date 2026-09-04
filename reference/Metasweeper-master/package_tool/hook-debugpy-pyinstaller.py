"""
PyInstaller runtime hook for debugpy compatibility.
在 import debugpy 之前修复路径问题，确保 _vendored 目录可被找到。
"""
import os
import sys

# PyInstaller 打包后 __file__ 指向临时目录或 zip 内，
# debugpy/_vendored/__init__.py 用 os.path.abspath(__file__) 定位资源会失败。
# 此 hook 在任何代码执行前将 debugpy 的真实解压路径注入 sys._MEIPASS 搜索逻辑。

def _fix_debugpy_paths():
    meipass = getattr(sys, "_MEIPASS", None)
    if not meipass:
        return

    debugpy_dir = os.path.join(meipass, "debugpy")

    if os.path.isdir(debugpy_dir):
        if debugpy_dir not in sys.path:
            sys.path.insert(0, debugpy_dir)
        # debugpy 1.8+ 将 vendored 包直接内联，不再有 _vendored 目录
        vendored_dir = os.path.join(debugpy_dir, "_vendored")
        if os.path.isdir(vendored_dir) and vendored_dir not in sys.path:
            sys.path.insert(0, vendored_dir)
        # debugpy 1.9+ 可能将 vendored 放到了 deeper 路径
        for sub in ("_vendored", "_vendored2"):
            p = os.path.join(debugpy_dir, sub)
            if os.path.isdir(p) and p not in sys.path:
                sys.path.insert(0, p)

_fix_debugpy_paths()
