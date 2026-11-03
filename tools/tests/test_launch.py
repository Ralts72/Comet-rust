import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("comet_launch", Path(__file__).resolve().parents[1] / "launch.py")
launch = importlib.util.module_from_spec(spec)
spec.loader.exec_module(launch)


class ProgramPreparation(unittest.TestCase):
    def test_build_identity_tracks_uncommitted_sources_and_lockfile_not_project_assets(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(launch, "ROOT", Path(directory)):
            root = Path(directory)
            for name in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "tools/launch.py", "engine/core/src/lib.rs"):
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

    def test_snapshots_are_reused_preserved_and_repaired_without_overwriting_old_files(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(launch, "ROOT", Path(directory)):
            root = Path(directory)
            editor, player = root / "comet-editor", root / "comet-player"
            notice=root / "engine/application/resources/fonts/OFL.txt"
            notice.parent.mkdir(parents=True);notice.write_text("font license")
            editor.write_bytes(b"debug editor")
            player.write_bytes(b"release player")
            build_id = "a" * 64
            info = json.dumps({"identity": {"build_id": build_id}, "debug": False})
            with patch.object(launch, "cargo_path", return_value="cargo"), \
                 patch.object(launch.shutil, "which", return_value="rustup"), \
                 patch.object(launch.subprocess, "run"), \
                 patch.object(launch.subprocess, "check_output", return_value=info), \
                 patch.object(launch, "source_fingerprint", return_value=build_id), \
                 patch.object(launch, "build_program", side_effect=lambda cargo, package, release, environment: player if release else editor):
                first = launch.prepare_programs()
                self.assertEqual(first["template"], launch.prepare_programs()["template"])
                self.assertEqual((first["template"].parent / "licenses/NotoSansSC-OFL.txt").read_text(),"font license")
                player.write_bytes(b"another cargo build")
                second = launch.prepare_programs()
                self.assertNotEqual(first["template"], second["template"])
                self.assertEqual(first["player"].read_bytes(), b"release player")
                second["player"].write_bytes(b"corrupted snapshot")
                repaired = launch.prepare_programs()
                self.assertNotEqual(second["template"], repaired["template"])
                self.assertEqual(repaired["player"].read_bytes(), b"another cargo build")
                self.assertEqual(repaired["template"], launch.prepare_programs()["template"])


if __name__ == "__main__":
    unittest.main()
