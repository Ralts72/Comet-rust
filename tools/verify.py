#!/usr/bin/env python3
"""Verify the current engine, editor, asset pipeline and standalone game package."""
import argparse
import hashlib
import json
import math
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "target/verification"
checks = {}


def report(status):
    (OUT / "verification.json").write_text(json.dumps({"status": status, "checks": checks, "unverified": ["native Vulkan execution", "Windows", "physical mouse drag (synthetic egui drag regression passes)"]}, indent=2) + "\n")


def run(name, command, cwd=ROOT, timeout=300, expect_failure=False):
    print(f"{name}: running", flush=True)
    start = time.monotonic()
    with (OUT / f"{name}.log").open("w") as log:
        result = subprocess.run([str(a) for a in command], cwd=cwd, stdout=log, stderr=subprocess.STDOUT, timeout=timeout)
    passed = (result.returncode != 0) if expect_failure else (result.returncode == 0)
    checks[name] = {"status": "passed" if passed else "failed", "command": [str(a) for a in command], "cwd": str(cwd), "seconds": round(time.monotonic() - start, 3)}
    report("running" if passed else "failed")
    if not passed:
        print((OUT / f"{name}.log").read_text()[-12000:], file=sys.stderr)
        raise RuntimeError(f"{name} failed")


def inspect_frames(name, native):
    path = OUT / f"{name}.json"
    value = json.loads(path.read_text())
    assert value["submitted_frames"] == 8
    frames = value["frames"]
    assert frames and all(f["draw_count"] == 9 and f["worker_thread"] == "comet-render" for f in frames)
    assert any(f["number"] == 2 and not f["presented"] for f in frames), "zero-extent frame did not skip presentation"
    if native:
        assert any(f["presented"] for f in frames)
        assert len({f["token"]["target_generation"] for f in frames}) >= 3
    image = OUT / f"{name}.png"
    assert image.read_bytes().startswith(b"\x89PNG\r\n\x1a\n") and image.stat().st_size > 4096
    runtime = value["application"]
    if "steps" in runtime:
        assert math.isclose(runtime["elapsed"], runtime["steps"] / 60, abs_tol=1e-9)
        assert 0 <= runtime["frame_steps"] <= 8
        assert math.isfinite(runtime["dropped_time"]) and runtime["dropped_time"] >= 0
        assert 0 <= runtime["interpolation_alpha"] < 1
        if value["headless"]:
            assert runtime["steps"] == 7, "eight smoke frames must execute seven fixed ticks after the startup baseline"
            assert runtime["dropped_time"] == 0
            character = runtime["character"]
            assert character is not None and all(math.isfinite(v) for v in character["position"])
            assert 0.8 < character["position"][1] < 1, "packaged character must actually fall during the smoke run"
            assert character["vertical_velocity"] < 0
    checks[f"{name}_evidence"] = {"status": "passed", "submitted_frames": 8, "recorded_frames": len(frames), "png_sha256": hashlib.sha256(image.read_bytes()).hexdigest()}


def executables(log):
    """Use Cargo's actual artifacts, including configured target directories/triples."""
    artifacts = {}
    for line in log.read_text().splitlines():
        try:
            message = json.loads(line)
        except ValueError:
            continue
        if isinstance(message, dict) and message.get("reason") == "compiler-artifact" and message.get("executable") and "bin" in message["target"]["kind"]:
            artifacts[message["target"]["name"]] = Path(message["executable"])
    return artifacts


def freeze_sources():
    files = [ROOT / name for name in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "AGENTS.md", "README.md", "start.sh", ".cargo/config.toml", ".gitignore", ".gitattributes", ".editorconfig")]
    for directory in ("engine", "editor", "app", "tools", "demo", "tests", ".github"):
        files.extend(path for path in (ROOT / directory).rglob("*") if path.is_file() and "__pycache__" not in path.parts and path.name != ".assetc.lock")
    files.extend((ROOT / "docs").glob("*.md"))
    sources = {str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest() for path in sorted(files)}
    manifest = {"sources_sha256": sources, "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(), "cargo": subprocess.check_output(["cargo", "--version"], text=True).strip()}
    (OUT / "source-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--gpu", action="store_true", help="Run actual offscreen GPU frames")
    parser.add_argument("--native", action="store_true", help="Also open and close native Editor/Player windows")
    parser.add_argument("--linux", action="store_true", help="Cross-check both apps and Vulkan for installed Linux target")
    args = parser.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    report("running")
    try:
        run("format", ["cargo", "fmt", "--all", "--check"])
        run("tests", ["cargo", "test", "--workspace", "--locked"], timeout=900)
        run("clippy", ["cargo", "clippy", "--workspace", "--all-targets", "--locked", "--", "-D", "warnings"], timeout=900)
        run("boundaries", [sys.executable, "tools/check_boundaries.py"])
        run("dependency_audit", [sys.executable, "tools/audit_dependencies.py"])
        run("python_tests", [sys.executable, "-m", "unittest", "discover", "-s", "tools/tests"])
        run("start_help", [ROOT / "start.sh", "--help"])
        run("build_asset_tool", ["cargo", "build", "-p", "comet-assetc", "--locked", "--message-format=json-render-diagnostics"], timeout=900)
        assetc = executables(OUT / "build_asset_tool.log")["comet-assetc"]
        prepare_code = ("import sys,json;from pathlib import Path;"
                        f"sys.path.insert(0,{str(ROOT / 'tools')!r});import launch;"
                        f"Path({str(OUT / 'programs.json')!r}).write_text(json.dumps(launch.prepare_programs(),default=str,indent=2))")
        run("program_templates", [sys.executable, "-c", prepare_code], timeout=900)
        programs = json.loads((OUT / "programs.json").read_text())
        editor, template = Path(programs["editor"]), Path(programs["template"])
        case = Path(tempfile.mkdtemp(prefix="verification-", dir=OUT))
        project, products = case / "project", case / "products"
        shutil.copytree(ROOT / "demo", project, ignore=shutil.ignore_patterns(".assetc.lock"))
        scene = "40000000-0000-4000-8000-000000000001"
        extra = "70000000-0000-4000-8000-000000000001"
        (project / "assets/extra.scene").write_text(json.dumps({"version": 1, "base": {"version": 2, "entities": []}, "instances": []}))
        (project / "assets/extra.scene.meta").write_text(json.dumps({"version": 1, "id": extra}))
        run("prepare", [assetc, "prepare", project, "--output", products])
        if args.gpu or args.native:
            for native in ([False, True] if args.native else [False]):
                name = "editor-native" if native else "editor-headless"
                command = ([ROOT / "start.sh"] if native else [editor, "--player-template", template])
                command += ["--project", project, "--products", products, "--smoke", "--report", OUT / f"{name}.json", "--capture", OUT / f"{name}.png"]
                if not native:
                    command.append("--headless")
                run(name, command, cwd=case)
                inspect_frames(name, native)
        built = case / ("package-build.app" if sys.platform == "darwin" else "package-build")
        run("editor_export", [editor, "--player-template", template, "--project", project, "--products", products,
                              "--export-game", built, "--include-scene", extra, "--report", OUT / "editor-export.json"], cwd=case)
        exported = json.loads((OUT / "editor-export.json").read_text())
        assert exported["status"] == "passed"
        built = Path(exported["root"])
        executable_relative = Path(exported["executable"]).relative_to(built)
        products_relative = Path(exported["products"]).relative_to(built)
        manifest_relative = products_relative.parent / "package.json"
        manifest = json.loads((built / manifest_relative).read_text())
        assert set(manifest["scenes"]) == {scene, extra} and manifest["entry_scene"] == scene
        assert manifest["runtime"]["build_id"] == programs["build_id"]
        template_manifest = json.loads(template.read_text())
        assert template_manifest["version"] == 2
        for name, expected in template_manifest["notices"].items():
            assert hashlib.sha256((built / manifest_relative.parent / "licenses" / name).read_bytes()).hexdigest() == expected
        original_manifest = (built / manifest_relative).read_bytes()
        run("existing_export_rejected", [editor, "--player-template", template, "--project", project, "--products", products, "--export-game", built], expect_failure=True)
        assert (built / manifest_relative).read_bytes() == original_manifest
        bad_template = case / "mismatched-template.json"
        bad = json.loads(template.read_text())
        bad["identity"]["build_id"] = "0" * 64
        bad_template.write_text(json.dumps(bad))
        rejected_output = case / "must-not-publish"
        run("template_mismatch_rejected", [editor, "--player-template", bad_template, "--project", project, "--products", products, "--export-game", rejected_output], expect_failure=True)
        assert not rejected_output.exists() and "start.sh" in (OUT / "template_mismatch_rejected.log").read_text()
        relocated = case / ("relocated-game.app" if sys.platform == "darwin" else "relocated-game")
        built.rename(relocated)
        shutil.rmtree(project)
        shutil.rmtree(products)
        empty = case / "empty-cwd"
        empty.mkdir()
        run("standalone_products", [assetc, "inspect", relocated / products_relative, scene], cwd=empty)
        player = relocated / executable_relative
        run("standalone_runtime_identity", [player, "--template-info"], cwd=empty)
        identity = json.loads((OUT / "standalone_runtime_identity.log").read_text())
        assert identity["identity"] == manifest["runtime"] and not identity["debug"]
        if args.gpu or args.native:
            for native in ([False, True] if args.native else [False]):
                name = "player-native" if native else "player-headless"
                command = [player, "--smoke", "--report", OUT / f"{name}.json", "--capture", OUT / f"{name}.png"]
                if not native:
                    command.append("--headless")
                run(name, command, cwd=empty)
                inspect_frames(name, native)
        checks["package_isolation"] = {"status": "passed", "package": str(relocated), "authoring_exists": project.exists(), "development_cache_exists": products.exists(), "included_scenes": manifest["scenes"], "product_count": len(json.loads((relocated / products_relative / "index.json").read_text())["products"])}
        assert checks["package_isolation"]["product_count"] == 13
        if args.linux:
            run("vulkan_linux", ["cargo", "check", "--workspace", "--target", "x86_64-unknown-linux-gnu", "--locked"])
            run("vulkan_linux_clippy", ["cargo", "clippy", "--workspace", "--all-targets", "--target", "x86_64-unknown-linux-gnu", "--locked", "--", "-D", "warnings"])
            run("linux_boundaries", [sys.executable, "tools/check_boundaries.py", "--target", "x86_64-unknown-linux-gnu"])
            run("linux_audit", [sys.executable, "tools/audit_dependencies.py", "--target", "x86_64-unknown-linux-gnu"])
        if args.gpu or args.native:
            run("gpu_capabilities", ["cargo", "run", "-p", "comet-gpu-check", "--locked", "--", "--output", case / "gpu"])
        freeze_sources()
        report("passed")
    except Exception:
        report("failed")
        raise
    print(f"Passed {len(checks)} checks. Evidence: {OUT}")


if __name__ == "__main__":
    main()
