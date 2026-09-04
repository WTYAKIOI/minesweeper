% SPDX-FileCopyrightText: 2022 James R. Barlow
% SPDX-License-Identifier: CC-BY-SA-4.0

# v14

## v14.4.0

- Digitally signed PDFs are now detected. If the PDF is signed, OCRmyPDF will
  refuse to modify it. Previously, only encrypted PDFs were detected, not
  those that were signed but not encrypted. {issue}`1040`
- In addition, `--invalidate-digital-signatures` can be used to override the
  above behavior and modify the PDF anyway. {issue}`1040`
- tqdm progress bars replaced with "rich" progress bars. The rich library is
  a new dependency. Certain APIs that used tqdm are now deprecated and will
  be removed in the next major release.
- Improved integration with GitHub Releases. Thanks to @stumpylog.

## v14.3.0

- Renamed master branch to main.
- Improve PDF rasterization accuracy by using the `-dPDFSTOPONERROR` option
  to Ghostscript. Use `--continue-on-soft-render-error` if you want to render
  the PDF anyway. The plugin specification was adjusted to support this feature;
  plugin authors may want to adapt PDF rasterizing and rendering
  plugins. {issue}`1083`
- The calculated deskew angle is now recorded in the logged output. {issue}`1101`
- Metadata can now be unset by setting a metadata type such as `--title` to an
  empty string. {issue}`1117,1059`
- Fixed random order of languages due to use of a set. This may have caused output
  to vary when multiple languages were set for OCR. {issue}`1113`
- Clarified the optimization ratio reported in the log output.
- Documentation improvements.

## v14.2.1

- Fixed {issue}`977`, where images inside Form XObjects were always excluded
  from image optimization.

## v14.2.0

- Added `--tesseract-downsample-above` to downsample larger images even when
  they do not exceed Tesseract's internal limits. This can be used to speed
  up OCR, possibly sacrificing accuracy.
- Fixed resampling AttributeError on older Pillow. {issue}`1096`
- Removed an error about using Ghostscript on PDFs with that have the /UserUnit
  feature in use. Previously, Ghostscript would fail to process these PDFs,
  but in all supported versions it is now supported, so the error is no longer
  needed.
- Improved documentation around installing other language packs for Tesseract.

## v14.1.0

- Added `--tesseract-non-ocr-timeout`. This allows using Tesseract's deskew
  and other non-OCR features while disabling OCR using `--tesseract-timeout 0`.
- Added `--tesseract-downsample-large-images`. This downsamples larges images
  that exceed the maximum image size Tesseract can handle. Large images may still
  take a long time to process, but this allows them to be processed if that
  is desired.
- Fixed {issue}`1082`, an issue with snap packaged building.
- Change linter to ruff, fix lint errors, update documentation.

## v14.0.4

- Fixed {issue}`1066, 1075`, an exception when processing certain malformed PDFs.

## v14.0.3

- Fixed {issue}`1068`, avoid deleting /dev/null when running as root.
- Other documentation fixes.

## v14.0.2

- Fixed {issue}`1052`, an exception on attempting to process certain nonconforming PDFs.
- Explicitly documented that Windows 32-bit is no longer supported.
- Fixed source installation instructions.
- Other documentation fixes.

## v14.0.1

- Fixed some version checks done with smart version comparison.
- Added missing jbig2dec to Docker image.

## v14.0.0

- Dropped support for Python 3.7.
- Dropped support generally speaking, all dependencies older than what Ubuntu 20.04
  provides.
- Ghostscript 9.50 or newer is now required. Shims to support old versions were
  removed.
- Tesseract 4.1.1 or newer is now required. Shims to support old versions were
  removed.
- Docker image now uses Tesseract 5.
- Dropped setup.cfg configuration for pyproject.toml.
- Removed deprecation exception PdfMergeFailedError.
- A few more public domain test files were removed or replaced. We are aiming for
  100% compliance with SPDX and generally towards simplifying copyright.

