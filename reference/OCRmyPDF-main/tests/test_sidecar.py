# SPDX-FileCopyrightText: 2022 James R. Barlow
# SPDX-License-Identifier: MPL-2.0

"""Tests for the --sidecar text file output."""

from __future__ import annotations

from ocrmypdf.pdfinfo import PdfInfo

from .conftest import check_ocrmypdf


def test_sidecar_pagecount(resources, outpdf):
    sidecar = outpdf.with_suffix('.txt')
    check_ocrmypdf(
        resources / '3small.pdf',
        outpdf,
        '--skip-text',
        '--sidecar',
        sidecar,
        '--output-type',
        'pdf',
        '--plugin',
        'tests/plugins/tesseract_cache.py',
    )

    pdfinfo = PdfInfo(resources / '3small.pdf')
    num_pages = len(pdfinfo)

    with sidecar.open(encoding='utf-8') as f:
        ocr_text = f.read()

    # There should a formfeed between each pair of pages, so the count of
    # formfeeds is the page count less one
    assert ocr_text.count('\f') == num_pages - 1, (
        "Sidecar page count does not match PDF page count"
    )


def test_sidecar_nonempty(resources, outpdf):
    sidecar = outpdf.with_suffix('.txt')
    check_ocrmypdf(
        resources / 'ccitt.pdf',
        outpdf,
        '--sidecar',
        sidecar,
        '--output-type',
        'pdf',
        '--plugin',
        'tests/plugins/tesseract_cache.py',
    )

    with sidecar.open(encoding='utf-8') as f:
        ocr_text = f.read()
    assert 'the' in ocr_text
