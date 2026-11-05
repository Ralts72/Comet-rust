import importlib.util
import io
import tempfile
import tarfile
import unittest
from pathlib import Path
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("comet_linux_sdk", Path(__file__).resolve().parents[1] / "linux_sdk.py")
sdk = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sdk)


def deb(members):
    stream = io.BytesIO()
    with tarfile.open(fileobj=stream, mode="w:xz") as archive:
        for name, value, kind in members:
            entry = tarfile.TarInfo(name)
            entry.type = kind
            if kind == tarfile.SYMTYPE:
                entry.linkname = value
                archive.addfile(entry)
            else:
                entry.size = len(value)
                archive.addfile(entry, io.BytesIO(value))
    payload = stream.getvalue()
    header = f"{'data.tar.xz/':<16}{0:<12}{0:<6}{0:<6}{'100644':<8}{len(payload):<10}`\n".encode()
    return b"!<arch>\n" + header + payload + (b"\n" if len(payload) % 2 else b"")


class LinuxSdkTests(unittest.TestCase):
    def test_cross_environment_overrides_both_target_spellings_without_changing_the_caller(self):
        variables = {f"PKG_CONFIG_{name}_{sdk.TARGET}": "/foreign" for name in ("LIBDIR", "PATH", "SYSROOT_DIR")}
        variables[f"PKG_CONFIG_{sdk.TARGET}"] = "/foreign/pkg-config"
        root = Path("/verified-sdk")
        with patch.dict(sdk.os.environ, variables), patch.object(sdk.subprocess, "check_output", return_value="host: aarch64-apple-darwin\n"), \
                patch.object(sdk.shutil, "which", return_value="/native/pkg-config"), patch.object(sdk, "prepare_sdk", return_value=root):
            result = sdk.environment(sdk.TARGET)
            for suffix in (sdk.TARGET, sdk.TARGET.replace("-", "_")):
                self.assertEqual(result[f"PKG_CONFIG_{suffix}"], "/native/pkg-config")
                self.assertEqual(result[f"PKG_CONFIG_LIBDIR_{suffix}"], str(root / "usr/lib/x86_64-linux-gnu/pkgconfig"))
                self.assertEqual(result[f"PKG_CONFIG_SYSROOT_DIR_{suffix}"], str(root))
                self.assertEqual(result[f"PKG_CONFIG_PATH_{suffix}"], "")
            for key, value in variables.items():
                self.assertEqual(sdk.os.environ[key], value)

    def test_extracts_only_data_and_preserves_internal_library_links(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            data = deb([("usr/lib/libasound.so.2", b"test library", tarfile.REGTYPE),
                        ("usr/lib/libasound.so", "libasound.so.2", tarfile.SYMTYPE)])
            sdk.extract_data(data, root)
            self.assertTrue((root / "usr/lib/libasound.so").is_symlink())
            self.assertEqual((root / "usr/lib/libasound.so").read_bytes(), b"test library")
            self.assertEqual(len(list(root.rglob("*"))), 4)

    def test_rejects_outside_files_links_truncation_and_expansion_budgets(self):
        cases = [deb([("../outside", b"replace", tarfile.REGTYPE)]),
                 deb([("/outside", b"replace", tarfile.REGTYPE)]),
                 deb([("escape", "../outside", tarfile.SYMTYPE)]),
                 deb([("data", b"12345678", tarfile.REGTYPE)]),
                 b"!<arch>\ntruncated"]
        for data in cases:
            with self.subTest(data=data[:20]), tempfile.TemporaryDirectory() as folder:
                parent = Path(folder)
                outside = parent / "outside"
                outside.write_bytes(b"untouched")
                root = parent / "sdk"
                root.mkdir()
                with patch.object(sdk, "MAX_EXTRACTED", 4), self.assertRaises(ValueError):
                    sdk.extract_data(data, root)
                self.assertEqual(outside.read_bytes(), b"untouched")


if __name__ == "__main__":
    unittest.main()
