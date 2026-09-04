# SPDX-FileCopyrightText: 2026 James R. Barlow
# SPDX-License-Identifier: MPL-2.0

"""Tests for --max-ocr-image-mpixels."""

from __future__ import annotations

import pytest

from ocrmypdf._options import OcrOptions
from ocrmypdf.exceptions import ExitCode

from .conftest import run_ocrmypdf_api

RECORDER = 'tests/plugins/ocr_image_recorder.py'


@pytest.fixture
def ocr_image_sizes(monkeypatch, tmp_path):
    """Report the size of each image the OCR engine was given."""
    logfile = tmp_path / 'ocr_image_sizes.txt'
    monkeypatch.setenv('OCRMYPDF_TEST_OCR_IMAGE_LOG', str(logfile))

    def read():
        return [
            (int(w), int(h), mode)
            for w, h, mode in (
                line.split() for line in logfile.read_text().splitlines()
            )
        ]

    return read


def test_ocr_image_uncapped_by_default(resources, outpdf, ocr_image_sizes):
    assert (
        run_ocrmypdf_api(
            resources / 'ccitt.pdf', outpdf, '--plugin', RECORDER, '--pages', '1'
        )
        == ExitCode.ok
    )
    (width, height, _mode), *_ = ocr_image_sizes()
    assert width * height > 8_000_000, "ccitt.pdf should rasterize to 8.4 MP"


def test_max_ocr_image_mpixels_downsamples(resources, outpdf, ocr_image_sizes):
    assert (
        run_ocrmypdf_api(
            resources / 'ccitt.pdf',
            outpdf,
            '--plugin',
            RECORDER,
            '--max-ocr-image-mpixels',
            '2',
            '--pages',
            '1',
        )
        == ExitCode.ok
    )
    (width, height, _mode), *_ = ocr_image_sizes()
    assert width * height <= 2_000_000

    # The aspect ratio must survive, or the text layer will not line up.
    assert width / height == pytest.approx(2550 / 3300, rel=1e-2)


def test_max_ocr_image_mpixels_leaves_small_pages_alone(
    resources, outpdf, ocr_image_sizes
):
    """A cap above the page's size must not resample it."""
    assert (
        run_ocrmypdf_api(
            resources / 'ccitt.pdf',
            outpdf,
            '--plugin',
            RECORDER,
            '--max-ocr-image-mpixels',
            '100',
            '--pages',
            '1',
        )
        == ExitCode.ok
    )
    (width, height, _mode), *_ = ocr_image_sizes()
    assert (width, height) == (2550, 3300)


@pytest.mark.parametrize('value', [0, -1])
def test_max_ocr_image_mpixels_must_be_positive(value):
    """A cap of zero or less would leave no image to recognize."""
    with pytest.raises(
        ValueError, match=r"max_ocr_image_mpixels\n.*Input should be greater than 0"
    ):
        OcrOptions(input_file='a.pdf', output_file='b.pdf', max_ocr_image_mpixels=value)


def test_max_ocr_image_mpixels_defaults_to_unbounded():
    opts = OcrOptions(input_file='a.pdf', output_file='b.pdf')
    assert opts.max_ocr_image_mpixels is None
