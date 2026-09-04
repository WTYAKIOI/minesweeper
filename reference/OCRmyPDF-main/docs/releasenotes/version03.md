% SPDX-FileCopyrightText: 2022 James R. Barlow
% SPDX-License-Identifier: CC-BY-SA-4.0

# v3

## v3.2.1

Changes

- Fixed {issue}`47`
  "convert() got and unexpected keyword argument 'dpi'" by upgrading to
  img2pdf 0.2
- Tweaked the Dockerfiles

## v3.2

New features

- Lossless reconstruction: when possible, OCRmyPDF will inject text
  layers without otherwise manipulating the content and layout of a PDF
  page. For example, a PDF containing a mix of vector and raster
  content would see the vector content preserved. Images may still be
  transcoded during PDF/A conversion. (`--deskew` and
  `--clean-final` disable this mode, necessarily.)
- New argument `--tesseract-pagesegmode` allows you to pass page
  segmentation arguments to Tesseract OCR. This helps for two column
  text and other situations that confuse Tesseract.
- Added a new "polyglot" version of the Docker image, that generates
  Tesseract with all languages packs installed, for the polyglots among
  us. It is much larger.

Changes

- JPEG transcoding quality is now 95 instead of the default 75. Bigger
  file sizes for less degradation.

## v3.1.1

Changes

- Fixed bug that caused incorrect page size and DPI calculations on
  documents with mixed page sizes

## v3.1

Changes

- Default output format is now PDF/A-2b instead of PDF/A-1b
- Python 3.5 and macOS El Capitan are now supported platforms - no
  changes were needed to implement support
- Improved some error messages related to missing input files
- Fixed {issue}`20`: uppercase .PDF extension not accepted
- Fixed an issue where OCRmyPDF failed to text that certain pages
  contained previously OCR'ed text, such as OCR text produced by
  Tesseract 3.04
- Inserts /Creator tag into PDFs so that errors can be traced back to
  this project
- Added new option `--pdf-renderer=auto`, to let OCRmyPDF pick the
  best PDF renderer. Currently it always chooses the 'hocrtransform'
  renderer but that behavior may change.
- Set up Travis CI automatic integration testing

## v3.0

New features

- Easier installation with a Docker container or Python's `pip`
  package manager
- Eliminated many external dependencies, so it's easier to setup
- Now installs `ocrmypdf` to `/usr/local/bin` or equivalent for
  system-wide access and easier typing
- Improved command line syntax and usage help (`--help`)
- Tesseract 3.03+ PDF page rendering can be used instead for better
  positioning of recognized text (`--pdf-renderer tesseract`)
- PDF metadata (title, author, keywords) are now transferred to the
  output PDF
- PDF metadata can also be set from the command line (`--title`,
  etc.)
- Automatic repairs malformed input PDFs if possible
- Added test cases to confirm everything is working
- Added option to skip extremely large pages that take too long to OCR
  and are often not OCRable (e.g. large scanned maps or diagrams);
  other pages are still processed (`--skip-big`)
- Added option to kill Tesseract OCR process if it seems to be taking
  too long on a page, while still processing other pages
  (`--tesseract-timeout`)
- Less common colorspaces (CMYK, palette) are now supported by
  conversion to RGB
- Multiple images on the same PDF page are now supported

Changes

- New, robust rewrite in Python 3.4+ with
  [ruffus](http://www.ruffus.org.uk/index.html) pipelines

- Now uses Ghostscript 9.14's improved color conversion model to
  preserve PDF colors

- OCR text is now rendered in the PDF as invisible text. Previous
  versions of OCRmyPDF incorrectly rendered visible text with an image
  on top.

- All "tasks" in the pipeline can be executed in parallel on any
  available CPUs, increasing performance

- The `-o DPI` argument has been phased out, in favor of
  `--oversample DPI`, in case we need `-o OUTPUTFILE` in the future

- Removed several dependencies, so it's easier to install. We no longer
  use:

  - GNU [parallel](https://www.gnu.org/software/parallel/)
  - [ImageMagick](http://www.imagemagick.org/script/index.php)
  - Python 2.7
  - Poppler
  - [MuPDF](http://mupdf.com/docs/) tools
  - shell scripts
  - Java and [JHOVE](http://jhove.sourceforge.net/)
  - libxml2

- Some new external dependencies are required or optional, compared to
  v2.x:

  - Ghostscript 9.14+
  - [qpdf](http://qpdf.sourceforge.net/) 5.0.0+
  - [Unpaper](https://github.com/Flameeyes/unpaper) 6.1 (optional)
  - some automatically managed Python packages

Release candidates^

- rc9:

  - Fix
    {issue}`118`:
    report error if ghostscript iccprofiles are missing
  - fixed another issue related to
    {issue}`111`: PDF
    rasterized to palette file
  - add support image files with a palette
  - don't try to validate PDF file after an exception occurs

- rc8:

  - Fix
    {issue}`111`:
    exception thrown if PDF is missing DocumentInfo dictionary

- rc7:

  - fix error when installing direct from pip, "no such file
    'requirements.txt'"

- rc6:

  - dropped libxml2 (Python lxml) since Python 3's internal XML parser
    is sufficient
  - set up Docker container
  - fix Unicode errors if recognized text contains Unicode characters
    and system locale is not UTF-8

- rc5:

  - dropped Java and JHOVE in favour of qpdf
  - improved command line error output
  - additional tests and bug fixes
  - tested on Ubuntu 14.04 LTS

- rc4:

  - dropped MuPDF in favour of qpdf
  - fixed some installer issues and errors in installation
    instructions
  - improve performance: run Ghostscript with multithreaded rendering
  - improve performance: use multiple cores by default
  - bug fix: checking for wrong exception on process timeout

- rc3: skipping version number intentionally to avoid confusion with
  Tesseract

- rc2: first release for public testing to test-PyPI, Github

- rc1: testing release process

## Compatibility notes

- `./OCRmyPDF.sh` script is still available for now
- Stacking the verbosity option like `-vvv` is no longer supported
- The configuration file `config.sh` has been removed. Instead, you
  can feed a file to the arguments for common settings:

```
ocrmypdf input.pdf output.pdf @settings.txt
```

where `settings.txt` contains *one argument per line*, for example:

```
-l
deu
--author
A. Merkel
--pdf-renderer
tesseract
```

Fixes

- Handling of filenames containing spaces: fixed

Notes and known issues

- Some dependencies may work with lower versions than tested, so try
  overriding dependencies if they are "in the way" to see if they work.
- `--pdf-renderer tesseract` will output files with an incorrect page
  size in Tesseract 3.03, due to a bug in Tesseract.
- PDF files containing "inline images" are not supported and won't be
  for the 3.0 release. Scanned images almost never contain inline
  images.

