% SPDX-FileCopyrightText: 2022 James R. Barlow
% SPDX-License-Identifier: CC-BY-SA-4.0

# v7

## v7.4.0

- `--force-ocr` may now be used with the new `--threshold` and
  `--mask-barcodes` features
- pikepdf >= 0.9.1 is now required.
- Changed metadata handling to pikepdf 0.9.1. As a result, metadata
  handling of non-ASCII characters in Ghostscript 9.25 or later is
  fixed.
- chardet >= 3.0.4 is temporarily listed as required. pdfminer.six
  depends on it, but the most recent release does not specify this
  requirement.
  ({issue}`326`)
- python-xmp-toolkit and libexempi are no longer required.
- A new Docker image is now being provided for users who wish to access
  OCRmyPDF over a simple HTTP interface, instead of the command line.
- Increase tolerance of PDFs that overflow or underflow the PDF
  graphics stack.
  ({issue}`325`)

## v7.3.1

- Fixed performance regression from v7.3.0; fast page analysis was not
  selected when it should be.
- Fixed a few exceptions related to the new `--mask-barcodes` feature
  and improved argument checking
- Added missing detection of TrueType fonts that lack a Unicode mapping

## v7.3.0

- Added a new feature `--redo-ocr` to detect existing OCR in a file,
  remove it, and redo the OCR. This may be particularly helpful for
  anyone who wants to take advantage of OCR quality improvements in
  Tesseract 4.0. Note that OCR added by OCRmyPDF before version 3.0
  cannot be detected since it was not properly marked as invisible text
  in the earliest versions. OCR that constructs a font from visible
  text, such as Adobe Acrobat's ClearScan.

- OCRmyPDF's content detection is generally more sophisticated. It
  learns more about the contents of each PDF and makes better
  recommendations:

  - OCRmyPDF can now detect when a PDF contains text that cannot be
    mapped to Unicode (meaning it is readable to human eyes but
    copy-pastes as gibberish). In these cases it recommends
    `--force-ocr` to make the text searchable.
  - PDFs containing vector objects are now rendered at more
    appropriate resolution for OCR.
  - We now exit with an error for PDFs that contain Adobe LiveCycle
    Designer's dynamic XFA forms. Currently the open source community
    does not have tools to work with these files.
  - OCRmyPDF now warns when a PDF that contains Adobe AcroForms, since
    such files probably do not need OCR. It can work with these files.

- Added three new **experimental** features to improve OCR quality in
  certain conditions. The name, syntax and behavior of these arguments
  is subject to change. They may also be incompatible with some other
  features.

  - `--remove-vectors` which strips out vector graphics. This can
    improve OCR quality since OCR will not search artwork for readable
    text; however, it currently removes "text as curves" as well.
  - `--mask-barcodes` to detect and suppress barcodes in files. We
    have observed that barcodes can interfere with OCR because they
    are "text-like" but not actually textual.
  - `--threshold` which uses a more sophisticated thresholding
    algorithm than is currently in use in Tesseract OCR. This works
    around a [known issue in Tesseract
    4.0](https://github.com/tesseract-ocr/tesseract/issues/1990)
    with dark text on bright backgrounds.

- Fixed an issue where an error message was not reported when the
  installed Ghostscript was very old.

- The PDF optimizer now saves files with object streams enabled when
  the optimization level is `--optimize 1` or higher (the default).
  This makes files a little bit smaller, but requires PDF 1.5. PDF 1.5
  was first released in 2003 and is broadly supported by PDF viewers,
  but some rudimentary PDF parsers such as PyPDF2 do not understand
  object streams. You can use the command line tool
  `qpdf --object-streams=disable` or
  [pikepdf](https://github.com/pikepdf/pikepdf) library to remove
  them.

- New dependency: pdfminer.six 20181108. Note this is a fork of the
  Python 2-only pdfminer.

- Deprecation notice: At the end of 2018, we will be ending support for
  Python 3.5 and Tesseract 3.x. OCRmyPDF v7 will continue to work with
  older versions.

## v7.2.1

- Fixed compatibility with an API change in pikepdf 0.3.5.
- A kludge to support Leptonica versions older than 1.72 in the test
  suite was dropped. Older versions of Leptonica are likely still
  compatible. The only impact is that a portion of the test suite will
  be skipped.

## v7.2.0

**Lossy JBIG2 behavior change**

A user reported that ocrmypdf was in fact using JBIG2 in **lossy**
compression mode. This was not the intended behavior. Users should
[review the technical concerns with JBIG2 in lossy
mode](https://abbyy.technology/en:kb:tip:jbig2_compression_and_ocr)
and decide if this is a concern for their use case.

JBIG2 lossy mode does achieve higher compression ratios than any other
monochrome compression technology; for large text documents the savings
are considerable. JBIG2 lossless still gives great compression ratios
and is a major improvement over the older CCITT G4 standard.

Only users who have reviewed the concerns with JBIG2 in lossy mode
should opt-in. As such, lossy mode JBIG2 is only turned on when the new
argument `--jbig2-lossy` is issued. This is independent of the setting
for `--optimize`.

Users who did not install an optional JBIG2 encoder are unaffected.

(Thanks to user 'bsdice' for reporting this issue.)

**Other issues**

- When the image optimizer quantizes an image to 1 bit per pixel, it
  will now attempt to further optimize that image as CCITT or JBIG2,
  instead of keeping it in the "flate" encoding which is not efficient
  for 1 bpp images.
  ({issue}`297`)
- Images in PDFs that are used as soft masks (i.e. transparency masks
  or alpha channels) are now excluded from optimization.
- Fixed handling of Tesseract 4.0-rc1 which now accepts invalid
  Tesseract configuration files, which broke the test suite.

## v7.1.0

- Improve the performance of initial text extraction, which is done to
  determine if a file contains existing text of some kind or not. On
  large files, this initial processing is now about 20x times faster.
  ({issue}`299`)
- pikepdf 0.3.3 is now required.
- Fixed {issue}`231`, a
  problem with JPEG2000 images where image metadata was only available
  inside the JPEG2000 file.
- Fixed some additional Ghostscript 9.25 compatibility issues.
- Improved handling of KeyboardInterrupt error messages.
  ({issue}`301`)
- README.md is now served in GitHub markdown instead of
  reStructuredText.

## v7.0.6

- Blacklist Ghostscript 9.24, now that 9.25 is available and fixes many
  regressions in 9.24.

## v7.0.5

- Improve capability with Ghostscript 9.24, and enable the JPEG
  passthrough feature when this version in installed.
- Ghostscript 9.24 lost the ability to set PDF title, author, subject
  and keyword metadata to Unicode strings. OCRmyPDF will set ASCII
  strings and warn when Unicode is suppressed. Other software may be
  used to update metadata. This is a short term work around.
- PDFs generated by Kodak Capture Desktop, or generally PDFs that
  contain indirect references to null objects in their table of
  contents, would have an invalid table of contents after processing by
  OCRmyPDF that might interfere with other viewers. This has been
  fixed.
- Detect PDFs generated by Adobe LiveCycle, which can only be displayed
  in Adobe Acrobat and Reader currently. When these are encountered,
  exit with an error instead of performing OCR on the "Please wait"
  error message page.

## v7.0.4

- Fixed exception thrown when trying to optimize a certain type of PNG
  embedded in a PDF with the `-O2`
- Update to pikepdf 0.3.2, to gain support for optimizing some
  additional image types that were previously excluded from
  optimization (CMYK and grayscale). Fixes
  {issue}`285`.

## v7.0.3

- Fixed {issue}`284`, an error
  when parsing inline images that have are also image masks, by
  upgrading pikepdf to 0.3.1

## v7.0.2

- Fixed a regression with `--rotate-pages` on pages that already had
  rotations applied.
  ({issue}`279`)
- Improve quality of page rotation in some cases by rasterizing a
  higher quality preview image.
  ({issue}`281`)

## v7.0.1

- Fixed compatibility with img2pdf >= 0.3.0 by rejecting input images
  that have an alpha channel
- Add forward compatibility for pikepdf 0.3.0 (unrelated to img2pdf)
- Various documentation updates for v7.0.0 changes

## v7.0.0

- The core algorithm for combining OCR layers with existing PDF pages
  has been rewritten and improved considerably. PDFs are no longer
  split into single page PDFs for processing; instead, images are
  rendered and the OCR results are grafted onto the input PDF. The new
  algorithm uses less temporary disk space and is much more performant
  especially for large files.

- New dependency: [pikepdf](https://github.com/pikepdf/pikepdf).
  pikepdf is a powerful new Python PDF library driving the latest
  OCRmyPDF features, built on the QPDF C++ library (libqpdf).

- New feature: PDF optimization with `-O` or `--optimize`. After
  OCR, OCRmyPDF will perform image optimizations relevant to OCR PDFs.

  - If a JBIG2 encoder is available, then monochrome images will be
    converted, with the potential for huge savings on large black and
    white images, since JBIG2 is far more efficient than any other
    monochrome (bi-level) compression. (All known US patents related
    to JBIG2 have probably expired, but it remains the responsibility
    of the user to supply a JBIG2 encoder such as
    [jbig2enc](https://github.com/agl/jbig2enc). OCRmyPDF does not
    implement JBIG2 encoding.)
  - If `pngquant` is installed, OCRmyPDF will optionally use it to
    perform lossy quantization and compression of PNG images.
  - The quality of JPEGs can also be lowered, on the assumption that a
    lower quality image may be suitable for storage after OCR.
  - This image optimization component will eventually be offered as an
    independent command line utility.
  - Optimization ranges from `-O0` through `-O3`, where `0`
    disables optimization and `3` implements all options. `1`, the
    default, performs only safe and lossless optimizations. (This is
    similar to GCC's optimization parameter.) The exact type of
    optimizations performed will vary over time.

- Small amounts of text in the margins of a page, such as watermarks,
  page numbers, or digital stamps, will no longer prevent the rest of a
  page from being OCRed when `--skip-text` is issued. This behavior
  is based on a heuristic.

- Removed features

  - The deprecated `--pdf-renderer tesseract` PDF renderer was
    removed.
  - `-g`, the option to generate debug text pages, was removed
    because it was a maintenance burden and only worked in isolated
    cases. HOCR pages can still be previewed by running the
    hocrtransform.py with appropriate settings.

- Removed dependencies

  - `PyPDF2`
  - `defusedxml`
  - `PyMuPDF`

- The `sandwich` PDF renderer can be used with all supported versions
  of Tesseract, including that those prior to v3.05 which don't support
  `-c textonly`. (Tesseract v4.0.0 is recommended and more
  efficient.)

- `--pdf-renderer auto` option and the diagnostics used to select a
  PDF renderer now work better with old versions, but may make
  different decisions than past versions.

- If everything succeeds but PDF/A conversion fails, a distinct return
  code is now returned (`ExitCode.pdfa_conversion_failed (10)`) where
  this situation previously returned
  `ExitCode.invalid_output_pdf (4)`. The latter is now returned only
  if there is some indication that the output file is invalid.

- Notes for downstream packagers

  - There is also a new dependency on `python-xmp-toolkit` which in
    turn depends on `libexempi3`.
  - It may be necessary to separately `pip install pycparser` to
    avoid [another Python 3.7
    issue](https://github.com/eliben/pycparser/pull/135).

