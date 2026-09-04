"""
统一日志模块

基于 loguru 的日志系统。在整个应用中作为唯一日志入口使用。
"""
from __future__ import annotations

import sys

from loguru import logger

logger.remove()

if sys.stderr is not None:
    logger.add(
        sys.stderr,
        format="<level>{level:7}</level> | <cyan>{name}</cyan>:<cyan>{function}</cyan>:<cyan>{line}</cyan> - <level>{message}</level>",
        level="DEBUG",
        colorize=True,
    )

__all__ = ["logger"]
