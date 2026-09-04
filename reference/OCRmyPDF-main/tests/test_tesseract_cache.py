# SPDX-FileCopyrightText: 2026 James R. Barlow
# SPDX-License-Identifier: MPL-2.0

"""Tests for the Tesseract test cache and its invalidation mechanism.

These tests never invoke Tesseract. They check that the cache keys are built
the way we think they are, that uncacheable inputs bypass the cache, that
cache writes are atomic, and that the cache tree committed to the repository
is internally consistent with tests/resources/.
"""

from __future__ import annotations

import hashlib
import json
import os
from io import BytesIO
from pathlib import Path
from types import SimpleNamespace

import pytest

from .plugins import tesseract_cache as tc

REGENERATE_HINT = (
    "See the \"Tesseract OCR cache\" section of docs/contributing.md for the "
    "cache regeneration procedure."
)


@pytest.fixture
def parsed_args():
    def parse(run_args):
        return tc.parser.parse_args(run_args[1:])

    return parse


def test_digest_is_short_sha256(tmp_path):
    f = tmp_path / 'sample.pdf'
    f.write_bytes(b'hello world')
    expected = hashlib.sha256(b'hello world').hexdigest()[: tc.DIGEST_LENGTH]
    assert tc.file_digest(str(f)) == expected
    assert tc.DIGEST_SUFFIX.search(f'name-{expected}')


def test_digest_is_cached(tmp_path):
    f = tmp_path / 'sample.pdf'
    f.write_bytes(b'first')
    first = tc.file_digest(str(f))
    f.write_bytes(b'second')
    assert tc.file_digest(str(f)) == first, "digest should be memoized per session"
    tc.file_digest.cache_clear()
    assert tc.file_digest(str(f)) != first


def test_resource_input_is_digest_keyed(resources):
    trivial = resources / 'trivial.pdf'
    folder = tc.get_cache_key_folder(trivial)
    assert folder.parent == tc.CACHE_ROOT
    assert folder.name == f'trivial-{tc.file_digest(str(trivial))}'
    assert tc.DIGEST_SUFFIX.search(folder.name)


def test_generated_input_is_legacy_keyed(tmp_path, resources):
    generated = tmp_path / 'trivial.pdf'
    generated.write_bytes((resources / 'trivial.pdf').read_bytes())
    folder = tc.get_cache_key_folder(generated)
    assert folder == tc.CACHE_ROOT / 'trivial'
    assert not tc.DIGEST_SUFFIX.search(folder.name)


def test_argv_slug(parsed_args):
    run_args = [
        'tesseract',
        '-l',
        'eng',
        '--psm',
        '3',
        '/tmp/ocrmypdf.io.abc/000001_ocr.png',
        '/tmp/ocrmypdf.io.abc/000001_ocr_hocr',
        'hocr',
        'txt',
    ]
    slug = tc.get_argv_slug(run_args, parsed_args(run_args))
    assert slug == '__-l__eng__--psm__3__000001_ocr.png__000001_ocr_hocr__hocr__txt'
    assert '/' not in slug


def test_argv_slug_drops_textonly_config(parsed_args):
    run_args = [
        'tesseract',
        '-l',
        'eng',
        '-c',
        'textonly_pdf=1',
        'image.png',
        'outputbase',
        'pdf',
    ]
    slug = tc.get_argv_slug(run_args, parsed_args(run_args))
    assert 'textonly' not in slug
    assert slug == '__-l__eng__image.png__outputbase__pdf'


def test_cache_folder_is_key_folder_plus_slug(resources, parsed_args):
    run_args = ['tesseract', '-l', 'eng', 'image.png', 'outputbase', 'hocr']
    args = parsed_args(run_args)
    folder = tc.get_cache_folder(resources / 'trivial.pdf', run_args, args)
    assert folder.parent == tc.get_cache_key_folder(resources / 'trivial.pdf')
    assert folder.name == tc.get_argv_slug(run_args, args)


@pytest.mark.parametrize(
    'input_file',
    [
        None,
        BytesIO(b'%PDF-1.7'),
        42,
        '/dev/null',
        '/nonexistent/path/to/input.pdf',
    ],
    ids=['none', 'stream', 'int', 'devnull', 'missing'],
)
def test_uncacheable_inputs_rejected(input_file):
    assert tc.cacheable_input(input_file) is None


def test_cacheable_input_accepts_path_and_str(resources):
    trivial = resources / 'trivial.pdf'
    assert tc.cacheable_input(trivial) == trivial
    assert tc.cacheable_input(str(trivial)) == trivial


def test_directory_input_is_uncacheable(tmp_path):
    assert tc.cacheable_input(tmp_path) is None


def test_stream_input_bypasses_cache(monkeypatch):
    """A stream input must call through to tesseract and touch no cache."""
    calls = []

    def fake_run(run_args, **kwargs):
        calls.append((run_args, kwargs))
        return 'passed through'

    monkeypatch.setattr(tc, 'run', fake_run)
    before = set(tc.CACHE_ROOT.iterdir()) if tc.CACHE_ROOT.exists() else set()

    options = SimpleNamespace(input_file=BytesIO(b'%PDF-1.7'))
    run_args = ['tesseract', '-l', 'eng', 'image.png', 'outputbase', 'hocr']
    result = tc.cached_run(options, run_args, timeout=180)

    assert result == 'passed through'
    assert len(calls) == 1
    after = set(tc.CACHE_ROOT.iterdir()) if tc.CACHE_ROOT.exists() else set()
    assert before == after, "cache bypass must not create cache directories"


def test_stdin_input_bypasses_cache(monkeypatch):
    calls = []
    monkeypatch.setattr(tc, 'run', lambda run_args, **kw: calls.append(run_args))
    options = SimpleNamespace(input_file=None)
    tc.cached_run(options, ['tesseract', '-l', 'eng', 'stdin', 'stdout'])
    assert len(calls) == 1


def test_atomic_write_leaves_no_temporary_files(tmp_path):
    dest = tmp_path / 'stdout.bin'
    tc.atomic_write_bytes(dest, b'result')
    assert dest.read_bytes() == b'result'
    assert list(tmp_path.glob('*.tmp.*')) == []


def test_atomic_write_overwrites_leftover_temporary(tmp_path):
    """A temporary file abandoned by a crashed run must not corrupt a write."""
    dest = tmp_path / 'stdout.bin'
    leftover = tmp_path / f'stdout.bin.tmp.{os.getpid()}'
    leftover.write_bytes(b'garbage from a crashed writer')

    tc.atomic_write_bytes(dest, b'result')

    assert dest.read_bytes() == b'result'
    assert list(tmp_path.glob('*.tmp.*')) == []


def test_atomic_write_is_all_or_nothing(tmp_path, monkeypatch):
    dest = tmp_path / 'stdout.bin'

    def failing_replace(self, target):
        raise OSError("simulated failure")

    monkeypatch.setattr(Path, 'replace', failing_replace)
    with pytest.raises(OSError):
        tc.atomic_write_bytes(dest, b'result')

    assert not dest.exists(), "readers must never see a partial file"
    assert list(tmp_path.glob('*.tmp.*')) == [], "temporary file must be cleaned up"


def test_atomic_copy(tmp_path):
    src = tmp_path / 'src.txt'
    src.write_bytes(b'copied')
    dest = tmp_path / 'dest.bin'
    tc.atomic_copy(src, dest)
    assert dest.read_bytes() == b'copied'
    assert list(tmp_path.glob('*.tmp.*')) == []


def test_manifest_append_only_adds_lines(tmp_path, monkeypatch):
    manifest = tmp_path / 'manifest.jsonl'
    manifest.write_text('{"existing": true}\n')
    monkeypatch.setattr(tc, 'MANIFEST_FILE', manifest)

    tc.append_manifest({'entry': 1})
    tc.append_manifest({'entry': 2})

    lines = manifest.read_text().splitlines()
    assert len(lines) == 3
    assert json.loads(lines[0]) == {'existing': True}
    assert json.loads(lines[2]) == {'entry': 2}


def test_update_cache_info(tmp_path, monkeypatch):
    info_file = tmp_path / 'CACHE_INFO.json'
    monkeypatch.setattr(tc, 'CACHE_INFO_FILE', info_file)
    monkeypatch.setattr(tc.TesseractOcrEngine, 'version', staticmethod(lambda: '5.5.1'))

    tc.update_cache_info()

    info = json.loads(info_file.read_text())
    assert info['cache_version'] == tc.CACHE_VERSION
    assert info['tesseract_version'] == '5.5.1'
    assert info['system'] and info['generated']


def test_cache_info_committed():
    info_file = tc.CACHE_INFO_FILE
    assert info_file.exists(), f"{info_file} is missing. {REGENERATE_HINT}"
    info = json.loads(info_file.read_text())
    assert info['cache_version'] == tc.CACHE_VERSION, (
        f"{info_file} records cache_version {info['cache_version']} but the "
        f"plugin is at version {tc.CACHE_VERSION}. {REGENERATE_HINT}"
    )
    assert info['tesseract_version']


def test_cache_entries_match_resources(resources):
    """Every digest-keyed cache folder must match a current test resource."""
    if not tc.CACHE_ROOT.exists():
        pytest.skip("no cache tree")

    checked = 0
    for entry in sorted(tc.CACHE_ROOT.iterdir()):
        if not entry.is_dir():
            continue
        match = tc.DIGEST_SUFFIX.search(entry.name)
        if not match:
            continue  # Legacy-keyed entry, from an input generated at test time
        stem, digest = entry.name[: match.start()], match.group()[1:]
        resource = resources / f'{stem}.pdf'
        assert resource.exists(), (
            f"Cache entry {entry.name} refers to {resource.name}, which no "
            f"longer exists. Delete the stale cache folder. {REGENERATE_HINT}"
        )
        assert tc.file_digest(str(resource)) == digest, (
            f"{resource.name} has changed since cache entry {entry.name} was "
            f"generated (digest is now {tc.file_digest(str(resource))}). Delete "
            f"the stale cache folder and let it regenerate. {REGENERATE_HINT}"
        )
        checked += 1

    assert checked > 0, "expected at least one digest-keyed cache entry"
