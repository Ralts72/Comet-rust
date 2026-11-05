"""Reuse prepared program slots while keeping live sessions isolated."""
import errno
import os
from pathlib import Path
import shutil


class ProgramCache:
    def __init__(self, root):
        self.root = Path(root)
        self.lock = None

    def __enter__(self):
        self.root.mkdir(parents=True, exist_ok=True)
        index = 0
        while True:
            slot = self.root / f"slot-{index}"
            slot.mkdir(exist_ok=True)
            lock = (slot / "lease.lock").open("a+b")
            try:
                if os.name == "nt":
                    import msvcrt
                    if lock.seek(0, os.SEEK_END) == 0:
                        lock.write(b"\0")
                        lock.flush()
                    lock.seek(0)
                    msvcrt.locking(lock.fileno(), msvcrt.LK_NBLCK, 1)
                else:
                    import fcntl
                    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except OSError as error:
                lock.close()
                if error.errno not in (errno.EACCES, errno.EAGAIN, errno.EDEADLK):
                    raise
                index += 1
                continue
            self.lock = lock
            self.slot = slot
            try:
                # A crash between the two renames must not lose the last complete pair.
                previous = slot / "previous"
                if previous.exists() and not (slot / "programs").exists():
                    previous.rename(slot / "programs")
                for abandoned in slot.glob(".prepare-*.tmp"):
                    if abandoned.is_dir() and not abandoned.is_symlink():
                        shutil.rmtree(abandoned)
                return self
            except BaseException:
                self.__exit__(None, None, None)
                raise

    def __exit__(self, *_):
        if self.lock is not None:
            # Closing rather than explicitly unlocking preserves a POSIX child's lease.
            self.lock.close()
            self.lock = None


def publish(slot, candidate):
    """Publish a validated candidate while the caller holds the slot lease."""
    current, previous = slot / "programs", slot / "previous"
    if previous.exists():
        shutil.rmtree(previous)
    if current.exists():
        current.rename(previous)
    try:
        candidate.rename(current)
    except OSError:
        if previous.exists():
            previous.rename(current)
        raise
    if previous.exists():
        shutil.rmtree(previous)
    return current
