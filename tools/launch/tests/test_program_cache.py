import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from program_cache import ProgramCache, publish


class ProgramSlots(unittest.TestCase):
    def test_release_reuses_slot_and_recovers_interrupted_publication(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with ProgramCache(root) as cache:
                first = cache.slot
                previous = first / "previous"
                previous.mkdir()
                (previous / "program").write_bytes(b"last complete program")
                abandoned = first / ".prepare-interrupted.tmp"
                abandoned.mkdir()
                (abandoned / "partial").write_bytes(b"partial")
            with ProgramCache(root) as cache:
                self.assertEqual(cache.slot, first)
                self.assertEqual((first / "programs/program").read_bytes(), b"last complete program")
                self.assertFalse(abandoned.exists())
                candidate = first / "candidate"
                candidate.mkdir()
                (candidate / "program").write_bytes(b"new program")
                rename = Path.rename
                def fail_candidate(path, target):
                    if path == candidate:
                        raise OSError("publication failed")
                    return rename(path, target)
                with patch.object(Path, "rename", fail_candidate):
                    with self.assertRaises(OSError):
                        publish(first, candidate)
                self.assertEqual((first / "programs/program").read_bytes(), b"last complete program")
                publish(first, candidate)
                self.assertEqual((first / "programs/program").read_bytes(), b"new program")
                self.assertFalse(previous.exists())

    @unittest.skipIf(os.name == "nt", "POSIX inherited flock lease")
    def test_child_keeps_lease_after_launcher_closes_it(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            child = None
            try:
                with ProgramCache(root) as cache:
                    first = cache.slot
                    child = subprocess.Popen([sys.executable, "-B", "-c", "import sys; print('ready', flush=True); sys.stdin.read(1)"],
                                             stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True,
                                             pass_fds=(cache.lock.fileno(),))
                    self.assertEqual(child.stdout.readline().strip(), "ready")
                with ProgramCache(root) as other:
                    self.assertNotEqual(other.slot, first)
                child.communicate("x", timeout=10)
                self.assertEqual(child.returncode, 0)
                with ProgramCache(root) as reused:
                    self.assertEqual(reused.slot, first)
            finally:
                if child is not None and child.poll() is None:
                    child.kill()
                    child.communicate()


if __name__ == "__main__":
    unittest.main()
