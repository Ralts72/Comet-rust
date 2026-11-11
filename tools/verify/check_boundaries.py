#!/usr/bin/env python3
"""Audit normal/build edges between production modules and their domain boundaries."""
import argparse
import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
EXPECTED = {
    "comet-core": set(),
    "comet-scene": {"comet-core"},
    "comet-asset-data": {"comet-core", "comet-scene"},
    "comet-asset": {"comet-core", "comet-scene", "comet-asset-data"},
    "comet-asset-pipeline": {"comet-core", "comet-scene", "comet-asset-data", "comet-asset"},
    "comet-editor-core": {"comet-core", "comet-scene"},
    "comet-runtime": {"comet-core", "comet-scene", "comet-asset-data", "comet-audio"},
    "comet-audio": {"comet-core", "comet-asset-data"},
    "comet-render": {"comet-core", "comet-asset-data"},
    "comet-application": {"comet-core", "comet-scene", "comet-asset-data", "comet-asset", "comet-render", "comet-audio"},
    "comet-editor": {"comet-asset-data", "comet-core", "comet-scene", "comet-asset", "comet-asset-pipeline", "comet-editor-core", "comet-runtime", "comet-application", "comet-render"},
    "comet-app": {"comet-asset-data", "comet-core", "comet-scene", "comet-asset", "comet-runtime", "comet-application"},
    "comet-assetc": {"comet-core", "comet-asset", "comet-asset-pipeline"},
    "comet-gpu-check": set(),
}
DATA = {"comet-core", "comet-scene", "comet-asset-data", "comet-asset", "comet-asset-pipeline", "comet-editor-core", "comet-assetc"}


def audit(target=None):
    if target is None:
        target = next(line.split(": ", 1)[1] for line in subprocess.check_output(["rustc", "-vV"], text=True).splitlines() if line.startswith("host: "))
    metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--format-version", "1", "--locked", "--filter-platform", target], cwd=ROOT))
    packages = {p["id"]: p for p in metadata["packages"]}
    nodes = {n["id"]: n for n in metadata["resolve"]["nodes"]}
    members = {packages[p]["name"] for p in metadata["workspace_members"]}
    if members != set(EXPECTED):
        raise RuntimeError(f"workspace modules differ from the declared architecture: {members ^ set(EXPECTED)}")
    graph = {}
    for name, allowed in EXPECTED.items():
        root = next(p["id"] for p in packages.values() if p["name"] == name)
        seen, pending = set(), [root]
        while pending:
            node = pending.pop()
            if node in seen:
                continue
            seen.add(node)
            pending.extend(d["pkg"] for d in nodes[node]["deps"] if any(k["kind"] != "dev" for k in d["dep_kinds"]))
        names = {packages[p]["name"] for p in seen}
        forbidden = {"wgpu", "wgpu-core", "cc", "cmake"}
        if name in DATA:
            forbidden |= {"winit", "ash", "objc2-metal", "egui", "rapier3d", "rhai", "rune"}
        if name != "comet-editor":
            forbidden.add("egui_tiles")
        if name not in {"comet-runtime", "comet-editor", "comet-app"}:
            forbidden.add("rapier3d")
            forbidden.add("landmass")
        if name not in {"comet-asset-pipeline", "comet-editor", "comet-assetc"}:
            forbidden.add("rerecast")
        if name not in {"comet-audio", "comet-runtime", "comet-application", "comet-editor", "comet-app"}:
            forbidden |= {"kira", "cpal"}
        if name not in {"comet-asset-pipeline", "comet-editor", "comet-assetc"}:
            forbidden.add("hound")
        if name == "comet-audio":
            forbidden |= {"winit", "ash", "objc2-metal", "egui", "rapier3d", "rhai", "comet-asset", "comet-asset-pipeline", "comet-runtime", "comet-editor-core"}
        if name == "comet-runtime":
            forbidden |= {"winit", "ash", "objc2-metal", "egui", "comet-asset-pipeline", "comet-editor-core"}
        if name == "comet-render":
            forbidden |= {"winit", "comet-asset", "comet-asset-pipeline", "comet-editor-core", "comet-runtime", "comet-application"}
        if name == "comet-app":
            forbidden |= {"comet-asset-pipeline", "comet-editor-core", "comet-editor", "gltf", "image"}
        if names & forbidden:
            raise RuntimeError(f"{name} reaches forbidden dependencies: {sorted(names & forbidden)}")
        direct = {packages[d["pkg"]]["name"] for d in nodes[root]["deps"] if any(k["kind"] != "dev" for k in d["dep_kinds"]) and packages[d["pkg"]]["name"].startswith("comet-")}
        if direct != allowed:
            raise RuntimeError(f"{name} dependencies: {direct}, expected {allowed}")
        direct_packages={packages[d["pkg"]]["name"] for d in nodes[root]["deps"] if any(k["kind"] != "dev" for k in d["dep_kinds"])}
        if name != "comet-core" and "glam" in direct_packages:
            raise RuntimeError(f"{name} must use comet_core::math instead of depending directly on glam")
        if name != "comet-runtime" and "rhai" in direct_packages:
            raise RuntimeError(f"{name} must use the runtime script API instead of depending directly on rhai")
        if name != "comet-runtime" and "landmass" in direct_packages:
            raise RuntimeError(f"{name} must use the runtime navigation API instead of depending directly on landmass")
        if name != "comet-asset-pipeline" and "rerecast" in direct_packages:
            raise RuntimeError(f"{name} must use the asset pipeline navigation API instead of depending directly on rerecast")
        if name != "comet-audio" and direct_packages & {"kira", "cpal"}:
            raise RuntimeError(f"{name} must use the engine audio API instead of depending directly on Kira or CPAL")
        if name != "comet-asset-pipeline" and "hound" in direct_packages:
            raise RuntimeError(f"{name} must use the asset pipeline audio API instead of depending directly on Hound")
        graph[name] = {"direct_modules": sorted(direct), "transitive_packages": len(seen)}
    return {"target": target, "modules": graph}


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--target")
    args = parser.parse_args()
    print(json.dumps(audit(args.target), indent=2))
