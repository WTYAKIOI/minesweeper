from __future__ import annotations

import math


def format_interval_ms(seconds: float) -> str:
    return f"{seconds * 1000:.0f}ms"


def format_number(value: float) -> str:
    if value.is_integer():
        return str(int(value))
    return f"{value:.2f}"


def format_pluck(value: float) -> str:
    if math.isinf(value):
        return "inf"
    return f"{value:.3f}"


def format_probability_percent(value: float) -> str:
    if math.isnan(value):
        return "nan"
    return f"{value * 100:.2f}%"
