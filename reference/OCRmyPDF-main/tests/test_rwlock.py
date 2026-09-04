# SPDX-FileCopyrightText: 2026 James R. Barlow
# SPDX-License-Identifier: MPL-2.0

from __future__ import annotations

import threading

import pytest

from ocrmypdf._rwlock import RWLock


def _spawn(targets, timeout=30):
    """Run each target in its own thread; return exceptions raised."""
    errors: list[BaseException] = []

    def wrapper(target):
        try:
            target()
        except BaseException as e:  # noqa: BLE001 - report, don't swallow
            errors.append(e)

    threads = [threading.Thread(target=wrapper, args=(t,)) for t in targets]
    for t in threads:
        t.start()
    for t in threads:
        t.join(timeout=timeout)
    return errors


def test_shared_holders_run_concurrently():
    lock = RWLock()
    barrier = threading.Barrier(3, timeout=10)

    def reader():
        with lock.shared():
            barrier.wait()

    assert not _spawn([reader] * 3)


def test_exclusive_excludes_shared():
    lock = RWLock()
    writer_inside = threading.Event()
    reader_entered = threading.Event()
    release_writer = threading.Event()

    def writer():
        with lock.exclusive():
            writer_inside.set()
            release_writer.wait(timeout=10)

    def reader():
        assert writer_inside.wait(timeout=10)
        with lock.shared():
            reader_entered.set()

    t_w = threading.Thread(target=writer)
    t_r = threading.Thread(target=reader)
    t_w.start()
    t_r.start()
    try:
        assert writer_inside.wait(timeout=10)
        # Reader must not get in while the writer holds the lock.
        assert not reader_entered.wait(timeout=1)
    finally:
        release_writer.set()
        t_w.join(timeout=10)
        t_r.join(timeout=10)
    assert reader_entered.is_set()


def test_writer_waits_for_in_flight_shared_holders():
    """The property this lock exists to provide."""
    lock = RWLock()
    reader_inside = threading.Event()
    writer_entered = threading.Event()
    release_reader = threading.Event()

    def reader():
        with lock.shared():
            reader_inside.set()
            release_reader.wait(timeout=10)

    def writer():
        assert reader_inside.wait(timeout=10)
        with lock.exclusive():
            writer_entered.set()

    t_r = threading.Thread(target=reader)
    t_w = threading.Thread(target=writer)
    t_r.start()
    t_w.start()
    try:
        assert reader_inside.wait(timeout=10)
        assert not writer_entered.wait(timeout=1)
    finally:
        release_reader.set()
        t_r.join(timeout=10)
        t_w.join(timeout=10)
    assert writer_entered.is_set()


def test_shared_acquisition_is_reentrant():
    """A thread already holding shared must not deadlock against a waiting writer."""
    lock = RWLock()
    outer_held = threading.Event()
    writer_waiting = threading.Event()
    inner_done = threading.Event()

    def nested_reader():
        with lock.shared():
            outer_held.set()
            assert writer_waiting.wait(timeout=10)
            with lock.shared():  # must not queue behind the waiting writer
                inner_done.set()

    def writer():
        assert outer_held.wait(timeout=10)
        writer_waiting.set()
        with lock.exclusive():
            pass

    assert not _spawn([nested_reader, writer])
    assert inner_done.is_set()


def test_upgrade_raises_instead_of_deadlocking():
    lock = RWLock()

    def upgrade():
        with lock.exclusive():
            pass  # pragma: no cover

    with lock.shared(), pytest.raises(RuntimeError, match='shared'):
        upgrade()


def test_downgrade_hands_off_without_a_gap():
    """No writer may intervene between the exclusive body and the shared hold."""
    lock = RWLock()
    downgraded = threading.Event()
    release = threading.Event()
    order: list[str] = []

    def downgrader():
        with lock.exclusive() as downgrade:
            order.append('exclusive')
            downgrade()
            downgraded.set()
            release.wait(timeout=10)
            order.append('shared-end')

    def writer():
        assert downgraded.wait(timeout=10)
        with lock.exclusive():
            order.append('second-writer')

    t_d = threading.Thread(target=downgrader)
    t_w = threading.Thread(target=writer)
    t_d.start()
    t_w.start()
    try:
        assert downgraded.wait(timeout=10)
        # The second writer must wait for the downgraded shared hold to end.
        assert order == ['exclusive']
    finally:
        release.set()
        t_d.join(timeout=10)
        t_w.join(timeout=10)
    assert order == ['exclusive', 'shared-end', 'second-writer']


def test_exclusive_released_on_exception():
    lock = RWLock()
    with pytest.raises(ZeroDivisionError), lock.exclusive():
        raise ZeroDivisionError
    with lock.exclusive():
        pass  # would hang if the lock leaked


def test_downgraded_shared_released_on_exception():
    lock = RWLock()
    with pytest.raises(ZeroDivisionError), lock.exclusive() as downgrade:
        downgrade()
        raise ZeroDivisionError
    with lock.exclusive():
        pass  # would hang if the shared hold leaked


def test_exclusive_after_shared_is_allowed_in_one_thread():
    """Releasing a shared hold must not leave the reentrancy guard armed."""
    lock = RWLock()
    with lock.shared():
        pass
    with lock.exclusive():  # would raise if the shared depth leaked
        pass


def test_exclusive_after_downgraded_shared_is_allowed_in_one_thread():
    lock = RWLock()
    with lock.exclusive() as downgrade:
        downgrade()
    with lock.exclusive():
        pass
