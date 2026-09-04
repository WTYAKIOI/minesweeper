% SPDX-FileCopyrightText: 2022 James R. Barlow
% SPDX-License-Identifier: CC-BY-SA-4.0

# v15

## v15.4.4

- Fixed documentation for installing Ghostscript on Windows. {issue}`1198`
- Added warning message about security issue in older versions of Ghostscript.

## v15.4.3

- Fixed deprecation warning in pikepdf older than 8.7.1; pikepdf >= 8.7.1 is
  now required.

## v15.4.2

- We now raise an exception on a certain class of PDFs that likely need an
  explicit color conversion strategy selected to display correctly
  for PDF/A conversion.
- Fixed an error that occurred while trying to write a log message after the
  debug log handler was removed.

## v15.4.1

- Fixed misc/watcher.py regressions: accept `--ocr-json-settings` as either
  filename or JSON string, as previously; and argument count mismatch.
  {issue}`1183,1185`
- We no longer attempt to set /ProcSet in the PDF output, since this is an
  obsolete PDF feature.
- Documentation improvements.

## v15.4.0

- Added new experimental APIs to support offline editing of the final text.
  Specifically, one can now generate hOCR files with OCRmyPDF, edit them with
  some other tool, and then finalize the PDF. They are experimental and
  subject to change, including details of how the working folder is used.
  There is no command line interface.
- Code reorganization: executors, progress bars, initialization and setup.
- Fixed test coverage in cases where the coverage tool did not properly trace
  into threads or subprocesses. This code was still being tested but appeared
  as not covered.
- In the test suite, reduced use of subprocesses and other techniques that
  interfere with coverage measurement.
- Improved error check for when we appear to be running inside a snap container
  and files are not available.
- Plugin specification now properly defines progress bars as a protocol rather
  than defining them as "tqdm-like".
- We now default to using "forkserver" process creation on POSIX platforms
  rather than fork, since this is method is more robust and avoids some
  issues when threads are present.
- Fixed an instance where the user's request to `--no-use-threads` was ignored.
- If a PDF does not have language metadata on its top level object, we add
  the OCR language.
- Replace some cryptic test error messages with more helpful ones.
- Debug messages for how OCRmyPDF picks the colorspace for a page are now
  more descriptive.

## v15.3.1

- Fixed an issue with logging settings for misc/watcher.py introduced in the
  previous release. {issue}`1180`
- We now attempt to preserve the input's extended attributes when creating
  the output file.
- For some reason, the macOS build now needs OpenSSL explicitly installed.
- Updated documentation on Docker performance concerns.

## v15.3.0

- Update misc/watcher.py to improve command line interface using Typer, and
  support `.env` specification of environment variables. Improved error
  messages. Thanks to @mflagg2814 for the PR that prompted this improvement.
- Improved error message when a file cannot be read because we are running in
  a snap container.

## v15.2.0

- Added a Docker image based on Alpine Linux. This image is smaller than the
  Ubuntu-based image and may be useful in some situations. Currently hosted at
  jbarlow83/ocrmypdf-alpine. Currently not available in ARM flavor.
- The Ubuntu Docker is now aliased to jbarlow83/ocrmypdf-ubuntu.
- Updated Docker documentation.

## v15.1.0

- We now require Pillow 10.0.1, due a serious security vulnerability in all earlier
  versions of that dependency. The vulnerability concerns WebP images and could
  be triggered in OCRmyPDF when creating a PDF from a malicious WebP image.
- Added some keyword arguments to `ocrmypdf.ocr` that were previously accepted
  but undocumented.
- Documentation updates and typing improvements.

## v15.0.2

- Added Python 3.12 to test matrix.
- Updated documentation for notes on Python 3.12, 32-bit support and some new
  features in v15.

## v15.0.1

- Wheels Python tag changed to py39.
- Marked as a expected fail a test that fails on recent Ghostscript versions.
- Clarified documentation and release notes around the extent of 32-bit support.
- Updated installation documentation to changes in v15.

## v15.0.0

- Dropped support for Python 3.8.
- Dropped support some older dependencies, specifically `coloredlogs` and
  `tqdm` in favor of rich - see `pyproject.toml` for details.
  Generally speaking, Ubuntu 22.04 is our new baseline system.
- Tightened version requirements for some dependencies.
- Dropped support for 32-bit Linux wheels. We strongly recommend a 64-bit operating
  system, and 64-bit versions of Python, Tesseract and Ghostscript to use OCRmyPDF.
  Many of our dependencies are dropping 32-bit builds (e.g. Pillow), and we are
  following suit. (Maintainers may still build 32-bit versions from source.)
- Changed to trusted release for PyPI publishing.
- pikepdf memory mapping is enabled again for improved performance, now that an
  issue with feature in pikepdf is fixed.
- `ocrmypdf.helpers.calculate_downsample` previously had two variants, one
  that took a `PIL.Image` and one that took a `tuple[int, int]`. The latter
  was removed.
- The snap version of ocrmypdf is now based on Ubuntu core22.
- We now account for situations where a small portion of an image on a page is drawn
  at high DPI (resolution). Previously, the entire page would be rasterized at the
  highest resolution of any feature, which caused performance problems. Now,
  the page is rasterized
  at a resolution based on the average DPI of the page, weighted by the area that
  each feature occupies. Typically, small areas of high resolution in PDFs are
  errors or quirks from the repeated use of assets and high resolution is not
  beneficial. {issue}`1010,1104,1004,1079,1010`
- Ghostscript color conversion strategy is now configurable using
  `--color-conversion-strategy`. {issue}`1143`
- JBIG2 threshold for optimization is now configurable using
  `--jbig2-threshold`. {issue}`1133`

