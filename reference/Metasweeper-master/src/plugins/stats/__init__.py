"""
统计插件

功能：
- 从 HistoryService 获取历史数据，展示统计图表
- 使用 QML + Qt Charts 绘制图表
- 支持按难度/模式筛选
"""

from .plugin import StatsPlugin

__all__ = ["StatsPlugin"]
