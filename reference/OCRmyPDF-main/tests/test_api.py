# SPDX-FileCopyrightText: 2022 James R. Barlow
# SPDX-License-Identifier: MPL-2.0

from __future__ import annotations

import io
import logging
import pickle
import sys
import warnings
from io import BytesIO
from pathlib import Path

import pytest
from pdfminer.high_level import extract_text

import ocrmypdf
import ocrmypdf._pipelines
import ocrmypdf.api
from ocrmypdf import Verbosity
from ocrmypdf._logging import PageNumberFilter, RichLoggingHandler
from ocrmypdf._options import OcrOptions
from ocrmypdf.api import (
    configure_logging,
    create_options,
    setup_plugin_infrastructure,
)

NOOP_PLUGIN = 'tests/plugins/tesseract_noop.py'

# Loggers that configure_logging() is documented to touch. All of them must be
# restored, or handlers leak into unrelated tests (and across xdist workers).
TOUCHED_LOGGERS = ('', 'ocrmypdf', 'pdfminer', 'PIL', 'fontTools', 'py.warnings')


@pytest.fixture
def isolated_loggers():
    """Snapshot and restore every logger configure_logging() may modify."""
    saved = {}
    for name in TOUCHED_LOGGERS:
        log = logging.getLogger(name)
        saved[name] = (log.level, log.propagate, log.handlers[:], log.filters[:])
    saved_showwarning = warnings.showwarning
    saved_logging_showwarning = getattr(logging, '_warnings_showwarning', None)
    try:
        yield
    finally:
        for name, (level, propagate, handlers, filters) in saved.items():
            log = logging.getLogger(name)
            log.setLevel(level)
            log.propagate = propagate
            log.handlers[:] = handlers
            log.filters[:] = filters
        warnings.showwarning = saved_showwarning
        logging._warnings_showwarning = saved_logging_showwarning


def test_language_list():
    with pytest.raises(
        (ocrmypdf.exceptions.InputFileError, ocrmypdf.exceptions.MissingDependencyError)
    ):
        ocrmypdf.ocr('doesnotexist.pdf', '_.pdf', language=['eng', 'deu'])


def test_language_parameter_mapped_to_languages():
    """Test that the API 'language' parameter is mapped to OcrOptions 'languages'.

    Regression test for GitHub issue #1640: the Python API ignored the language
    parameter, always defaulting to 'eng'.
    """
    from ocrmypdf.api import create_options, setup_plugin_infrastructure
    from ocrmypdf.cli import get_parser

    setup_plugin_infrastructure()
    parser = get_parser()

    options = create_options(
        input_file='test.pdf',
        output_file='output.pdf',
        parser=parser,
        language=['tam'],
    )
    assert options.languages == ['tam']

    # Test with a list of multiple languages
    options = create_options(
        input_file='test.pdf',
        output_file='output.pdf',
        parser=parser,
        language=['fra', 'deu'],
    )
    assert options.languages == ['fra', 'deu']

    # Test with a bare string (single language)
    options = create_options(
        input_file='test.pdf',
        output_file='output.pdf',
        parser=parser,
        language='tam',
    )
    assert options.languages == ['tam']

    # Test '+'-separated string is split like CLI --language
    options = create_options(
        input_file='test.pdf',
        output_file='output.pdf',
        parser=parser,
        language='eng+spa',
    )
    assert options.languages == ['eng', 'spa']

    # Test '+'-separated entry within a list is also split
    options = create_options(
        input_file='test.pdf',
        output_file='output.pdf',
        parser=parser,
        language=['eng+spa'],
    )
    assert options.languages == ['eng', 'spa']


def test_jpeg_quality_parameter_reaches_options():
    """The canonical 'jpeg_quality' API parameter must reach OcrOptions.

    Regression test for GitHub issue #1723: --jpeg-quality was silently
    dropped by the CLI's namespace_to_options() because the OcrOptions field
    was named jpg_quality. create_options(), used by the Python API, has the
    same field-name matching logic and is affected the same way when passed
    the alias name.
    """
    from ocrmypdf.api import create_options, setup_plugin_infrastructure
    from ocrmypdf.cli import get_parser

    setup_plugin_infrastructure()
    parser = get_parser()

    options = create_options(
        input_file='test.pdf', output_file='output.pdf', parser=parser, jpeg_quality=10
    )
    assert options.jpeg_quality == 10


def test_jpg_quality_parameter_deprecated_alias():
    """The old 'jpg_quality' API parameter still works but warns."""
    from ocrmypdf.api import create_options, setup_plugin_infrastructure
    from ocrmypdf.cli import get_parser

    setup_plugin_infrastructure()
    parser = get_parser()

    with pytest.warns(UserWarning, match='jpg_quality'):
        options = create_options(
            input_file='test.pdf',
            output_file='output.pdf',
            parser=parser,
            jpg_quality=42,
        )
    assert options.jpeg_quality == 42


def test_stream_api(resources: Path):
    in_ = (resources / 'graph.pdf').open('rb')
    out = BytesIO()

    ocrmypdf.ocr(in_, out, tesseract_timeout=0.0)
    out.seek(0)
    assert b'%PDF' in out.read(1024)


def test_sidecar_stringio(resources: Path, outdir: Path, outpdf: Path):
    s = BytesIO()
    ocrmypdf.ocr(
        resources / 'ccitt.pdf',
        outpdf,
        plugins=['tests/plugins/tesseract_cache.py'],
        sidecar=s,
    )
    s.seek(0)
    assert b'the' in s.getvalue()


def test_hocr_api_multipage(resources: Path, outdir: Path, outpdf: Path):
    ocrmypdf.api._pdf_to_hocr(
        resources / 'multipage.pdf',
        outdir,
        language='eng',
        skip_text=True,
        plugins=['tests/plugins/tesseract_cache.py'],
    )
    assert (outdir / '000001_ocr_hocr.hocr').exists()
    assert (outdir / '000006_ocr_hocr.hocr').exists()
    assert not (outdir / '000004_ocr_hocr.hocr').exists()

    ocrmypdf.api._hocr_to_ocr_pdf(outdir, outpdf, optimize=0, output_type='pdf')
    assert outpdf.exists()


def test_hocr_to_pdf_api(resources: Path, outdir: Path, outpdf: Path):
    ocrmypdf.api._pdf_to_hocr(
        resources / 'ccitt.pdf',
        outdir,
        language='eng',
        skip_text=True,
        plugins=['tests/plugins/tesseract_cache.py'],
    )
    assert (outdir / '000001_ocr_hocr.hocr').exists()
    hocr = (outdir / '000001_ocr_hocr.hocr').read_text(encoding='utf-8')
    mangled = hocr.replace('the', 'hocr')
    (outdir / '000001_ocr_hocr.hocr').write_text(mangled, encoding='utf-8')

    ocrmypdf.api._hocr_to_ocr_pdf(outdir, outpdf, optimize=0)

    text = extract_text(outpdf)
    assert 'hocr' in text and 'the' not in text


def test_hocr_result_json():
    result = ocrmypdf._pipelines._common.HOCRResult(
        pageno=1,
        pdf_page_from_image=Path('a'),
        hocr=Path('b'),
        textpdf=Path('c'),
        orientation_correction=180,
    )
    assert (
        result.to_json()
        == '{"pageno": 1, "pdf_page_from_image": {"Path": "a"}, "hocr": {"Path": "b"}, '
        '"textpdf": {"Path": "c"}, "orientation_correction": 180, "ocr_tree": null}'
    )
    assert ocrmypdf._pipelines._common.HOCRResult.from_json(result.to_json()) == result


def test_hocr_result_pickle():
    result = ocrmypdf._pipelines._common.HOCRResult(
        pageno=1,
        pdf_page_from_image=Path('a'),
        hocr=Path('b'),
        textpdf=Path('c'),
        orientation_correction=180,
    )
    assert result == pickle.loads(pickle.dumps(result))


def test_nested_plugin_option_access():
    """Test that plugin options can be accessed via nested namespaces."""
    from ocrmypdf._options import OcrOptions
    from ocrmypdf.api import setup_plugin_infrastructure

    # Set up plugin infrastructure to register plugin models
    setup_plugin_infrastructure()

    # Create options with tesseract settings
    options = OcrOptions(
        input_file='test.pdf',
        output_file='output.pdf',
        tesseract_timeout=120.0,
        tesseract_oem=1,
        optimize=2,
    )

    # Test flat access still works
    assert options.tesseract_timeout == 120.0
    assert options.tesseract_oem == 1
    assert options.optimize == 2

    # Test nested access for tesseract
    tesseract = options.tesseract
    assert tesseract is not None
    assert tesseract.timeout == 120.0
    assert tesseract.oem == 1

    # Test nested access for ghostscript
    ghostscript = options.ghostscript
    assert ghostscript is not None
    assert ghostscript.color_conversion_strategy == "LeaveColorUnchanged"

    # Test that cached instances are returned
    assert options.tesseract is tesseract


def test_default_tesseract_timeout():
    """Test that OcrOptions without explicit tesseract_timeout uses plugin default.

    Regression test for GitHub issue #1636: when using the Python API without
    specifying tesseract_timeout, the default was 0.0 which caused Tesseract
    to immediately time out and produce no OCR output.
    """
    from ocrmypdf._options import OcrOptions
    from ocrmypdf.api import setup_plugin_infrastructure

    setup_plugin_infrastructure()

    # Default OcrOptions should leave tesseract_timeout as None
    options = OcrOptions(
        input_file='test.pdf',
        output_file='output.pdf',
    )
    assert options.tesseract_timeout is None

    # The plugin default (180s) should be used when tesseract_timeout is None
    assert options.tesseract.timeout == 180.0


# --- configure_logging() ---


@pytest.mark.parametrize(
    'verbosity, expected_handler_level',
    [
        (Verbosity.quiet, logging.ERROR),
        (Verbosity.default, logging.INFO),
        (Verbosity.debug, logging.DEBUG),
        (Verbosity.debug_all, logging.DEBUG),
    ],
)
@pytest.mark.parametrize('manage_root_logger', [False, True])
@pytest.mark.parametrize('progress_bar_friendly', [False, True])
def test_configure_logging_matrix(
    isolated_loggers,
    verbosity,
    expected_handler_level,
    manage_root_logger,
    progress_bar_friendly,
):
    """configure_logging() installs one handler configured by its arguments."""
    plugin_manager = setup_plugin_infrastructure()
    third_party = ('pdfminer', 'PIL', 'fontTools')
    levels_before = {name: logging.getLogger(name).level for name in third_party}
    showwarning_before = warnings.showwarning

    log = configure_logging(
        verbosity,
        progress_bar_friendly=progress_bar_friendly,
        manage_root_logger=manage_root_logger,
        plugin_manager=plugin_manager,
    )

    # The logger returned is the root logger only when we are managing it.
    assert log is logging.getLogger('' if manage_root_logger else 'ocrmypdf')
    assert log.level == logging.DEBUG

    handler = log.handlers[-1]
    assert handler.level == expected_handler_level

    # Progress-bar-friendly logging asks the plugin manager for its console.
    if progress_bar_friendly:
        assert isinstance(handler, RichLoggingHandler)
    else:
        assert type(handler) is logging.StreamHandler
        assert handler.stream is sys.stderr

    assert any(isinstance(f, PageNumberFilter) for f in handler.filters)

    # Only the most verbose level identifies the logger and level in each line.
    fmt = handler.formatter._fmt
    if verbosity >= Verbosity.debug_all:
        assert fmt == '%(levelname)7s %(name)s -%(pageno)s %(message)s'
    else:
        assert fmt == '%(pageno)s%(message)s'

    # Chatty third-party libraries are quieted, except at maximum verbosity
    # where the user has asked to see everything.
    if verbosity <= Verbosity.debug:
        assert logging.getLogger('pdfminer').level == logging.ERROR
        assert logging.getLogger('PIL').level == logging.INFO
        assert logging.getLogger('fontTools').level == logging.ERROR
    else:
        assert {
            name: logging.getLogger(name).level for name in third_party
        } == levels_before

    # Warnings are only rerouted through logging when we own the root logger.
    if manage_root_logger:
        assert warnings.showwarning is not showwarning_before
    else:
        assert warnings.showwarning is showwarning_before


def test_configure_logging_without_plugin_manager_logs_to_stderr(isolated_loggers):
    """With no plugin manager there is no custom console, even if asked for one."""
    log = configure_logging(Verbosity.default, progress_bar_friendly=True)
    handler = log.handlers[-1]
    assert type(handler) is logging.StreamHandler
    assert handler.stream is sys.stderr


def test_configure_logging_emits_page_numbers(isolated_loggers):
    """The installed handler renders the page number of the emitting thread."""
    from ocrmypdf._pipelines._common import set_thread_pageno

    log = configure_logging(Verbosity.default, progress_bar_friendly=False)
    buffer = io.StringIO()
    log.handlers[-1].stream = buffer

    log.info("no page number here")
    set_thread_pageno(7)
    try:
        log.info("page message")
    finally:
        set_thread_pageno(None)

    assert buffer.getvalue() == "no page number here\n    7 page message\n"


def test_configure_logging_root_logger_captures_warnings(isolated_loggers, caplog):
    """manage_root_logger=True routes warnings.warn() into logging."""
    configure_logging(
        Verbosity.default, progress_bar_friendly=False, manage_root_logger=True
    )
    with caplog.at_level(logging.WARNING, logger='py.warnings'):
        warnings.warn("a warning that logging should capture", stacklevel=1)
    assert 'a warning that logging should capture' in caplog.text


# --- setup_plugin_infrastructure() ---


def test_setup_plugin_infrastructure_rejects_plugins_and_manager():
    """Supplying a prebuilt manager and a plugin list is contradictory."""
    plugin_manager = setup_plugin_infrastructure()
    with pytest.raises(ValueError, match='mutually exclusive'):
        setup_plugin_infrastructure(
            plugins=[NOOP_PLUGIN], plugin_manager=plugin_manager
        )


def test_setup_plugin_infrastructure_accepts_bare_string_plugin():
    """A single plugin may be given without wrapping it in a list."""
    from_string = setup_plugin_infrastructure(plugins=NOOP_PLUGIN)
    from_list = setup_plugin_infrastructure(plugins=[NOOP_PLUGIN])
    assert str(from_string.get_ocr_engine()) == str(from_list.get_ocr_engine())
    assert 'NO-OP' in str(from_string.get_ocr_engine())


def test_setup_plugin_infrastructure_accepts_path_plugin():
    """A single plugin may be given as a Path."""
    plugin_manager = setup_plugin_infrastructure(plugins=Path(NOOP_PLUGIN))
    assert 'NO-OP' in str(plugin_manager.get_ocr_engine())


def test_setup_plugin_infrastructure_reuses_given_plugin_manager():
    """A prebuilt manager is initialized and returned, not replaced."""
    plugin_manager = setup_plugin_infrastructure(plugins=[NOOP_PLUGIN])
    again = setup_plugin_infrastructure(plugin_manager=plugin_manager)
    assert again is plugin_manager
    assert again._option_registry is not None


# --- configure_stdout_protection() ---


def test_configure_stdout_protection_declines_for_in_memory_stdout(monkeypatch):
    """Protection is declined when stdout has no OS file descriptor."""
    from ocrmypdf import _stdoutprotect

    if _stdoutprotect._active:
        pytest.skip("stdout protection is already active in this process")
    monkeypatch.setattr(sys, 'stdout', io.StringIO())
    assert ocrmypdf.configure_stdout_protection() is False
    assert not _stdoutprotect._active


# --- create_options() ---


def test_create_options_wraps_validation_error_in_type_error():
    """Invalid field values surface as TypeError, not a pydantic error."""
    setup_plugin_infrastructure()
    with pytest.raises(TypeError, match='Failed to create OcrOptions'):
        create_options(
            input_file='test.pdf',
            output_file='output.pdf',
            parser=ocrmypdf.api.get_parser(),
            output_type='no-such-output-type',
        )


def test_create_options_keeps_unknown_kwargs_as_extra_attrs():
    """Plugin-supplied options are preserved rather than rejected."""
    setup_plugin_infrastructure()
    options = create_options(
        input_file='test.pdf',
        output_file='output.pdf',
        parser=ocrmypdf.api.get_parser(),
        some_plugin_setting='xyzzy',
    )
    assert options.extra_attrs['some_plugin_setting'] == 'xyzzy'
    assert options.some_plugin_setting == 'xyzzy'


def test_create_options_prefers_explicit_languages_over_language():
    """When both spellings are given, the canonical 'languages' field wins."""
    setup_plugin_infrastructure()
    options = create_options(
        input_file='test.pdf',
        output_file='output.pdf',
        parser=ocrmypdf.api.get_parser(),
        language='eng',
        languages=['deu'],
    )
    assert options.languages == ['deu']


def test_create_options_prefers_jpeg_quality_over_deprecated_jpg_quality():
    """The deprecated alias never overwrites an explicitly-set jpeg_quality."""
    setup_plugin_infrastructure()
    with pytest.warns(UserWarning, match='jpg_quality'):
        options = create_options(
            input_file='test.pdf',
            output_file='output.pdf',
            parser=ocrmypdf.api.get_parser(),
            jpg_quality=42,
            jpeg_quality=10,
        )
    assert options.jpeg_quality == 10


# --- ocr() with an OcrOptions object ---


def make_options(resources: Path, outpdf: Path, **kwargs) -> OcrOptions:
    """Build an OcrOptions for the cheapest possible real pipeline run."""
    setup_plugin_infrastructure()
    return OcrOptions(
        input_file=resources / 'trivial.pdf',
        output_file=outpdf,
        mode='force',
        output_type='pdf',
        optimize=0,
        progress_bar=False,
        **kwargs,
    )


def test_ocr_with_options_object(resources, outpdf):
    """The new-style calling convention runs the pipeline."""
    options = make_options(resources, outpdf)
    assert ocrmypdf.ocr(options, plugins=[NOOP_PLUGIN]) == ocrmypdf.ExitCode.ok
    assert outpdf.exists()


def test_ocr_with_options_inherits_plugins_from_options(resources, outpdf):
    """Plugins declared inside OcrOptions are loaded when none are passed."""
    options = make_options(resources, outpdf, plugins=[NOOP_PLUGIN])
    assert ocrmypdf.ocr(options) == ocrmypdf.ExitCode.ok
    # The no-op engine produces a blank text layer; a real engine would not run
    # at all here since tesseract is not requested by these options.
    assert outpdf.exists()


def test_ocr_with_options_accepts_bare_string_plugin(resources, outpdf):
    """A single plugin may be given without wrapping it in a list."""
    options = make_options(resources, outpdf)
    assert ocrmypdf.ocr(options, plugins=NOOP_PLUGIN) == ocrmypdf.ExitCode.ok
    assert outpdf.exists()


@pytest.mark.parametrize(
    'kwargs',
    [
        {'language': ['eng']},
        {'optimize': 0},  # falsy but not None: still a conflict
        {'output_file': 'somewhere_else.pdf'},
        {'not_a_real_parameter': 1},
    ],
)
def test_ocr_with_options_rejects_extra_parameters(resources, outpdf, kwargs):
    """Settings must be put in OcrOptions, not passed alongside it."""
    options = make_options(resources, outpdf)
    with pytest.raises(ValueError, match='do not pass'):
        ocrmypdf.ocr(options, **kwargs)
    assert not outpdf.exists()


def test_ocr_with_options_rejects_plugins_and_plugin_manager(resources, outpdf):
    """plugins= and plugin_manager= remain mutually exclusive in new style."""
    options = make_options(resources, outpdf)
    plugin_manager = setup_plugin_infrastructure(plugins=[NOOP_PLUGIN])
    with pytest.raises(ValueError, match='mutually exclusive'):
        ocrmypdf.ocr(options, plugins=[NOOP_PLUGIN], plugin_manager=plugin_manager)
    assert not outpdf.exists()


# --- ocr() old-style error paths ---


def test_ocr_requires_output_file(resources):
    """Old-style calls without an output file get a helpful TypeError."""
    with pytest.raises(TypeError, match='output_file'):
        ocrmypdf.ocr(resources / 'trivial.pdf')


def test_ocr_old_style_accepts_bare_string_plugin(resources, outpdf):
    """A single plugin may be given without wrapping it in a list."""
    result = ocrmypdf.ocr(
        resources / 'trivial.pdf',
        outpdf,
        force_ocr=True,
        output_type='pdf',
        optimize=0,
        progress_bar=False,
        plugins=NOOP_PLUGIN,
    )
    assert result == ocrmypdf.ExitCode.ok
    assert outpdf.exists()


def test_ocr_old_style_rejects_plugins_and_plugin_manager(resources, outpdf):
    """plugins= and plugin_manager= remain mutually exclusive in old style."""
    plugin_manager = setup_plugin_infrastructure(plugins=[NOOP_PLUGIN])
    with pytest.raises(ValueError, match='mutually exclusive'):
        ocrmypdf.ocr(
            resources / 'trivial.pdf',
            outpdf,
            plugins=[NOOP_PLUGIN],
            plugin_manager=plugin_manager,
        )
    assert not outpdf.exists()


@pytest.mark.parametrize(
    'deprecated, match',
    [
        ({'verbose': 1}, 'verbose'),
        ({'jbig2_lossy': True}, 'jbig2_lossy'),
        ({'jbig2_page_group_size': 10}, 'jbig2_page_group_size'),
    ],
)
def test_ocr_deprecated_keywords_warn_but_still_run(
    resources, outpdf, deprecated, match
):
    """Deprecated keywords are ignored with a warning, not an error."""
    with pytest.warns(UserWarning, match=match):
        result = ocrmypdf.ocr(
            resources / 'trivial.pdf',
            outpdf,
            force_ocr=True,
            output_type='pdf',
            optimize=0,
            progress_bar=False,
            plugins=[NOOP_PLUGIN],
            **deprecated,
        )
    assert result == ocrmypdf.ExitCode.ok
    assert outpdf.exists()


# --- _pdf_to_hocr() and _hocr_to_ocr_pdf() ---


def test_pdf_to_hocr_rejects_plugins_and_plugin_manager(resources, outdir):
    plugin_manager = setup_plugin_infrastructure(plugins=[NOOP_PLUGIN])
    with pytest.raises(ValueError, match='mutually exclusive'):
        ocrmypdf.api._pdf_to_hocr(
            resources / 'trivial.pdf',
            outdir,
            plugins=[NOOP_PLUGIN],
            plugin_manager=plugin_manager,
        )


def test_pdf_to_hocr_wraps_invalid_option_in_type_error(resources, outdir):
    """Invalid options surface as TypeError naming the hOCR pipeline.

    No plugins are given, exercising the default plugin set as well.
    """
    with pytest.raises(TypeError, match='hOCR pipeline'):
        ocrmypdf.api._pdf_to_hocr(
            resources / 'trivial.pdf',
            outdir,
            output_type='no-such-output-type',
        )


def test_hocr_to_ocr_pdf_rejects_plugins_and_plugin_manager(outdir, outpdf):
    plugin_manager = setup_plugin_infrastructure(plugins=[NOOP_PLUGIN])
    with pytest.raises(ValueError, match='mutually exclusive'):
        ocrmypdf.api._hocr_to_ocr_pdf(
            outdir, outpdf, plugins=[NOOP_PLUGIN], plugin_manager=plugin_manager
        )


def test_hocr_to_ocr_pdf_wraps_invalid_option_in_type_error(outdir, outpdf):
    """Invalid options surface as TypeError naming the hOCR to PDF pipeline."""
    with pytest.raises(TypeError, match='hOCR to PDF pipeline'):
        ocrmypdf.api._hocr_to_ocr_pdf(
            outdir, outpdf, jpeg_quality='not a number', plugins=[NOOP_PLUGIN]
        )


def test_hocr_roundtrip_with_string_plugin_and_deprecated_jbig2(resources, outdir):
    """A bare string plugin works, and deprecated jbig2 options only warn."""
    ocrmypdf.api._pdf_to_hocr(
        resources / 'trivial.pdf',
        outdir,
        mode='force',
        plugins=NOOP_PLUGIN,
    )
    assert (outdir / '000001_ocr_hocr.hocr').exists()

    outpdf = outdir / 'roundtrip.pdf'
    with (
        pytest.warns(UserWarning, match='jbig2_lossy'),
        pytest.warns(UserWarning, match='jbig2_page_group_size'),
    ):
        ocrmypdf.api._hocr_to_ocr_pdf(
            outdir,
            outpdf,
            optimize=0,
            jbig2_lossy=True,
            jbig2_page_group_size=10,
            plugins=NOOP_PLUGIN,
        )
    assert outpdf.exists()


@pytest.mark.parametrize(
    'use_options_object', [False, True], ids=['positional', 'options']
)
def test_plugin_lock_not_held_exclusively_during_pipeline(
    monkeypatch, resources, outpdf, use_options_object
):
    """Option checking and the pipeline must run without exclusive access.

    The lock is taken exclusively only to install plugins, which mutates
    interpreter-global state. A job holds it shared for its run, so concurrent
    jobs overlap; holding it exclusively any longer would serialize them.
    """
    observed = []
    real_run_pipeline = ocrmypdf.api.run_pipeline
    real_check_options = ocrmypdf.api.check_options

    def _held_exclusively():
        # pylint: disable=protected-access
        return ocrmypdf.api.plugin_lock._writer

    def spy_run_pipeline(*args, **kwargs):
        observed.append(('run_pipeline', _held_exclusively()))
        return real_run_pipeline(*args, **kwargs)

    def spy_check_options(*args, **kwargs):
        observed.append(('check_options', _held_exclusively()))
        return real_check_options(*args, **kwargs)

    monkeypatch.setattr(ocrmypdf.api, 'run_pipeline', spy_run_pipeline)
    monkeypatch.setattr(ocrmypdf.api, 'check_options', spy_check_options)

    if use_options_object:
        ocrmypdf.ocr(
            OcrOptions(
                input_file=resources / 'trivial.pdf',
                output_file=outpdf,
                optimize=0,
                progress_bar=False,
            ),
            plugins=[NOOP_PLUGIN],
        )
    else:
        ocrmypdf.ocr(
            resources / 'trivial.pdf',
            outpdf,
            optimize=0,
            plugins=[NOOP_PLUGIN],
            progress_bar=False,
        )

    assert observed == [('check_options', False), ('run_pipeline', False)]
