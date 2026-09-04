% SPDX-FileCopyrightText: 2022 James R. Barlow
% SPDX-License-Identifier: CC-BY-SA-4.0

# Performance

Some users have noticed that current versions of OCRmyPDF do not run as
quickly as some older versions (specifically 6.x and older). This is
because OCRmyPDF added image optimization as a postprocessing step, and
it is enabled by default.

## Speed

If running OCRmyPDF quickly is your main goal, you can use settings such
as:

-   `--optimize 0` to disable file size optimization
-   `--output-type pdf` to disable PDF/A generation
-   `--fast-web-view 999999` to disable fast web view optimization
-   `--skip-big` to skip large images, if some pages have large images

You can also avoid:

-   `--force-ocr`
-   Image preprocessing

## Memory

Peak memory is set by the number of *pixels* in the largest page, not by the
size of the PDF on disk. OCRmyPDF rasterizes each page at the resolution of the
images it contains, and there is no ceiling on that by default, so a 1200 dpi
scan costs four times as much as the same page at 600 dpi.

As a rule of thumb, budget about **15 bytes per pixel of the largest page, per
concurrent job**. A 34 megapixel page (letter size at 600 dpi) therefore peaks
at roughly 500 MB with `--jobs 1`, and roughly 2 GB with `--jobs 4`. Three
things account for that:

-   the OCR engine, which is the largest single consumer -- Tesseract peaks at
    about 12 bytes per pixel of the image it is given
-   rasterizing the page, which holds the renderer's bitmap and the copy taken
    from it at the same time
-   masking and writing the image that goes to OCR

They run one after another rather than at once, so the peak is the largest of
them, not their sum -- but each concurrent job pays it separately.

To bound memory on files with very large images, use `--max-ocr-image-mpixels`.
It downsamples the image sent to OCR when a page exceeds the given size, which
caps the largest consumer:

```bash
ocrmypdf --max-ocr-image-mpixels 8 --jobs 2 input.pdf output.pdf
```

The visible page is never downsampled, so the appearance of the output is
unaffected in every mode, including `--force-ocr`. What you trade is OCR
accuracy on very high resolution scans. Tesseract is tuned for roughly 300 dpi
and gains little above 400 dpi, so a cap around 8 megapixels is usually free for
letter or A4 pages; on documents with unusually small print, check the results
before adopting it.

Two other settings matter:

-   `--jobs` multiplies the peak, since each worker processes its own page.
    Lowering it is the bluntest way to fit a memory limit.
-   `--max-image-mpixels` is *not* a memory control. It is Pillow's
    decompression-bomb guard, and it aborts the job rather than downsampling.
