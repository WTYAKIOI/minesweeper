# SPDX-FileCopyrightText: 2026 James R. Barlow
# SPDX-License-Identifier: MIT
"""Test plugin that records the size of every image handed to the OCR engine.

Writes one ``width height mode`` line per page to the file named by the
``OCRMYPDF_TEST_OCR_IMAGE_LOG`` environment variable, so a test can assert on the
image the OCR engine actually receives. The hook runs in worker processes or
threads, hence the file rather than an in-memory record.
"""

from __future__ import annotations

import os
from pathlib import Path

from ocrmypdf import hookimpl


@hookimpl
def filter_ocr_image(page, image):
    """Record the image size, then decline so the real filters still run."""
    path = os.environ.get('OCRMYPDF_TEST_OCR_IMAGE_LOG')
    if path:
        with Path(path).open('a', encoding='utf-8') as f:
            f.write(f"{image.width} {image.height} {image.mode}\n")
    return None  # firstresult hook: None lets the next implementation filter
