.. SPDX-FileCopyrightText: 2026 James R. Barlow
.. SPDX-License-Identifier: CC-BY-SA-4.0

Tesseract OCR cache
===================

Recorded Tesseract output, replayed by ``tests/plugins/tesseract_cache.py``
so the test suite does not have to run OCR. Do not edit these files by hand.

Layout::

    v<CACHE_VERSION>/
        CACHE_INFO.json     Tesseract version and platform that generated the tree
        manifest.jsonl      One line per entry, appended when the entry is created
        <stem>-<sha256[:8]>/<argv-slug>/{stdout,stderr,hocr,pdf,txt}.bin
        <stem>/<argv-slug>/{stdout,stderr,hocr,pdf,txt}.bin

Inputs from ``tests/resources/`` are keyed by stem plus the first 8 hex
digits of the SHA-256 of their contents, so editing a resource invalidates
only the entries derived from it. Inputs generated during a test run are
keyed by stem alone, since their bytes differ from run to run.

Invalidation:

* Resource file changed: nothing to do; the digest key handles it. Delete
  any folder that ``tests/test_tesseract_cache.py`` reports as orphaned.
* OCRmyPDF changed what it feeds Tesseract (preprocessing, rasterization):
  bump ``CACHE_VERSION`` in the plugin, delete the old ``v<N>/`` tree, and
  regenerate.
* Tesseract version drift: ``pytest_sessionstart`` warns when the installed
  Tesseract's major.minor differs from ``CACHE_INFO.json``. Set
  ``OCRMYPDF_TEST_CACHE_STRICT=1`` to make it an error instead.

To regenerate: delete the versioned tree, run the full suite with the
reference Tesseract installed, confirm ``tests/test_tesseract_cache.py``
passes, and commit with the Tesseract version in the commit message.

See the "Tesseract OCR cache" section of ``docs/contributing.md`` for
details.
