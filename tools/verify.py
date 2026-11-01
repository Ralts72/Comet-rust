#!/usr/bin/env python3
"""Verify the current engine, editor, asset pipeline and standalone game package."""
import argparse
import hashlib
import json
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


def run(name, command, cwd=ROOT, timeout=300):
    print(f"{name}: running", flush=True)
    start = time.monotonic()
    with (OUT / f"{name}.log").open("w") as log:
        result = subprocess.run([str(a) for a in command], cwd=cwd, stdout=log, stderr=subprocess.STDOUT, timeout=timeout)
    checks[name] = {"status": "passed" if result.returncode == 0 else "failed", "command": [str(a) for a in command], "cwd": str(cwd), "seconds": round(time.monotonic() - start, 3)}
    report("running" if result.returncode == 0 else "failed")
    if result.returncode:
        print((OUT / f"{name}.log").read_text()[-12000:], file=sys.stderr)
        raise RuntimeError(f"{name} failed")


def inspect_frames(name, native):
    path = OUT / f"{name}.json"
    value = json.loads(path.read_text())
    assert value["submitted_frames"] == 8
    frames = value["frames"]
    assert frames and all(f["draw_count"] == 5 and f["worker_thread"] == "comet-render" for f in frames)
    assert any(f["number"] == 2 and not f["presented"] for f in frames), "zero-extent frame did not skip presentation"
    if native:
        assert any(f["presented"] for f in frames)
        assert len({f["token"]["target_generation"] for f in frames}) >= 3
    image = OUT / f"{name}.png"
    assert image.read_bytes().startswith(b"\x89PNG\r\n\x1a\n") and image.stat().st_size > 4096
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
    files = [ROOT / name for name in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "AGENTS.md", "README.md", "editor.sh", "run_game.sh", ".cargo/config.toml", ".gitignore", ".gitattributes", ".editorconfig")]
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
        run("build_apps", ["cargo", "build", "-p", "comet-editor", "-p", "comet-player", "-p", "comet-assetc", "--locked", "--message-format=json-render-diagnostics"], timeout=900)
        artifacts = executables(OUT / "build_apps.log")
        case = Path(tempfile.mkdtemp(prefix="verification-", dir=OUT))
        project, products = case / "project", case / "products"
        shutil.copytree(ROOT / "demo", project, ignore=shutil.ignore_patterns(".assetc.lock"))
        assetc = artifacts["comet-assetc"]
        scene = "40000000-0000-4000-8000-000000000001"
        run("prepare", [assetc, "prepare", project, "--output", products])
        if args.gpu or args.native:
            editor = artifacts["comet-editor"]
            for native in ([False, True] if args.native else [False]):
                name = "editor-native" if native else "editor-headless"
                command = [ROOT / "editor.sh" if native else editor, "--project", project, "--products", products, "--smoke", "--report", OUT / f"{name}.json", "--capture", OUT / f"{name}.png"]
                if not native:
                    command.append("--headless")
                run(name, command)
                inspect_frames(name, native)
        player = artifacts["comet-player"]
        if args.native:
            run("release_game", [ROOT / "run_game.sh", "--products", products, "--scene", scene, "--smoke", "--report", OUT / "release-game.json", "--capture", OUT / "release-game.png"], cwd=case)
            inspect_frames("release-game", True)
            run("release_build", ["cargo", "build", "-p", "comet-player", "--release", "--locked", "--message-format=json-render-diagnostics"])
            player = executables(OUT / "release_build.log")["comet-player"]
        built = case / "package-build"
        run("pack", [assetc, "pack", products, scene, "--player", player, "--output", built])
        relocated = case / "relocated-game"
        built.rename(relocated)
        # Only this run's disposable source copy and generated cache are removed.
        shutil.rmtree(project)
        shutil.rmtree(products)
        empty = case / "empty-cwd"
        empty.mkdir()
        run("standalone_products", [assetc, "inspect", relocated / "products", scene], cwd=empty)
        if args.gpu or args.native:
            for native in ([False, True] if args.native else [False]):
                name = "player-native" if native else "player-headless"
                command = [relocated / "comet-player", "--smoke", "--report", OUT / f"{name}.json", "--capture", OUT / f"{name}.png"]
                if not native:
                    command.append("--headless")
                run(name, command, cwd=empty)
                inspect_frames(name, native)
        checks["package_isolation"] = {"status": "passed", "package": str(relocated), "authoring_exists": project.exists(), "development_cache_exists": products.exists(), "product_count": len(json.loads((relocated / "products/index.json").read_text())["products"])}
        assert checks["package_isolation"]["product_count"] == 12
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
