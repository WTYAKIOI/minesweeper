# SPDX-FileCopyrightText: 2022 James R. Barlow
# SPDX-License-Identifier: MPL-2.0

"""Deliberate whole-pipeline canaries."""

from __future__ import annotations

import pytest

from .conftest import check_ocrmypdf, have_unpaper


def test_quick(resources, outpdf):
    check_ocrmypdf(
        resources / 'ccitt.pdf', outpdf, '--plugin', 'tests/plugins/tesseract_cache.py'
    )


# Pairwise: each renderer and each output type appears once. The full
# renderer x output_type crossing is covered by test_preprocessing.py::
# test_exotic_image, test_page_boxes.py and test_rotation.py.
@pytest.mark.parametrize(
    'renderer, output_type', [('fpdf2', 'pdfa'), ('sandwich', 'pdf')]
)
def test_maximum_options(renderer, output_type, multipage, outpdf):
    check_ocrmypdf(
        multipage,
        outpdf,
        '--pages',
        '1-3',
        '-d',
        '-ci' if have_unpaper() else None,
        '-f',
        '-k',
        '--oversample',
        '300',
        '--skip-big',
        '10',
        '--title',
        'Too Many Weird Files',
        '--author',
        'py.test',
        '--pdf-renderer',
        renderer,
        '--output-type',
        output_type,
        '--plugin',
        'tests/plugins/tesseract_cache.py',
    )
