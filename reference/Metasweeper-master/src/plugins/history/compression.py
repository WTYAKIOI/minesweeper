"""
录像数据压缩/解压工具

使用 zlib 压缩 raw_data (BLOB)，通过 magic byte 前缀检测是否已压缩，
无需依赖 compressed 列即可正确读取。
"""

from __future__ import annotations

import zlib

# zlib 压缩数据以 0x78 开头 (level 6 = 0x78 0x9C)
_ZLIB_MAGIC = b'\x78'


def compress(data: bytes | None) -> bytes | None:
    """压缩数据，返回压缩后的 bytes。None 输入返回 None。"""
    if data is None:
        return None
    return zlib.compress(data, level=6)


def decompress(data: bytes | None) -> bytes | None:
    """
    解压数据。自动检测是否已压缩：
    - None → None
    - 以 zlib magic byte 开头 → 解压
    - 否则 → 原样返回（兼容未压缩的旧数据）
    """
    if data is None:
        return None
    if data[:1] == _ZLIB_MAGIC:
        return zlib.decompress(data)
    return data


def is_compressed(data: bytes | None) -> bool:
    """判断数据是否已压缩"""
    if data is None or len(data) == 0:
        return False
    return data[:1] == _ZLIB_MAGIC
