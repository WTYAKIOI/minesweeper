from __future__ import annotations

import msgspec


class BaseEvent(msgspec.Struct, tag=True):
    """事件推送基类（Server → Client）

    上层继承时使用 tag 指定类型名：
        class BoardUpdate(BaseEvent, tag="board_update"):
            cells: list[CellData]
    """
    timestamp: float = 0.0


class BaseCommand(msgspec.Struct, tag=True):
    """控制指令基类（Client → Server）

    上层继承时使用 tag 指定类型名：
        class ClickCommand(BaseCommand, tag="click"):
            row: int
            col: int

    如需同步响应，Server handler 返回 CommandResponse 后由框架自动回传。
    """
    request_id: str = ""


class CommandResponse(msgspec.Struct, tag=True):
    """控制指令响应（Server → Client）"""
    request_id: str
    success: bool
    error: str | None = None
    data: bytes | None = None


class SyncCommand(BaseCommand, tag="__sync__"):
    """快照同步请求（库内部使用，Client 订阅时自动发送）

    Client 订阅某 topic 后，通过控制通道发送此请求给 Server，
    Server 调用对应 topic 的 snapshot_provider 获取当前完整状态并返回。
    用于解决 ZMQ PUB/SUB 的 Slow Joiner 问题。
    """
    topic: str = ""


def get_event_tag(event_type: type[BaseEvent]) -> str:
    """从事件类提取 tag 字符串，用作 ZMQ 订阅 topic"""
    tag = event_type.__struct_config__.tag
    if isinstance(tag, type):
        tag = tag.__name__
    return str(tag)
