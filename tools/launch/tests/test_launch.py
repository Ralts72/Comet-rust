import importlib.util
import json
import plistlib
import sys
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
spec = importlib.util.spec_from_file_location("comet_launch", Path(__file__).resolve().parents[1] / "launch.py")
launch = importlib.util.module_from_spec(spec)
spec.loader.exec_module(launch)


def icns(payload=b"icon"):
    return b"icns" + (8 + len(payload)).to_bytes(4, "big") + payload


def mock_codesign(command, **_):
    if "--force" in command:
        binary = Path(command[-1]) / "Contents/MacOS/comet-editor"
        binary.write_bytes(binary.read_bytes() + b" signed")


class ProgramPreparation(unittest.TestCase):
    def test_windows_cargo_discovery_and_child_exit_status(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(launch.sys, "platform", "win32"), \
             patch.object(launch.shutil, "which", return_value=None), \
             patch.dict(launch.os.environ, {"CARGO_HOME": directory}):
            cargo = Path(directory) / "bin/cargo.exe"
            cargo.parent.mkdir()
            cargo.write_bytes(b"cargo placeholder")
            cargo.chmod(0o755)
            self.assertEqual(launch.cargo_path(), str(cargo))
        command = ["C:/Comet/comet-editor.exe", "--project", "C:/项目"]
        with patch.object(launch.sys, "platform", "win32"), \
             patch.object(launch.subprocess, "run") as run, patch.object(launch.os, "execv") as execute:
            run.return_value.returncode = 7
            with self.assertRaises(SystemExit) as result:
                launch.launch_editor(command)
            self.assertEqual(result.exception.code, 7)
            run.assert_called_once_with(command)
            execute.assert_not_called()

    def test_build_identity_tracks_uncommitted_sources_and_lockfile_not_project_assets(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(launch, "ROOT", Path(directory)):
            root = Path(directory)
            for name in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "tools/launch/launch.py", "tools/launch/windows_icon.py", "engine/core/src/lib.rs", "engine/asset/pipeline/src/lib.rs"):
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("initial")
            before = launch.source_fingerprint()
            (root / "engine/core/src/lib.rs").write_text("uncommitted change")
            changed = launch.source_fingerprint()
            self.assertNotEqual(before, changed)
            (root / "demo/assets").mkdir(parents=True)
            (root / "demo/assets/main.scene").write_text("project data")
            self.assertEqual(changed, launch.source_fingerprint())
            (root / "Cargo.lock").write_text("new resolution")
            self.assertNotEqual(changed, launch.source_fingerprint())
            font=root / "engine/application/resources/fonts/ui.otf"
            font.parent.mkdir(parents=True);font.write_bytes(b"font data")
            before_font=launch.source_fingerprint();font.write_bytes(b"changed font data")
            self.assertNotEqual(before_font,launch.source_fingerprint())
            before_helper = launch.source_fingerprint()
            (root / "tools/launch/windows_icon.py").write_text("updated")
            self.assertNotEqual(before_helper, launch.source_fingerprint())
            before_pipeline = launch.source_fingerprint()
            (root / "engine/asset/pipeline/src/lib.rs").write_text("updated pipeline")
            self.assertNotEqual(before_pipeline, launch.source_fingerprint())

    def test_snapshots_are_reused_preserved_and_repaired_without_overwriting_old_files(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(launch, "ROOT", Path(directory)):
            root = Path(directory)
            editor, app = root / "comet-editor", root / "comet-app"
            notice=root / "engine/application/resources/fonts/OFL.txt"
            notice.parent.mkdir(parents=True);notice.write_text("font license")
            for name in ("app/resources/icons/app.icns", "editor/resources/icons/editor.icns"):
                icon = root / name
                icon.parent.mkdir(parents=True)
                icon.write_bytes(icns())
            editor.write_bytes(b"debug editor")
            app.write_bytes(b"release app")
            build_id = "a" * 64
            info = json.dumps({"identity": {"build_id": build_id}, "debug": False})
            with patch.object(launch, "cargo_path", return_value="cargo"), \
                 patch.object(launch.shutil, "which", return_value="rustup"), \
                 patch.object(launch.subprocess, "run", side_effect=mock_codesign), \
                 patch.object(launch.subprocess, "check_output", return_value=info), \
                 patch.object(launch, "source_fingerprint", return_value=build_id), \
                 patch.object(launch, "prepare_windows_program", side_effect=lambda executable, icon: executable), \
                 patch.object(launch, "build_program", side_effect=lambda cargo, package, release, environment: app if release else editor):
                first = launch.prepare_programs()
                self.assertEqual(first["template"], launch.prepare_programs()["template"])
                self.assertEqual((first["template"].parent / "licenses/NotoSansSC-OFL.txt").read_text(),"font license")
                self.assertEqual((first["template"].parent / "icons/app.icns").read_bytes(), icns())
                manifest = json.loads(first["template"].read_text())
                self.assertEqual(manifest["version"], 4)
                self.assertEqual(manifest["editor_sha256"], launch.file_digest(first["editor"]))
                app.write_bytes(b"another cargo build")
                second = launch.prepare_programs()
                self.assertNotEqual(first["template"], second["template"])
                self.assertEqual(first["app"].read_bytes(), b"release app")
                second["app"].write_bytes(b"corrupted snapshot")
                repaired = launch.prepare_programs()
                self.assertNotEqual(second["template"], repaired["template"])
                self.assertEqual(repaired["app"].read_bytes(), b"another cargo build")
                self.assertEqual(repaired["template"], launch.prepare_programs()["template"])
                (repaired["template"].parent / "icons/app.icns").write_bytes(icns(b"corrupted"))
                fixed = launch.prepare_programs()
                self.assertNotEqual(fixed["template"], repaired["template"])
                self.assertEqual((fixed["template"].parent / "icons/app.icns").read_bytes(), icns())

    def test_windows_icon_snapshots_preserve_cargo_outputs_and_failed_candidates(self):
        def apply(binary, data):
            binary.write_bytes(binary.read_bytes() + b" embedded:" + data)
        def verify(binary, data):
            if not binary.read_bytes().endswith(b" embedded:" + data):
                raise ValueError("icon mismatch")
        with tempfile.TemporaryDirectory() as directory, patch.object(launch, "ROOT", Path(directory)), \
             patch.object(launch.windows_icon, "apply_icon", side_effect=apply), \
             patch.object(launch.windows_icon, "verify_icon", side_effect=verify):
            root = Path(directory)
            executable = root / "target/debug/comet-editor.exe"
            executable.parent.mkdir(parents=True)
            executable.write_bytes(b"original Cargo executable")
            icon = root / "editor.ico"
            icon.write_bytes((Path(__file__).resolve().parents[3] / "editor/resources/icons/editor.ico").read_bytes())
            first = launch.prepare_windows_program(executable, icon)
            self.assertEqual(executable.read_bytes(), b"original Cargo executable")
            self.assertNotEqual(first.read_bytes(), executable.read_bytes())
            receipt = json.loads((first.parent / "program.json").read_text())
            self.assertEqual(receipt["executable_sha256"], launch.file_digest(first))
            self.assertEqual(launch.prepare_windows_program(executable, icon), first)
            first.write_bytes(b"corrupted old snapshot")
            repaired = launch.prepare_windows_program(executable, icon)
            self.assertNotEqual(repaired, first)
            self.assertEqual(first.read_bytes(), b"corrupted old snapshot")
            executable.write_bytes(b"next Cargo executable")
            snapshots = root / "target/windows-programs"
            before = {p.relative_to(snapshots): p.read_bytes() for p in snapshots.rglob("*") if p.is_file()}
            with patch.object(launch.windows_icon, "apply_icon", side_effect=OSError("resource write failed")):
                with self.assertRaises(OSError):
                    launch.prepare_windows_program(executable, icon)
            self.assertEqual(before, {p.relative_to(snapshots): p.read_bytes() for p in snapshots.rglob("*") if p.is_file()})

    def test_macos_editor_app_signing_identity_and_snapshot_repair(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(launch, "ROOT", Path(directory)), \
             patch.object(launch.sys, "platform", "darwin"), \
             patch.object(launch.subprocess, "run", side_effect=mock_codesign):
            root = Path(directory)
            icon = root / "editor/resources/icons/editor.icns"
            editor = root / "target/debug/comet-editor"
            editor.parent.mkdir(parents=True)
            editor.write_bytes(b"debug editor")
            icon.parent.mkdir(parents=True)
            icon.write_bytes(icns())
            first = launch.prepare_editor_app(editor)
            self.assertEqual(first.read_bytes(), b"debug editor signed")
            self.assertEqual(editor.read_bytes(), b"debug editor")
            plist = plistlib.loads((first.parents[1] / "Info.plist").read_bytes())
            self.assertEqual(plist["CFBundleIconFile"], "editor.icns")
            self.assertEqual(launch.prepare_editor_app(editor), first)
            (first.parents[1] / "Resources/editor.icns").write_bytes(icns(b"changed"))
            repaired = launch.prepare_editor_app(editor)
            self.assertNotEqual(repaired, first)
            self.assertEqual((repaired.parents[1] / "Resources/editor.icns").read_bytes(), icns())
            (repaired.parents[3] / "bundle.json").write_text("[]")
            repaired_again = launch.prepare_editor_app(editor)
            self.assertNotEqual(repaired_again, repaired)
            editor.write_bytes(b"new editor")
            updated = launch.prepare_editor_app(editor)
            self.assertNotEqual(updated, repaired)
            self.assertEqual(repaired.read_bytes(), b"debug editor signed")
            self.assertEqual(updated.read_bytes(), b"new editor signed")


if __name__ == "__main__":
    unittest.main()
