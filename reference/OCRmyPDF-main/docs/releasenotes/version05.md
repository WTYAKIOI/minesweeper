% SPDX-FileCopyrightText: 2022 James R. Barlow
% SPDX-License-Identifier: CC-BY-SA-4.0

# v5

## v5.7.0

- Fixed an issue that caused poor CPU utilization on machines with more
  than 4 cores when running Tesseract 4. (Related to {issue}`217`.)

- The 'hocr' renderer has been improved. The 'sandwich' and 'tesseract'
  renderers are still better for most use cases, but 'hocr' may be
  useful for people who work with the PDF.js renderer in English/ASCII
  languages. ({issue}`225`)

  - It now formats text in a matter that is easier for certain PDF
    viewers to select and extract copy and paste text. This should
    help macOS Preview and PDF.js in particular.
  - The appearance of selected text and behavior of selecting text is
    improved.
  - The PDF content stream now uses relative moves, making it more
    compact and easier for viewers to determine when two words on the
    same line.
  - It can now deal with text on a skewed baseline.
  - Thanks to @cforcey for the pull request, @jbreiden for many
    helpful suggestions, @ctbarbour for another round of improvements,
    and @acaloiaro for an independent review.

## v5.6.3

- Suppress two debug messages that were too verbose

## v5.6.2

- Development branch accidentally tagged as release. Do not use.

## v5.6.1

- Fixed {issue}`219`: change
  how the final output file is created to avoid triggering permission
  errors when the output is a special file such as `/dev/null`
- Fixed test suite failures due to a qpdf 8.0.0 regression and Python
  3.5's handling of symlink
- The "encrypted PDF" error message was different depending on the type
  of PDF encryption. Now a single clear message appears for all types
  of PDF encryption.
- ocrmypdf is now in Homebrew. Homebrew users are advised to the
  version of ocrmypdf in the official homebrew-core formulas rather
  than the private tap.
- Some linting

## v5.6.0

- Fixed {issue}`216`: preserve
  "text as curves" PDFs without rasterizing file
- Related to the above, messages about rasterizing are more consistent
- For consistency versions minor releases will now get the trailing .0
  they always should have had.

## v5.5

- Add new argument `--max-image-mpixels`. Pillow 5.0 now raises an
  exception when images may be decompression bombs. This argument can
  be used to override the limit Pillow sets.
- Fixed output page cropped when using the sandwich renderer and OCR is
  skipped on a rotated and image-processed page
- A warning is now issued when old versions of Ghostscript are used in
  cases known to cause issues with non-Latin characters
- Fixed a few parameter validation checks for `-output-type pdfa-1` and
  `pdfa-2`

## v5.4.4

- Fixed {issue}`181`: fix
  final merge failure for PDFs with more pages than the system file
  handle limit (`ulimit -n`)
- Fixed {issue}`200`: an
  uncommon syntax for formatting decimal numbers in a PDF would cause
  qpdf to issue a warning, which ocrmypdf treated as an error. Now this
  the warning is relayed.
- Fixed an issue where intermediate PDFs would be created at version 1.3
  instead of the version of the original file. It's possible but
  unlikely this had side effects.
- A warning is now issued when older versions of qpdf are used since
  issues like
  {issue}`200` cause
  qpdf to infinite-loop
- Address issue
  {issue}`140`: if
  Tesseract outputs invalid UTF-8, escape it and print its message
  instead of aborting with a Unicode error
- Adding previously unlisted setup requirement, pytest-runner
- Update documentation: fix an error in the example script for Synology
  with Docker images, improved security guidance, advised
  `pip install --user`

## v5.4.3

- If a subprocess fails to report its version when queried, exit
  cleanly with an error instead of throwing an exception
- Added test to confirm that the system locale is Unicode-aware and
  fail early if it's not
- Clarified some copyright information
- Updated pinned requirements.txt so the homebrew formula captures more
  recent versions

## v5.4.2

- Fixed a regression from v5.4.1 that caused sidecar files to be
  created as empty files

## v5.4.1

- Add workaround for Tesseract v4.00alpha crash when trying to obtain
  orientation and the latest language packs are installed

## v5.4

- Change wording of a deprecation warning to improve clarity
- Added option to generate PDF/A-1b output if desired
  (`--output-type pdfa-1`); default remains PDF/A-2b generation
- Update documentation

## v5.3.3

- Fixed missing error message that should occur when trying to force
  `--pdf-renderer sandwich` on old versions of Tesseract
- Update copyright information in test files
- Set system `LANG` to UTF-8 in Dockerfiles to avoid UTF-8 encoding
  errors

## v5.3.2

- Fixed a broken test case related to language packs

## v5.3.1

- Fixed wrong return code given for missing Tesseract language packs
- Fixed "brew audit" crashing on Travis when trying to auto-brew

## v5.3

- Added `--user-words` and `--user-patterns` arguments which are
  forwarded to Tesseract OCR as words and regular expressions
  respective to use to guide OCR. Supplying a list of subject-domain
  words should assist Tesseract with resolving words.
  ({issue}`165`)
- Using a non Latin-1 language with the "hocr" renderer now warns about
  possible OCR quality and recommends workarounds
  ({issue}`176`)
- Output file path added to error message when that location is not
  writable
  ({issue}`175`)
- Otherwise valid PDFs with leading whitespace at the beginning of the
  file are now accepted

## v5.2

- When using Tesseract 3.05.01 or newer, OCRmyPDF will select the
  "sandwich" PDF renderer by default, unless another PDF renderer is
  specified with the `--pdf-renderer` argument. The previous behavior
  was to select `--pdf-renderer=hocr`.
- The "tesseract" PDF renderer is now deprecated, since it can cause
  problems with Ghostscript on Tesseract 3.05.00
- The "tess4" PDF renderer has been renamed to "sandwich". "tess4" is
  now a deprecated alias for "sandwich".

## v5.1

- Files with pages larger than 200" (5080 mm) in either dimension are
  now supported with `--output-type=pdf` with the page size preserved
  (in the PDF specification this feature is called UserUnit scaling).
  Due to Ghostscript limitations this is not available in conjunction
  with PDF/A output.

## v5.0.1

- Fixed {issue}`169`,
  exception due to failure to create sidecar text files on some
  versions of Tesseract 3.04, including the jbarlow83/ocrmypdf Docker
  image

## v5.0

- Backward incompatible changes

  > - Support for Python 3.4 dropped. Python 3.5 is now required.
  > - Support for Tesseract 3.02 and 3.03 dropped. Tesseract 3.04 or
  >   newer is required. Tesseract 4.00 (alpha) is supported.
  > - The OCRmyPDF.sh script was removed.

- Add a new feature, `--sidecar`, which allows creating "sidecar"
  text files which contain the OCR results in plain text. These OCR
  text is more reliable than extracting text from PDFs. Closes
  {issue}`126`.

- New feature: `--pdfa-image-compression`, which allows overriding
  Ghostscript's lossy-or-lossless image encoding heuristic and making
  all images JPEG encoded or lossless encoded as desired. Fixes
  {issue}`163`.

- Fixed {issue}`143`, added
  `--quiet` to suppress "INFO" messages

- Fixed {issue}`164`, a typo

- Removed the command line parameters `-n` and `--just-print` since
  they have not worked for some time (reported as Ubuntu bug
  [#1687308](https://bugs.launchpad.net/ubuntu/+source/ocrmypdf/+bug/1687308))

