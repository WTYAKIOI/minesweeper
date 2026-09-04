# SPDX-FileCopyrightText: 2026 James R. Barlow
# SPDX-License-Identifier: MPL-2.0

"""Tests for ocrmypdf.subprocess._check, the external program version checker."""

from __future__ import annotations

import logging
import sys
from subprocess import CalledProcessError

import pytest
from packaging.version import Version

from ocrmypdf.exceptions import MissingDependencyError
from ocrmypdf.subprocess._check import (
    _error_trailer,
    _get_platform,
    check_external_program,
)

CHECK_LOGGER = 'ocrmypdf.subprocess'


@pytest.fixture
def check_log(caplog):
    caplog.set_level(logging.DEBUG, logger=CHECK_LOGGER)
    return caplog


@pytest.mark.parametrize(
    ('sys_platform', 'expected'),
    [
        ('freebsd13', 'freebsd'),
        ('freebsd', 'freebsd'),
        ('linux', 'linux'),
        ('win32', 'windows'),
        ('cygwin', 'cygwin'),  # unrecognized platforms pass through unchanged
        ('darwin', 'darwin'),
    ],
)
def test_get_platform(monkeypatch, sys_platform, expected):
    monkeypatch.setattr(sys, 'platform', sys_platform)
    assert _get_platform() == expected


@pytest.mark.parametrize(
    ('sys_platform', 'expected_command'),
    [
        ('darwin', 'brew install tesseract'),
        ('linux', 'sudo apt install tesseract'),
        ('win32', 'choco install tesseract'),
    ],
)
def test_error_trailer_gives_platform_install_advice(
    monkeypatch, check_log, sys_platform, expected_command
):
    """Each supported platform gets advice for its own package manager."""
    monkeypatch.setattr(sys, 'platform', sys_platform)
    _error_trailer('tesseract', 'tesseract')
    assert expected_command in check_log.text
    # Advice for the other platforms must not be emitted
    other_commands = {'brew install', 'sudo apt install', 'choco install'} - {
        expected_command.rsplit(' ', 1)[0]
    }
    for other in other_commands:
        assert other not in check_log.text


def test_error_trailer_linux_mentions_rpm_alternative(monkeypatch, check_log):
    monkeypatch.setattr(sys, 'platform', 'linux')
    _error_trailer('gs', 'ghostscript')
    assert 'sudo apt install ghostscript' in check_log.text
    assert 'sudo dnf install ghostscript' in check_log.text


def test_error_trailer_no_advice_for_unsupported_platform(monkeypatch, check_log):
    """FreeBSD (and anything else) has no packaging advice, so say nothing."""
    monkeypatch.setattr(sys, 'platform', 'freebsd14')
    _error_trailer('gs', 'ghostscript')
    assert check_log.text.strip() == ''


@pytest.mark.parametrize(
    ('sys_platform', 'expected_package'),
    [
        ('darwin', 'tesseract'),
        ('linux', 'tesseract-ocr'),
        ('win32', 'tesseract-win'),
    ],
)
def test_error_trailer_selects_package_name_per_platform(
    monkeypatch, check_log, sys_platform, expected_package
):
    monkeypatch.setattr(sys, 'platform', sys_platform)
    _error_trailer(
        'tesseract',
        {
            'darwin': 'tesseract',
            'linux': 'tesseract-ocr',
            'windows': 'tesseract-win',
        },
    )
    assert f'install {expected_package}' in check_log.text


def test_error_trailer_falls_back_to_program_name(monkeypatch, check_log):
    """A package mapping with no entry for this platform falls back to program."""
    monkeypatch.setattr(sys, 'platform', 'linux')
    _error_trailer('unpaper', {'darwin': 'unpaper-brew'})
    assert 'sudo apt install unpaper\n' in check_log.text


def _raise_file_not_found():
    raise FileNotFoundError('gs')


def _raise_called_process_error():
    raise CalledProcessError(1, 'gs')


def _raise_missing_dependency():
    raise MissingDependencyError('gs')


ALL_FAILURES = pytest.mark.parametrize(
    'version_checker',
    [_raise_file_not_found, _raise_called_process_error, _raise_missing_dependency],
    ids=['FileNotFoundError', 'CalledProcessError', 'MissingDependencyError'],
)


@ALL_FAILURES
def test_missing_required_program_raises(check_log, version_checker):
    with pytest.raises(MissingDependencyError):
        check_external_program(
            program='gs',
            package='ghostscript',
            version_checker=version_checker,
            need_version='9.54',
        )
    assert 'could not be executed or was not found' in check_log.text
    assert 'we will proceed' not in check_log.text


@ALL_FAILURES
def test_missing_recommended_program_warns_and_returns(check_log, version_checker):
    result = check_external_program(
        program='jbig2',
        package='jbig2enc',
        version_checker=version_checker,
        need_version='0.28',
        required_for='--optimize {2,3}',
        recommended=True,
    )
    assert result is None
    assert 'but not required, so we will proceed' in check_log.text
    assert '--optimize {2,3}' in check_log.text
    assert not any(r.levelno >= logging.ERROR for r in check_log.records)


@ALL_FAILURES
def test_missing_optional_program_message_names_the_feature(check_log, version_checker):
    """A required-but-only-for-a-feature program gets the 'omit these' advice."""
    with pytest.raises(MissingDependencyError):
        check_external_program(
            program='pngquant',
            package='pngquant',
            version_checker=version_checker,
            need_version='2.12.2',
            required_for='--optimize {2,3}',
        )
    assert 'This program is required when you use' in check_log.text
    assert '--optimize {2,3}' in check_log.text


def test_old_version_required_raises(check_log):
    with pytest.raises(MissingDependencyError):
        check_external_program(
            program='gs',
            package='ghostscript',
            version_checker=lambda: Version('9.20'),
            need_version='9.54',
        )
    assert "OCRmyPDF requires 'gs' 9.54 or higher" in check_log.text
    assert 'to have 9.20' in check_log.text


def test_old_version_recommended_warns_only(check_log):
    result = check_external_program(
        program='jbig2',
        package='jbig2enc',
        version_checker=lambda: Version('0.27'),
        need_version='0.28',
        recommended=True,
    )
    assert result is None
    assert "OCRmyPDF requires 'jbig2' 0.28 or higher" in check_log.text


def test_old_version_required_for_feature_uses_softer_message(check_log):
    with pytest.raises(MissingDependencyError):
        check_external_program(
            program='pngquant',
            package='pngquant',
            version_checker=lambda: Version('2.0.1'),
            need_version='2.12.2',
            required_for='--optimize {2,3}',
        )
    assert 'If you omit these arguments' in check_log.text
    assert '--optimize {2,3}' in check_log.text


@pytest.mark.parametrize('found', ['9.54', '9.55', '10.0'])
def test_new_enough_version_is_accepted(check_log, found):
    check_external_program(
        program='gs',
        package='ghostscript',
        version_checker=lambda: Version(found),
        need_version='9.54',
    )
    assert f'Found gs {found}' in check_log.text
    assert 'or higher' not in check_log.text


def test_need_version_may_be_a_version_object(check_log):
    """A pre-parsed Version is used directly, not re-parsed."""
    with pytest.raises(MissingDependencyError):
        check_external_program(
            program='gs',
            package='ghostscript',
            version_checker=lambda: Version('9.20'),
            need_version=Version('9.54'),
        )
    assert "'gs' 9.54 or higher" in check_log.text


def test_version_parser_is_used_for_need_version(check_log):
    """A custom version_parser controls how need_version is parsed/compared."""

    class ReversedVersion(Version):
        """Compares in reverse order, so 'higher' means 'lower'."""

        def __lt__(self, other):
            return super().__gt__(other)

    check_external_program(
        program='weird',
        package='weird',
        version_checker=lambda: ReversedVersion('1.0'),
        need_version='2.0',
        version_parser=ReversedVersion,
    )
    # 1.0 < 2.0 normally, but ReversedVersion says otherwise, so no error
    assert 'Found weird 1.0' in check_log.text
    assert 'or higher' not in check_log.text
