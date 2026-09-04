# SPDX-FileCopyrightText: 2025 James R. Barlow
# SPDX-License-Identifier: MPL-2.0

"""Unit tests for the base FontManager."""

from __future__ import annotations

from pathlib import Path

import pytest

from ocrmypdf.font import FontManager

FONT_DIR = Path(__file__).parent.parent / "src" / "ocrmypdf" / "data"

# A plane-16 private use codepoint: no real font assigns a glyph to it.
UNMAPPED_CHAR = chr(0x10FFFD)


@pytest.fixture
def font_dir():
    """Return path to the bundled font directory."""
    return FONT_DIR


@pytest.fixture
def noto(font_dir):
    """A FontManager for the bundled Latin font."""
    font_path = font_dir / 'NotoSans-Regular.ttf'
    if not font_path.exists():
        pytest.skip(f"{font_path} is not available")
    return FontManager(font_path)


class _StubHbFont:
    """uharfbuzz Font stand-in with controllable glyph lookup and extents."""

    def __init__(self, glyph_id: int | None, extents: object = None):
        self._glyph_id = glyph_id
        self._extents = extents

    def get_nominal_glyph(self, codepoint: int) -> int | None:
        return self._glyph_id

    def get_glyph_extents(self, glyph_id: int) -> object:
        return self._extents


def test_left_side_bearing_of_empty_string_is_zero(noto):
    """No character means no bearing; the font is never consulted."""
    assert noto.get_left_side_bearing('', 12.0) == 0.0


def test_left_side_bearing_of_unmapped_character_is_zero(noto):
    """A codepoint the font cannot map contributes no bearing."""
    assert noto.get_left_side_bearing(UNMAPPED_CHAR, 12.0) == 0.0


def test_left_side_bearing_zero_when_font_returns_notdef(noto, monkeypatch):
    """Glyph id 0 (.notdef) is treated as 'no glyph', not as a real glyph."""
    monkeypatch.setattr(noto, 'hb_font', _StubHbFont(glyph_id=0))
    assert noto.get_left_side_bearing('H', 12.0) == 0.0


def test_left_side_bearing_zero_when_glyph_has_no_extents(noto, monkeypatch):
    """A mapped glyph without extents (e.g. a space) yields no bearing."""
    monkeypatch.setattr(noto, 'hb_font', _StubHbFont(glyph_id=42, extents=None))
    assert noto.get_left_side_bearing('H', 12.0) == 0.0


def test_left_side_bearing_of_capital_h_is_nonzero(noto):
    """A real glyph has real sidebearing whitespace before its ink starts."""
    lsb = noto.get_left_side_bearing('H', 12.0)
    assert lsb > 0.0
    # Sanity bound: a sidebearing is a small fraction of the em, never near it.
    assert lsb < 12.0


def test_left_side_bearing_scales_linearly_with_font_size(noto):
    """The bearing is reported in points, so it scales with the font size."""
    lsb_12 = noto.get_left_side_bearing('H', 12.0)
    lsb_24 = noto.get_left_side_bearing('H', 24.0)
    lsb_6 = noto.get_left_side_bearing('H', 6.0)
    assert lsb_24 == pytest.approx(2 * lsb_12)
    assert lsb_6 == pytest.approx(lsb_12 / 2)
    assert noto.get_left_side_bearing('H', 0.0) == 0.0


def test_left_side_bearing_matches_font_units_conversion(noto):
    """The conversion is x_bearing * font_size / units_per_em, exactly."""
    glyph_id = noto.hb_font.get_nominal_glyph(ord('H'))
    extents = noto.hb_font.get_glyph_extents(glyph_id)
    expected = extents.x_bearing * 18.0 / noto.hb_face.upem
    assert noto.get_left_side_bearing('H', 18.0) == pytest.approx(expected)


def test_has_glyph_distinguishes_mapped_from_unmapped(noto):
    """has_glyph() reports real coverage, not merely a successful lookup."""
    assert noto.has_glyph(ord('H'))
    assert not noto.has_glyph(ord(UNMAPPED_CHAR))


def test_font_metrics_are_plausible(noto):
    """Ascent is above the baseline, descent below, both within one em."""
    ascent, descent, upem = noto.get_font_metrics()
    assert upem > 0
    assert ascent > 0
    assert descent < 0
    assert (ascent - descent) < 3 * upem


def test_get_hb_font_returns_the_loaded_font(noto):
    """The accessor exposes the font object built from the loaded file."""
    assert noto.get_hb_font() is noto.hb_font
    assert noto.font_index == 0
    assert noto.font_data == noto.font_path.read_bytes()
