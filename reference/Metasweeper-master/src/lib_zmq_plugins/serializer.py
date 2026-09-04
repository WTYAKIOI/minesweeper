from __future__ import annotations

from typing import TYPE_CHECKING, Any, Union

import enum
import msgspec

from lib_zmq_plugins.shared.base import CommandResponse

if TYPE_CHECKING:
    from lib_zmq_plugins.shared.base import BaseCommand, BaseEvent


def _enc_hook(obj: object) -> Any:
    """编码 hook：将 Enum 转为其原始值"""
    if isinstance(obj, enum.Enum):
        return obj.value
    raise NotImplementedError(f"Cannot encode {type(obj).__name__}")


def _dec_hook(type_hint: type, obj: object) -> object:
    """解码 hook：将原始值还原为 Enum"""
    if isinstance(type_hint, type) and issubclass(type_hint, enum.Enum):
        return type_hint(obj)
    raise NotImplementedError(f"Cannot decode to {type_hint.__name__}")


def _make_union(types: list[type]) -> type:
    """将类型列表转为 Union 类型，供 msgspec 多态反序列化使用"""
    if len(types) == 1:
        return types[0]
    return Union[tuple(types)]  # type: ignore


class Serializer:
    """序列化器，持有上层注册的类型信息"""

    def __init__(self) -> None:
        self._event_types: list[type[BaseEvent]] = []
        self._command_types: list[type[BaseCommand]] = []
        self._event_union: type = object
        self._command_union: type = object

    def register_event_types(self, *types: type[BaseEvent]) -> None:
        self._event_types.extend(types)
        self._event_union = _make_union(self._event_types)

    def register_command_types(self, *types: type[BaseCommand]) -> None:
        self._command_types.extend(types)
        self._command_union = _make_union(self._command_types)

    def encode_event(self, event: BaseEvent) -> bytes:
        return msgspec.msgpack.encode(event, enc_hook=_enc_hook)

    def decode_event(self, data: bytes) -> BaseEvent:
        if not self._event_types:
            raise ValueError("No event types registered")
        return msgspec.msgpack.decode(
            data, type=self._event_union, dec_hook=_dec_hook
        )

    def encode_command(self, cmd: BaseCommand) -> bytes:
        return msgspec.msgpack.encode(cmd, enc_hook=_enc_hook)

    def decode_command(self, data: bytes) -> BaseCommand:
        if not self._command_types:
            raise ValueError("No command types registered")
        return msgspec.msgpack.decode(
            data, type=self._command_union, dec_hook=_dec_hook
        )

    def encode_response(self, resp: CommandResponse) -> bytes:
        return msgspec.msgpack.encode(resp, enc_hook=_enc_hook)

    def decode_response(self, data: bytes) -> CommandResponse:
        return msgspec.msgpack.decode(
            data, type=CommandResponse, dec_hook=_dec_hook
        )
