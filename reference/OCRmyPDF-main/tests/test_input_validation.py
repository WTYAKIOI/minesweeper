# SPDX-FileCopyrightText: 2022 James R. Barlow
# SPDX-License-Identifier: MPL-2.0

"""Tests for how OCRmyPDF rejects or accepts the input file it is given."""

from __future__ import annotations

import os
import shutil
import sys

import pikepdf
import pytest

from ocrmypdf.exceptions import ExitCode
from ocrmypdf.helpers import running_in_docker

from .conftest import check_ocrmypdf, run_ocrmypdf_api


def test_invalid_input_pdf(resources, no_outpdf):
    result = run_ocrmypdf_api(resources / 'invalid.pdf', no_outpdf)
    assert result == ExitCode.input_file


def test_blank_input_pdf(resources, outpdf):
    result = run_ocrmypdf_api(resources / 'blank.pdf', outpdf)
    assert result == ExitCode.ok


def test_input_file_not_found(caplog, no_outpdf):
    input_file = "does not exist.pdf"
    result = run_ocrmypdf_api(input_file, no_outpdf)
    assert result == ExitCode.input_file
    assert input_file in caplog.text


@pytest.mark.skipif(os.name == 'nt' or running_in_docker(), reason="chmod")
def test_input_file_not_readable(caplog, resources, outdir, no_outpdf):
    input_file = outdir / 'trivial.pdf'
    shutil.copy(resources / 'trivial.pdf', input_file)
    input_file.chmod(0o000)
    result = run_ocrmypdf_api(input_file, no_outpdf)
    assert result == ExitCode.input_file
    assert str(input_file) in caplog.text


def test_input_file_not_a_pdf(caplog, no_outpdf):
    input_file = __file__  # Try to OCR this file
    result = run_ocrmypdf_api(input_file, no_outpdf)
    assert result == ExitCode.input_file
    if os.name != 'nt':  # name will be mangled with \\'s on nt
        assert input_file in caplog.text


def test_uppercase_extension(resources, outdir):
    shutil.copy(str(resources / "skew.pdf"), str(outdir / "UPPERCASE.PDF"))

    check_ocrmypdf(
        outdir / "UPPERCASE.PDF",
        outdir / "UPPERCASE_OUT.PDF",
        '--plugin',
        'tests/plugins/tesseract_noop.py',
    )


def test_livecycle(resources, no_outpdf, caplog):
    exitcode = run_ocrmypdf_api(resources / 'livecycle.pdf', no_outpdf)

    assert exitcode == ExitCode.input_file, caplog.text


@pytest.mark.parametrize('encryption_level', [2, 3, 4, 6])
def test_encrypted(resources, outpdf, encryption_level, caplog):
    if os.name == 'darwin' and sys.version_info >= (3, 12) and encryption_level <= 4:
        # Error is: RuntimeError: unable to load openssl legacy provider
        # pikepdf obtains encryption from qpdf, which gets it from openssl among other
        # providers.
        # Error message itself comes from here:
        # https://github.com/qpdf/qpdf/blob/da3eae39c8e5261196bbc1b460e5b556c6836dbf/libqpdf/QPDFCrypto_openssl.cc#L56
        # Somehow pikepdf + Python 3.12 + macOS does not have this problem, despite
        # using Homebrew's qpdf. Possibly the difference is that pikepdf's Python 3.12
        # comes from cibuildwheel, and our macOS Python 3.12 comes from GitHub Actions
        # setup-python. It may be necessary to build a custom qpdf for macOS.
        # In any case, OCRmyPDF doesn't support loading encrypted files at all, it
        # just complains about encryption, and it's using pikepdf to generate encrypted
        # files for testing.
        pytest.skip("GitHub Python 3.12 on macOS does not have openssl legacy support")
    encryption = pikepdf.models.encryption.Encryption(
        owner='ocrmypdf',
        user='ocrmypdf',
        R=encryption_level,
        aes=(encryption_level >= 4),
        metadata=(encryption_level == 6),
    )

    with pikepdf.open(resources / 'jbig2.pdf') as pdf:
        pdf.save(outpdf, encryption=encryption)

    exitcode = run_ocrmypdf_api(
        outpdf,
        outpdf,
        '--plugin',
        'tests/plugins/tesseract_noop.py',
    )
    assert exitcode == ExitCode.encrypted_pdf
    assert 'encryption must be removed' in caplog.text
