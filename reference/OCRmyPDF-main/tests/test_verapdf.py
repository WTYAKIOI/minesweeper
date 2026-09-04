# SPDX-FileCopyrightText: 2024 James R. Barlow
# SPDX-License-Identifier: CC-BY-SA-4.0

"""Tests for verapdf wrapper and speculative PDF/A conversion."""

from __future__ import annotations

import json
import subprocess
from unittest.mock import patch

import pikepdf
import pytest
from pikepdf import Name

from ocrmypdf._exec import verapdf
from ocrmypdf.exceptions import MissingDependencyError
from ocrmypdf.pdfa import (
    _pdfa_part_conformance,
    add_pdfa_metadata,
    add_srgb_output_intent,
    speculative_pdfa_conversion,
)


class TestVerapdfModule:
    """Tests for verapdf wrapper module."""

    def test_output_type_to_flavour(self):
        assert verapdf.output_type_to_flavour('pdfa') == '2b'
        assert verapdf.output_type_to_flavour('pdfa-1') == '1b'
        assert verapdf.output_type_to_flavour('pdfa-2') == '2b'
        assert verapdf.output_type_to_flavour('pdfa-3') == '3b'
        # Unknown should default to 2b
        assert verapdf.output_type_to_flavour('unknown') == '2b'

    @pytest.mark.skipif(not verapdf.available(), reason='verapdf not installed')
    def test_version(self):
        ver = verapdf.version()
        assert ver.major >= 1

    @pytest.mark.skipif(not verapdf.available(), reason='verapdf not installed')
    def test_validate_non_pdfa(self, tmp_path):
        """Test validation of a non-PDF/A file returns invalid."""
        test_pdf = tmp_path / 'test.pdf'
        with pikepdf.new() as pdf:
            pdf.add_blank_page()
            pdf.save(test_pdf)

        result = verapdf.validate(test_pdf, '2b')
        assert not result.valid
        assert result.failed_rules > 0


def _report(**validation_result) -> str:
    """Build a verapdf --format json report containing one job."""
    return json.dumps({'report': {'jobs': [{'validationResult': [validation_result]}]}})


def _run_verapdf(tmp_path, stdout, returncode=0):
    """Run verapdf.validate with the verapdf subprocess replaced."""
    captured = {}

    def fake_run(args, **kwargs):
        captured['args'] = list(args)
        return subprocess.CompletedProcess(args, returncode, stdout=stdout, stderr=b'')

    with patch('ocrmypdf._exec.verapdf.run', side_effect=fake_run):
        result = verapdf.validate(tmp_path / 'in.pdf', '2b')
    return result, captured['args']


class TestValidateFailures:
    """verapdf.validate must degrade gracefully when verapdf misbehaves."""

    def test_binary_missing(self, tmp_path):
        with (
            patch(
                'ocrmypdf._exec.verapdf.run', side_effect=FileNotFoundError('verapdf')
            ),
            pytest.raises(MissingDependencyError, match='verapdf'),
        ):
            verapdf.validate(tmp_path / 'in.pdf', '2b')

    def test_command_line(self, tmp_path):
        _result, args = _run_verapdf(tmp_path, _report(details={'failedRules': 0}))
        assert args[0] == 'verapdf'
        assert args[args.index('--flavour') + 1] == '2b'
        assert args[args.index('--format') + 1] == 'json'
        assert args[-1] == str(tmp_path / 'in.pdf')

    @pytest.mark.parametrize(
        'stdout',
        [b'', b'verapdf: command failed', b'{"report": '],
        ids=['empty', 'message', 'truncated-json'],
    )
    def test_unparseable_output(self, tmp_path, stdout):
        result, _args = _run_verapdf(tmp_path, stdout, returncode=1)
        assert result.valid is False
        assert result.failed_rules == -1
        assert 'Failed to parse verapdf output' in result.message

    def test_no_jobs_in_report(self, tmp_path):
        result, _args = _run_verapdf(tmp_path, json.dumps({'report': {'jobs': []}}))
        assert result == verapdf.ValidationResult(
            False, -1, 'No validation jobs in result'
        )

    def test_no_validation_result_in_job(self, tmp_path):
        result, _args = _run_verapdf(
            tmp_path, json.dumps({'report': {'jobs': [{'validationResult': []}]}})
        )
        assert result == verapdf.ValidationResult(
            False, -1, 'No validation result in output'
        )

    def test_report_of_wrong_type(self, tmp_path):
        """A JSON document of the wrong shape is a parse failure, not a crash."""
        result, _args = _run_verapdf(tmp_path, json.dumps({'report': 'nonsense'}))
        assert result.valid is False
        assert result.failed_rules == -1
        assert 'Failed to parse verapdf output' in result.message

    def test_passing_validation(self, tmp_path):
        result, _args = _run_verapdf(tmp_path, _report(details={'failedRules': 0}))
        assert result == verapdf.ValidationResult(True, 0, 'PDF/A validation passed')

    def test_failing_validation(self, tmp_path):
        result, _args = _run_verapdf(tmp_path, _report(details={'failedRules': 7}))
        assert result.valid is False
        assert result.failed_rules == 7
        assert '7 rule violations' in result.message

    def test_missing_details_counts_as_passing(self, tmp_path):
        """No details means no failures were reported."""
        result, _args = _run_verapdf(tmp_path, _report())
        assert result.valid is True


class TestPdfaPartConformance:
    """Tests for _pdfa_part_conformance helper."""

    def test_pdfa_part_conformance(self):
        assert _pdfa_part_conformance('pdfa') == ('2', 'B')
        assert _pdfa_part_conformance('pdfa-1') == ('1', 'B')
        assert _pdfa_part_conformance('pdfa-2') == ('2', 'B')
        assert _pdfa_part_conformance('pdfa-3') == ('3', 'B')
        # Unknown should default to 2B
        assert _pdfa_part_conformance('unknown') == ('2', 'B')


class TestAddPdfaMetadata:
    """Tests for add_pdfa_metadata function."""

    def test_add_pdfa_metadata(self, tmp_path):
        """Test adding PDF/A XMP metadata."""
        test_pdf = tmp_path / 'test.pdf'
        with pikepdf.new() as pdf:
            pdf.add_blank_page()
            pdf.save(test_pdf)

        with pikepdf.open(test_pdf, allow_overwriting_input=True) as pdf:
            add_pdfa_metadata(pdf, '2', 'B')
            with pdf.open_metadata() as meta:
                assert meta.pdfa_status == '2B'
            pdf.save(test_pdf)

        # Verify it persists after save
        with pikepdf.open(test_pdf) as pdf, pdf.open_metadata() as meta:
            assert meta.pdfa_status == '2B'


class TestAddSrgbOutputIntent:
    """Tests for add_srgb_output_intent function."""

    def test_add_srgb_output_intent(self, tmp_path):
        """Test adding sRGB OutputIntent to a PDF."""
        test_pdf = tmp_path / 'test.pdf'
        with pikepdf.new() as pdf:
            pdf.add_blank_page()
            pdf.save(test_pdf)

        with pikepdf.open(test_pdf, allow_overwriting_input=True) as pdf:
            add_srgb_output_intent(pdf)
            assert Name.OutputIntents in pdf.Root
            assert len(pdf.Root.OutputIntents) == 1
            intent = pdf.Root.OutputIntents[0]
            assert str(intent.get(Name.OutputConditionIdentifier)) == 'sRGB'
            pdf.save(test_pdf)

    def test_add_srgb_output_intent_idempotent(self, tmp_path):
        """Test that adding OutputIntent twice doesn't duplicate."""
        test_pdf = tmp_path / 'test.pdf'
        with pikepdf.new() as pdf:
            pdf.add_blank_page()
            pdf.save(test_pdf)

        with pikepdf.open(test_pdf, allow_overwriting_input=True) as pdf:
            add_srgb_output_intent(pdf)
            add_srgb_output_intent(pdf)  # Second call should be a no-op
            assert len(pdf.Root.OutputIntents) == 1
            pdf.save(test_pdf)


class TestSpeculativePdfaConversion:
    """Tests for speculative PDF/A conversion."""

    def test_speculative_conversion_creates_pdfa_structures(self, tmp_path, resources):
        """Test that speculative conversion adds PDF/A structures."""
        input_pdf = resources / 'graph.pdf'
        output_pdf = tmp_path / 'output.pdf'

        result = speculative_pdfa_conversion(input_pdf, output_pdf, 'pdfa-2')

        assert result.exists()
        with pikepdf.open(result) as pdf:
            assert Name.OutputIntents in pdf.Root
            with pdf.open_metadata() as meta:
                assert meta.pdfa_status == '2B'

    def test_speculative_conversion_different_parts(self, tmp_path, resources):
        """Test speculative conversion with different PDF/A parts."""
        input_pdf = resources / 'graph.pdf'

        for output_type, expected_status in [
            ('pdfa-1', '1B'),
            ('pdfa-2', '2B'),
            ('pdfa-3', '3B'),
        ]:
            output_pdf = tmp_path / f'output_{output_type}.pdf'
            speculative_pdfa_conversion(input_pdf, output_pdf, output_type)

            with pikepdf.open(output_pdf) as pdf, pdf.open_metadata() as meta:
                assert meta.pdfa_status == expected_status


@pytest.mark.skipif(not verapdf.available(), reason='verapdf not installed')
class TestVerapdfIntegration:
    """Integration tests requiring verapdf."""

    def test_speculative_conversion_validation(self, tmp_path, resources):
        """Test that speculative conversion can be validated by verapdf.

        Note: Most test PDFs will fail validation because they have issues
        that require Ghostscript to fix (fonts, colorspaces, etc.). This test
        verifies the validation pipeline works, not that all PDFs pass.
        """
        input_pdf = resources / 'graph.pdf'
        output_pdf = tmp_path / 'output.pdf'

        speculative_pdfa_conversion(input_pdf, output_pdf, 'pdfa-2')

        # The converted file can be validated (even if it fails)
        result = verapdf.validate(output_pdf, '2b')
        assert isinstance(result.valid, bool)
        assert isinstance(result.failed_rules, int)
