# SPDX-FileCopyrightText: 2022 James R. Barlow
# SPDX-License-Identifier: MIT
"""Cache output of tesseract to speed up test suite.

Cache layout::

    tests/cache/v{CACHE_VERSION}/
        CACHE_INFO.json
        manifest.jsonl
        <stem>-<sha256[:8]>/<argv-slug>/{stdout,stderr,hocr,pdf,txt}.bin
        <stem>/<argv-slug>/{stdout,stderr,hocr,pdf,txt}.bin

The cache is keyed by the input test file and the arguments passed to
tesseract. The arguments are slugged into a hideous filename that more or
less represents them literally.

There are three layers of invalidation:

- ``CACHE_VERSION`` is the epoch. Bump it when OCRmyPDF changes the images
  it hands to tesseract (preprocessing, rasterization), since the old
  answers no longer correspond to the new questions. The old tree is then
  deleted wholesale.
- Input files that live in ``tests/resources/`` are keyed by the first 8
  hex digits of the SHA-256 of their contents, so editing a test resource
  invalidates only the entries derived from it. Inputs generated at test
  time (temporary files whose bytes may differ from run to run) keep
  legacy stem-only keying, since digest keying them would miss every time.
  A digest-keyed directory name always ends with ``-`` plus exactly 8
  lowercase hex digits; take care not to add a test resource whose stem
  ends that way.
- ``CACHE_INFO.json`` records the tesseract version that generated the
  tree. ``tests/conftest.py`` compares it against the locally installed
  tesseract and warns on drift (or exits, with
  ``OCRMYPDF_TEST_CACHE_STRICT=1``).

By design, an input image that varies according to platform differences
(e.g. JPEG decoders are allowed to produce differing outputs, and in
practice they do) will still be a cache hit. By design, an invocation of
tesseract with the same parameters from a different test case will be a
hit.

``manifest.jsonl`` is a JSON lines file describing the system that produced
each entry. It is appended to only when an entry is created, so a fully
cached test run leaves the working tree clean.

Certain operations are not cached and routed to Tesseract OCR directly.

Assumes Tesseract 4+.

"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import logging
import os
import platform
import re
import shutil
import threading
from functools import cache, partial
from pathlib import Path
from subprocess import PIPE, CalledProcessError, CompletedProcess
from unittest.mock import patch

from ocrmypdf import hookimpl
from ocrmypdf.builtin_plugins.tesseract_ocr import TesseractOcrEngine
from ocrmypdf.subprocess import run

log = logging.getLogger(__name__)

CACHE_VERSION = 1

TESTS_ROOT = Path(__file__).resolve().parent.parent
RESOURCES_ROOT = TESTS_ROOT / 'resources'
CACHE_ROOT = TESTS_ROOT / 'cache' / f'v{CACHE_VERSION}'
CACHE_INFO_FILE = CACHE_ROOT / 'CACHE_INFO.json'
MANIFEST_FILE = CACHE_ROOT / 'manifest.jsonl'

DIGEST_LENGTH = 8
DIGEST_SUFFIX = re.compile(r'-[0-9a-f]{8}$')


parser = argparse.ArgumentParser(
    prog='tesseract-cache', description='cache output of tesseract'
)
parser.add_argument('-l', '--language', action='append')
parser.add_argument('imagename')
parser.add_argument('outputbase')
parser.add_argument('configfiles', nargs='*')
parser.add_argument('--user-words', type=str)
parser.add_argument('--user-patterns', type=str)
parser.add_argument('-c', action='append')
parser.add_argument('--psm', type=int)
parser.add_argument('--oem', type=int)


@cache
def file_digest(path: str) -> str:
    """Return a short SHA-256 digest of a file's contents."""
    with Path(path).open('rb') as f:
        return hashlib.file_digest(f, 'sha256').hexdigest()[:DIGEST_LENGTH]


def cacheable_input(input_file) -> Path | None:
    """Return the path of a cacheable input file, or None.

    Streams (``ocrmypdf.ocr(BytesIO(...))``) and placeholders such as
    ``/dev/null`` cannot be content-addressed, so they bypass the cache.
    """
    if not isinstance(input_file, (str, os.PathLike)):
        return None
    try:
        path = Path(os.fspath(input_file))
        if not path.is_file():
            return None
    except (OSError, TypeError, ValueError):
        return None
    return path


def is_test_resource(path: Path) -> bool:
    """Return True if path is a file checked into tests/resources/."""
    try:
        return path.resolve().parent == RESOURCES_ROOT
    except OSError:
        return False


def get_cache_key_folder(source_pdf: Path) -> Path:
    """Return the per-input folder that holds all entries for source_pdf."""
    if is_test_resource(source_pdf):
        return CACHE_ROOT / f'{source_pdf.stem}-{file_digest(str(source_pdf))}'
    return CACHE_ROOT / source_pdf.stem


def get_argv_slug(run_args, parsed_args) -> str:
    def slugs():
        yield ''  # so we don't start with a '-' which makes rm difficult
        for arg in run_args[1:]:
            if arg == parsed_args.imagename:
                yield Path(parsed_args.imagename).name
            elif arg == parsed_args.outputbase:
                yield Path(parsed_args.outputbase).name
            elif arg == '-c' or arg.startswith('textonly'):
                pass
            else:
                yield arg

    return '__'.join(slugs()).replace('/', '___')


def get_cache_folder(source_pdf: Path, run_args, parsed_args) -> Path:
    return get_cache_key_folder(source_pdf) / get_argv_slug(run_args, parsed_args)


def atomic_write_bytes(dest: Path, data: bytes) -> None:
    """Write data to dest, so that concurrent readers see all or nothing."""
    tmp = dest.with_name(f'{dest.name}.tmp.{os.getpid()}')
    try:
        tmp.write_bytes(data)
        tmp.replace(dest)
    finally:
        tmp.unlink(missing_ok=True)


def atomic_copy(src, dest: Path) -> None:
    """Copy src to dest, so that concurrent readers see all or nothing."""
    tmp = dest.with_name(f'{dest.name}.tmp.{os.getpid()}')
    try:
        shutil.copyfile(src, tmp)
        tmp.replace(dest)
    finally:
        tmp.unlink(missing_ok=True)


def append_manifest(manifest: dict) -> None:
    """Append one line to the manifest; O_APPEND keeps writers from interleaving."""
    line = (json.dumps(manifest) + '\n').encode('utf-8')
    fd = os.open(MANIFEST_FILE, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o644)
    try:
        os.write(fd, line)
    finally:
        os.close(fd)


def update_cache_info() -> None:
    """Record the tesseract version that generated the current cache tree."""
    info = {
        'cache_version': CACHE_VERSION,
        'tesseract_version': TesseractOcrEngine.version().replace('\n', ' '),
        'system': platform.system(),
        'generated': dt.datetime.now(dt.UTC).isoformat(timespec='seconds'),
    }
    atomic_write_bytes(
        CACHE_INFO_FILE, (json.dumps(info, indent=2) + '\n').encode('utf-8')
    )


def cached_run(options, run_args, **run_kwargs):
    run_args = [str(arg) for arg in run_args]  # flatten PosixPaths
    args = parser.parse_args(run_args[1:])

    if args.imagename in ('stdin', '-'):
        return run(run_args, **run_kwargs)

    source_file = cacheable_input(options.input_file)
    if source_file is None:
        log.debug("Tesseract cache bypassed: input is not a file on disk")
        return run(run_args, **run_kwargs)

    cache_folder = get_cache_folder(source_file, run_args, args)

    log.debug(f"Using Tesseract cache {cache_folder}")

    # Determine what configfiles we need
    configfiles = args.configfiles if args.configfiles else ['txt']

    # Check if cache has all required files
    def cache_complete():
        if not (cache_folder / 'stderr.bin').exists():
            return False
        if not (cache_folder / 'stdout.bin').exists():
            return False
        if args.outputbase != 'stdout':
            for configfile in configfiles:
                if not (cache_folder / f'{configfile}.bin').exists():
                    return False
        return True

    if cache_complete():
        log.debug("Cache HIT")

        # Replicate stdout/err
        if args.outputbase != 'stdout':
            for configfile in configfiles:
                # cp cache -> output
                tessfile = args.outputbase + '.' + configfile
                shutil.copy(str(cache_folder / configfile) + '.bin', tessfile)
        return CompletedProcess(
            args=run_args,
            returncode=0,
            stdout=(cache_folder / 'stdout.bin').read_bytes(),
            stderr=(cache_folder / 'stderr.bin').read_bytes(),
        )

    log.debug("Cache MISS")

    cache_kwargs = {
        k: v for k, v in run_kwargs.items() if k not in ('stdout', 'stderr')
    }
    # Don't pass timeout=0 to the actual run call - it would timeout immediately
    # A timeout of 0 means "use default/no timeout" in the caching context
    if cache_kwargs.get('timeout') == 0.0:
        cache_kwargs['timeout'] = None
    if 'check' not in cache_kwargs:
        cache_kwargs['check'] = True
    try:
        p = run(run_args, stdout=PIPE, stderr=PIPE, **cache_kwargs)
    except CalledProcessError as e:
        log.exception(e)
        raise  # Pass exception onward

    # Update cache
    cache_folder.mkdir(parents=True, exist_ok=True)
    atomic_write_bytes(cache_folder / 'stdout.bin', p.stdout)
    atomic_write_bytes(cache_folder / 'stderr.bin', p.stderr)

    if args.outputbase != 'stdout':
        for configfile in configfiles:
            if configfile not in ('fpdf2', 'hocr', 'pdf', 'txt'):
                continue
            # cp pwd/{outputbase}.{configfile} -> {cache}/{configfile}
            tessfile = args.outputbase + '.' + configfile
            atomic_copy(tessfile, cache_folder / f'{configfile}.bin')

    def clean_sys_argv():
        for arg in run_args[1:]:
            yield re.sub(r'.*/ocrmypdf[.]io[.][^/]+[/](.*)', r'$TMPDIR/\1', arg)

    try:
        sourcefile = str(source_file.relative_to(TESTS_ROOT))
    except ValueError:
        sourcefile = source_file.name

    manifest = {
        'tesseract_version': TesseractOcrEngine.version().replace('\n', ' '),
        'system': platform.system(),
        'python': platform.python_version(),
        'argv_slug': cache_folder.name,
        'cache_key': cache_folder.parent.name,
        'sourcefile': sourcefile,
        'args': list(clean_sys_argv()),
    }
    append_manifest(manifest)
    update_cache_info()
    return p


class CacheOcrEngine(TesseractOcrEngine):
    # Concurrent threads (with --use-threads) might try to use different parts
    # of the OcrEngine, so we need a lock to protect the state of patched
    # module whenever it's patched. Should refactor ocrmypdf._exec.tesseract so that
    # it does not to be patched at all for testing.
    lock = threading.Lock()

    @staticmethod
    def get_orientation(input_file, options):
        with (
            CacheOcrEngine.lock,
            patch('ocrmypdf._exec.tesseract.run', new=partial(cached_run, options)),
        ):
            return TesseractOcrEngine.get_orientation(input_file, options)

    @staticmethod
    def get_deskew(input_file, options) -> float:
        with (
            CacheOcrEngine.lock,
            patch('ocrmypdf._exec.tesseract.run', new=partial(cached_run, options)),
        ):
            return TesseractOcrEngine.get_deskew(input_file, options)

    @staticmethod
    def generate_hocr(input_file, output_hocr, output_text, options):
        with (
            CacheOcrEngine.lock,
            patch('ocrmypdf._exec.tesseract.run', new=partial(cached_run, options)),
        ):
            TesseractOcrEngine.generate_hocr(
                input_file, output_hocr, output_text, options
            )

    @staticmethod
    def generate_pdf(input_file, output_pdf, output_text, options):
        with (
            CacheOcrEngine.lock,
            patch('ocrmypdf._exec.tesseract.run', new=partial(cached_run, options)),
        ):
            TesseractOcrEngine.generate_pdf(
                input_file, output_pdf, output_text, options
            )


@hookimpl
def get_ocr_engine():
    return CacheOcrEngine()
