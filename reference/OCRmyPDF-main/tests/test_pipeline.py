# SPDX-FileCopyrightText: 2022 James R. Barlow
# SPDX-License-Identifier: MPL-2.0

from __future__ import annotations

import logging
import warnings
from unittest.mock import Mock

import pikepdf
import pytest
from PIL import Image
from reportlab.lib.units import inch
from reportlab.lib.utils import ImageReader
from reportlab.pdfgen.canvas import Canvas

from ocrmypdf import _metadata, _pipeline, pdfinfo
from ocrmypdf._pipeline import _select_raster_device
from ocrmypdf.helpers import Resolution
from ocrmypdf.pdfinfo import Encoding
from ocrmypdf.pluginspec import GhostscriptRasterDevice

warnings.filterwarnings(
    "ignore", category=DeprecationWarning, module="reportlab.lib.rl_safe_eval"
)


@pytest.fixture(scope='session')
def rgb_image():
    im = Image.new('RGB', (8, 8))
    im.putpixel((4, 4), (255, 0, 0))
    im.putpixel((5, 5), (0, 255, 0))
    im.putpixel((6, 6), (0, 0, 255))
    return ImageReader(im)


DUMMY_OVERSAMPLE_RESOLUTION = Resolution(42.0, 42.0)
VECTOR_RESOLUTION = Resolution(_pipeline.VECTOR_PAGE_DPI, _pipeline.VECTOR_PAGE_DPI)


@pytest.mark.parametrize(
    'image, text, vector, result',
    [
        (False, False, False, VECTOR_RESOLUTION),
        (False, True, False, VECTOR_RESOLUTION),
        (True, False, False, DUMMY_OVERSAMPLE_RESOLUTION),
        (True, True, False, VECTOR_RESOLUTION),
        (False, False, True, VECTOR_RESOLUTION),
        (False, True, True, VECTOR_RESOLUTION),
        (True, False, True, VECTOR_RESOLUTION),
        (True, True, True, VECTOR_RESOLUTION),
    ],
)
def test_dpi_needed(image, text, vector, result, rgb_image, outdir):
    c = Canvas(str(outdir / 'dpi.pdf'), pagesize=(5 * inch, 5 * inch))
    if image:
        c.drawImage(rgb_image, 1 * inch, 1 * inch, width=1 * inch, height=1 * inch)
    if text:
        c.drawString(1 * inch, 4 * inch, "Actual text")
    if vector:
        c.ellipse(3 * inch, 3 * inch, 4 * inch, 4 * inch)
    c.showPage()
    c.save()

    pi = pdfinfo.PdfInfo(outdir / 'dpi.pdf')
    pageinfo = pi[0]
    ctx = Mock()
    ctx.options.oversample = DUMMY_OVERSAMPLE_RESOLUTION[0]
    ctx.pageinfo = pageinfo

    assert _pipeline.get_canvas_square_dpi(ctx) == result
    assert _pipeline.get_page_square_dpi(ctx) == result


@pytest.mark.parametrize(
    # Name for nicer -v output
    'name,input,output',
    (
        (
            'empty_input',
            # Input:
            (),
            # Output:
            (),
        ),
        (
            'no_values',
            # Input:
            ('', '', '', '', ''),
            # Output:
            (((1, 5), None),),
        ),
        (
            'no_empty_values',
            # Input:
            ('v', 'w', 'x', 'y', 'z'),
            # Output:
            (
                ((1, 1), 'v'),
                ((2, 2), 'w'),
                ((3, 3), 'x'),
                ((4, 4), 'y'),
                ((5, 5), 'z'),
            ),
        ),
        (
            'skip_head',
            # Input:
            ('', '', 'x', 'y', 'z'),
            # Output:
            (
                ((1, 2), None),
                ((3, 3), 'x'),
                ((4, 4), 'y'),
                ((5, 5), 'z'),
            ),
        ),
        (
            'skip_tail',
            # Input:
            ('x', 'y', 'z', '', ''),
            # Output:
            (
                ((1, 1), 'x'),
                ((2, 2), 'y'),
                ((3, 3), 'z'),
                ((4, 5), None),
            ),
        ),
        (
            'range_in_middle',
            # Input:
            ('x', '', '', '', 'y'),
            # Output:
            (
                ((1, 1), 'x'),
                ((2, 4), None),
                ((5, 5), 'y'),
            ),
        ),
        (
            'range_in_middle_2',
            # Input:
            ('x', '', '', 'y', '', '', '', 'z'),
            # Output:
            (
                ((1, 1), 'x'),
                ((2, 3), None),
                ((4, 4), 'y'),
                ((5, 7), None),
                ((8, 8), 'z'),
            ),
        ),
    ),
)
def test_enumerate_compress_ranges(name, input, output):
    assert output == tuple(_pipeline.enumerate_compress_ranges(input))


@pytest.mark.parametrize(
    'encodings, expected',
    [
        # Empty images list returns False
        ([], False),
        # Single JPEG returns True
        ([Encoding.jpeg], True),
        # Single flate_jpeg returns True
        ([Encoding.flate_jpeg], True),
        # Mix of jpeg and flate_jpeg returns True
        ([Encoding.jpeg, Encoding.flate_jpeg], True),
        # Non-JPEG encoding returns False
        ([Encoding.flate], False),
        # Mix with non-JPEG returns False
        ([Encoding.jpeg, Encoding.flate], False),
        ([Encoding.flate_jpeg, Encoding.flate], False),
    ],
)
def test_should_visible_page_image_use_jpg(encodings, expected):
    """Test that should_visible_page_image_use_jpg correctly handles flate_jpeg."""
    pageinfo = Mock()
    pageinfo.images = [Mock(enc=enc) for enc in encodings]
    assert _pipeline.should_visible_page_image_use_jpg(pageinfo) == expected


def _make_image_mask_pdf(path, content_fill: bytes):
    pdf = pikepdf.Pdf.new()
    pdf.add_blank_page(page_size=(72, 72))
    mask = pikepdf.Stream(pdf, bytes([0x7E] * 8))
    mask.Type = pikepdf.Name.XObject
    mask.Subtype = pikepdf.Name.Image
    mask.Width = 8
    mask.Height = 8
    mask.ImageMask = True
    mask.BitsPerComponent = 1
    name = pdf.pages[0].add_resource(mask, pikepdf.Name.XObject)
    pdf.pages[0].Contents = pikepdf.Stream(
        pdf, b"q 72 0 0 72 0 0 cm %s %s Do Q" % (content_fill, bytes(name))
    )
    pdf.save(path)
    return path


def test_select_device_gray_mask(tmp_path):
    p = _make_image_mask_pdf(tmp_path / 'g.pdf', b"0.263 0.263 0.263 rg")
    pageinfo = pdfinfo.PdfInfo(p)[0]
    assert _select_raster_device(pageinfo) == GhostscriptRasterDevice.PNGGRAY


def test_select_device_color_mask(tmp_path):
    p = _make_image_mask_pdf(tmp_path / 'c.pdf', b"0.8 0.2 0.2 rg")
    pageinfo = pdfinfo.PdfInfo(p)[0]
    assert _select_raster_device(pageinfo) == GhostscriptRasterDevice.PNG16M


def test_select_device_black_mask_stays_mono(tmp_path):
    p = _make_image_mask_pdf(tmp_path / 'b.pdf', b"0 g")
    pageinfo = pdfinfo.PdfInfo(p)[0]
    assert _select_raster_device(pageinfo) == GhostscriptRasterDevice.PNGMONOD


# should_linearize is duplicated in _pipeline and _metadata; test both copies so
# they cannot drift apart.
@pytest.mark.parametrize('module', [_pipeline, _metadata], ids=['pipeline', 'metadata'])
@pytest.mark.parametrize(
    'filesize, expected',
    [(999_999, False), (1_000_000, False), (1_000_001, True)],
)
def test_should_linearize_threshold(module, filesize, expected, tmp_path):
    working_file = tmp_path / 'working.pdf'
    working_file.write_bytes(b'\0' * filesize)
    context = Mock()
    context.options.fast_web_view = 1.0  # megabytes
    assert module.should_linearize(working_file, context) is expected


def test_should_linearize_disabled_by_infinite_threshold(tmp_path):
    working_file = tmp_path / 'working.pdf'
    working_file.write_bytes(b'\0' * 5_000_000)
    context = Mock()
    context.options.fast_web_view = float('inf')
    assert _pipeline.should_linearize(working_file, context) is False


def test_triage_pdf_warns_image_dpi_ignored(resources, tmp_path, caplog):
    """--image-dpi is meaningless for PDF input, so triage() says so."""
    caplog.set_level(logging.WARNING)
    output_file = tmp_path / 'triaged.pdf'
    options = Mock()
    options.image_dpi = 100

    result = _pipeline.triage(
        'trivial.pdf', resources / 'trivial.pdf', output_file, options
    )

    assert result == output_file
    assert output_file.exists()
    assert '--image-dpi is being ignored' in caplog.text


def test_triage_pdf_silent_without_image_dpi(resources, tmp_path, caplog):
    caplog.set_level(logging.WARNING)
    output_file = tmp_path / 'triaged.pdf'
    options = Mock()
    options.image_dpi = None

    _pipeline.triage('trivial.pdf', resources / 'trivial.pdf', output_file, options)

    assert '--image-dpi' not in caplog.text
