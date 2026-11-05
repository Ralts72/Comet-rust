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
                launch.launch_editor(command, None)
            self.assertEqual(result.exception.code, 7)
            run.assert_called_once_with(command)
            execute.assert_not_called()
        with tempfile.TemporaryDirectory() as directory, launch.ProgramCache(directory) as cache, \
             patch.object(launch.sys, "platform", "linux"), patch.object(launch.subprocess, "run") as run:
            run.return_value.returncode = -15
            with self.assertRaises(SystemExit) as result:
                launch.launch_editor(command, cache)
            self.assertEqual(result.exception.code, 143)
            run.assert_called_once_with(command, pass_fds=(cache.lock.fileno(),))

    def test_build_identity_tracks_uncommitted_sources_and_lockfile_not_project_assets(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(launch, "ROOT", Path(directory)):
            root = Path(directory)
            for name in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "tools/launch/launch.py", "tools/launch/windows_icon.py", "tools/launch/program_cache.py", "tools/launch/build.rs", "engine/core/src/lib.rs", "engine/asset/pipeline/src/lib.rs"):
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
            before_tests = launch.source_fingerprint()
            for name in ("engine/core/src/tests.rs", "engine/core/src/ids/tests.rs", "engine/core/tests/identity.rs", "tools/asset/tests/cli.rs"):
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("test-only changes")
            self.assertEqual(before_tests, launch.source_fingerprint())
            (root / "tools/launch/build.rs").write_text("updated program identity injection")
            self.assertNotEqual(before_tests, launch.source_fingerprint())

    def test_program_pairs_reuse_slots_repair_corruption_and_keep_failed_candidates_private(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(launch, "ROOT", Path(directory)):
            root = Path(directory)
            editor, app = root / "comet-editor", root / "comet-app"
            notice = root / "engine/application/resources/fonts/OFL.txt"
            notice.parent.mkdir(parents=True)
            notice.write_text("font license")
            for name in ("app/resources/icons/app.icns", "editor/resources/icons/editor.icns"):
                icon = root / name
                icon.parent.mkdir(parents=True)
                icon.write_bytes(icns())
            editor.write_bytes(b"debug editor")
            app.write_bytes(b"release app")
            legacy = root / "target/editor-apps/old-hash/editor"
            legacy.parent.mkdir(parents=True)
            legacy.write_bytes(b"preserve existing cache")
            build_id = "a" * 64
            info = json.dumps({"identity": {"build_id": build_id}, "debug": False})
            with patch.object(launch.sys, "platform", "darwin"), \
                 patch.object(launch, "cargo_path", return_value="cargo"), \
                 patch.object(launch.shutil, "which", return_value="rustup"), \
                 patch.object(launch.subprocess, "run", side_effect=mock_codesign), \
                 patch.object(launch.subprocess, "check_output", return_value=info), \
                 patch.object(launch, "source_fingerprint", return_value=build_id), \
                 patch.object(launch, "build_program", side_effect=lambda cargo, package, release, environment: app if release else editor):
                def prepare(cache):
                    return launch.prepare_programs(cache.slot)
                with launch.ProgramCache(root / "target/launch") as first_cache:
                    first = prepare(first_cache)
                    self.assertEqual((first["template"].parent / "licenses/NotoSansSC-OFL.txt").read_text(), "font license")
                    self.assertEqual((first["template"].parent / "icons/app.icns").read_bytes(), icns())
                    manifest = json.loads(first["template"].read_text())
                    self.assertEqual(manifest["version"], 4)
                    self.assertEqual(manifest["editor_sha256"], launch.file_digest(first["editor"]))
                    with patch.object(launch.shutil, "copy2", side_effect=AssertionError("unchanged pair was copied")):
                        self.assertEqual(first, prepare(first_cache))
                    app.write_bytes(b"another cargo build")
                    with launch.ProgramCache(root / "target/launch") as second_cache:
                        second = prepare(second_cache)
                        self.assertNotEqual(first["template"], second["template"])
                        self.assertEqual(first["app"].read_bytes(), b"release app")
                        self.assertEqual(second["app"].read_bytes(), b"another cargo build")
                    self.assertEqual(first["app"].read_bytes(), b"release app")
                with launch.ProgramCache(root / "target/launch") as cache:
                    updated = prepare(cache)
                    self.assertEqual(updated, first)
                    self.assertEqual(updated["app"].read_bytes(), b"another cargo build")
                    for relative, damaged in (("app/comet-app", b"broken program"),
                                              ("app/icons/app.icns", icns(b"corrupt")),
                                              ("editor/bundle.json", b"[]"),
                                              ("prepared.json", b"[]")):
                        (cache.slot / "programs" / relative).write_bytes(damaged)
                        self.assertEqual(prepare(cache), first)
                        self.assertEqual(first["app"].read_bytes(), b"another cargo build")
                        self.assertEqual((first["template"].parent / "icons/app.icns").read_bytes(), icns())
                    before = {p.relative_to(cache.slot): p.read_bytes() for p in cache.slot.rglob("*") if p.is_file()}
                    app.write_bytes(b"new app that fails preparation")
                    with patch.object(launch.shutil, "copy2", side_effect=OSError("copy failed")):
                        with self.assertRaises(OSError):
                            prepare(cache)
                    self.assertEqual(before, {p.relative_to(cache.slot): p.read_bytes() for p in cache.slot.rglob("*") if p.is_file()})
                    for index in range(8):
                        app.write_bytes(f"app {index}".encode())
                        self.assertEqual(prepare(cache), first)
                    self.assertEqual(sorted(p.name for p in cache.slot.iterdir()), ["lease.lock", "programs"])
                self.assertEqual(sorted(p.name for p in (root / "target/launch").iterdir()), ["slot-0", "slot-1"])
                self.assertEqual(legacy.read_bytes(), b"preserve existing cache")

    def test_windows_icon_candidates_preserve_cargo_outputs_and_verify_resources(self):
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
            first = launch.prepare_windows_program(executable, icon, root / "candidate")
            self.assertEqual(executable.read_bytes(), b"original Cargo executable")
            self.assertNotEqual(first.read_bytes(), executable.read_bytes())
            receipt = json.loads((first.parent / "program.json").read_text())
            self.assertEqual(receipt["executable_sha256"], launch.file_digest(first))
            self.assertTrue(launch.windows_program_valid(first.parent, receipt["inputs"], icon.read_bytes()))
            first.write_bytes(b"corrupted candidate")
            self.assertFalse(launch.windows_program_valid(first.parent, receipt["inputs"], icon.read_bytes()))

    def test_linux_and_windows_pairs_use_copied_editors_and_reuse_complete_templates(self):
        for platform, suffix in (("linux", ""), ("win32", ".exe")):
            with self.subTest(platform=platform), tempfile.TemporaryDirectory() as directory, \
                 patch.object(launch, "ROOT", Path(directory)), patch.object(launch.sys, "platform", platform):
                root = Path(directory)
                editor, app = root / f"comet-editor{suffix}", root / f"comet-app{suffix}"
                editor.write_bytes(b"editor")
                app.write_bytes(b"app")
                for name in ("app/resources/icons/app.icns", "engine/application/resources/fonts/OFL.txt",
                             "editor/resources/icons/editor.ico", "app/resources/icons/app.ico"):
                    path = root / name
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_bytes(icns() if path.suffix == ".icns" else b"fixture")
                def apply(binary, data):
                    binary.write_bytes(binary.read_bytes() + b" icon")
                def verify(binary, data):
                    self.assertTrue(binary.read_bytes().endswith(b" icon"))
                with patch.object(launch, "cargo_path", return_value="cargo"), \
                     patch.object(launch.shutil, "which", return_value="rustup"), \
                     patch.object(launch.subprocess, "run"), \
                     patch.object(launch.subprocess, "check_output", return_value=json.dumps({"identity": {"build_id": "id"}, "debug": False})), \
                     patch.object(launch, "source_fingerprint", return_value="id"), \
                     patch.object(launch, "build_program", side_effect=lambda cargo, package, release, environment: app if release else editor), \
                     patch.object(launch.windows_icon, "icon_resources"), \
                     patch.object(launch.windows_icon, "apply_icon", side_effect=apply), \
                     patch.object(launch.windows_icon, "verify_icon", side_effect=verify), \
                     launch.ProgramCache(root / "target/launch") as cache:
                    programs = launch.prepare_programs(cache.slot)
                    self.assertNotEqual(programs["editor"], editor)
                    self.assertEqual(editor.read_bytes(), b"editor")
                    self.assertEqual(app.read_bytes(), b"app")
                    self.assertEqual(programs, launch.prepare_programs(cache.slot))
                    manifest = json.loads(programs["template"].read_text())
                    self.assertEqual(manifest["app_sha256"], launch.file_digest(programs["app"]))
                    self.assertEqual(manifest["editor_sha256"], launch.file_digest(programs["editor"]))

    def test_macos_editor_candidate_is_signed_without_changing_cargo_output(self):
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
            first = launch.prepare_editor_app(editor, root / "candidate")
            self.assertEqual(first.read_bytes(), b"debug editor signed")
            self.assertEqual(editor.read_bytes(), b"debug editor")
            plist = plistlib.loads((first.parents[1] / "Info.plist").read_bytes())
            self.assertEqual(plist["CFBundleIconFile"], "editor.icns")
            self.assertEqual(plist["LSMinimumSystemVersion"], "14.2")


if __name__ == "__main__":
    unittest.main()
