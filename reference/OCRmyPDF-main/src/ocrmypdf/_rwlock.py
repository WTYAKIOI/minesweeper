# SPDX-FileCopyrightText: 2026 James R. Barlow
# SPDX-License-Identifier: MPL-2.0

"""A readers-writer lock for guarding interpreter-global plugin state."""

from __future__ import annotations

import threading
from collections.abc import Callable, Generator
from contextlib import contextmanager


class RWLock:
    """A writer-preferring readers-writer lock.

    Many threads may hold the lock shared, or one thread may hold it
    exclusively, but not both. Writers are preferred: once a writer is waiting,
    new shared acquisitions block, so a steady stream of readers cannot starve
    it. This matters here because shared holders keep the lock for the length of
    an OCR job while exclusive holders need it only momentarily.

    Shared acquisition is reentrant within a thread, so a thread that already
    holds the lock shared does not deadlock against a waiting writer if it
    acquires again. Upgrading from shared to exclusive is not possible - two
    threads attempting it would deadlock - and raises :class:`RuntimeError`
    rather than hanging.
    """

    def __init__(self) -> None:
        self._cond = threading.Condition(threading.Lock())
        self._readers = 0
        self._writer = False
        self._waiting_writers = 0
        self._local = threading.local()

    @property
    def _depth(self) -> int:
        return getattr(self._local, 'depth', 0)

    @contextmanager
    def shared(self) -> Generator[None]:
        """Hold the lock shared for the duration of the block."""
        if self._depth:
            # Already held by this thread; re-entering must not queue behind a
            # waiting writer, which would deadlock against ourselves.
            self._local.depth = self._depth + 1
            try:
                yield
            finally:
                self._local.depth -= 1
            return

        self._acquire_shared()
        try:
            yield
        finally:
            self._release_shared()

    @contextmanager
    def exclusive(self) -> Generator[Callable[[], None]]:
        """Hold the lock exclusively, with the option to downgrade to shared.

        Yields a callable that atomically converts the exclusive hold into a
        shared one. Use it when the exclusive work establishes state that the
        rest of the block depends on: there is no window in which another
        writer can intervene. Calling it more than once has no effect.
        """
        if self._depth:
            raise RuntimeError(
                "cannot acquire this lock exclusively while holding it shared"
            )

        with self._cond:
            self._waiting_writers += 1
            try:
                while self._writer or self._readers:
                    self._cond.wait()
                self._writer = True
            finally:
                self._waiting_writers -= 1

        downgraded = False

        def downgrade() -> None:
            nonlocal downgraded
            if downgraded:
                return
            with self._cond:
                self._writer = False
                self._readers += 1
                self._cond.notify_all()
            self._local.depth = 1
            downgraded = True

        try:
            yield downgrade
        finally:
            if downgraded:
                self._release_shared()
            else:
                with self._cond:
                    self._writer = False
                    self._cond.notify_all()

    def _acquire_shared(self) -> None:
        with self._cond:
            while self._writer or self._waiting_writers:
                self._cond.wait()
            self._readers += 1
        self._local.depth = 1

    def _release_shared(self) -> None:
        # Clear the depth first: a thread that has fully released must be able
        # to acquire exclusively afterwards, which the reentrancy guard would
        # otherwise reject.
        self._local.depth = 0
        with self._cond:
            self._readers -= 1
            if self._readers == 0:
                self._cond.notify_all()
