# SPDX-FileCopyrightText: 2026 James R. Barlow
# SPDX-License-Identifier: MPL-2.0

"""Tests for ocrmypdf.pdfinfo.layout, our pdfminer.six layout analysis."""

from __future__ import annotations

from types import SimpleNamespace
from unittest.mock import patch

import pytest
from pdfminer.layout import LTChar, LTPage
from pdfminer.pdfcolor import PREDEFINED_COLORSPACE
from pdfminer.pdfdocument import PDFTextExtractionNotAllowed
from pdfminer.pdffont import PDFFont, PDFType3Font
from pdfminer.pdfinterp import PDFGraphicState, PDFTextState
from pdfminer.pdfpage import PDFPage

from ocrmypdf.exceptions import EncryptedPdfError, InputFileError
from ocrmypdf.pdfinfo.layout import (
    LTStateAwareChar,
    PdfMinerState,
    _is_undefined_char,
    get_page_analysis,
    get_text_boxes,
    patch_pdfminer,
    pdftype3font__pscript5_get_ascent,
    pdftype3font__pscript5_get_descent,
    pdftype3font__pscript5_get_height,
)

UNDEFINED = '�'


@pytest.mark.parametrize(
    ('text', 'expected'),
    [
        ('(cid:1)', True),
        ('(cid:1234)', True),
        ('(cid:)', True),  # we do not check the contents
        ('(cid:1', False),  # no closing parenthesis
        ('cid:1)', False),  # no opening marker
        ('A', False),
        ('', False),
        ('(cid:1) ', False),  # trailing text means it is a real string
    ],
)
def test_is_undefined_char(text, expected):
    assert _is_undefined_char(text) is expected


def make_char(text: str, fontname: str = 'Foo', rendermode: int = 0):
    """Construct an LTStateAwareChar the way TextPositionTracker.render_char does."""
    font = PDFFont(
        descriptor={'FontName': fontname, 'Ascent': 800, 'Descent': -200}, widths={}
    )
    textstate = PDFTextState()
    textstate.render = rendermode
    return LTStateAwareChar(
        (1, 0, 0, 1, 0, 0),
        font,
        10.0,
        100.0,
        0.0,
        text,
        1.0,
        (None, 0.5),
        PREDEFINED_COLORSPACE['DeviceGray'],
        PDFGraphicState(),
        textstate,
    )


def test_get_text_replaces_undefined_char():
    assert make_char('(cid:42)').get_text() == UNDEFINED


def test_get_text_passes_through_defined_char():
    assert make_char('A').get_text() == 'A'


def test_repr_shows_rendermode_and_resolved_text():
    text = repr(make_char('(cid:42)', fontname='Bar', rendermode=3))
    assert 'rendermode=3' in text
    assert "font='Bar'" in text
    assert repr(UNDEFINED) in text


def test_rendermode_is_captured_from_textstate():
    assert make_char('A', rendermode=3).rendermode == 3


@pytest.mark.parametrize(
    ('text1', 'font1', 'mode1', 'text2', 'font2', 'mode2', 'compatible'),
    [
        # Both Unicode-mapped: only the render mode matters
        ('A', 'Foo', 0, 'B', 'Bar', 0, True),
        ('A', 'Foo', 0, 'B', 'Foo', 3, False),
        # Undefined characters may only join text from the same font
        ('(cid:1)', 'Foo', 0, '(cid:2)', 'Foo', 0, True),
        ('(cid:1)', 'Foo', 0, '(cid:2)', 'Bar', 0, False),
        ('(cid:1)', 'Foo', 0, '(cid:2)', 'Foo', 3, False),
        # One undefined character is enough to require the same font
        ('(cid:1)', 'Foo', 0, 'B', 'Bar', 0, False),
        ('A', 'Foo', 0, '(cid:2)', 'Foo', 0, True),
    ],
)
def test_is_compatible(text1, font1, mode1, text2, font2, mode2, compatible):
    char1 = make_char(text1, fontname=font1, rendermode=mode1)
    char2 = make_char(text2, fontname=font2, rendermode=mode2)
    assert char1.is_compatible(char2) is compatible


def test_is_compatible_rejects_plain_ltchar():
    """A character without a tracked render mode cannot be merged."""
    font = PDFFont(descriptor={'FontName': 'Foo'}, widths={})
    plain = LTChar(
        (1, 0, 0, 1, 0, 0),
        font,
        10.0,
        100.0,
        0.0,
        'A',
        1.0,
        (None, 0.5),
        PREDEFINED_COLORSPACE['DeviceGray'],
        PDFGraphicState(),
    )
    assert make_char('A').is_compatible(plain) is False


def make_type3_stub(bbox=(0, 0, 100, 200), ascent=800, descent=-200, vscale=0.001):
    return SimpleNamespace(bbox=bbox, ascent=ascent, descent=descent, vscale=vscale)


def test_pscript5_height_from_bbox():
    assert pdftype3font__pscript5_get_height(make_type3_stub()) == 200


def test_pscript5_height_falls_back_to_ascent_minus_descent():
    """A degenerate (zero-height) FontBBox is the bug this patch exists for."""
    stub = make_type3_stub(bbox=(0, 50, 100, 50))
    assert pdftype3font__pscript5_get_height(stub) == 1000


@pytest.mark.parametrize('vscale', [0.001, -0.001])
def test_pscript5_metrics_follow_sign_of_vscale(vscale):
    """PScript5 encodes a flipped glyph space in the sign of vscale."""
    stub = make_type3_stub(vscale=vscale)
    sign = 1 if vscale > 0 else -1
    assert pdftype3font__pscript5_get_height(stub) == 200 * sign
    assert pdftype3font__pscript5_get_ascent(stub) == 800 * sign
    assert pdftype3font__pscript5_get_descent(stub) == -200 * sign


def test_patch_pdfminer_installs_and_removes_patches():
    original = (
        PDFType3Font.get_ascent,
        PDFType3Font.get_descent,
        PDFType3Font.get_height,
    )
    with patch_pdfminer(True):
        assert PDFType3Font.get_ascent is pdftype3font__pscript5_get_ascent
        assert PDFType3Font.get_descent is pdftype3font__pscript5_get_descent
        assert PDFType3Font.get_height is pdftype3font__pscript5_get_height
    assert (
        PDFType3Font.get_ascent,
        PDFType3Font.get_descent,
        PDFType3Font.get_height,
    ) == original


def test_patch_pdfminer_disabled_is_a_no_op():
    original = (
        PDFType3Font.get_ascent,
        PDFType3Font.get_descent,
        PDFType3Font.get_height,
    )
    with patch_pdfminer(False):
        assert (
            PDFType3Font.get_ascent,
            PDFType3Font.get_descent,
            PDFType3Font.get_height,
        ) == original


class TestPdfMinerState:
    def test_analyzes_a_page(self, resources):
        with PdfMinerState(resources / 'link.pdf', pscript5_mode=False) as state:
            page = state.get_page_analysis(0)
        assert isinstance(page, LTPage)
        assert list(get_text_boxes(page)), "should find text on this page"

    def test_pages_are_cached(self, resources):
        with PdfMinerState(resources / 'multipage.pdf', pscript5_mode=False) as state:
            state.get_page_analysis(1)
            assert len(state.page_cache) == 2
            state.get_page_analysis(0)
            assert len(state.page_cache) == 2, "should reuse the cached page"

    def test_page_past_end_of_file(self, resources):
        with (
            PdfMinerState(resources / 'trivial.pdf', pscript5_mode=False) as state,
            pytest.raises(InputFileError, match='did not find page 5'),
        ):
            state.get_page_analysis(5)

    def test_unusable_page_in_cache(self, resources):
        """A page that pdfminer produced but cannot interpret is an input error."""
        with PdfMinerState(resources / 'trivial.pdf', pscript5_mode=False) as state:
            state.page_cache.append(None)
            with pytest.raises(InputFileError, match='could not process page 0'):
                state.get_page_analysis(0)

    def test_exit_closes_file(self, resources):
        state = PdfMinerState(resources / 'trivial.pdf', pscript5_mode=False)
        with state:
            assert state.file is not None
        assert state.file.closed


class TestDeprecatedGetPageAnalysis:
    def test_analyzes_a_page(self, resources):
        with pytest.warns(DeprecationWarning, match='PdfMinerState'):
            page = get_page_analysis(resources / 'link.pdf', 0, False)
        assert isinstance(page, LTPage)
        assert list(get_text_boxes(page)), "should find text on this page"

    def test_page_past_end_of_file(self, resources):
        with (
            pytest.warns(DeprecationWarning),
            pytest.raises(InputFileError, match='could not process page 5'),
        ):
            get_page_analysis(resources / 'trivial.pdf', 5, False)

    def test_encrypted_file(self, resources):
        """Refusal by pdfminer to extract text means the file is encrypted."""
        with (
            patch.object(
                PDFPage, 'get_pages', side_effect=PDFTextExtractionNotAllowed()
            ),
            pytest.warns(DeprecationWarning),
            pytest.raises(EncryptedPdfError),
        ):
            get_page_analysis(resources / 'trivial.pdf', 0, False)

    def test_pscript5_mode_analyzes_and_unpatches(self, resources):
        """pscript5_mode analyzes normally and leaves pdfminer unpatched after."""
        original = PDFType3Font.get_height
        with pytest.warns(DeprecationWarning):
            page = get_page_analysis(resources / 'link.pdf', 0, True)
        assert isinstance(page, LTPage)
        assert PDFType3Font.get_height is original
