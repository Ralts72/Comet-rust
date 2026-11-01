#!/usr/bin/env python3
"""Audit the resolved target/features, not crates that merely occur in Cargo.lock."""
import argparse
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
BANNED = {"cc", "cmake", "cxx-build", "bindgen", "shaderc-sys", "glslang-sys", "spirv-reflect", "spirv-reflect-sys", "mlua-sys", "imgui-sys", "rmlui-sys", "recastnavigation-sys", "vk-mem", "jolt-sys", "wgpu", "wgpu-core", "rune"}
CRITICAL = {"bevy_ecs", "rapier3d", "rerecast", "landmass", "rhai", "egui", "egui-winit", "naga", "objc2-metal", "objc-sys", "ash", "winit"}

def audit(target):
    args = ["cargo", "metadata", "--locked", "--format-version", "1", "--filter-platform", target]
    data = json.loads(subprocess.check_output(args, cwd=ROOT))
    packages = {p["id"]: p for p in data["packages"]}
    nodes = {n["id"]: n for n in data["resolve"]["nodes"]}
    roots = data["workspace_members"]
    reachable = set()
    todo = roots[:]
    while todo:
        node = todo.pop()
        if node in reachable:
            continue
        reachable.add(node)
        for dep in nodes[node]["deps"]:
            if any(k["kind"] in (None, "build") for k in dep["dep_kinds"]):
                todo.append(dep["pkg"])
    selected = [packages[p] for p in sorted(reachable)]
    banned = sorted({p["name"] for p in selected} & BANNED)
    assert not banned, f"Unexpected native compilation dependency: {banned}"
    # objc-sys contains an optional Objective-C exception shim. It must stay disabled.
    for p in selected:
        if p["name"] == "objc-sys":
            assert "unstable-exception" not in nodes[p["id"]]["features"]
    report = {
        "target": target,
        "status": "passed",
        "roots": sorted(packages[p]["name"] for p in roots),
        "resolved_package_count": len(selected),
        "forbidden_native_build_dependencies": banned,
        "build_scripts": sorted(f'{p["name"]}@{p["version"]}' for p in selected if any("custom-build" in t["kind"] for t in p["targets"])),
        "critical_features": {f'{p["name"]}@{p["version"]}': nodes[p["id"]]["features"] for p in selected if p["name"] in CRITICAL},
        "system_dependencies": ["Apple Objective-C/Metal/AppKit frameworks"] if target.endswith("apple-darwin") else ["system Vulkan loader/driver at runtime"],
        "scope": "Resolved normal/build graph; development-only test dependencies are excluded. This is not proof of native execution on another OS.",
    }
    out = ROOT / "target/verification/dependencies"
    out.mkdir(parents=True, exist_ok=True)
    (out / f"{target}.json").write_text(json.dumps(report, indent=2) + "\n")
    return report

if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    host = next(line.split(": ", 1)[1] for line in subprocess.check_output(["rustc", "-vV"], text=True).splitlines() if line.startswith("host: "))
    parser.add_argument("--target", default=host)
    options = parser.parse_args()
    result = audit(options.target)
    print(json.dumps(result, indent=2))
