% SPDX-FileCopyrightText: 2022 James R. Barlow
% SPDX-License-Identifier: CC-BY-SA-4.0

# v4

## v4.5.6

- Fixed {issue}`156`,
  'NoneType' object has no attribute 'getObject' on pages with no
  optional /Contents record. This should resolve all issues related to
  pages with no /Contents record.
- Fixed {issue}`158`, ocrmypdf
  now stops and terminates if Ghostscript fails on an intermediate
  step, as it is not possible to proceed.
- Fixed {issue}`160`,
  exception thrown on certain invalid arguments instead of error
  message

## v4.5.5

- Automated update of macOS homebrew tap
- Fixed {issue}`154`, KeyError
  '/Contents' when searching for text on blank pages that have no
  /Contents record. Note: incomplete fix for this issue.

## v4.5.4

- Fixed `--skip-big` raising an exception if a page contains no images
  ({issue}`152`) (thanks
  to @TomRaz)
- Fixed an issue where pages with no images might trigger "cannot write
  mode P as JPEG"
  ({issue}`151`)

## v4.5.3

- Added a workaround for Ghostscript 9.21 and probably earlier versions
  would fail with the error message "VMerror -25", due to a Ghostscript
  bug in XMP metadata handling
- High Unicode characters (U+10000 and up) are no longer accepted for
  setting metadata on the command line, as Ghostscript may not handle
  them correctly.
- Fixed an issue where the `tess4` renderer would duplicate content
  onto output pages if tesseract failed or timed out
- Fixed `tess4` renderer not recognized when lossless reconstruction
  is possible

## v4.5.2

- Fixed {issue}`147`,
  `--pdf-renderer tess4 --clean` will produce an oversized page
  containing the original image in the bottom left corner, due to loss
  DPI information.
- Make "using Tesseract 4.0" warning less ominous
- Set up machinery for homebrew OCRmyPDF tap

## v4.5.1

- Fixed {issue}`137`,
  proportions of images with a non-square pixel aspect ratio would be
  distorted in output for `--force-ocr` and some other combinations
  of flags

## v4.5

- PDFs containing "Form XObjects" are now supported (issue
  {issue}`134`; PDF
  reference manual 8.10), and images they contain are taken into
  account when determining the resolution for rasterizing
- The Tesseract 4 Docker image no longer includes all languages,
  because it took so long to build something would tend to fail
- OCRmyPDF now warns about using `--pdf-renderer tesseract` with
  Tesseract 3.04 or lower due to issues with Ghostscript corrupting the
  OCR text in these cases

## v4.4.2

- The Docker images (ocrmypdf, ocrmypdf-polyglot, ocrmypdf-tess4) are
  now based on Ubuntu 16.10 instead of Debian stretch

  - This makes supporting the Tesseract 4 image easier
  - This could be a disruptive change for any Docker users who built
    customized these images with their own changes, and made those
    changes in a way that depends on Debian and not Ubuntu

- OCRmyPDF now prevents running the Tesseract 4 renderer with Tesseract
  3.04, which was permitted in v4.4 and v4.4.1 but will not work

## v4.4.1

- To prevent a [TIFF output
  error](https://github.com/python-pillow/Pillow/issues/2206) caused
  by img2pdf >= 0.2.1 and Pillow \<= 3.4.2, dependencies have been
  tightened
- The Tesseract 4.00 simultaneous process limit was increased from 1 to
  2, since it was observed that 1 lowers performance
- Documentation improvements to describe the `--tesseract-config`
  feature
- Added test cases and fixed error handling for `--tesseract-config`
- Tweaks to setup.py to deal with issues in the v4.4 release

## v4.4

- Tesseract 4.00 is now supported on an experimental basis.

  - A new rendering option `--pdf-renderer tess4` exploits Tesseract
    4's new text-only output PDF mode. See the documentation on PDF
    Renderers for details.
  - The `--tesseract-oem` argument allows control over the Tesseract
    4 OCR engine mode (tesseract's `--oem`). Use
    `--tesseract-oem 2` to enforce the new LSTM mode.
  - Fixed poor performance with Tesseract 4.00 on Linux

- Fixed an issue that caused corruption of output to stdout in some
  cases

- Removed test for Pillow JPEG and PNG support, as the minimum
  supported version of Pillow now enforces this

- OCRmyPDF now tests that the intended destination file is writable
  before proceeding

- The test suite now requires `pytest-helpers-namespace` to run (but
  not install)

- Significant code reorganization to make OCRmyPDF re-entrant and
  improve performance. All changes should be backward compatible for
  the v4.x series.

  - However, OCRmyPDF's dependency "ruffus" is not re-entrant, so no
    Python API is available. Scripts should continue to use the
    command line interface.

## v4.3.5

- Update documentation to confirm Python 3.6.0 compatibility. No code
  changes were needed, so many earlier versions are likely supported.

## v4.3.4

- Fixed "decimal.InvalidOperation: quantize result has too many digits"
  for high DPI images

## v4.3.3

- Fixed PDF/A creation with Ghostscript 9.20 properly
- Fixed an exception on inline stencil masks with a missing optional
  parameter

## v4.3.2

- Fixed a PDF/A creation issue with Ghostscript 9.20 (note: this fix
  did not actually work)

## v4.3.1

- Fixed an issue where pages produced by the "hocr" renderer after a
  Tesseract timeout would be rotated incorrectly if the input page was
  rotated with a /Rotate marker
- Fixed a file handle leak in LeptonicaErrorTrap that would cause a
  "too many open files" error for files around hundred pages of pages
  long when `--deskew` or `--remove-background` or other Leptonica
  based image processing features were in use, depending on the system
  value of `ulimit -n`
- Ability to specify multiple languages for multilingual documents is
  now advertised in documentation
- Reduced the file sizes of some test resources
- Cleaned up debug output
- Tesseract caching in test cases is now more cautious about false
  cache hits and reproducing exact output, not that any problems were
  observed

## v4.3

- New feature `--remove-background` to detect and erase the
  background of color and grayscale images

- Better documentation

- Fixed an issue with PDFs that draw images when the raster stack depth
  is zero

- ocrmypdf can now redirect its output to stdout for use in a shell
  pipeline

  - This does not improve performance since temporary files are still
    used for buffering
  - Some output validation is disabled in this mode

## v4.2.5

- Fixed an issue
  ({issue}`100`) with
  PDFs that omit the optional /BitsPerComponent parameter on images
- Removed non-free file milk.pdf

## v4.2.4

- Fixed an error
  ({issue}`90`) caused by
  PDFs that use stencil masks properly
- Fixed handling of PDFs that try to draw images or stencil masks
  without properly setting up the graphics state (such images are now
  ignored for the purposes of calculating DPI)

## v4.2.3

- Fixed an issue with PDFs that store page rotation (/Rotate) in an
  indirect object

- Integrated a few fixes to simplify downstream packaging (Debian)

  - The test suite no longer assumes it is installed
  - If running Linux, skip a test that passes Unicode on the command
    line

- Added a test case to check explicit masks and stencil masks

- Added a test case for indirect objects and linearized PDFs

- Deprecated the OCRmyPDF.sh shell script

## v4.2.2

- Improvements to documentation

## v4.2.1

- Fixed an issue where PDF pages that contained stencil masks would
  report an incorrect DPI and cause Ghostscript to abort
- Implemented stdin streaming

## v4.2

- ocrmypdf will now try to convert single image files to PDFs if they
  are provided as input
  ({issue}`15`)

  - This is a basic convenience feature. It only supports a single
    image and always makes the image fill the whole page.
  - For better control over image to PDF conversion, use `img2pdf`
    (one of ocrmypdf's dependencies)

- New argument `--output-type {pdf|pdfa}` allows disabling
  Ghostscript PDF/A generation

  - `pdfa` is the default, consistent with past behavior
  - `pdf` provides a workaround for users concerned about the
    increase in file size from Ghostscript forcing JBIG2 images to
    CCITT and transcoding JPEGs
  - `pdf` preserves as much as it can about the original file,
    including problems that PDF/A conversion fixes

- PDFs containing images with "non-square" pixel aspect ratios, such as
  200x100 DPI, are now handled and converted properly (fixing a bug
  that caused to be cropped)

- `--force-ocr` rasterizes pages even if they contain no images

  - supports users who want to use OCRmyPDF to reconstruct text
    information in PDFs with damaged Unicode maps (copy and paste text
    does not match displayed text)
  - supports reinterpreting PDFs where text was rendered as curves for
    printing, and text needs to be recovered
  - fixes issue
    {issue}`82`

- Fixes an issue where, with certain settings, monochrome images in
  PDFs would be converted to 8-bit grayscale, increasing file size
  ({issue}`79`)

- Support for Ubuntu 12.04 LTS "precise" has been dropped in favor of
  (roughly) Ubuntu 14.04 LTS "trusty"

  - Some Ubuntu "PPAs" (backports) are needed to make it work

- Support for some older dependencies dropped

  - Ghostscript 9.15 or later is now required (available in Ubuntu
    trusty with backports)
  - Tesseract 3.03 or later is now required (available in Ubuntu
    trusty)

- Ghostscript now runs in "safer" mode where possible

## v4.1.4

- Bug fix: monochrome images with an ICC profile attached were
  incorrectly converted to full color images if lossless reconstruction
  was not possible due to other settings; consequence was increased
  file size for these images

## v4.1.3

- More helpful error message for PDFs with version 4 security handler
- Update usage instructions for Windows/Docker users
- Fixed order of operations for matrix multiplication (no effect on most
  users)
- Add a few leptonica wrapper functions (no effect on most users)

## v4.1.2

- Replace IEC sRGB ICC profile with Debian's sRGB (from
  icc-profiles-free) which is more compatible with the MIT license
- More helpful error message for an error related to certain types of
  malformed PDFs

## v4.1

- `--rotate-pages` now only rotates pages when reasonably confidence
  in the orientation. This behavior can be adjusted with the new
  argument `--rotate-pages-threshold`
- Fixed problems in error checking if `unpaper` is uninstalled or
  missing at run-time
- Fixed problems with "RethrownJobError" errors during error handling
  that suppressed the useful error messages

## v4.0.7

- Minor correction to Ghostscript output settings

## v4.0.6

- Update install instructions
- Provide a sRGB profile instead of using Ghostscript's

## v4.0.5

- Remove some verbose debug messages from v4.0.4
- Fixed temporary that wasn't being deleted
- DPI is now calculated correctly for cropped images, along with other
  image transformations
- Inline images are now checked during DPI calculation instead of
  rejecting the image

## v4.0.4

Released with verbose debug message turned on. Do not use. Skip to
v4.0.5.

## v4.0.3

New features

- Page orientations detected are now reported in a summary comment

Fixes

- Show stack trace if unexpected errors occur
- Treat "too few characters" error message from Tesseract as a reason
  to skip that page rather than abort the file
- Docker: fix blank JPEG2000 issue by insisting on Ghostscript versions
  that have this fixed

## v4.0.2

Fixes

- Fixed compatibility with Tesseract 3.04.01 release, particularly its
  different way of outputting orientation information
- Improved handling of Tesseract errors and crashes
- Fixed use of chmod on Docker that broke most test cases

## v4.0.1

Fixes

- Fixed a KeyError if tesseract fails to find page orientation
  information

## v4.0

New features

- Automatic page rotation (`-r`) is now available. It uses ignores
  any prior rotation information on PDFs and sets rotation based on the
  dominant orientation of detectable text. This feature is fairly
  reliable but some false positives occur especially if there is not
  much text to work with.
  ({issue}`4`)
- Deskewing is now performed using Leptonica instead of unpaper.
  Leptonica is faster and more reliable at image deskewing than
  unpaper.

Fixes

- Fixed an issue where lossless reconstruction could cause some pages
  to be appear incorrectly if the page was rotated by the user in
  Acrobat after being scanned (specifically if it a /Rotate tag)
- Fixed an issue where lossless reconstruction could misalign the
  graphics layer with respect to text layer if the page had been
  cropped such that its origin is not (0, 0)
  ({issue}`49`)

Changes

- Logging output is now much easier to read
- `--deskew` is now performed by Leptonica instead of unpaper
  ({issue}`25`)
- libffi is now required
- Some changes were made to the Docker and Travis build environments to
  support libffi
- `--pdf-renderer=tesseract` now displays a warning if the Tesseract
  version is less than 3.04.01, the planned release that will include
  fixes to an important OCR text rendering bug in Tesseract 3.04.00.
  You can also manually install ./share/sharp2.ttf on top of pdf.ttf in
  your Tesseract tessdata folder to correct the problem.

