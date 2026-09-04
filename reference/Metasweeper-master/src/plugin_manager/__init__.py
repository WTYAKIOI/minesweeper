"""
插件管理器模块

基于 lib_zmq_plugins 实现的插件系统，支持：
- 插件同时具备后台数据处理和界面交互能力
- 事件分发机制
- 动态加载插件
- 独立的主界面窗口
- 插件自定义配置系统

模块结构：
- plugin_sdk: 插件开发 SDK（BasePlugin, config_types, service_registry）
- plugin_manager: 插件管理器内部实现（PluginManager, EventDispatcher, MainWindow）
"""

from .logging_setup import LogConfig
from .plugin_manager import PluginManager, run_plugin_manager_process
from .event_dispatcher import EventDispatcher
from .plugin_loader import PluginLoader
from .main_window import PluginManagerWindow
from .config_widget import OtherInfoWidget, OtherInfoScrollArea
from .config_manager import PluginConfigManager

__all__ = [
    "LogConfig",
    "PluginManager",
    "PluginManagerWindow",
    "EventDispatcher",
    "PluginLoader",
    "run_plugin_manager_process",
    "OtherInfoWidget",
    "OtherInfoScrollArea",
    "PluginConfigManager",
]