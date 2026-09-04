% SPDX-FileCopyrightText: 2022 James R. Barlow
% SPDX-License-Identifier: CC-BY-SA-4.0

# v16

## v16.13.0

- Added detection and repair for Ghostscript 10.6 JPEG corruption. When GS 10.6
  truncates JPEG data by 1-15 bytes, OCRmyPDF now restores the original image
  bytes from the input PDF. A warning is issued when GS 10.6+ is detected.
  {issue}`1603`
- We continue to force re-optimization of JPEGs, since this catches some issues with corruption for situations where Ghostscript modifies an image. It is likely there are still cases where we cannot mitigate all corruption issues. {issue}`1585`
- Fixed handling of PDF page boxes (ArtBox, BleedBox) which were not being
  processed correctly in some cases. {issue}`1181,1360`
- Documentation: clarified podman usage instructions.

## v16.12.0

- Disable Ghostscript's subset fonts feature, which was found to corrupt text in certain
  PDFs. Thanks @mnaegler for identifying this issue. {issue}`1592`
- Users of Ghostscript 10.6.0+ reported that Ghostscript seems to generate corrupted
  JPEGs. We force re-optimization of these JPEGs to mitigate the corruption until
  Ghostscript fixes the issue. {issue}`1585`
- OCRmyPDF now avoids applying flate compression to large JPEG images, unless maximum
  optimization is requested, since flate+DCT compression reduces performances in PDF
  viewers with large images.
- Updated Dockerfiles to use more recent base operating systems.
- Updated build and test matrix to include Python 3.14.
- Minor documentation improvements.
- pikepdf >= 10.0.0 is now required.

## v16.11.1

- Fixed issue with Tesseract changing an error message related to skew. {issue}`1576`
- Dropped macOS 13 from build-test matrix since it is no longer supported by Apple.

## v16.11.0

- Deprecated "semfree" plugin in favor of falling back to threads if the platform
  does not support semaphores. Fixes an issue with Python 3.14.
- Fixed references to PDF/A compliances levels to be consistent with ISO nomenclature.
  Thanks @5HT2. {issue}`1557`
- Fixed an issue around using plugin_manager as an argument. {issue}`1555`
- Added OpenBSD install steps to README. {issue}`1554`
- Removed PyPy from test matrix due to declining support in third party libraries.
- Documentation improvements.

## v16.10.4

- Corrected build errors in Python 3.13.3 and 3.13.4.

## v16.10.3 (not released)

- Blocked optimization of images with pre-blended soft masks. {issue}`1536`
- Fixed warning from hypothesis on running tests.
- Release incomplete due to new test failures in Python 3.13.3 and 3.13.4.

## v16.10.2

- Blacklist pikepdf 9.8.0 due to an incompatible change.

## v16.10.1

- No changes affecting OCRmyPDF functionality for command line end users.
- webservice: made page specification easier to find in UI.
- webservice: fix download button downloads wrong file.
- Converted project documentation from rST to Markdown.
- Added README translation to Simplified Chinese. Thanks @HuaPai.
- Modernized license specification in pyproject.toml.
- Modernized SPDX license to REUSE.toml.

## v16.10.0

- Added hocr textangle processing, improving handling of text at angles.
  Thanks @0dinD {issue}`1467`
- Docker documentation updates related to podman. Thanks @rugk. {issue}`1489,1488`
- Dropped webservice.py's fragile use of ttyd. Instead, messages from ocrmypdf are
  printed to the console.
- Fixed broken test test_hocrtransform_matches_sandwich, which had become
  an invalid test. Thanks @QuLogic for reporting.
- Improved install instructions for Windows. Thanks @alex.

## v16.9.0

- Added hocr caption processing. Thanks @0dinD {issue}`1466`
- ocrmypdf-alpine Docker image is now built with Alpine 3.21.
- Fixed error handling of PDFs that contain invalid images with both ImageMask
  and ColorSpace defined. {issue}`1453`
- Fixed test suite regression when only older Ghostscripts are installed.
- Improved documetnation of \_progressbar.py. Thanks @QuentinFuxa. {issue}`1456`
- Disabling building of documentation as PDF on ReadTheDocs, as this caused
  complex build issues deemed not worth solving.

## v16.8.0

- Upgraded webservice.py demonstration using streamlit. It's now possible to
  exercise most of OCRmyPDF's functionality in a simple web UI.
- Added cache to Dockerfiles to improve build speed.
- Fixed numerous formatting errors in the documentation that prevented some
  parts of documentation from generating correctly.
- Improved OCR text rendering by suppressing negative-width spaces. Thanks
  @pajowu. {issue}`1446`
- Improved detecting of invisible text when using `--redo-ocr`. Thanks
  @pajowu. {issue}`1448``

## v16.7.0

- Fixed further issues with Docker build and updated some versions.
- Main Docker image returned to Ubuntu 24.04 since the fix in v16.6.2 resolved
  that concern.
- Code that previously sent Ghostscript output to stdout has been changed to
  output to temporary files, since Ghostscript was doing that anyway internally.
  This is a modest efficiency improvement.
- Fixed an issue with debug log output being parsed as rich markup. {issue}`1444`

## v16.6.2

- Remove invalid hyperlink annotations to satisfy Ghostscript 10.x during PDF/A
  conversion. {issue}`1425`

## v16.6.1

- Fixed some issues with Docker build, such as removing unnecessary content and using
  a stable Tesseract version.
- Reverted Docker image to Ubuntu 22.04 to access older/more stable Ghostscript
  for now.
- Clarified batch commands in documentation.
- Fixed an issue with JSON serialization and pickling of HOCRResult. {issue}`1427`

## v16.6.0

- Fixed an issue where damaged PDFs would fail with `--redo-ocr`. {issue}`1403`
- Fixed an error that prevented JBIG2 optimization on Windows if the image
  was optimized in an earlier step. {issue}`1396`
- Fixed an error detecting the version of unpaper 7.0.0. {issue}`1409`
- Fixed a performance regression when scanning pages. {issue}`1378`. Thanks @aliemjay.
- Fixed Alpine Docker image by enforcing Alpine 3.19. Alpine 3.20 includes a
  defective version of Tesseract OCR and so is not usable.
- Upgraded Ubuntu Docker image to use Ubuntu 24.04.
- Build and test scripts/actions switched to uv.
- When running in a container, we now remind the user that temporary folders
  are inside the container and may not be accessible.
- Fixed Linux test coverage matrix, which was missing some key versions.

## v16.5.0

- Fixed issue with interpreting PDFs that have images with array masks.
  {issue}`1377`
- Enabled testing on Python 3.13.
- Fixed a test that did not work correctly but still passed. {issue}`1382`
- Improved "PDF/A conversion failed" warning message to better describe implications.
- Updated documentation to better explain OCR_JSON_SETTINGS in batch processing.
- Build backend changed from setuptools to hatchling.

## v16.4.3

- Work around pdfminer.six issue where a token on the buffer boundary is incorrectly
  parsed as two tokens. {issue}`1361`
- New rules are applied to stencil masks and explicit masks when calculating the
  optimal page DPI for rendering. {issue}`1362`
- Fixed attempts to use an incompatible jbig2.EXE provided by TeX Live. {issue}`1363`

## v16.4.2

- Fixed order of filenames passed to Ghostscript for PDF/A generation. {issue}`1359`
- Suppressed missing jbig2dec warning message. {issue}`1358`
- Fixed calculation of image size when soft mask dimensions don't match image
  dimension. {issue}`1351`
- Several fixes to documentation. Thanks to users Iris and JoKalliauer
  who contributed these changes.
- Fixed error on processing PDFs that are missing certain image metadata. {issue}`1315`

## v16.4.1

- Fixed calculation of image printed area (used in finding weighted DPI for OCR).
  {issue}`1334`
- Fixed "NotImplementedError: not sure how to get colorspace" error
  messages in logs which simply records a failure to optimize images with
  print production colorspaces. {issue}`1315`

## v16.4.0

- Selecting the `osd` and `equ` pseudo-languages with `-l/--language` now
  exits with an error when using Tesseract OCR, because these are not
  regular Tesseract languages but implementation details implemented.
  Using them can cause Tesseract to crash.
- The hOCR renderer is more tolerant of extra whitespace in input files.
- watcher.py now changes the output file extension to .pdf when the input is not
  .pdf.
- Improved handling of PDFs that contain circularly referenced Form XObjects.
  {issue}`1321`
- Fixed Alpine Docker image for ARM64, which was not building correctly.
- Docker images now use pikepdf 9.0.0.
- Prevent use of Tesseract OCR 5.4.0, a version with known regressions.
- Disabled progressbar for "Linearizing" when `--no-progress-bar` set.
- Fixed some tests that warn about missing JBIG2 decoding via pikepdf, by
  installing the necessary libraries during tests.

## v16.3.1

- Fixed a test suite failure with Ghostscript 10.03.0+. {issue}`1316`
- Fixed an issue with the presentation of the "OCR" progress bar. {issue}`1313`

## v16.3.0

- Fixed progress bar not displaying for Ghostscript PDF/A conversion. {issue}`1313`
- Added progress bar for linearization. {issue}`1313`
- If `--rotate-pages-threshold` issued without `--rotate-pages` we now exit with
  an error since the user likely intended to use `--rotate-pages`. {issue}`1309`
- If Tesseract hOCR gives an invalid line box, print an error message instead of
  exiting with an error. {issue}`1312`

## v16.2.0

- Fixed issue 'NoneType' object has no attribute 'get' when optimizing certain PDFs.
  {issue}`1293,1271`
- Switched formatting from black to ruff.
- Added support for sending sidecar output to io.BytesIO.
- Added support for converting HEIF/HEIC images (the native image of iPhones and
  some other devices) to PDFs, when the appropriate pi-hief library is installed.
  This library is marked as a dependency, but maintainers may opt out if needed.
- We now default to downsampling large images that would exceed Tesseract's internal
  limits, but only if it cause processing to fail. Previously, this behavior only
  occurred if specifically requested on command line. It can still be configured
  and disabled. See the --tesseract command line options.
- Added Macports install instructions. Thanks @akierig.
- Improved logging output when an unexpected error occurs while trying to obtain
  the version of a third party program.

## v16.1.2

- Fixed test suite failure when using Ghostscript 10.3.
- Other minor corrections.

## v16.1.1

- Fixed PyPy 3.10 support.

## v16.1.0

- Improved hOCR renderer is now default for left to right languages.
- Improved handling of rotated pages. Previously, OCR text might be missing for
  pages that were rotated with a /Rotate tag on the page entry.
- Improved handling of cropped pages. Previously, in some cases a page with a
  crop box would not have its OCR applied correctly and misalignment between
  OCR text and visible text coudl occur.
- Documentation improvements, especially installation instructions for less
  common platforms.

## v16.0.4

- Fixed some issues for left-to-right text with the new hOCR renderer. It is still
  not default yet but will be made so soon. Right-to-left text is still in progress.
- Added an error to prevent use of several versions of Ghostscript that seem
  corrupt existing text in input PDFs. Newly generated OCR is not affected.
  For best results, use Ghostscript 10.02.1 or newer, which contains the fix
  for the issue.

## v16.0.3

- Changed minimum required Ghostscript to 9.54, to support users of RHEL 9 and its
  derivatives, since that is the latest version available there.
- Removed warning message about CVE-2023-43115, on the assumption that most
  distributions have backported the patch by now.

## v16.0.2

- Temporarily changed PDF text renderer back to sandwich by default to address
  regressions in macOS Preview.

## v16.0.1

- Fixed text rendering issue with new hOCR text renderer - extraneous byte order
  marks.
- Tightened dependencies.

## v16.0.0

- Added OCR text renderer, combined the best ideas of Tesseract's PDF
  generator and the older hOCR transformer renderer. The result is a hopefully
  permanent fix for wordssmushedtogetherwithoutspaces issues in extracted text,
  better registration/position of text on skewed baselines {issue}`1009`,
  fixes to character output when the German Fraktur script is used {issue}`1191`,
  proper rendering of right to left languages (Arabic, Hebrew, Persian) {issue}`1157`.
  Asian languages may still have excessive word breaks compared to expectations.
  The new renderer is the default; the old sandwich renderer is still available
  using `--pdf-renderer sandwich`; the old hOCR renderer is no more.
- The `ocrmypdf.hocrtransform` API has changed substantially.
- Support for Python 3.9 has been dropped. Python 3.10+ is now required.
- pikepdf >= 8.8.0 is now required.

