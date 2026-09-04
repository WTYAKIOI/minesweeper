# SPDX-FileCopyrightText: 2022 James R. Barlow
# SPDX-License-Identifier: MPL-2.0

"""Tests for PDF constructs that the pipeline has to cope with.

Form XObjects, image masks, vector-only pages, missing /Contents, corrupt ICC
profiles, linearization and indirect objects.
"""

from __future__ import annotations

from unittest.mock import patch

import pikepdf
import pytest

from ocrmypdf.exceptions import ExitCode
from ocrmypdf.pdfinfo import PdfInfo

from .conftest import check_ocrmypdf, run_ocrmypdf_api

# pylint: disable=redefined-outer-name


def test_overlay(resources, outpdf):
    check_ocrmypdf(
        resources / 'overlay.pdf',
        outpdf,
        '--skip-text',
        '--output-type',
        'pdf',
        '--plugin',
        'tests/plugins/tesseract_noop.py',
    )


def test_no_contents(resources, outpdf):
    check_ocrmypdf(
        resources / 'no_contents.pdf',
        outpdf,
        '--force-ocr',
        '--output-type',
        'pdf',
        '--plugin',
        'tests/plugins/tesseract_noop.py',
    )


def test_form_xobject(resources, outpdf):
    check_ocrmypdf(
        resources / 'formxobject.pdf',
        outpdf,
        '--force-ocr',
        '--output-type',
        'pdf',
        '--plugin',
        'tests/plugins/tesseract_noop.py',
    )


def test_linearized_pdf_and_indirect_object(resources, outpdf):
    check_ocrmypdf(
        resources / 'epson.pdf',
        outpdf,
        '--output-type',
        'pdf',
        '--plugin',
        'tests/plugins/tesseract_noop.py',
    )


def test_text_curves(resources, outpdf):
    with patch('ocrmypdf._pipeline.VECTOR_PAGE_DPI', 100):
        check_ocrmypdf(
            resources / 'vector.pdf',
            outpdf,
            '--output-type',
            'pdf',
            '--plugin',
            'tests/plugins/tesseract_noop.py',
        )

        info = PdfInfo(outpdf)
        assert len(info.pages[0].images) == 0, "added images to the vector PDF"


def test_text_curves_force(resources, outpdf):
    with patch('ocrmypdf._pipeline.VECTOR_PAGE_DPI', 100):
        check_ocrmypdf(
            resources / 'vector.pdf',
            outpdf,
            '--force-ocr',
            '--output-type',
            'pdf',
            '--plugin',
            'tests/plugins/tesseract_noop.py',
        )

        info = PdfInfo(outpdf)
        assert len(info.pages[0].images) != 0, "force did not rasterize"


@pytest.fixture
def graph_bad_icc(resources, outdir):
    synth_input_file = outdir / 'graph-bad-icc.pdf'
    with pikepdf.open(resources / 'graph.pdf') as pdf:
        icc = pdf.make_stream(
            b'invalid icc profile', N=3, Alternate=pikepdf.Name.DeviceRGB
        )
        pdf.pages[0].Resources.XObject['/Im0'].ColorSpace = pikepdf.Array(
            [pikepdf.Name.ICCBased, icc]
        )
        pdf.save(synth_input_file)
        yield synth_input_file


def test_corrupt_icc(graph_bad_icc, outpdf, caplog):
    result = run_ocrmypdf_api(graph_bad_icc, outpdf)
    assert result == ExitCode.ok
    assert any(
        'corrupt or unreadable ICC profile' in rec.message for rec in caplog.records
    )
