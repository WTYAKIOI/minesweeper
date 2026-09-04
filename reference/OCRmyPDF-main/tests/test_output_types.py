# SPDX-FileCopyrightText: 2022 James R. Barlow
# SPDX-License-Identifier: MPL-2.0

"""Tests for --output-type and related output packaging options."""

from __future__ import annotations

import pikepdf
import pytest

from ocrmypdf.exceptions import ExitCode

from .conftest import check_ocrmypdf, run_ocrmypdf_api


def test_outputtype_none(resources, outtxt):
    exitcode = run_ocrmypdf_api(
        resources / 'trivial.pdf',
        '-',
        '--output-type=none',
        '--sidecar',
        outtxt,
        '--plugin',
        'tests/plugins/tesseract_noop.py',
    )
    assert exitcode == ExitCode.ok
    assert outtxt.exists()


# The linearization threshold logic itself is unit-tested in
# test_pipeline.py::test_should_linearize_threshold; these cases confirm the
# option reaches the save path for both optimize settings, plus one PDF/A case.
@pytest.mark.parametrize(
    'threshold, optimize, output_type, expected',
    [
        [1.0, 0, 'pdf', False],
        [0.0, 0, 'pdf', True],
        [1.0, 1, 'pdf', False],
        [0.0, 1, 'pdf', True],
        [0.0, 0, 'pdfa', True],
    ],
)
def test_fast_web_view(resources, outpdf, threshold, optimize, output_type, expected):
    check_ocrmypdf(
        resources / 'trivial.pdf',
        outpdf,
        '--fast-web-view',
        threshold,
        '--optimize',
        optimize,
        '--output-type',
        output_type,
        '--plugin',
        'tests/plugins/tesseract_noop.py',
    )
    with pikepdf.open(outpdf) as pdf:
        assert pdf.is_linearized == expected
