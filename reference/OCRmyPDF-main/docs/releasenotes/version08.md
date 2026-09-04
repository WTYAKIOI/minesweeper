% SPDX-FileCopyrightText: 2022 James R. Barlow
% SPDX-License-Identifier: CC-BY-SA-4.0

# v8

## v8.3.2

- Dropped workaround for macOS that allowed it work without pdfminer.six,
  now a proper sdist release of pdfminer.six is available.
- pikepdf 1.5.0 is now required.

## v8.3.1

- Fixed an issue where PDFs with malformed metadata would be rendered as
  blank pages. {issue}`398`.

## v8.3.0

- Improved the strategy for updating pages when a new image of the page
  was produced. We now attempt to preserve more content from the
  original file, for annotations in particular.
- For PDFs with more than 100 pages and a sequence where one PDF page
  was replaced and one or more subsequent ones were skipped, an
  intermediate file would be corrupted while grafting OCR text, causing
  processing to fail. This is a regression, likely introduced in
  v8.2.4.
- Previously, we resized the images produced by Ghostscript by a small
  number of pixels to ensure the output image size was an exactly what
  we wanted. Having discovered a way to get Ghostscript to produce the
  exact image sizes we require, we eliminated the resizing step.
- Command line completions for `bash` are now available, in addition
  to `fish`, both in `misc/completion`. Package maintainers, please
  install these so users can take advantage.
- Updated requirements.
- pikepdf 1.3.0 is now required.

## v8.2.4

- Fixed a false positive while checking for a certain type of PDF that
  only Acrobat can read. We now more accurately detect Acrobat-only
  PDFs.
- OCRmyPDF holds fewer open file handles and is more prompt about
  releasing those it no longer needs.
- Minor optimization: we no longer traverse the table of contents to
  ensure all references in it are resolved, as changes to libqpdf have
  made this unnecessary.
- pikepdf 1.2.0 is now required.

## v8.2.3

- Fixed that `--mask-barcodes` would occasionally leave a unwanted
  temporary file named `junkpixt` in the current working folder.
- Fixed (hopefully) handling of Leptonica errors in an environment
  where a non-standard `sys.stderr` is present.
- Improved help text for `--verbose`.

## v8.2.2

- Fixed a regression from v8.2.0, an exception that occurred while
  attempting to report that `unpaper` or another optional dependency
  was unavailable.
- In some cases, `ocrmypdf [-c|--clean]` failed to exit with an error
  when `unpaper` is not installed.

## v8.2.1

- This release was canceled.

## v8.2.0

- A major improvement to our Docker image is now available thanks to
  hard work contributed by @mawi12345. The new Docker image,
  ocrmypdf-alpine, is based on Alpine Linux, and includes most of the
  functionality of three existed images in a smaller package. This
  image will replace the main Docker image eventually but for now all
  are being built. [See documentation for
  details](https://ocrmypdf.readthedocs.io/en/latest/docker.html).
- Documentation reorganized especially around the use of Docker images.
- Fixed a problem with PDF image optimization, where the optimizer
  would unnecessarily decompress and recompress PNG images, in some
  cases losing the benefits of the quantization it just had just
  performed. The optimizer is now capable of embedding PNG images into
  PDFs without transcoding them.
- Fixed a minor regression with lossy JBIG2 image optimization. All
  JBIG2 candidates images were incorrectly placed into a single
  optimization group for the whole file, instead of grouping pages
  together. This usually makes a larger JBIG2Globals dictionary and
  results in inferior compression, so it worked less well than
  designed. However, quality would not be impacted. Lossless JBIG2 was
  entirely unaffected.
- Updated dependencies, including pikepdf to 1.1.0. This fixes
  {issue}`358`.
- The install-time version checks for certain external programs have
  been removed from setup.py. These tests are now performed at
  run-time.
- The non-standard option to override install-time checks
  (`setup.py install --force`) is now deprecated and prints a
  warning. It will be removed in a future release.

## v8.1.0

- Added a feature, `--unpaper-args`, which allows passing arbitrary
  arguments to `unpaper` when using `--clean` or `--clean-final`.
  The default, very conservative unpaper settings are suppressed.
- The argument `--clean-final` now implies `--clean`. It was
  possible to issue `--clean-final` on its before this, but it would
  have no useful effect.
- Fixed an exception on traversing corrupt table of contents entries
  (specifically, those with invalid destination objects)
- Fixed an issue when using `--tesseract-timeout` and image
  processing features on a file with more than 100 pages.
  {issue}`347`
- OCRmyPDF now always calls `os.nice(5)` to signal to operating
  systems that it is a background process.

## v8.0.1

- Fixed an exception when parsing PDFs that are missing a required
  field. {issue}`325`
- pikepdf 1.0.5 is now required, to address some other PDF parsing
  issues.

## v8.0.0

No major features. The intent of this release is to sever support for
older versions of certain dependencies.

**Breaking changes**

- Dropped support for Tesseract 3.x. Tesseract 4.0 or newer is now
  required.
- Dropped support for Python 3.5.
- Some `ocrmypdf.pdfa` APIs that were deprecated in v7.x were
  removed. This functionality has been moved to pikepdf.

**Other changes**

- Fixed an unhandled exception when attempting to mask barcodes.
  {issue}`322`
- It is now possible to use ocrmypdf without pdfminer.six, to support
  distributions that do not have it or cannot currently use it (e.g.
  Homebrew). Downstream maintainers should include pdfminer.six if
  possible.
- A warning is now issue when PDF/A conversion removes some XMP
  metadata from the input PDF. (Only a "whitelist" of certain XMP
  metadata types are allowed in PDF/A.)
- Fixed several issues that caused PDF/As to be produced with
  nonconforming XMP metadata (would fail validation with veraPDF).
- Fixed some instances where invalid DocumentInfo from a PDF cause XMP
  metadata creation to fail.
- Fixed a few documentation problems.
- pikepdf 1.0.2 is now required.

