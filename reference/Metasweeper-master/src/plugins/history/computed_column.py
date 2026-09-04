"""
计算列定义

用户可配置的 SQL 计算列，通过子查询实现。
"""

from __future__ import annotations

import json
from dataclasses import dataclass


@dataclass
class ComputedColumn:
    """
    计算列定义

    Attributes:
        name: 列名（如 "bbbvs"），需为合法 SQL 别名
        expression: SQL 表达式（如 "bbbv * 1.0 / rtime"）
        result_type: 结果类型 "int" | "float"，决定显示精度和 delegate
    """

    name: str
    expression: str
    result_type: str = "float"

    def to_dict(self) -> dict:
        return {
            "name": self.name,
            "expression": self.expression,
            "result_type": self.result_type,
        }

    @classmethod
    def from_dict(cls, data: dict) -> ComputedColumn:
        return cls(
            name=data.get("name", ""),
            expression=data.get("expression", ""),
            result_type=data.get("result_type", "float"),
        )

    @classmethod
    def from_json(cls, json_str: str) -> list[ComputedColumn]:
        """从 JSON 字符串解析计算列列表"""
        if not json_str:
            return []
        try:
            items = json.loads(json_str)
        except (json.JSONDecodeError, TypeError):
            return []
        if not isinstance(items, list):
            return []
        return [cls.from_dict(item) for item in items if isinstance(item, dict)]

    @staticmethod
    def to_json(columns: list[ComputedColumn]) -> str:
        """将计算列列表序列化为 JSON 字符串"""
        return json.dumps(
            [col.to_dict() for col in columns], ensure_ascii=False
        )

    @staticmethod
    def build_subquery_sql(columns: list[ComputedColumn]) -> str | None:
        """
        构建计算列子查询的 FROM 部分

        Returns:
            子查询 SQL（如 "SELECT *, (bbbv*1.0/rtime) AS \"bbbvs\" FROM history"），
            或 None（无计算列时）
        """
        if not columns:
            return None
        computed_parts = ", ".join(
            f"({col.expression}) AS \"{col.name}\"" for col in columns
        )
        return f"SELECT *, {computed_parts} FROM history"
