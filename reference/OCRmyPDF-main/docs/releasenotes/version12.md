% SPDX-FileCopyrightText: 2022 James R. Barlow
% SPDX-License-Identifier: CC-BY-SA-4.0

# v12

## v12.7.2

- Fixed "invalid version number" error for Tesseract packaging with nonstandard
  version "5.0.0-rc1.20211030".
- Fixed use of deprecated `importlib.resources.read_binary`.
- Replace some uses of string paths with `pathlib.Path`.
- Fixed a leaked file handle when using `--output-type none`.
- Removed shims to support versions of pikepdf that are no longer supported.

## v12.7.1

- Declare support for pdfminer.six v20211012.

## v12.7.0

- Fixed test suite failure when using pikepdf 3.2.0 that was compiled with pybind11
  2.8.0. {issue}`843`
- Improve advice to user about using `--max-image-mpixels` if OCR fails for this
  reason.
- Minor documentation fixes. (Thanks to @mara004.)
- Don't require importlib-metadata and importlib-resources backports on versions of
  Python where the standard library implementation is sufficient.
  (Thanks to Marco Genasci.)

## v12.6.0

- Implemented `--output-type=none` to skip producing PDFs for applications that
  only want sidecar files ({issue}`787`).
- Fixed ambiguities in descriptions of behavior of `--jbig2-lossy`.
- Various improvements to documentation.

## v12.5.0

- Fixed build failure for the combination of PyPy 3.6 and pikepdf 3.0. This
  combination can work in a source build but does not work with wheels.
- Accepted bot that wanted to upgrade our deprecated requirements.txt.
- Documentation updates.
- Replace pkg_resources and install dependency on setuptools with
  importlib-metadata and importlib-resources.
- Fixed regression in hocrtransform causing text to be omitted when this
  renderer was used.
- Fixed some typing errors.

## v12.4.0

- When grafting text layers, use pikepdf's `unparse_content_stream` if available.
- Confirmed support for pluggy 1.0. (Thanks @QuLogic.)
- Fixed some typing issues, improved pre-commit settings, and fixed issues
  flagged by linters.
- PyPy 7.3.3 (=Python 3.6) is now supported. Note that PyPy does not necessarily
  run faster, because the vast majority of OCRmyPDF's execution time is spent
  running OCR or generally executing native code. However, PyPy may bring speed
  improvements in some areas.

## v12.3.3

- watcher.py: fixed interpretation of boolean env vars ({issue}`821`).
- Adjust CI scripts to test Tesseract 5 betas.
- Document our support for the Tesseract 5 betas.

## v12.3.2

- Indicate support for flask 2.x, watcher 2.x ({issue}`815, 816`).

## v12.3.1

- Fixed issue with selection of text when using the hOCR renderer ({issue}`813`).
- Fixed build errors with the Docker image by upgrading to a newer Ubuntu.
  Also set the timezone of this image to UTC.

## v12.3.0

- Fixed a regression introduced in Pillow 8.3.0. Pillow no longer rounds DPI
  for image resolutions. We now account for this ({issue}`802`).
- We no longer use some API calls that are deprecated in the latest versions of
  pikepdf.
- Improved error message when a language is requested that doesn't look like a
  typical ISO 639-2 code.
- Fixed some tests that attempted to symlink on Windows, breaking tests on a
  Windows desktop but not usually on CI.
- Documentation fixes (thanks to @mara004)

## v12.2.0

- Fixed invalid Tesseract version number on Windows ({issue}`795`).
- Documentation tweaks. Documentation build now depends on sphinx-issues package.

## v12.1.0

- For security reasons we now require Pillow >= 8.2.x. (Older versions will continue
  to work if upgrading is not an option.)
- The build system was reorganized to rely on `setup.cfg` instead of `setup.py`.
  All changes should work with previously supported versions of setuptools.
- The files in `requirements/*` are now considered deprecated but will be retained for v12.
  Instead use `pip install ocrmypdf[test]` instead of `requirements/test.txt`, etc.
  These files will be removed in v13.

## v12.0.3

- Expand the list of languages supported by the hocr PDF renderer.
  Several languages were previously considered not supported, particularly those
  non-European languages that use the Latin alphabet.
- Fixed a case where the exception stack trace was suppressed in verbose mode.
- Improved documentation around commercial OCR.

## v12.0.2

- Fixed exception thrown when using `--remove-background` on files containing small
  images ({issue}`769`).
- Improve documentation for description of adding language packs to the Docker image
  and corrected name of French language pack.

## v12.0.1

- Fixed "invalid version number" for untagged tesseract versions ({issue}`770`).

## v12.0.0

**Breaking changes**

- Due to recent security issues in pikepdf, Pillow and reportlab, we now require
  newer versions of these libraries and some of their dependencies. (If necessary,
  package maintainers may override these versions at their discretion; lower
  versions will often work.)
- We now use the "LeaveColorUnchanged" color conversion strategy when directing
  Ghostscript to create a PDF/A. Generally this is faster than performing a
  color conversion, which is not always necessary.
- OCR text is now packaged in a Form XObject. This makes it easier to isolate
  OCR from other document content. However, some poorly implemented PDF text
  extraction algorithms may fail to detect the text.
- Many API functions have stricter parameter checking or expect keyword arguments
  were they previously did not.
- Some deprecated functions in `ocrmypdf.optimize` were removed.
- The `ocrmypdf.leptonica` module is now deprecated, due to difficulties with
  the current strategy of ABI binding on newer platforms like Apple Silicon.
  It will be removed and replaced, either by repackaging Leptonica as an
  independent library using or using a different image processing library.
- Continuous integration moved to GitHub Actions.
- We no longer depend on `pytest_helpers_namespace` for testing.

**New features**

- New plugin hook: `get_progressbar_class`, for progress reporting,
  allowing developers to replace the standard console progress bar with some
  other mechanism, such as updating a GUI progress bar.
- New plugin hook: `get_executor`, for replacing the concurrency model.
  This is primarily to support execution on AWS Lambda, which does not support
  standard Python `multiprocessing` due to its lack of shared memory.
- New plugin hook: `get_logging_console`, for replacing the standard
  way OCRmyPDF outputs its messages.
- New plugin hook: `filter_pdf_page`, for modifying individual PDF
  pages produced by OCRmyPDF.
- OCRmyPDF now runs on nonstandard execution environments that do not have
  interprocess semaphores, such as AWS Lambda and Android Termux. If the environment
  does not have semaphores, OCRmyPDF will automatically select an alternate
  process executor that does not use semaphores.
- Continuous integration moved to GitHub Actions.
- We now generate an ARM64-compatible Docker image alongside the x64 image.
  Thanks to @andkrause for doing most of the work in a pull request several months
  ago, which we were finally able to integrate now. Also thanks to @0x326 for
  review comments.

**Fixes**

- Fixed a possible deadlock on attempting to flush `sys.stderr` when older
  versions of Leptonica are in use.
- Some worker processes inherited resources from their parents such as log
  handlers that may have also lead to deadlocks. These resources are now released.
- Improvements to test coverage.
- Removed vestiges of support for Tesseract versions older than 4.0.0-beta1 (
  which ships with Ubuntu 18.04).
- OCRmyPDF can now parse all of Tesseract version numbers, since several
  schemes have been in use.
- Fixed an issue with parsing PDFs that contain images drawn at a scale of 0. ({issue}`761`)
- Removed a frequently repeated message about disabling mmap.

