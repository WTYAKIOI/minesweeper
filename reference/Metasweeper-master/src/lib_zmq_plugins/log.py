from __future__ import annotations

from typing import Protocol, runtime_checkable


@runtime_checkable
class LogHandler(Protocol):
    """日志处理器协议，使用者自行实现并传入 Server/Client

    参考 loguru 的日志级别：DEBUG < INFO < WARNING < ERROR

    示例 - 对接 loguru::

        from loguru import logger as loguru_logger

        class LoguruHandler:
            def debug(self, msg: str, /, **kwargs: object) -> None:
                loguru_logger.debug(msg)
            def info(self, msg: str, /, **kwargs: object) -> None:
                loguru_logger.info(msg)
            def warning(self, msg: str, /, **kwargs: object) -> None:
                loguru_logger.warning(msg)
            def error(self, msg: str, /, **kwargs: object) -> None:
                loguru_logger.error(msg, **kwargs)

    示例 - 对接标准库 logging::

        import logging
        _log = logging.getLogger("zmq_plugins")

        class StdlibHandler:
            def debug(self, msg: str, /, **kwargs: object) -> None:
                _log.debug(msg, **kwargs)
            def info(self, msg: str, /, **kwargs: object) -> None:
                _log.info(msg)
            def warning(self, msg: str, /, **kwargs: object) -> None:
                _log.warning(msg)
            def error(self, msg: str, /, **kwargs: object) -> None:
                _log.error(msg, **kwargs)
    """

    def debug(self, msg: str, /, *args: object, **kwargs: object) -> None: ...
    def info(self, msg: str, /, *args: object, **kwargs: object) -> None: ...
    def warning(self, msg: str, /, *args: object, **kwargs: object) -> None: ...
    def error(self, msg: str, /, *args: object, **kwargs: object) -> None: ...


class NullHandler:
    """默认日志处理器，所有日志丢弃"""

    def debug(self, msg: str, /, *args: object, **kwargs: object) -> None: ...
    def info(self, msg: str, /, *args: object, **kwargs: object) -> None: ...
    def warning(self, msg: str, /, *args: object, **kwargs: object) -> None: ...
    def error(self, msg: str, /, *args: object, **kwargs: object) -> None: ...
