# SPDX-FileCopyrightText: 2022 James R. Barlow
# SPDX-License-Identifier: MPL-2.0

"""Tests for the OCR mode options: force, skip, redo, skip-big and timeouts."""

from __future__ import annotations

import pytest

from ocrmypdf.exceptions import ExitCode
from ocrmypdf.pdfinfo import PdfInfo

from .conftest import RENDERERS, check_ocrmypdf, run_ocrmypdf_api


def test_repeat_ocr(resources, no_outpdf):
    result = run_ocrmypdf_api(resources / 'graph_ocred.pdf', no_outpdf)
    assert result == ExitCode.already_done_ocr


def test_force_ocr(resources, outpdf):
    out = check_ocrmypdf(
        resources / 'graph_ocred.pdf',
        outpdf,
        '-f',
        '--output-type',
        'pdf',
        '--plugin',
        'tests/plugins/tesseract_cache.py',
    )
    pdfinfo = PdfInfo(out)
    assert pdfinfo[0].has_text


def test_skip_ocr(resources, outpdf):
    out = check_ocrmypdf(
        resources / 'graph_ocred.pdf',
        outpdf,
        '-s',
        '--output-type',
        'pdf',
        '--plugin',
        'tests/plugins/tesseract_cache.py',
    )
    pdfinfo = PdfInfo(out)
    assert pdfinfo[0].has_text


def test_redo_ocr(resources, outpdf):
    in_ = resources / 'graph_ocred.pdf'
    before = PdfInfo(in_, detailed_analysis=True)
    out = outpdf
    out = check_ocrmypdf(in_, out, '--redo-ocr', '--output-type', 'pdf')
    after = PdfInfo(out, detailed_analysis=True)
    assert before[0].has_text and after[0].has_text
    assert before[0].get_textareas() != after[0].get_textareas(), (
        "Expected text to be different after re-OCR"
    )


@pytest.mark.parametrize('renderer', RENDERERS)
def test_ocr_timeout(renderer, resources, outpdf):
    out = check_ocrmypdf(
        resources / 'skew.pdf',
        outpdf,
        '--tesseract-timeout',
        '0',
        '--pdf-renderer',
        renderer,
        '--output-type',
        'pdf',
    )
    pdfinfo = PdfInfo(out)
    assert not pdfinfo[0].has_text


def test_skip_big(resources, outpdf):
    out = check_ocrmypdf(
        resources / 'jbig2.pdf',
        outpdf,
        '--skip-big',
        '1',
        '--output-type',
        'pdf',
        '--plugin',
        'tests/plugins/tesseract_cache.py',
    )
    pdfinfo = PdfInfo(out)
    assert not pdfinfo[0].has_text


def test_skip_big_with_no_images(resources, outpdf):
    check_ocrmypdf(
        resources / 'blank.pdf',
        outpdf,
        '--skip-big',
        '5',
        '--force-ocr',
        '--output-type',
        'pdf',
        '--plugin',
        'tests/plugins/tesseract_noop.py',
    )


def test_force_ocr_on_pdf_with_no_images(resources, no_outpdf):
    # As a correctness test, make sure that --force-ocr on a PDF with no
    # content still triggers tesseract. If tesseract crashes, then it was
    # called.
    exitcode = run_ocrmypdf_api(
        resources / 'blank.pdf',
        no_outpdf,
        '--force-ocr',
        '--plugin',
        'tests/plugins/tesseract_crash.py',
    )
    assert exitcode == ExitCode.child_process_error
    assert not no_outpdf.exists()
