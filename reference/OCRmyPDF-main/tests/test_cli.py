# SPDX-FileCopyrightText: 2022 James R. Barlow
# SPDX-License-Identifier: MPL-2.0

"""Tests for command line argument handling."""

from __future__ import annotations

from .conftest import check_ocrmypdf


def test_argsfile(resources, outdir, outpdf):
    path_argsfile = outdir / 'test_argsfile.txt'
    with path_argsfile.open('w') as argsfile:
        print(
            '--title',
            'ArgsFile Test',
            '--author',
            'Test Cases',
            '--plugin',
            'tests/plugins/tesseract_noop.py',
            sep='\n',
            end='\n',
            file=argsfile,
        )
    check_ocrmypdf(resources / 'graph.pdf', outpdf, '@' + str(path_argsfile))
