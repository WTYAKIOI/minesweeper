# SPDX-FileCopyrightText: 2022 James R. Barlow
# SPDX-License-Identifier: MPL-2.0

from __future__ import annotations

import logging
import re
from io import StringIO

import pytest
from rich.console import Console

from ocrmypdf._logging import PageNumberFilter, RichLoggingHandler
from ocrmypdf._pipelines._common import configure_debug_logging


def test_debug_logging(tmp_path):
    # Just exercise the debug logger but don't validate it
    # See https://github.com/pytest-dev/pytest/issues/5502 for pytest logging quirks
    prefix = 'test_debug_logging'
    log = logging.getLogger(prefix)
    _handler, remover = configure_debug_logging(tmp_path / 'test.log', prefix)
    log.info("test message")
    remover()


def make_record(**attrs) -> logging.LogRecord:
    """Build a log record, optionally with extra attributes attached."""
    record = logging.LogRecord(
        name='ocrmypdf',
        level=logging.INFO,
        pathname=__file__,
        lineno=1,
        msg="hello",
        args=None,
        exc_info=None,
    )
    for key, value in attrs.items():
        setattr(record, key, value)
    return record


def test_page_number_filter_formats_int_pageno():
    """An int page number becomes a right-aligned, space-padded field."""
    record = make_record(pageno=3)
    assert PageNumberFilter().filter(record) is True
    assert record.pageno == '    3 '
    # The field is wide enough that a four digit page number still aligns.
    record = make_record(pageno=1234)
    PageNumberFilter().filter(record)
    assert record.pageno == ' 1234 '


def test_page_number_filter_absent_pageno_becomes_empty_string():
    """Records without a page number must still format under '%(pageno)s'."""
    record = make_record()
    assert PageNumberFilter().filter(record) is True
    assert record.pageno == ''
    assert logging.Formatter('%(pageno)s%(message)s').format(record) == "hello"


def test_page_number_filter_leaves_non_int_pageno_alone():
    """A caller that pre-formatted its own page number keeps it."""
    record = make_record(pageno='page five ')
    assert PageNumberFilter().filter(record) is True
    assert record.pageno == 'page five '


def test_page_number_filter_never_drops_records():
    """The filter annotates records; it must never suppress any of them."""
    assert PageNumberFilter().filter(make_record(pageno=0)) is True
    assert PageNumberFilter().filter(make_record(pageno=None)) is True


def test_rich_logging_handler_emits_plain_unadorned_message():
    """The handler shows the message only: no level, no timestamp, no markup."""
    buffer = StringIO()
    console = Console(file=buffer, width=120, no_color=True)
    handler = RichLoggingHandler(console)
    handler.setFormatter(logging.Formatter('%(message)s'))

    handler.emit(make_record(msg="[bold]literal[/bold] brackets"))
    output = buffer.getvalue()

    # markup=False: rich tags are printed literally instead of being consumed.
    assert '[bold]literal[/bold] brackets' in output
    # show_level=False
    assert 'INFO' not in output
    # show_time=False
    assert not re.search(r'\d\d:\d\d:\d\d', output)


def test_rich_logging_handler_accepts_extra_richhandler_kwargs():
    """Additional RichHandler options pass through to the base class."""
    console = Console(file=StringIO(), width=120, no_color=True)
    handler = RichLoggingHandler(console, show_path=False)
    assert handler.console is console
    assert handler._log_render.show_path is False


@pytest.mark.parametrize('pageno', [1, None])
def test_page_number_filter_usable_as_handler_filter(pageno):
    """End to end: the filter makes '%(pageno)s' safe in a real handler."""
    buffer = StringIO()
    handler = logging.StreamHandler(stream=buffer)
    handler.addFilter(PageNumberFilter())
    handler.setFormatter(logging.Formatter('%(pageno)s%(message)s'))
    record = make_record()
    if pageno is not None:
        record.pageno = pageno
    handler.handle(record)
    expected = "    1 hello\n" if pageno == 1 else "hello\n"
    assert buffer.getvalue() == expected
