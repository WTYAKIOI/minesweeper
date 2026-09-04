"""
历史记录插件

功能：
- 监听 GameFinishedEvent，将游戏录像数据持久化到 SQLite 数据库
- 提供 GUI 界面：表格浏览、筛选、分页、播放/导出录像
- 使用 self.data_dir 存储数据库文件（每个插件独立目录）
"""

from .plugin import HistoryPlugin

__all__ = ["HistoryPlugin"]
