# SPDX-FileCopyrightText: 2022 James R. Barlow
# SPDX-License-Identifier: MPL-2.0

"""Tests for Pillow decompression bomb handling and --max-image-mpixels."""

from __future__ import annotations

import PIL.Image
import pytest

from ocrmypdf.exceptions import ExitCode

from .conftest import preserve_pil_max_image_pixels, run_ocrmypdf_api


def test_max_image_pixels_restored_between_tests():
    # Running the pipeline in-process with an explicit --max-image-mpixels
    # mutates the process-global PIL.Image.MAX_IMAGE_PIXELS. The autouse
    # conftest fixture must restore it so the limit does not leak into
    # subsequent tests in the same worker.
    fixture_body = preserve_pil_max_image_pixels.__wrapped__
    saved = PIL.Image.MAX_IMAGE_PIXELS
    try:
        gen = fixture_body()
        next(gen)
        PIL.Image.MAX_IMAGE_PIXELS = 1_000_000
        next(gen, None)
        assert saved == PIL.Image.MAX_IMAGE_PIXELS
    finally:
        PIL.Image.MAX_IMAGE_PIXELS = saved


def test_max_image_mpixels_zero_means_unlimited(resources, outpdf, caplog):
    # 0 disables Pillow's limit entirely, where 1 would reject this 8.4 MP page.
    assert (
        run_ocrmypdf_api(resources / 'ccitt.pdf', outpdf, '--max-image-mpixels', '0')
        == ExitCode.ok
    )
    assert 'decompression bomb' not in caplog.text


def test_decompression_bomb_error(resources, outpdf, caplog):
    # ccitt.pdf is 2550x3300 (8.4 MP), which is more than 2x the 1 MP limit set
    # here, so Pillow raises DecompressionBombError at the same rasterization
    # call site that hugemono.pdf reaches - but far more cheaply.
    run_ocrmypdf_api(resources / 'ccitt.pdf', outpdf, '--max-image-mpixels', '1')
    assert 'decompression bomb' in caplog.text
    assert 'max-image-mpixels' in caplog.text


@pytest.mark.slow
def test_decompression_bomb_succeeds(resources, outpdf):
    exitcode = run_ocrmypdf_api(
        resources / 'hugemono.pdf', outpdf, '--max-image-mpixels', '2000'
    )
    assert exitcode == 0
