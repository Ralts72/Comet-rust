import ctypes
from pathlib import Path
import shutil
import stat
import struct
import sys
import tempfile
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import windows_icon


def ico(images=(b"first image", b"second image")):
    directory = bytearray(struct.pack("<HHH", 0, 1, len(images)))
    offset = 6 + len(images) * 16
    for size, image in zip((16, 32), images):
        directory.extend(struct.pack("<BBBBHHII", size, size, 0, 0, 1, 32, len(image), offset))
        offset += len(image)
    return bytes(directory) + b"".join(images)


class WindowsIcon(unittest.TestCase):
    def test_groups_use_resource_ids_and_preserve_original_images(self):
        resources = windows_icon.icon_resources(ico())
        self.assertEqual(resources[:2], [(3, 1, b"first image"), (3, 2, b"second image")])
        expected = (struct.pack("<HHH", 0, 1, 2)
                    + struct.pack("<BBBBHHIH", 16, 16, 0, 0, 1, 32, 11, 1)
                    + struct.pack("<BBBBHHIH", 32, 32, 0, 0, 1, 32, 12, 2))
        self.assertEqual(resources[2], (14, 1, expected))
        root = Path(__file__).resolve().parents[2]
        for role, name in (("editor", "editor"), ("app", "app")):
            data = (root / role / f"resources/icons/{name}.ico").read_bytes()
            self.assertEqual(len(windows_icon.icon_resources(data)), 8)

    def test_malformed_ico_and_lfs_pointers_are_rejected(self):
        valid = ico()
        inside_directory = bytearray(valid)
        struct.pack_into("<I", inside_directory, 18, 6)
        outside_file = bytearray(valid)
        struct.pack_into("<I", outside_file, 18, len(valid))
        for data in (b"", b"version https://git-lfs.github.com/spec/v1", valid[:20],
                     struct.pack("<HHH", 0, 1, 0), inside_directory, outside_file):
            with self.subTest(data=bytes(data[:20])):
                with self.assertRaises(ValueError):
                    windows_icon.icon_resources(data)

    def test_failed_resource_update_discards_the_transaction(self):
        api = Mock()
        api.BeginUpdateResourceW.return_value = 77
        api.UpdateResourceW.return_value = False
        with patch.object(windows_icon, "_api", return_value=api), \
             patch.object(ctypes, "get_last_error", return_value=5, create=True), \
             patch.object(ctypes, "WinError", return_value=OSError("access denied"), create=True), \
             patch.object(windows_icon, "verify_icon") as verify:
            with self.assertRaises(OSError):
                windows_icon.apply_icon(Path("private-candidate.exe"), ico())
            self.assertFalse(api.BeginUpdateResourceW.call_args.args[1])
            api.EndUpdateResourceW.assert_called_once_with(77, True)
            verify.assert_not_called()

    def test_missing_resource_section_is_a_valid_starting_point(self):
        api = Mock()
        api.EnumResourceLanguagesW.return_value = False
        with patch.object(ctypes, "get_last_error", return_value=1812, create=True):
            self.assertEqual(windows_icon._group_languages(api, 77), [0])

    def test_updates_all_existing_icon_languages_before_committing(self):
        from ctypes import wintypes
        api = Mock()
        api.BeginUpdateResourceW.return_value = 77
        factory = getattr(ctypes, "WINFUNCTYPE", ctypes.CFUNCTYPE)
        callback_type = factory(wintypes.BOOL, wintypes.HMODULE, ctypes.c_void_p, ctypes.c_void_p, wintypes.WORD, wintypes.LPARAM)
        def enumerate_languages(module, kind, name, callback, context):
            collect = ctypes.cast(callback, callback_type)
            collect(77, kind, name, 1033, context)
            return True
        api.EnumResourceLanguagesW.side_effect = enumerate_languages
        with patch.object(windows_icon, "_api", return_value=api), \
             patch.object(windows_icon, "verify_icon") as verify:
            windows_icon.apply_icon(Path("candidate.exe"), ico())
        self.assertEqual([call.args[3] for call in api.UpdateResourceW.call_args_list], [0, 0, 0, 1033, 1033, 1033])
        api.EndUpdateResourceW.assert_called_once_with(77, False)
        verify.assert_called_once()

    def test_readback_mismatch_always_releases_the_module(self):
        api = Mock()
        data = ctypes.create_string_buffer(b"wrong data")
        api.LoadLibraryExW.return_value = 77
        api.FindResourceExW.return_value = 88
        api.SizeofResource.return_value = len(b"wrong data")
        api.LoadResource.return_value = 99
        api.LockResource.return_value = ctypes.addressof(data)
        with patch.object(windows_icon, "_api", return_value=api):
            with self.assertRaises(ValueError):
                windows_icon.verify_icon(Path("candidate.exe"), ico())
        api.FreeLibrary.assert_called_once_with(77)

    @unittest.skipUnless(sys.platform == "win32", "requires native Windows resource APIs")
    def test_native_windows_resource_roundtrip_on_a_private_pe_copy(self):
        icon = (Path(__file__).resolve().parents[2] / "app/resources/icons/app.ico").read_bytes()
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "图标验收.exe"
            shutil.copy2(sys.executable, binary)
            binary.chmod(binary.stat().st_mode | stat.S_IWRITE)
            before = Path(sys.executable).read_bytes()
            windows_icon.apply_icon(binary, icon)
            windows_icon.verify_icon(binary, icon)
            self.assertEqual(Path(sys.executable).read_bytes(), before)


if __name__ == "__main__":
    unittest.main()
