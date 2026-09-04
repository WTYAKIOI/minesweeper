% SPDX-FileCopyrightText: 2022 James R. Barlow
% SPDX-License-Identifier: CC-BY-SA-4.0

# v11

## v11.7.3

- Exclude CCITT Group 3 images from being optimized. Some libraries
  OCRmyPDF uses do not seem to handle this obscure compression format properly.
  You may get errors or possible corrupted output images without this fix.

## v11.7.2

- Updated pinned versions in main.txt, primarily to upgrade Pillow to 8.1.2, due
  to recently disclosed security vulnerabilities in that software.
- The `--sidecar` parameter now causes an exception if set to the same file as
  the input or output PDF.

## v11.7.1

- Some exceptions while attempting image optimization were only logged at the debug
  level, causing them to be suppressed. These errors are now logged appropriately.
- Improved the error message related to `--unpaper-args`.
- Updated documentation to mention the new conda distribution.

## v11.7.0

- We now support using `--sidecar` in conjunction with `--pages`; these arguments
  used to be mutually exclusive. ({issue}`735`)
- Fixed a possible issue with PDF/A-1b generation. Acrobat complained that our PDFs use
  object streams. More robust PDF/A validators like veraPDF don't consider this a
  problem, but we'll honor Acrobat's objection from here on. This may increase file
  size of PDF/A-1b files. PDF/A-2b files will not be affected.

## v11.6.2

- Fixed a regression where the wrong page orientation would be produced when using
  arguments such as `--deskew --rotate-pages` ({issue}`730`).

## v11.6.1

- Fixed an issue with attempting optimize unusually narrow-width images by excluding
  these images from optimization ({issue}`732`).
- Remove an obsolete compatibility shim for a version of pikepdf that is no longer
  supported.

## v11.6.0

- OCRmyPDF will now automatically register plugins from the same virtual environment
  with an appropriate setuptools entrypoint.
- Refactor the plugin manager to remove unnecessary complications and make plugin
  registration more automatic.
- `PageContext` and `PdfContext` are now formally part of the API, as they
  should have been, since they were part of `ocrmypdf.pluginspec`.

## v11.5.0

- Fixed an issue where the output page size might differ by a fractional amount
  due to rounding, when `--force-ocr` was used and the page contained objects
  with multiple resolutions.
- When determining the resolution at which to rasterize a page, we now consider
  printed text on the page as requiring a higher resolution. This fixes issues
  with certain pages being rendered with unacceptably low resolution text, but
  may increase output file sizes in some workflows where low resolution text
  is acceptable.
- Added a workaround to fix an exception that occurs when trying to
  `import ocrmypdf.leptonica` on Apple ARM silicon (or potentially, other
  platforms that do not permit write+executable memory).

## v11.4.5

- Fixed an issue where files may not be closed when the API is used.
- Improved `setup.cfg` with better settings for test coverage.

## v11.4.4

- Fixed `AttributeError: 'NoneType' object has no attribute 'userunit'` ({issue}`700`),
  related to OCRmyPDF not properly forwarded an error message from pdfminer.six.
- Adjusted typing of some arguments.
- `ocrmypdf.ocr` now takes a `threading.Lock` for reasons outlined in the
  documentation.

## v11.4.3

- Removed a redundant debug message.
- Test suite now asserts that most patched functions are called when they should be.
- Test suite now skips a test that fails on two particular versions of piekpdf.

## v11.4.2

- Fixed support for Cygwin, hopefully.
- watcher.py: Fixed an issue with the OCR_LOGLEVEL not being interpreted.

## v11.4.1

- Fixed an issue where invalid pages ranges passed using the `pages` argument,
  such as "1-0" would cause unhandled exceptions.
- Accepted a user-contributed to the Synology demo script in misc/synology.py.
- Clarified documentation about change of temporary file location `ocrmypdf.io`.
- Fixed Python wheel tag which was incorrectly set to py35 even though we long
  since dropped support for Python 3.5.

## v11.4.0

- When looking for Tesseract and Ghostscript, we now check the Windows Registry to
  see if their installers registered the location of their executables. This should
  help Windows users who have installed these programs to non-standard
  locations.
- We now report on the progress of PDF/A conversion, since this operation is
  sometimes slow.
- Improved command line completions.
- The prefix of the temporary folder OCRmyPDF creates has been changed from
  `com.github.ocrmypdf` to `ocrmypdf.io`. Scripts that chose to depend on this
  prefix may need to be adjusted. (This has always been an implementation detail so is
  not considered part of the semantic versioning "contract".)
- Fixed {issue}`692`, where a particular file with malformed fonts would flood an
  internal message cue by generating so many debug messages.
- Fixed an exception on processing hOCR files with no page record. Tesseract
  is not known to generate such files.

## v11.3.4

- Fixed an error message 'called readLinearizationData for file that is not
  linearized' that may occur when pikepdf 2.1.0 is used. (Upgrading to pikepdf
  2.1.1 also fixes the issue.)
- File watcher now automatically includes `.PDF` in addition to `.pdf` to
  better support case sensitive file systems.
- Some documentation and comment improvements.

## v11.3.3

- If unpaper outputs non-UTF-8 data, quietly fix this rather than choke on the
  conversion. (Possibly addresses {issue}`671`.)

## v11.3.2

- Explicitly require pikepdf 2.0.0 or newer when running on Python 3.9. (There are
  concerns about the stability of pybind11 2.5.x with Python 3.9, which is used in
  pikepdf 1.x.)
- Fixed another issue related to page rotation.
- Fixed an issue where image marked as image masks were not properly considered
  as optimization candidates.
- On some systems, unpaper seems to be unable to process the PNGs we offer it
  as input. We now convert the input to PNM format, which unpaper always accepts.
  Fixes {issue}`665` and {issue}`667`.
- DPI sent to unpaper is now rounded to a more reasonable number of decimal digits.
- Debug and error messages from unpaper were being suppressed.
- Some documentation tweaks.

## v11.3.1

- Declare support for new versions: pdfminer.six 20201018 and pikepdf 2.x
- Fixed warning related to `--pdfa-image-compression` that appears at the wrong
  time.

## v11.3.0

- The "OCR" step is describing as "Image processing" in the output messages when
  OCR is disabled, to better explain the application's behavior.
- Debug logs are now only created when run as a command line, and not when OCR
  is performed for an API call. It is the calling application's responsibility
  to set up logging.
- For PDFs with a low number of pages, we gathered information about the input PDF
  in a thread rather than process (when there are more pages). When run as a
  thread, we did not close the file handle to the working PDF, leaking one file
  handle per call of `ocrmypdf.ocr`.
- Fixed an issue where debug messages send by child worker processes did not match
  the log settings of parent process, causing messages to be dropped. This affected
  macOS and Windows only where the parent process is not forked.
- Fixed the hookspec of rasterize_pdf_page to remove default parameters that
  were not handled in an expected way by pluggy.
- Fixed another issue with automatic page rotation ({issue}`658`) due to the issue above.

## v11.2.1

- Fixed an issue where optimization of a 1-bit image with a color palette or
  associated ICC that was optimized to JBIG2 could have its colors inverted.

## v11.2.0

- Fixed an issue with optimizing PNG-type images that had soft masks or image masks.
  This is a regression introduced in (or about) v11.1.0.
- Improved type checking of the `plugins` parameter for the `ocrmypdf.ocr`
  API call.

## v11.1.2

- Fixed hOCR renderer writing the text in roughly reverse order. This should not
  affect reasonably smart PDF readers that properly locate the position of all
  text, but may confuse those that rely on the order of objects in the content
  stream. ({issue}`642`)

## v11.1.1

- We now avoid using named temporary files when using pngquant allowing containerized
  pngquant installs to be used.
- Clarified an error message.
- Highest number of 1's in a release ever!

## v11.1.0

- Fixed page rotation issues: {issue}`634,589`.
- Fixed some cases where optimization created an invalid image such as a
  1-bit "RGB" image: {issue}`629,620`.
- Page numbers are now displayed in debug logs when pages are being grafted.
- ocrmypdf.optimize.rewrite_png and ocrmypdf.optimize.rewrite_png_as_g4 were
  marked deprecated. Strictly speaking these should have been internal APIs,
  but they were never hidden.
- As a precaution, pikepdf mmap-based file access has been disabled due to a
  rare race condition that causes a crash when certain objects are deallocated.
  The problem is likely in pikepdf's dependency pybind11.
- Extended the example plugin to demonstrate conversion to mono.

## v11.0.2

- Fixed {issue}`612`, TypeError exception. Fixed by eliminating unnecessary repair of
  input PDF metadata in memory.

## v11.0.1

- Blacklist pdfminer.six 20200720, which has a regression fixed in 20200726.
- Approve img2pdf 0.4 as it passes tests.
- Clarify that the GPL-3 portion of pdfa.py was removed with the changes in v11.0.0;
  the debian/copyright file did not properly annotate this change.

## v11.0.0

- Project license changed to Mozilla Public License 2.0. Some miscellaneous
  code is now under MIT license and non-code content/media remains under
  CC-BY-SA 4.0. License changed with approval of all people who were found
  to have contributed to GPLv3 licensed sections of the project. ({issue}`600`)
- Because the license changed, this is being treated as a major version number
  change; however, there are no known breaking changes in functional behavior
  or API compared to v10.x.

