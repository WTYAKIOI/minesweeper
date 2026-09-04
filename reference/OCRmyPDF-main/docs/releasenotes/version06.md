% SPDX-FileCopyrightText: 2022 James R. Barlow
% SPDX-License-Identifier: CC-BY-SA-4.0

# v6

## v6.2.5

- Disable a failing test due to Tesseract 4.0rc1 behavior change.
  Previously, Tesseract would exit with an error message if its
  configuration was invalid, and OCRmyPDF would intercept this message.
  Now Tesseract issues a warning, which OCRmyPDF v6.2.5 may relay or
  ignore. (In v7.x, OCRmyPDF will respond to the warning.)
- This release branch no longer supports using the optional PyMuPDF
  installation, since it was removed in v7.x.
- This release branch no longer supports macOS. macOS users should
  upgrade to v7.x.

## v6.2.4

- Backport Ghostscript 9.25 compatibility fixes, which removes support
  for setting Unicode metadata
- Backport blacklisting Ghostscript 9.24
- Older versions of Ghostscript are still supported

## v6.2.3

- Fixed compatibility with img2pdf >= 0.3.0 by rejecting input images
  that have an alpha channel
- This version will be included in Ubuntu 18.10

## v6.2.2

- Backport compatibility fixes for Python 3.7 and ruffus 2.7.0 from
  v7.0.0
- Backport fix to ignore masks when deciding what colors are on a page
- Backport some minor improvements from v7.0.0: better argument
  validation and warnings about the Tesseract 4.0.0 `--user-words`
  regression

## v6.2.1

- Fixed recent versions of Tesseract (after 4.0.0-beta1) not being
  detected as supporting the `sandwich` renderer ({issue}`271`).

## v6.2.0

- **Docker**: The Docker image `ocrmypdf-tess4` has been removed. The
  main Docker images, `ocrmypdf` and `ocrmypdf-polyglot` now use
  Ubuntu 18.04 as a base image, and as such Tesseract 4.0.0-beta1 is
  now the Tesseract version they use. There is no Docker image based on
  Tesseract 3.05 anymore.
- Creation of PDF/A-3 is now supported. However, there is no ability to
  attach files to PDF/A-3.
- Lists more reasons why the file size might grow.
- Fixed {issue}`262`,
  `--remove-background` error on PDFs contained colormapped
  (paletted) images.
- Fixed another XMP metadata validation issue, in cases where the input
  file's creation date has no timezone and the creation date is not
  overridden.

## v6.1.5

- Fixed {issue}`253`, a
  possible division by zero when using the `hocr` renderer.
- Fixed incorrectly formatted `<xmp:ModifyDate>` field inside XMP
  metadata for PDF/As. veraPDF flags this as a PDF/A validation
  failure. The error is caused the timezone and final digit of the
  seconds of modified time to be omitted, so at worst the modification
  time stamp is rounded to the nearest 10 seconds.

## v6.1.4

- Fixed {issue}`248`
  `--clean` argument may remove OCR from left column of text on
  certain documents. We now set `--layout none` to suppress this.
- The test cache was updated to reflect the change above.
- Change test suite to accommodate Ghostscript 9.23's new ability to
  insert JPEGs into PDFs without transcoding.
- XMP metadata in PDFs is now examined using `defusedxml` for safety.
- If an external process exits with a signal when asked to report its
  version, we now print the system error message instead of suppressing
  it. This occurred when the required executable was found but was
  missing a shared library.
- qpdf 7.0.0 or newer is now required as the test suite can no longer
  pass without it.

### Notes

- An apparent [regression in Ghostscript
  9.23](https://bugs.ghostscript.com/show_bug.cgi?id=699216) will
  cause some ocrmypdf output files to become invalid in rare cases; the
  workaround for the moment is to set `--force-ocr`.

## v6.1.3

- Fixed {issue}`247`,
  `/CreationDate` metadata not copied from input to output.
- A warning is now issued when Python 3.5 is used on files with a large
  page count, as this case is known to regress to single core
  performance. The cause of this problem is unknown.

## v6.1.2

- Upgrade to PyMuPDF v1.12.5 which includes a more complete fix to
  {issue}`239`.
- Add `defusedxml` dependency.

## v6.1.1

- Fixed text being reported as found on all pages if PyMuPDF is not
  installed.

## v6.1.0

- PyMuPDF is now an optional but recommended dependency, to alleviate
  installation difficulties on platforms that have less access to
  PyMuPDF than the author anticipated. (For version 6.x only) install
  OCRmyPDF with `pip install ocrmypdf[fitz]` to use it to its full
  potential.
- Fixed `FileExistsError` that could occur if OCR timed out while it
  was generating the output file.
  ({issue}`218`)
- Fixed table of contents/bookmarks all being redirected to page 1 when
  generating a PDF/A (with PyMuPDF). (Without PyMuPDF the table of
  contents is removed in PDF/A mode.)
- Fixed "RuntimeError: invalid key in dict" when table of
  contents/bookmarks titles contained the character `)`.
  ({issue}`239`)
- Added a new argument `--skip-repair` to skip the initial PDF repair
  step if the PDF is already well-formed (because another program
  repaired it).

## v6.0.0

- The software license has been changed to GPLv3 [it has since changed again].
  Test resource files and some individual sources may have other licenses.

- OCRmyPDF now depends on
  [PyMuPDF](https://pymupdf.readthedocs.io/en/latest/installation/).
  Including PyMuPDF is the primary reason for the change to GPLv3.

- Other backward incompatible changes

  - The `OCRMYPDF_TESSERACT`, `OCRMYPDF_QPDF`, `OCRMYPDF_GS` and
    `OCRMYPDF_UNPAPER` environment variables are no longer used.
    Change `PATH` if you need to override the external programs
    OCRmyPDF uses.
  - The `ocrmypdf` package has been moved to `src/ocrmypdf` to
    avoid issues with accidental import.
  - The function `ocrmypdf.exec.get_program` was removed.
  - The deprecated module `ocrmypdf.pageinfo` was removed.
  - The `--pdf-renderer tess4` alias for `sandwich` was removed.

- Fixed an issue where OCRmyPDF failed to detect existing text on
  pages, depending on how the text and fonts were encoded within the
  PDF. ({issue}`233,232`)

- Fixed an issue that caused dramatic inflation of file sizes when
  `--skip-text --output-type pdf` was used. OCRmyPDF now removes
  duplicate resources such as fonts, images and other objects that it
  generates. ({issue}`237`)

- Improved performance of the initial page splitting step. Originally
  this step was not believed to be expensive and ran in a process.
  Large file testing revealed it to be a bottleneck, so it is now
  parallelized. On a 700 page file with quad core machine, this change
  saves about 2 minutes. ({issue}`234`)

- The test suite now includes a cache that can be used to speed up test
  runs across platforms. This also does not require computing
  checksums, so it's faster. ({issue}`217`)

