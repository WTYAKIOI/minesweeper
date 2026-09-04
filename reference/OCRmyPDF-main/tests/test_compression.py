# SPDX-FileCopyrightText: 2022 James R. Barlow
# SPDX-License-Identifier: MPL-2.0

"""Tests that image compression and colorspace survive the pipeline."""

from __future__ import annotations

import pytest
from PIL import Image

from ocrmypdf.exceptions import ExitCode
from ocrmypdf.pdfinfo import Colorspace, Encoding, PdfInfo

from .conftest import check_ocrmypdf, run_ocrmypdf_api


# These run in-process against the image file directly; piping an image over
# stdin is exercised separately by test_stdio.py.
@pytest.mark.parametrize(
    'image', ['baiona.png', 'baiona_gray.png', 'baiona_alpha.png', 'baiona_color.jpg']
)
def test_compression_preserved(resources, image, outpdf, caplog):
    input_file = resources / image
    output_file = str(outpdf)

    im = Image.open(input_file)

    if im.mode in ('RGBA', 'LA'):
        # If alpha image is input, expect an error
        exitcode = run_ocrmypdf_api(
            input_file,
            outpdf,
            '--optimize',
            '0',
            '--image-dpi',
            '150',
            '--output-type',
            'pdf',
            '--plugin',
            'tests/plugins/tesseract_noop.py',
        )
        assert exitcode != ExitCode.ok
        assert 'alpha' in caplog.text
        im.close()
        return

    check_ocrmypdf(
        input_file,
        outpdf,
        '--optimize',
        '0',
        '--image-dpi',
        '150',
        '--output-type',
        'pdf',
        '--plugin',
        'tests/plugins/tesseract_noop.py',
    )

    pdfinfo = PdfInfo(output_file)

    pdfimage = pdfinfo[0].images[0]

    if image.endswith('.png'):
        assert pdfimage.enc != Encoding.jpeg, "Lossless compression changed to lossy!"
    elif image.endswith('.jpg'):
        assert pdfimage.enc == Encoding.jpeg, "Lossy compression changed to lossless!"
    if im.mode.startswith('RGB') or im.mode.startswith('BGR'):
        assert pdfimage.color == Colorspace.rgb, "Colorspace changed"
    elif im.mode.startswith('L'):
        assert pdfimage.color == Colorspace.gray, "Colorspace changed"
    im.close()


@pytest.mark.parametrize(
    'image,compression',
    [
        ('baiona.png', 'jpeg'),
        ('baiona_gray.png', 'lossless'),
        ('baiona_color.jpg', 'lossless'),
    ],
)
def test_compression_changed(resources, image, compression, outpdf):
    input_file = resources / image
    output_file = str(outpdf)

    im = Image.open(input_file)

    check_ocrmypdf(
        input_file,
        outpdf,
        '--image-dpi',
        '150',
        '--output-type',
        'pdfa',
        '--optimize',
        '0',
        '--pdfa-image-compression',
        compression,
        '--plugin',
        'tests/plugins/tesseract_noop.py',
    )

    pdfinfo = PdfInfo(output_file)

    pdfimage = pdfinfo[0].images[0]

    if compression == "jpeg":
        assert pdfimage.enc == Encoding.jpeg
    else:
        if image.endswith('jpg'):
            # Ghostscript JPEG passthrough - no issue
            assert pdfimage.enc == Encoding.jpeg
        else:
            assert pdfimage.enc not in (Encoding.jpeg, Encoding.jpeg2000)

    if im.mode.startswith('RGB') or im.mode.startswith('BGR'):
        assert pdfimage.color == Colorspace.rgb, "Colorspace changed"
    elif im.mode.startswith('L'):
        assert pdfimage.color == Colorspace.gray, "Colorspace changed"
    im.close()


def test_jbig2_passthrough(resources, outpdf):
    out = check_ocrmypdf(
        resources / 'jbig2.pdf',
        outpdf,
        '--output-type',
        'pdf',
        '--pdf-renderer',
        'fpdf2',
        '--plugin',
        'tests/plugins/tesseract_cache.py',
    )
    out_pageinfo = PdfInfo(out)
    assert out_pageinfo[0].images[0].enc == Encoding.jbig2
