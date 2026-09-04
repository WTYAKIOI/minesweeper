% SPDX-FileCopyrightText: 2022 James R. Barlow
% SPDX-License-Identifier: CC-BY-SA-4.0

# v10

## v10.3.3

- Fixed a "KeyError: 'dpi'" error message when using `--threshold` on an image.
  ({issue}`607`)

## v10.3.2

- Fixed a case where we reported "no reason" for a file size increase, when we
  could determine the reason.
- Enabled support for pdfminer.six 20200726.

## v10.3.1

- Fixed a number of test suite failures with pdfminer.six older than version 20200402.
- Enabled support for pdfminer.six 20200720.

## v10.3.0

- Fixed an issue where we would consider images that were already JBIG2-encoded
  for optimization, potentially producing a less optimized image than the original.
  We do not believe this issue would ever cause an image to loss fidelity.
- Where available, pikepdf memory mapping is now used. This improves performance.
- When Leptonica 1.79+ is installed, use its new error handling API to avoid
  a "messy" redirection of stderr which was necessary to capture its error
  messages.
- For older versions of Leptonica, added a new thread level lock. This fixes a
  possible race condition in handling error conditions in Leptonica (although
  there is no evidence it ever caused issues in practice).
- Documentation improvements and more type hinting.

## v10.2.1

- Disabled calculation of text box order with pdfminer. We never needed this result
  and it is expensive to calculate on files with complex pre-existing text.
- Fixed plugin manager to accept `Path(plugin)` as a path to a plugin.
- Fixed some typing errors.
- Documentation improvements.

## v10.2.0

- Update Docker image to use Ubuntu 20.04.
- Fixed issue PDF/A acquires title "Untitled" after conversion. ({issue}`582`)
- Fixed a problem where, when using `--pdf-renderer hocr`, some text would
  be missing from the output when using a more recent version of Tesseract.
  Tesseract began adding more detailed markup about the semantics of text
  that our HOCR transform did not recognize, so it ignored them. This option is
  not the default. If necessary `--redo-ocr` also redoing OCR to fix such issues.
- Fixed an error in Python 3.9 beta, due to removal of deprecated
  `Element.getchildren()`. ({issue}`584`)
- Implemented support using the API with `BytesIO` and other file stream objects.
  ({issue}`545`)

## v10.1.1

- Fixed `OMP_THREAD_LIMIT` set to invalid value error messages on some input
  files. (The error was harmless, apart from less than optimal performance in
  some cases.)

## v10.1.0

- Previously, we `--clean-final` would cause an unpaper-cleaned page image to
  be produced twice, which was necessary in some cases but not in general. We
  now take this optimization opportunity and reuse the image if possible.
- We now provide PNG files as input to unpaper, since it accepts them, instead
  of generating PPM files which can be very large. This can improve performance
  and temporary disk usage.
- Documentation updated for plugins.

## v10.0.1

- Fixed regression when `-l lang1+lang2` is used from command line.

## v10.0.0

**Breaking changes**

- Support for pdfminer.six version 20181108 has been dropped, along with a
  monkeypatch that made this version work.
- Output messages are now displayed in color (when supported by the terminal)
  and prefixes describing the severity of the message are removed. As such
  programs that parse OCRmyPDF's log message will need to be revised. (Please
  consider using OCRmyPDF as a library instead.)
- The minimum version for certain dependencies has increased.
- Many API changes; see developer changes.
- The Python libraries pluggy and coloredlogs are now required.

**New features and improvements**

- PDF page scanning is now parallelized across CPUs, speeding up this phase
  dramatically for files with a high page counts.
- PDF page scanning is optimized, addressing some performance regressions.
- PDF page scanning is no longer run on pages that are not selected when the
  `--pages` argument is used.
- PDF page scanning is now independent of Ghostscript, ending our past reliance
  on this occasionally unstable feature in Ghostscript.
- A plugin architecture has been added, currently allowing one to more easily
  use a different OCR engine or PDF renderer from Tesseract and Ghostscript,
  respectively. A plugin can also override some decisions, such changing
  the OCR settings after initial scanning.
- Colored log messages.

**Developer changes**

- The test spoofing mechanism, used to test correct handling of failures in
  Tesseract and Ghostscript, has been removed in favor of using plugins for
  testing. The spoofing mechanism was fairly complex and required many special
  hacks for Windows.
- Code describing the resolution in DPI of images was refactored into a
  `ocrmypdf.helpers.Resolution` class.
- The module `ocrmypdf._exec` is now private to OCRmyPDF.
- The `ocrmypdf.hocrtransform` module has been updated to follow PEP8 naming
  conventions.
- Ghostscript is no longer used for finding the location of text in PDFs, and
  APIs related to this feature have been removed.
- Lots of internal reorganization to support plugins.

