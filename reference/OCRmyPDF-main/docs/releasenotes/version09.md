% SPDX-FileCopyrightText: 2022 James R. Barlow
% SPDX-License-Identifier: CC-BY-SA-4.0

# v9

## v9.8.2

- Fixed an issue where OCRmyPDF would ignore text inside Form XObject when
  making certain decisions about whether a document already had text.
- Fixed file size increase warning to take overhead of small files into account.
- Added instructions for installing on Cygwin.

## v9.8.1

- Fixed an issue where unexpected files in the `%PROGRAMFILES%\gs` directory
  (Windows) caused an exception.
- Mark pdfminer.six 20200517 as supported.
- If jbig2enc is missing and optimization is requested, a warning is issued
  instead of an error, which was the intended behavior.
- Documentation updates.

## v9.8.0

- Fixed issue where only the first PNG (FlateDecode) image in a file would be
  considered for optimization. File sizes should be improved from here on.
- Fixed a startup crash when the chosen language was Japanese ({issue}`543`).
- Added options to configure polling and log level to watcher.py.

## v9.7.2

- Fixed an issue with `ocrmypdf.ocr(...language=)` not accepting a list of
  languages as documented.
- Updated setup.py to confirm that pdfminer.six version 20200402 is supported.

## v9.7.1

- Fixed version check failing when used with qpdf 10.0.0.
- Added some missing type annotations.
- Updated documentation to warn about need for "ifmain" guard and Windows.

## v9.7.0

- Fixed an error in watcher.py if `OCR_JSON_SETTINGS` was not defined.
- Ghostscript 9.51 is now blacklisted, due to numerous problems with this version.
- Added a workaround for a problem with "txtwrite" in Ghostscript 9.52.
- Fixed an issue where the incorrect number of threads used was shown when
  `OMP_THREAD_LIMIT` was manipulated.
- Removed a possible performance bottlenecks for files that use hundreds to
  thousands of images on the same page.
- Documentation improvements.
- Optimization will now be applied to some monochrome images that have a color
  profile defined instead of only black and white.
- ICC profiles are consulted when determining the simplified colorspace of an
  image.

## v9.6.1

- Documentation improvements - thanks to many users for their contributions!

  > - Fixed installation instructions for ArchLinux (@pigmonkey)
  > - Updated installation instructions for FreeBSD and other OSes (@knobix)
  > - Added instructions for using Docker Compose with watchdog (@ianalexander,
  >   @deisi)
  > - Other miscellany (@mb720, @toy, @caiofacchinato)
  > - Some scripts provided in the documentation have been migrated out so that
  >   they can be copied out as whole files, and to ensure syntax checking
  >   is maintained.

- Fixed an error that caused bash completions to fail on macOS. ({issue}`502,504`;
  @AlexanderWillner)

- Fixed a rare case where OCRmyPDF threw an exception while processing a PDF
  with the wrong object type in its `/Trailer /Info`. The error is now logged
  and incorrect object is ignored. ({issue}`497`)

- Removed potentially non-free file `enron1.pdf` and simplified the test that
  used it.

- Removed potentially non-free file `misc/media/logo.afdesign`.

## v9.6.0

- Fixed a regression with transferring metadata from the input PDF to the output
  PDF in certain situations.
- pdfminer.six is now supported up to version 2020-01-24.
- Messages are explaining page rotation decisions are now shown at the standard
  verbosity level again when `--rotate-pages`. In some previous version they
  were set to debug level messages that only appeared with the parameter `-v1`.
- Improvements to `misc/watcher.py`. Thanks to @ianalexander and @svenihoney.
- Documentation improvements.

## v9.5.0

- Added API functions to measure OCR quality.
- Modest improvements to handling PDFs with difficult/non compliant metadata.

## v9.4.0

- Updated recommended dependency versions.
- Improvements to test coverage and changes to facilitate better measurement of
  test coverage, such as when tests run in subprocesses.
- Improvements to error messages when Leptonica is not installed correctly.
- Fixed use of pytest "session scope" that may have caused some intermittent
  CI failures.
- When the argument `--keep-temporary-files` or verbosity is set to `-v1`,
  a debug log file is generated in the working temporary folder.

## v9.3.0

- Improved native Windows support: we now check in the obvious places in
  the "Program Files" folders installations of Tesseract and Ghostscript,
  rather than relying on the user to edit `PATH` to specify their location.
  The `PATH` environment variable can still be used to differentiate when
  multiple installations are present or the programs are installed to non-
  standard locations.
- Fixed an exception on parsing Ghostscript error messages.
- Added an improved example demonstrating how to set up a watched folder
  for automated OCR processing (thanks to @ianalexander for the contribution).

## v9.2.0

- Native Windows is now supported.
- Continuous integration moved to Azure Pipelines.
- Improved test coverage and speed of tests.
- Fixed an issue where a page that was originally a JPEG would be saved as a
  PNG, increasing file size. This occurred only when a preprocessing option
  was selected along with `--output-type=pdf` and all images on the original
  page were JPEGs. Regression since v7.0.0.
- OCRmyPDF no longer depends on the QPDF executable `qpdf` or `libqpdf`.
  It uses pikepdf (which in turn depends on `libqpdf`). Package maintainers
  should adjust dependencies so that OCRmyPDF no longer calls for libqpdf on
  its own. For users of Python binary wheels, this change means a separate
  installation of QPDF is no longer necessary. This change is mainly to
  simplify installation on Windows.
- Fixed a rare case where log messages from Tesseract would be discarded.
- Fixed incorrect function signature for pixFindPageForeground, causing
  exceptions on certain platforms/Leptonica versions.

## v9.1.1

- Expand the range of pdfminer.six versions that are supported.
- Fixed Docker build when using pikepdf 1.7.0.
- Fixed documentation to recommend using pip from get-pip.py.

## v9.1.0

- Improved diagnostics when file size increases at output. Now warns if JBIG2
  or pngquant were not available.
- pikepdf 1.7.0 is now required, to pick up changes that remove the need for
  a source install on Linux systems running Python 3.8.

## v9.0.5

- The Alpine Docker image (jbarlow83/ocrmypdf-alpine) has been dropped due to
  the difficulties of supporting Alpine Linux.
- The primary Docker image (jbarlow83/ocrmypdf) has been improved to take on
  the extra features that used to be exclusive to the Alpine image.
- No changes to application code.
- pdfminer.six version 20191020 is now supported.

## v9.0.4

- Fixed compatibility with Python 3.8 (but requires source install for the moment).
- Fixed Tesseract settings for `--user-words` and `--user-patterns`.
- Changed to pikepdf 1.6.5 (for Python 3.8).
- Changed to Pillow 6.2.0 (to mitigate a security vulnerability in earlier Pillow).
- A debug message now mentions when English is automatically selected if the locale
  is not English.

## v9.0.3

- Embed an encoded version of the sRGB ICC profile in the intermediate
  Postscript file (used for PDF/A conversion). Previously we included the
  filename, which required Postscript to run with file access enabled. For
  security, Ghostscript 9.28 enables `-dSAFER` and as such, no longer
  permits access to any file by default. This fix is necessary for
  compatibility with Ghostscript 9.28.
- Exclude a test that sometimes times out and fails in continuous integration
  from the standard test suite.

## v9.0.2

- The image optimizer now skips optimizing flate (PNG) encoded images in some
  situations where the optimization effort was likely wasted.
- The image optimizer now ignores images that specify arbitrary decode arrays,
  since these are rare.
- Fixed an issue that caused inversion of black and white in monochrome images.
  We are not certain but the problem seems to be linked to Leptonica 1.76.0 and
  older.
- Fixed some cases where the test suite failed if
  English or German Tesseract language packs were not installed.
- Fixed a runtime error if the Tesseract English language is not installed.
- Improved explicit closing of Pillow images after use.
- Actually fixed of Alpine Docker image build.
- Changed to pikepdf 1.6.3.

## v9.0.1

- Fixed test suite failing when either of optional dependencies unpaper and
  pngquant were missing.
- Attempted fix of Alpine Docker image build.
- Documented that FreeBSD ports are now available.
- Changed to pikepdf 1.6.1.

## v9.0.0

**Breaking changes**

- The `--mask-barcodes` experimental feature has been dropped due to poor
  reliability and occasional crashes, both due to the underlying library that
  implements this feature (Leptonica).
- The `-v` (verbosity level) parameter now accepts only `0`, `1`, and
  `2`.
- Dropped support for Tesseract 4.00.00-alpha releases. Tesseract 4.0 beta and
  later remain supported.
- Dropped the `ocrmypdf-polyglot` and `ocrmypdf-webservice` images.

**New features**

- Added a high level API for applications that want to integrate OCRmyPDF.
  Special thanks to Martin Wind (@mawi1988) whose made significant contributions
  to this effort.
- Added progress bars for long-running steps. ■■■■■■■□□
- We now create linearized ("fast web view") PDFs by default. The new parameter
  `--fast-web-view` provides control over when this feature is applied.
- Added a new `--pages` feature to limit OCR to only a specific page range.
  The list may contain commas or single pages, such as `1, 3, 5-11`.
- When the number of pages is small compared to the number of allowed jobs, we
  run Tesseract in multithreaded (OpenMP) mode when available. This should
  improve performance on files with low page counts.
- Removed dependency on `ruffus`, and with that, the non-reentrancy
  restrictions that previous made an API impossible.
- Output and logging messages overhauled so that ocrmypdf may be integrated
  into applications that use the logging module.
- pikepdf 1.6.0 is required.
- Added a logo. 😊

**Bug fixes**

- Pages with vector artwork are treated as full color. Previously, vectors
  were ignored when considering the colorspace needed to cover a page, which
  could cause loss of color under certain settings.
- Test suite now spawns processes less frequently, allowing more accurate
  measurement of code coverage.
- Improved test coverage.
- Fixed a rare division by zero (if optimization produced an invalid file).
- Updated Docker images to use newer versions.
- Fixed images encoded as JBIG2 with a colorspace other than `/DeviceGray`
  were not interpreted correctly.
- Fixed a OCR text-image registration (i.e. alignment) problem when the page
  when MediaBox had a nonzero corner.

