# SPDX-FileCopyrightText: 2022 James R. Barlow
# SPDX-License-Identifier: MPL-2.0

"""Unit tests for ocrmypdf.subprocess helpers."""

from __future__ import annotations

import pytest

from ocrmypdf.exceptions import MissingDependencyError
from ocrmypdf.subprocess import get_version


def test_version_check():
    with pytest.raises(MissingDependencyError):
        get_version('NOT_FOUND_UNLIKELY_ON_PATH')

    with pytest.raises(MissingDependencyError):
        get_version('sh', version_arg='-c')

    with pytest.raises(MissingDependencyError):
        get_version('echo')


def test_get_version_skips_leading_warning_lines(monkeypatch):
    """VeraPDF 1.30.0 prints JVM warnings before its version line."""
    from subprocess import CompletedProcess

    import ocrmypdf.subprocess as sp

    output = (
        "WARNING: Final field flavour has been mutated reflectively\n"
        "WARNING: Use --enable-final-field-mutation=ALL-UNNAMED to avoid this\n"
        "veraPDF 1.30.0\n"
        "Built: Wed Jun 03 13:29:00 PDT 2026\n"
    )

    def fake_run(args, **kwargs):
        return CompletedProcess(args, 0, stdout=output, stderr="")

    monkeypatch.setattr(sp, 'run', fake_run)
    version = get_version('verapdf', regex=r'veraPDF (\d+(\.\d+)*)')
    assert version == '1.30.0'
