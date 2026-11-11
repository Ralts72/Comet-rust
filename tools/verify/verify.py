#!/usr/bin/env python3
"""Verify the current engine, editor, asset pipeline and standalone game package."""
import argparse
from contextlib import ExitStack
import hashlib
import json
import math
import os
import plistlib
import shutil
import struct
import subprocess
import sys
import tempfile
import time
import uuid
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "target/verification"
checks = {}
DEMO_SCRIPT = "90000000-0000-4000-8000-000000000001"
DEMO_SCRIPT_OWNER = "60000000-0000-4000-8000-000000000005"
DEMO_SCRIPT_TARGET = "60000000-0000-4000-8000-000000000004"
DEMO_JOINT_SCENE = "a0000000-0000-4000-8000-000000000001"
NAVIGATION_SCENE = "a0000000-0000-4000-8000-000000000005"
NAVIGATION_DATA = "a0000000-0000-4000-8000-000000000006"
NAVIGATION_SCRIPT = "a0000000-0000-4000-8000-000000000007"
NAVIGATION_AGENT = "b1000000-0000-4000-8000-000000000004"
TRAVEL_SCENES = ("a0000000-0000-4000-8000-000000000002", "a0000000-0000-4000-8000-000000000003")
TRAVEL_SCRIPT = "a0000000-0000-4000-8000-000000000004"
SCRIPT_START = "落体方块示例已启动：按 E 弹起方块。"
SCRIPT_STOP = "落体方块示例已停止。"


def report(status):
    unverified = []
    for platform, backend in (("darwin", "macOS Metal"), ("linux", "Linux Vulkan"), ("win32", "Windows Vulkan")):
        if sys.platform != platform:
            unverified.append(f"{backend}: native build/link and GPU/window execution")
        else:
            if checks.get("app-headless", {}).get("status") != "passed":
                unverified.append(f"{backend}: offscreen GPU execution")
            if checks.get("app-native", {}).get("status") != "passed":
                unverified.append(f"{backend}: native window execution")
    unverified.append("physical mouse drag (synthetic egui drag regression passes)")
    if sys.platform != "win32":
        unverified.append("Windows EXE icon resource API roundtrip")
    unverified.append("Windows Explorer icon display")
    (OUT / "verification.json").write_text(json.dumps({"status": status, "checks": checks, "unverified": unverified}, indent=2) + "\n")


def run(name, command, cwd=ROOT, timeout=300, expect_failure=False, environment=None):
    print(f"{name}: running", flush=True)
    start = time.monotonic()
    with (OUT / f"{name}.log").open("w") as log:
        result = subprocess.run([str(a) for a in command], cwd=cwd, stdout=log, stderr=subprocess.STDOUT, timeout=timeout, env=environment)
    passed = (result.returncode != 0) if expect_failure else (result.returncode == 0)
    checks[name] = {"status": "passed" if passed else "failed", "command": [str(a) for a in command], "cwd": str(cwd), "seconds": round(time.monotonic() - start, 3)}
    report("running" if passed else "failed")
    if not passed:
        print((OUT / f"{name}.log").read_text()[-12000:], file=sys.stderr)
        raise RuntimeError(f"{name} failed")


def run_capture(name, command, cwd=ROOT):
    outputs = [OUT / f"{name}.json", OUT / f"{name}.png"]
    previous = {path: path.stat().st_mtime_ns if path.exists() else None for path in outputs}
    run(name, command, cwd=cwd)
    assert all(path.exists() and path.stat().st_mtime_ns != previous[path] for path in outputs), \
        f"{name} did not write a new report and capture"


def read_png(image):
    data = image.read_bytes()
    assert data.startswith(b"\x89PNG\r\n\x1a\n")
    offset, dimensions, pixels = 8, None, bytearray()
    ended = False
    while offset < len(data):
        assert offset + 12 <= len(data), "truncated PNG chunk"
        length, kind = struct.unpack_from(">I4s", data, offset)
        end = offset + 8 + length
        assert end + 4 <= len(data), "truncated PNG payload"
        payload = data[offset + 8:end]
        assert zlib.crc32(kind + payload) == struct.unpack_from(">I", data, end)[0], "PNG CRC mismatch"
        if offset == 8:
            assert kind == b"IHDR" and length == 13
            width, height, depth, color, compression, filtering, interlace = struct.unpack(">IIBBBBB", payload)
            assert width > 0 and height > 0 and (depth, color, compression, filtering, interlace) == (8, 6, 0, 0, 0)
            dimensions = (width, height)
        elif kind == b"IDAT":
            pixels.extend(payload)
        offset = end + 4
        if kind == b"IEND":
            assert length == 0
            ended = True
            break
    assert ended and offset == len(data) and dimensions is not None
    width, height = dimensions
    decoded = zlib.decompress(pixels)
    stride = width * 4 + 1
    assert len(decoded) == height * stride, "PNG pixel dimensions do not match its header"
    assert all(decoded[row * stride] < 5 for row in range(height)), "invalid PNG row filter"
    return {"width": width, "height": height, "sha256": hashlib.sha256(data).hexdigest()}, decoded


def inspect_png(image):
    return read_png(image)[0]


def decode_rgba_png(image):
    """Decode the RGBA8 capture format using only the standard library."""
    metadata, filtered = read_png(image)
    width, height = metadata["width"], metadata["height"]
    stride = width * 4
    pixels = bytearray()
    previous = bytearray(stride)
    for y in range(height):
        offset = y * (stride + 1)
        kind = filtered[offset]
        row = bytearray(filtered[offset + 1:offset + 1 + stride])
        for x, value in enumerate(row):
            left = row[x - 4] if x >= 4 else 0
            above = previous[x]
            upper_left = previous[x - 4] if x >= 4 else 0
            if kind == 0:
                predictor = 0
            elif kind == 1:
                predictor = left
            elif kind == 2:
                predictor = above
            elif kind == 3:
                predictor = (left + above) // 2
            else:
                prediction = left + above - upper_left
                distances = [abs(prediction - sample) for sample in (left, above, upper_left)]
                predictor = (left, above, upper_left)[distances.index(min(distances))]
            row[x] = (value + predictor) & 255
        pixels.extend(row)
        previous = row
    return metadata, pixels


def inspect_triangle_capture(image):
    metadata, pixels = decode_rgba_png(image)
    width, height = metadata["width"], metadata["height"]

    def rgb(x, y):
        offset = (y * width + x) * 4
        return pixels[offset:offset + 3]

    samples = [rgb(int(width * x), int(height * y)) for x, y in ((0.1, 0.1), (0.9, 0.1), (0.1, 0.9), (0.9, 0.9))]
    background = [sorted(pixel[channel] for pixel in samples)[len(samples) // 2] for channel in range(3)]
    assert all(max(abs(pixel[channel] - background[channel]) for channel in range(3)) <= 8 for pixel in samples), "capture background is not uniform"
    roi = [width // 5, height // 5, width * 4 // 5, height * 4 // 5]
    mask, totals = set(), [0, 0, 0]
    for y in range(roi[1], roi[3]):
        for x in range(roi[0], roi[2]):
            pixel = rgb(x, y)
            if (pixels[(y * width + x) * 4 + 3] >= 250
                    and max(abs(pixel[channel] - background[channel]) for channel in range(3)) >= 24
                    and pixel[0] > pixel[1] + 16 and pixel[0] > pixel[2] + 16):
                mask.add(y * width + x)
                totals = [total + channel for total, channel in zip(totals, pixel)]
    area = (roi[2] - roi[0]) * (roi[3] - roi[1])
    assert len(mask) >= max(64, area // 200), "visible red triangle is missing from the scene ROI"
    xs, ys = [pixel % width for pixel in mask], [pixel // width for pixel in mask]
    assert max(xs) - min(xs) >= width // 20 and max(ys) - min(ys) >= height // 20, "foreground is too small to be the test triangle"
    return {**metadata, "roi": roi, "foreground_pixels": len(mask), "foreground_ratio": len(mask) / area,
            "background_rgb": background, "mean_foreground_rgb": [total / len(mask) for total in totals]}, mask


def inspect_capture(name, native, draw_count):
    """Check a fixed count or an explicit frame-number schedule, including sparse feedback."""
    path = OUT / f"{name}.json"
    value = json.loads(path.read_text())
    assert value["submitted_frames"] == 8
    frames = value["frames"]
    assert frames and all(f["draw_count"] == (draw_count[f["number"]] if isinstance(draw_count, dict) else draw_count)
                          and f["worker_thread"] == "comet-render" for f in frames)
    assert any(f["number"] == 7 for f in frames), "final capture frame did not complete"
    assert any(f["number"] == 2 and not f["presented"] for f in frames), "zero-extent frame did not skip presentation"
    if native:
        assert any(f["presented"] for f in frames)
        assert len({f["token"]["target_generation"] for f in frames}) >= 3
    image = OUT / f"{name}.png"
    png = inspect_png(image)
    return value, png


def inspect_frames(name, native, draw_count=9, expect_character=True):
    value, png = inspect_capture(name, native, draw_count)
    frames = value["frames"]
    runtime = value["application"]
    if "checkpoint" in runtime:
        assert runtime["checkpoint"]["persistent"] is False, "smoke run reached persistent player storage"
        assert runtime["settings"] == {"master_volume": 1., "shadows": True}
        assert value["audio"]["master_gain"] == 1.
    diagnostics = runtime["script_diagnostics"]
    assert isinstance(diagnostics, list) and all(not entry["failed"] for entry in diagnostics), "script execution reported a failure"
    if "steps" in runtime:
        assert math.isclose(runtime["elapsed"], runtime["steps"] / 60, abs_tol=1e-9)
        assert 0 <= runtime["frame_steps"] <= 8
        assert math.isfinite(runtime["dropped_time"]) and runtime["dropped_time"] >= 0
        assert 0 <= runtime["interpolation_alpha"] < 1
        if value["headless"]:
            assert runtime["steps"] == 7, "eight smoke frames must execute seven fixed ticks after the startup baseline"
            assert runtime["dropped_time"] == 0
            if expect_character:
                character = runtime["character"]
                assert character is not None and all(math.isfinite(v) for v in character["position"])
                assert 0.8 < character["position"][1] < 1, "packaged character must actually fall during the smoke run"
                assert character["vertical_velocity"] < 0
        if not expect_character:
            assert runtime["character"] is None, "test scene must not invent a player character"
            assert diagnostics == [], "test scene must not invent script diagnostics"
        else:
            starts = [entry for entry in diagnostics if entry["phase"] == "on_start" and entry["message"] == SCRIPT_START]
            stops = [entry for entry in diagnostics if entry["phase"] == "on_stop" and entry["message"] == SCRIPT_STOP]
            assert len(starts) == len(stops) == 1, "packaged demo must execute both script lifecycle callbacks"
            assert starts[0]["tick"] == 0 and stops[0]["tick"] == runtime["steps"]
            assert starts[0]["sequence"] < stops[0]["sequence"]
            assert all(entry["asset"] == DEMO_SCRIPT and entry["entity"] == DEMO_SCRIPT_OWNER and entry["world"] == runtime["world"] for entry in diagnostics)
    if draw_count == 0 and "entities" in runtime:
        assert runtime["entities"] == 0
    checks[f"{name}_evidence"] = {"status": "passed", "submitted_frames": 8, "recorded_frames": len(frames), "draw_count": draw_count, "png_sha256": png["sha256"], "capture_size": [png["width"], png["height"]], "script_diagnostics": len(diagnostics)}


def inspect_travel_frames(name):
    value, png = inspect_capture(name, False, 6)
    runtime = value["application"]
    assert runtime["scene"] == TRAVEL_SCENES[1], "exported game did not enter the second scene"
    assert 0 < runtime["steps"] < 7 and math.isclose(runtime["elapsed"], runtime["steps"] / 60, abs_tol=1e-9)
    assert runtime["character"] is not None
    assert all(math.isfinite(v) for v in runtime["character"]["position"])
    diagnostics = runtime["script_diagnostics"]
    assert all(entry["asset"] == TRAVEL_SCRIPT and not entry["failed"] for entry in diagnostics)
    starts = [entry for entry in diagnostics if entry["phase"] == "on_start"]
    stops = [entry for entry in diagnostics if entry["phase"] == "on_stop"]
    assert len(starts) == len(stops) == 2
    assert "第一关·庭院" in starts[0]["message"] and "第二关·方塔" in starts[1]["message"]
    assert starts[0]["world"] == stops[0]["world"] != runtime["world"]
    assert starts[1]["world"] == stops[1]["world"] == runtime["world"]
    assert starts[0]["tick"] == starts[1]["tick"] == 0
    assert stops[0]["tick"] > 0 and stops[1]["tick"] == runtime["steps"]
    assert starts[0]["sequence"] < stops[0]["sequence"] < starts[1]["sequence"] < stops[1]["sequence"]
    worlds = {frame["token"]["world"] for frame in value["frames"]}
    assert worlds == {starts[0]["world"], starts[1]["world"]}
    checks["travel_package_evidence"] = {"status": "passed", "scene": runtime["scene"],
                                          "worlds": sorted(worlds), "steps_after_transition": runtime["steps"],
                                          "draw_count": 6, "png_sha256": png["sha256"],
                                          "script_diagnostics": len(diagnostics)}


def inspect_navigation_frames(name):
    value, png = inspect_capture(name, False, 12)
    runtime = value["application"]
    assert runtime["scene"] == NAVIGATION_SCENE and runtime["steps"] == 7
    assert math.isclose(runtime["elapsed"], 7 / 60, abs_tol=1e-9)
    diagnostics = runtime["script_diagnostics"]
    assert len(diagnostics) == 2 and all(not entry["failed"] for entry in diagnostics)
    assert [entry["message"] for entry in diagnostics] == ["导航巡逻开始。", "导航巡逻停止。"]
    assert all(entry["asset"] == NAVIGATION_SCRIPT and entry["entity"] == NAVIGATION_AGENT
               and entry["world"] == runtime["world"] for entry in diagnostics)
    assert diagnostics[0]["tick"] == 0 and diagnostics[1]["tick"] == 7
    agents = runtime["navigation_agents"]
    assert len(agents) == 1 and agents[0]["entity"] == NAVIGATION_AGENT and agents[0]["enabled"]
    assert agents[0]["status"] == "moving"
    position = agents[0]["position"]
    assert all(math.isfinite(number) for number in position)
    assert math.hypot(position[0] + 3, position[2]) > 0.05
    player = runtime["character"]
    assert player is not None and all(math.isfinite(number) for number in player["position"])
    checks["navigation_runtime_evidence"] = {"status": "passed", "scene": NAVIGATION_SCENE,
                                             "navigation_agents": agents, "source_free": True,
                                             "script_diagnostics": len(diagnostics), "png_sha256": png["sha256"]}


def read_product(products, entries, asset, protocols):
    entry = entries[asset]
    data = (products / "products" / asset / f"{entry['digest']}.json").read_bytes()
    assert hashlib.sha256(data).hexdigest() == entry["digest"]
    value = json.loads(data)
    assert value["id"] == asset and value["version"] == protocols["product"]
    assert value["payload"]["kind"] == entry["kind"]
    return value["payload"]


def inspect_script_package(products, scene, protocols, additional_scripts):
    index = json.loads((products / "index.json").read_text())
    assert index["version"] == protocols["index"]
    entries = index["products"]
    assert all("editor" not in entry for entry in entries.values()), "game package retains editor provenance"
    assert {asset for asset, entry in entries.items() if entry["kind"] == "script"} == {DEMO_SCRIPT, TRAVEL_SCRIPT, NAVIGATION_SCRIPT, *additional_scripts}
    assert DEMO_SCRIPT in entries[scene]["dependencies"] and entries[DEMO_SCRIPT]["dependencies"] == []

    script = read_product(products, entries, DEMO_SCRIPT, protocols)
    assert script["kind"] == "script" and script["data"]["version"] == 1
    expected = (ROOT / "demo/assets/scripts/falling_cube.rhai").read_text()
    assert script["data"]["source"] == expected
    source = read_product(products, entries, scene, protocols)
    assert source["kind"] == "scene" and source["data"]["base"]["version"] >= 4
    entities = source["data"]["base"]["entities"]
    owner = next(entity for entity in entities if entity["id"] == DEMO_SCRIPT_OWNER)
    binding = owner["script"]
    assert binding["asset"] == DEMO_SCRIPT and binding["enabled"]
    assert binding["parameters"]["target"] == {"type": "entity", "value": {"scope": "local", "entity": DEMO_SCRIPT_TARGET}}
    target = next(entity for entity in entities if entity["id"] == DEMO_SCRIPT_TARGET)
    assert target["physics"]["motion"] == "dynamic"
    checks["script_package"] = {"status": "passed", "script": DEMO_SCRIPT, "owner": DEMO_SCRIPT_OWNER,
                                "target": DEMO_SCRIPT_TARGET, "source_sha256": hashlib.sha256(expected.encode()).hexdigest(),
                                "product_version": protocols["product"], "index_version": protocols["index"],
                                "editor_provenance_stripped": True}


def inspect_navigation_package(products, protocols):
    entries = json.loads((products / "index.json").read_text())["products"]
    assert NAVIGATION_DATA in entries[NAVIGATION_SCENE]["dependencies"]
    assert entries[NAVIGATION_DATA]["dependencies"] == []
    source = read_product(products, entries, NAVIGATION_SCENE, protocols)["data"]
    assert source["base"]["version"] == 6 and protocols["scene"] >= 6
    region = next(entity["navigation_region"] for entity in source["base"]["entities"]
                  if entity.get("navigation_region"))
    assert region["data"] == NAVIGATION_DATA
    data = read_product(products, entries, NAVIGATION_DATA, protocols)["data"]
    expected = json.loads((ROOT / "demo/assets/navigation.nav").read_text())
    assert data == expected and data["version"] == 1
    script = read_product(products, entries, NAVIGATION_SCRIPT, protocols)["data"]
    assert script["source"] == (ROOT / "demo/assets/scripts/npc_patrol.rhai").read_text()
    npc = next(entity for entity in source["base"]["entities"] if entity["id"] == NAVIGATION_AGENT)
    assert npc["script"]["asset"] == NAVIGATION_SCRIPT and npc["script"]["enabled"]
    checks["navigation_package"] = {"status": "passed", "scene": NAVIGATION_SCENE,
                                     "navigation": NAVIGATION_DATA, "input_digest": data["input_digest"],
                                     "vertices": len(data["vertices"]), "polygons": len(data["polygons"])}


def verify_navigation_bake(case, assetc, editor, template, project, products):
    navigation_source = project / "assets/navigation.scene"
    navigation_meta = project / "assets/navigation.nav.meta"
    original_navigation_source = navigation_source.read_bytes()
    original_navigation_meta = navigation_meta.read_bytes()
    run("bake_navigation", [assetc, "bake-navigation", project, "navigation.scene", "--products", products,
                             "--output", "navigation.nav"])
    assert json.loads((OUT / "bake_navigation.log").read_text())["id"] == NAVIGATION_DATA
    assert navigation_source.read_bytes() == original_navigation_source
    assert navigation_meta.read_bytes() == original_navigation_meta
    assert json.loads((project / "assets/navigation.nav").read_text()) == json.loads((ROOT / "demo/assets/navigation.nav").read_text())
    # A stale additional scene must block export even when the startup scene is valid.
    stale_navigation = json.loads(original_navigation_source)
    wall = next(entity for entity in stale_navigation["base"]["entities"] if entity["name"] == "静态隔墙")
    wall["transform"]["translation"][0] += 0.25
    navigation_source.write_text(json.dumps(stale_navigation, ensure_ascii=False))
    run("prepare_stale_navigation", [assetc, "prepare", project, "--output", products])
    stale_output = case / "stale-navigation-must-not-publish"
    run("stale_navigation_export_rejected", [editor, "--app-template", template, "--project", project,
        "--products", products, "--export-game", stale_output, "--include-scene", NAVIGATION_SCENE], expect_failure=True)
    assert not stale_output.exists()
    assert "navigation data is stale" in (OUT / "stale_navigation_export_rejected.log").read_text()
    navigation_source.write_bytes(original_navigation_source)


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


def verify_program_identity_cache():
    """A changed program identity must leave unchanged engine library artifacts fresh."""
    for name, build_id in (("identity_cache_first", "1" * 64), ("identity_cache_changed", "2" * 64),
                           ("identity_cache_same", "2" * 64)):
        run(name, ["cargo", "check", "--locked", "-p", "comet-editor", "-p", "comet-app",
                   "--message-format=json-render-diagnostics"], timeout=900,
            environment=dict(os.environ, COMET_BUILD_ID=build_id))
    def freshness(name):
        artifacts = {}
        for line in (OUT / f"{name}.log").read_text().splitlines():
            try:
                message = json.loads(line)
            except ValueError:
                continue
            if isinstance(message, dict) and message.get("reason") == "compiler-artifact":
                target = message["target"]
                if "lib" in target["kind"] or "bin" in target["kind"]:
                    artifacts[target["name"]] = message["fresh"]
        return artifacts
    artifacts = freshness("identity_cache_changed")
    libraries = {"comet_core", "comet_scene", "comet_asset_data", "comet_asset", "comet_asset_pipeline",
                 "comet_runtime", "comet_render", "comet_audio", "comet_application", "comet_editor_core"}
    assert all(artifacts.get(name) is True for name in libraries), f"program identity rebuilt engine libraries: {artifacts}"
    assert all(artifacts.get(name) is False for name in ("comet-editor", "comet-app")), "program identities were not updated"
    unchanged = freshness("identity_cache_same")
    assert all(unchanged.get(name) is True for name in libraries | {"comet-editor", "comet-app"}), f"unchanged program build was not reused: {unchanged}"
    checks["program_identity_cache"] = {"status": "passed", "reused_libraries": sorted(libraries),
                                        "updated_programs": ["comet-editor", "comet-app"],
                                        "build_ids": ["1" * 64, "2" * 64], "unchanged_programs_reused": True}


def offscreen_test(name, test, report_name, package="comet-editor", captures=None):
    report_path = OUT / report_name
    outputs = [report_path, *([OUT / name for name in captures] if captures is not None else [report_path.with_suffix(".png")])]
    previous = {path: path.stat().st_mtime_ns if path.exists() else None for path in outputs}
    run(name, ["cargo", "test", "--locked", "-p", package, test,
               "--", "--ignored", "--exact"], timeout=900)
    assert "test result: ok. 1 passed;" in (OUT / f"{name}.log").read_text(), "GPU test filter did not execute one test"
    assert all(path.exists() and path.stat().st_mtime_ns != previous[path] for path in outputs), \
        "GPU test did not replace its report and capture"
    return json.loads(report_path.read_text())


def freeze_sources():
    files = [ROOT / name for name in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "AGENTS.md", "README.md", "start.sh", ".cargo/config.toml", ".gitignore", ".gitattributes", ".editorconfig")]
    for directory in ("engine", "editor", "app", "tools", "demo", "tests", ".github"):
        files.extend(path for path in (ROOT / directory).rglob("*") if path.is_file() and "__pycache__" not in path.parts and path.name != ".assetc.lock")
    files.extend((ROOT / "docs").glob("*.md"))
    sources = {str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest() for path in sorted(files)}
    manifest = {"sources_sha256": sources, "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(), "cargo": subprocess.check_output(["cargo", "--version"], text=True).strip()}
    (OUT / "source-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


def check_target(name, target):
    import linux_sdk
    environment = linux_sdk.environment(target)
    run(f"{name}_check", ["cargo", "check", "--workspace", "--all-targets", "--target", target, "--locked"], timeout=900, environment=environment)
    run(f"{name}_clippy", ["cargo", "clippy", "--workspace", "--all-targets", "--target", target, "--locked", "--", "-D", "warnings"], timeout=900, environment=environment)
    run(f"{name}_boundaries", [sys.executable, "tools/verify/check_boundaries.py", "--target", target])
    run(f"{name}_audit", [sys.executable, "tools/verify/audit_dependencies.py", "--target", target])


def start_command():
    return [sys.executable, ROOT / "tools/launch/launch.py"] if sys.platform == "win32" else [ROOT / "start.sh"]


def verify_empty_project(case, assetc, editor, template, args):
    project, products = case / "empty-project", case / "empty-products"
    run("create_project", [assetc, "create-project", project], cwd=case)
    created = json.loads((OUT / "create_project.log").read_text())
    assert Path(created["project"]).samefile(project) and created["entry_scene"] == "main.scene"
    assert {path.name for path in project.iterdir()} == {"assets", "project.json"}
    assert {path.name for path in (project / "assets").iterdir()} == {"main.scene", "main.scene.meta"}
    source = json.loads((project / "assets/main.scene").read_text())
    assert source["base"]["entities"] == [] and source["instances"] == []
    scene = json.loads((project / "assets/main.scene.meta").read_text())["id"]
    run("prepare_empty_project", [assetc, "prepare", project, "--output", products], cwd=case)
    index = json.loads((products / "index.json").read_text())["products"]
    assert set(index) == {scene} and index[scene]["kind"] == "scene" and index[scene]["dependencies"] == []
    if args.gpu or args.native:
        for native in ([False, True] if args.native else [False]):
            name = "empty-editor-native" if native else "empty-editor-headless"
            command = [editor, "--app-template", template, "--project", project, "--products", products,
                       "--smoke", "--report", OUT / f"{name}.json", "--capture", OUT / f"{name}.png"]
            if not native:
                command.append("--headless")
            run_capture(name, command, cwd=case)
            inspect_frames(name, native, draw_count=0, expect_character=False)
    built = case / ("empty-package-build.app" if sys.platform == "darwin" else "empty-package-build")
    run("empty_editor_export", [editor, "--app-template", template, "--project", project, "--products", products,
                                "--export-game", built, "--report", OUT / "empty-editor-export.json"], cwd=case)
    exported = json.loads((OUT / "empty-editor-export.json").read_text())
    assert exported["status"] == "passed"
    built = Path(exported["root"])
    executable_relative = Path(exported["executable"]).relative_to(built)
    products_relative = Path(exported["products"]).relative_to(built)
    manifest = json.loads((built / products_relative.parent / "package.json").read_text())
    assert manifest["entry_scene"] == scene and manifest["scenes"] == [scene]
    assert manifest["runtime"] == json.loads(template.read_text())["identity"]
    relocated = case / ("relocated-empty-game.app" if sys.platform == "darwin" else "relocated-empty-game")
    built.rename(relocated)
    shutil.rmtree(project)
    shutil.rmtree(products)
    empty = case / "empty-project-cwd"
    empty.mkdir()
    run("empty_standalone_products", [assetc, "inspect", relocated / products_relative, scene], cwd=empty)
    expanded = case / "empty-expanded.scene"
    run("empty_standalone_scene", [assetc, "expand", relocated / products_relative, scene, "--output", expanded], cwd=empty)
    assert json.loads(expanded.read_text()) == source["base"]
    index = json.loads((relocated / products_relative / "index.json").read_text())["products"]
    assert set(index) == {scene} and index[scene]["kind"] == "scene" and index[scene]["dependencies"] == []
    app = relocated / executable_relative
    run("empty_standalone_runtime_identity", [app, "--template-info"], cwd=empty)
    identity = json.loads((OUT / "empty_standalone_runtime_identity.log").read_text())
    assert identity["identity"] == manifest["runtime"] and not identity["debug"]
    if args.gpu or args.native:
        for native in ([False, True] if args.native else [False]):
            name = "empty-app-native" if native else "empty-app-headless"
            command = [app, "--smoke", "--report", OUT / f"{name}.json", "--capture", OUT / f"{name}.png"]
            if not native:
                command.append("--headless")
            run_capture(name, command, cwd=empty)
            inspect_frames(name, native, draw_count=0, expect_character=False)
    checks["empty_package_isolation"] = {"status": "passed", "package": str(relocated), "authoring_exists": project.exists(),
                                         "development_cache_exists": products.exists(), "included_scenes": manifest["scenes"], "product_count": len(index)}
    assert not project.exists() and not products.exists()


def verify_prefab_document(case, assetc, editor, template, args):
    project, products = case / "prefab-project", case / "prefab-products"
    run("prefab_create_project", [assetc, "create-project", project], cwd=case)
    scene_path = project / "assets/main.scene"
    scene = json.loads(scene_path.read_text())
    scene_id = json.loads(scene_path.with_name("main.scene.meta").read_text())["id"]
    prefab_id, instance = str(uuid.uuid4()), str(uuid.uuid4())
    prefab = json.loads(scene_path.read_text())
    roots = [str(uuid.uuid4()) for _ in range(3)]
    for index, name in enumerate(("Prefab root", "Prefab child", "Second root")):
        prefab["base"]["entities"].append({"id": roots[index], "name": name,
            "parent": roots[0] if index == 1 else None,
            "transform": {"translation": [0., 0., 0.], "rotation": [0., 0., 0., 1.], "scale": [1., 1., 1.]},
            "model": None, "links": {"target": None, "connections": []}})
    prefab_path = project / "assets/native.prefab"
    prefab_path.write_text(json.dumps(prefab))
    metadata = json.dumps({"version": 1, "id": prefab_id}).encode()
    prefab_path.with_name("native.prefab.meta").write_bytes(metadata)
    scene["instances"] = [{"id": instance, "prefab": prefab_id, "parent": None, "overrides": []}]
    scene_path.write_text(json.dumps(scene))
    run("prefab_prepare", [assetc, "prepare", project, "--output", products], cwd=case)
    if args.gpu or args.native:
        run_capture("prefab-editor-headless", [editor, "--project", project, "--products", products,
            "--scene", "native.prefab", "--headless", "--smoke", "--report", OUT / "prefab-editor-headless.json",
            "--capture", OUT / "prefab-editor-headless.png"], cwd=case)
        value, _ = inspect_capture("prefab-editor-headless", False, 0)
        assert value["application"]["prefab_asset"] == prefab_id
        assert value["application"]["entities"] == 3 and not value["application"]["dirty"]
    built = case / ("prefab-build.app" if sys.platform == "darwin" else "prefab-build")
    run("prefab_editor_export", [editor, "--app-template", template, "--project", project, "--products", products,
        "--scene", "native.prefab", "--export-game", built, "--report", OUT / "prefab-editor-export.json"], cwd=case)
    assert prefab_path.with_name("native.prefab.meta").read_bytes() == metadata
    assert {entity["id"] for entity in json.loads(prefab_path.read_text())["base"]["entities"]} == set(roots)
    exported = json.loads((OUT / "prefab-editor-export.json").read_text())
    built = Path(exported["root"])
    executable_relative = Path(exported["executable"]).relative_to(built)
    products_relative = Path(exported["products"]).relative_to(built)
    manifest = json.loads((built / products_relative.parent / "package.json").read_text())
    assert manifest["entry_scene"] == scene_id and manifest["scenes"] == [scene_id]
    index = json.loads((built / products_relative / "index.json").read_text())["products"]
    assert set(index) == {scene_id, prefab_id} and index[prefab_id]["kind"] == "prefab"
    assert all("editor" not in entry for entry in index.values())
    relocated = case / ("relocated-prefab-game.app" if sys.platform == "darwin" else "relocated-prefab-game")
    built.rename(relocated)
    shutil.rmtree(project)
    shutil.rmtree(products)
    expanded = case / "prefab-expanded.scene"
    run("prefab_standalone_scene", [assetc, "expand", relocated / products_relative, scene_id,
        "--output", expanded], cwd=case)
    entities = json.loads(expanded.read_text())["entities"]
    assert {entity["name"] for entity in entities} == {entity["name"] for entity in prefab["base"]["entities"]}
    assert not set(roots).intersection(entity["id"] for entity in entities)
    if args.gpu or args.native:
        run_capture("prefab-app-headless", [relocated / executable_relative, "--headless", "--smoke",
            "--report", OUT / "prefab-app-headless.json", "--capture", OUT / "prefab-app-headless.png"], cwd=case)
        inspect_frames("prefab-app-headless", False, draw_count=0, expect_character=False)
    checks["prefab_document_isolation"] = {"status": "passed", "prefab": prefab_id, "entry_scene": scene_id,
        "entities": len(entities), "package": str(relocated), "authoring_exists": project.exists(),
        "development_cache_exists": products.exists()}


def scaled_world_positions(positions, scale, rotation):
    x, y, z, w = rotation
    rows = ((1 - 2 * (y * y + z * z), 2 * (x * y - w * z), 2 * (x * z + w * y)),
            (2 * (x * y + w * z), 1 - 2 * (x * x + z * z), 2 * (y * z - w * x)),
            (2 * (x * z - w * y), 2 * (y * z + w * x), 1 - 2 * (x * x + y * y)))
    return [[sum(row[axis] * point[axis] * scale[axis] for axis in range(3)) + (0.7 if index == 1 else 0.)
             for index, row in enumerate(rows)] for point in positions]


def scaled_geometry_project(project, scale, normal_magnitude=1., *, gltf_missing_normals=False):
    """Author the same world triangle through raw products or glTF normal generation."""
    assets = project / "assets"
    assets.mkdir(parents=True)
    scene = str(uuid.uuid4())
    # The shared non-identity rotation challenges R*S rather than a diagonal-only matrix.
    pitch, yaw = -0.25, 0.45
    sx, cx, sy, cy = math.sin(pitch / 2), math.cos(pitch / 2), math.sin(yaw / 2), math.cos(yaw / 2)
    rotation = [sx * cy, cx * sy, -sx * sy, cx * cy]
    triangle = [[-1.8, -1.4, 0.], [1.8, -1.4, 0.], [0., 1.68, 0.]]
    positions = [[point[axis] / scale[axis] for axis in range(3)] for point in triangle]
    transform = {"translation": [0., 0.7, 0.], "rotation": rotation, "scale": scale}
    material_data = {"version": 1, "base_color": [0.85, 0.18, 0.08, 1.],
                     "roughness": 0.6, "metallic": 0., "base_color_texture": None}
    content = {"version": 1, "base": {"version": 1, "entities": []}, "instances": []}
    identities = {"scene": scene}
    sources = {"main.scene": (scene, content)}
    if gltf_missing_normals:
        prefab, instance = [str(uuid.uuid4()) for _ in range(2)]
        identities.update(prefab=prefab, instance=instance)
        content["instances"] = [{"id": instance, "prefab": prefab, "parent": None, "overrides": []}]
        binary = struct.pack("<9f3H", *(value for point in positions for value in point), 0, 1, 2)
        (assets / "triangle.bin").write_bytes(binary)
        sources["triangle.gltf"] = (prefab, {
            "asset": {"version": "2.0"}, "scene": 0, "scenes": [{"nodes": [0]}],
            "nodes": [{"name": "Scaled triangle", "mesh": 0, **transform}],
            "meshes": [{"name": "Triangle", "primitives": [{"attributes": {"POSITION": 0}, "indices": 1, "material": 0}]}],
            "materials": [{"name": "Red", "pbrMetallicRoughness": {
                "baseColorFactor": material_data["base_color"], "roughnessFactor": material_data["roughness"],
                "metallicFactor": material_data["metallic"],
            }}],
            "buffers": [{"uri": "triangle.bin", "byteLength": len(binary)}],
            "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": 36, "target": 34962},
                            {"buffer": 0, "byteOffset": 36, "byteLength": 6, "target": 34963}],
            "accessors": [{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
                           "min": [min(point[axis] for point in positions) for axis in range(3)],
                           "max": [max(point[axis] for point in positions) for axis in range(3)]},
                          {"bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR"}],
        })
    else:
        mesh, material, entity = [str(uuid.uuid4()) for _ in range(3)]
        identities.update(mesh=mesh, material=material, entity=entity)
        normal = [0., 0., math.copysign(normal_magnitude, scale[2])]
        content["base"]["entities"] = [{
            "id": entity, "name": "Scaled triangle", "parent": None, "transform": transform,
            "model": {"mesh": mesh, "surface": {"material": material, "roughness": 0.6}},
            "links": {"target": None, "connections": []},
        }]
        sources.update({
            "triangle.mesh": (mesh, {"version": 1, "positions": positions, "normals": [normal] * 3,
                                     "uvs": [[0., 0.], [1., 0.], [0.5, 1.]], "indices": [0, 1, 2]}),
            "red.material": (material, material_data),
        })
    for filename, (asset, value) in sources.items():
        (assets / filename).write_text(json.dumps(value, allow_nan=False) + "\n")
        (assets / f"{filename}.meta").write_text(json.dumps({"version": 1, "id": asset}) + "\n")
    (project / "project.json").write_text(json.dumps({"version": 1, "game_icon": None}) + "\n")
    return {**identities, "scale": scale, "rotation": rotation, "normal_magnitude": normal_magnitude,
            "normal_source": "generated" if gltf_missing_normals else "authored",
            "world_positions": scaled_world_positions(triangle, [1.] * 3, rotation),
            "world_normal": [math.sin(yaw) * math.cos(pitch), -math.sin(pitch), math.cos(yaw) * math.cos(pitch)]}


def inspect_scaled_products(products, authored):
    index = json.loads((products / "index.json").read_text())["products"]

    def product_data(asset, kind):
        entry = index[asset]
        assert entry["kind"] == kind
        data = (products / "products" / asset / f"{entry['digest']}.json").read_bytes()
        assert hashlib.sha256(data).hexdigest() == entry["digest"]
        product = json.loads(data)
        assert product["id"] == asset and product["payload"]["kind"] == kind
        return product["payload"]["data"]

    source = product_data(authored["scene"], "scene")
    if "prefab" in authored:
        assert source["base"]["entities"] == []
        assert source["instances"] == [{"id": authored["instance"], "prefab": authored["prefab"], "parent": None, "overrides": []}]
        assert set(index[authored["scene"]]["dependencies"]) == {authored["prefab"]}
        prefab = product_data(authored["prefab"], "prefab")
        models = [entity for entity in prefab["base"]["entities"] if entity.get("model") is not None]
        assert len(models) == 1
        model = models[0]
        assert model["transform"] == {"translation": [0., 0., 0.], "rotation": [0., 0., 0., 1.], "scale": [1., 1., 1.]}
        transform = next(entity["transform"] for entity in prefab["base"]["entities"] if entity["id"] == model["parent"])
        authored.update(mesh=model["model"]["mesh"], material=model["model"]["surface"]["material"])
        assert set(index[authored["prefab"]]["dependencies"]) == {authored["mesh"], authored["material"]}
        required = {authored["scene"], authored["prefab"], authored["mesh"], authored["material"]}
        assert all(asset in required or entry["kind"] == "material" for asset, entry in index.items())
    else:
        assert {asset: entry["kind"] for asset, entry in index.items()} == {authored["scene"]: "scene", authored["mesh"]: "mesh", authored["material"]: "material"}
        assert set(index[authored["scene"]]["dependencies"]) == {authored["mesh"], authored["material"]}
        transform = source["base"]["entities"][0]["transform"]
    material = product_data(authored["material"], "material")
    assert all(math.isclose(actual, expected, abs_tol=1e-6) for actual, expected in zip(material["base_color"], [0.85, 0.18, 0.08, 1.]))
    assert math.isclose(material["roughness"], 0.6, abs_tol=1e-6) and material["metallic"] == 0.
    assert all(math.isclose(actual, expected, abs_tol=1e-6) for actual, expected in zip(transform["translation"], [0., 0.7, 0.]))
    mesh = product_data(authored["mesh"], "mesh")
    assert mesh["indices"] == [0, 1, 2] and len(mesh["positions"]) == len(mesh["normals"]) == 3
    assert all(math.isfinite(value) for attribute in (mesh["positions"], mesh["normals"]) for vector in attribute for value in vector)
    assert all(any(value != 0 for value in normal) for normal in mesh["normals"]), "import discarded the normal's direction"
    if authored["normal_source"] == "generated":
        assert all(math.isclose(value, expected, abs_tol=1e-6) for normal in mesh["normals"] for value, expected in zip(normal, [0., 0., 1.])), "generated glTF normals have the wrong direction"
    actual = scaled_world_positions(mesh["positions"], transform["scale"], transform["rotation"])
    assert all(math.isclose(value, expected, rel_tol=2e-6, abs_tol=1e-6) for point, expected_point in zip(actual, authored["world_positions"]) for value, expected in zip(point, expected_point))
    return {"status": "passed", **authored, "product_count": len(index), "mesh_sha256": index[authored["mesh"]]["digest"],
            "product_normal_magnitude": max(abs(value) for normal in mesh["normals"] for value in normal)}


def verify_scaled_geometry(case, assetc, app, args):
    captures = []
    configurations = (("tiny", [1e-3] * 3, 1., False), ("huge", [1e20] * 3, 1., False),
                      ("mirror", [-1e20, 1e20, 1e20], 1., False), ("subnormal-normal", [1.] * 3, 2. ** -149, False),
                      ("rotated-anisotropic", [-0.002, 3., 5e8], 1., False),
                      ("gltf-large-source", [1e-20] * 3, 1., True), ("gltf-small-source", [1e30] * 3, 1., True))
    for name, scale, normal_magnitude, gltf_missing_normals in configurations:
        project, products = case / f"scale-{name}-project", case / f"scale-{name}-products"
        authored = scaled_geometry_project(project, scale, normal_magnitude, gltf_missing_normals=gltf_missing_normals)
        run(f"prepare_scale_{name}", [assetc, "prepare", project, "--output", products], cwd=case)
        checks[f"scale_{name}_products"] = inspect_scaled_products(products, authored)
        if args.gpu or args.native:
            capture = f"scale-{name}-headless"
            run_capture(capture, [app, "--products", products, "--scene", authored["scene"], "--headless", "--smoke",
                          "--report", OUT / f"{capture}.json", "--capture", OUT / f"{capture}.png"], cwd=case)
            inspect_frames(capture, False, draw_count=1, expect_character=False)
            evidence, mask = inspect_triangle_capture(OUT / f"{capture}.png")
            checks[f"{capture}_evidence"]["triangle"] = evidence
            captures.append((name, evidence, mask))
    if captures:
        _, reference, expected_mask = captures[0]
        comparisons = {}
        for name, evidence, mask in captures[1:]:
            assert (evidence["width"], evidence["height"]) == (reference["width"], reference["height"])
            overlap = len(expected_mask & mask) / len(expected_mask | mask)
            difference = max(abs(actual - expected) for actual, expected in zip(evidence["mean_foreground_rgb"], reference["mean_foreground_rgb"]))
            assert overlap > 0.97, f"{name}: equivalent world geometry changed the visible triangle"
            assert difference < 12, f"{name}: equivalent world normals changed triangle lighting"
            comparisons[name] = {"foreground_iou": overlap, "mean_rgb_max_difference": difference}
        checks["scaled_geometry_equivalence"] = {"status": "passed", "reference": captures[0][0], "comparisons": comparisons}


def main():
    import audio as audio_verification
    import collect as collect_verification
    import animation as animation_verification
    import character as character_verification
    import combat as combat_verification
    import spatial_audio as spatial_verification
    parser = argparse.ArgumentParser()
    parser.add_argument("--gpu", action="store_true", help="Run actual offscreen GPU frames")
    parser.add_argument("--native", action="store_true", help="Also open and close native Editor/App windows")
    parser.add_argument("--linux", action="store_true", help="Cross-check both apps and Vulkan for installed Linux target")
    parser.add_argument("--windows", action="store_true", help="Cross-check the workspace and Vulkan for installed Windows MSVC target; no linking or execution")
    args = parser.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    report("running")
    resources = ExitStack()
    try:
        run("format", ["cargo", "fmt", "--all", "--check"])
        checkpoint_evidence = OUT / "checkpoint-process.json"
        previous_checkpoint = checkpoint_evidence.stat().st_mtime_ns if checkpoint_evidence.exists() else None
        run("tests", ["cargo", "test", "--workspace", "--locked"], timeout=900)
        assert checkpoint_evidence.exists() and checkpoint_evidence.stat().st_mtime_ns != previous_checkpoint
        checkpoint = json.loads(checkpoint_evidence.read_text())
        assert checkpoint["status"] == "passed" and checkpoint["two_processes"] is True
        assert checkpoint["progress_entries"] == 2 and checkpoint["saved_scene"] == checkpoint["loaded_scene"]
        checks["checkpoint_process_evidence"] = checkpoint
        run("clippy", ["cargo", "clippy", "--workspace", "--all-targets", "--locked", "--", "-D", "warnings"], timeout=900)
        run("boundaries", [sys.executable, "tools/verify/check_boundaries.py"])
        run("dependency_audit", [sys.executable, "tools/verify/audit_dependencies.py"])
        run("python_tests", [sys.executable, "-m", "unittest", "discover", "-s", "tools/launch/tests"])
        run("verification_python_tests", [sys.executable, "-m", "unittest", "discover", "-s", "tools/verify/tests"])
        run("start_help", [*start_command(), "--help"])
        verify_program_identity_cache()
        run("build_asset_tool", ["cargo", "build", "-p", "comet-assetc", "--locked", "--message-format=json-render-diagnostics"], timeout=900)
        assetc = executables(OUT / "build_asset_tool.log")["comet-assetc"]
        sys.path.insert(0, str(ROOT / "tools/launch"))
        from program_cache import ProgramCache
        cache = resources.enter_context(ProgramCache(ROOT / "target/launch"))
        prepare_code = ("import sys,json;from pathlib import Path;"
                        f"sys.path.insert(0,{str(ROOT / 'tools/launch')!r});import launch;"
                        f"Path({str(OUT / 'programs.json')!r}).write_text(json.dumps(launch.prepare_programs(Path({str(cache.slot)!r})),default=str,indent=2))")
        run("program_templates", [sys.executable, "-c", prepare_code], timeout=900)
        programs = json.loads((OUT / "programs.json").read_text())
        editor, template = Path(programs["editor"]), Path(programs["template"])
        if sys.platform == "win32":
            sys.path.insert(0, str(ROOT / "tools/launch"))
            import windows_icon
            windows_icon.verify_icon(editor, (ROOT / "editor/resources/icons/editor.ico").read_bytes())
            windows_icon.verify_icon(Path(programs["app"]), (ROOT / "app/resources/icons/app.ico").read_bytes())
            checks["windows_executable_icons"] = {"status": "passed", "scope": "native Windows resource readback for Editor and Release App"}
        editor_icon = ROOT / "editor/resources/icons/editor.icns"
        if sys.platform == "darwin":
            bundle = editor.parents[2]
            assert bundle.suffix == ".app"
            plist = plistlib.loads((bundle / "Contents/Info.plist").read_bytes())
            assert plist["CFBundleIconFile"] == "editor.icns" and plist["LSMinimumSystemVersion"] == "14.2"
            assert (bundle / "Contents/Resources/editor.icns").read_bytes() == editor_icon.read_bytes()
            run("editor_bundle_signature", ["/usr/bin/codesign", "--verify", "--deep", "--strict", bundle])
        case = Path(tempfile.mkdtemp(prefix="verification-", dir=OUT))
        project, products = case / "project", case / "products"
        shutil.copytree(ROOT / "demo", project, ignore=shutil.ignore_patterns(".assetc.lock"))
        # Only this temporary copy changes scenes automatically during the smoke run.
        travel_path = project / "assets/travel.scene"
        travel = json.loads(travel_path.read_text())
        controller = next(entity for entity in travel["base"]["entities"] if entity.get("script"))
        controller["script"]["parameters"]["auto_after"]["value"] = 0.02
        travel_path.write_text(json.dumps(travel, ensure_ascii=False))
        collect_verification.prepare_smoke_project(project)
        # Use the blue Editor artwork to prove exports select project content instead of defaults.
        shutil.copyfile(ROOT / "editor/resources/icons/editor.png", project / "assets/icons/game.png")
        scene = "40000000-0000-4000-8000-000000000001"
        extra = "70000000-0000-4000-8000-000000000001"
        (project / "assets/extra.scene").write_text(json.dumps({"version": 1, "base": {"version": 2, "entities": []}, "instances": []}))
        (project / "assets/extra.scene.meta").write_text(json.dumps({"version": 1, "id": extra}))
        verify_navigation_bake(case, assetc, editor, template, project, products)
        run("prepare", [assetc, "prepare", project, "--output", products])
        if args.gpu or args.native:
            for native in ([False, True] if args.native else [False]):
                name = "editor-native" if native else "editor-headless"
                command = (start_command() if native else [editor, "--app-template", template])
                command += ["--project", project, "--products", products, "--smoke", "--report", OUT / f"{name}.json", "--capture", OUT / f"{name}.png"]
                if not native:
                    command.append("--headless")
                run_capture(name, command, cwd=case)
                inspect_frames(name, native)
            run_capture("navigation-editor-headless", [editor, "--app-template", template, "--project", project,
                        "--products", products, "--scene", "navigation.scene", "--headless", "--smoke",
                        "--report", OUT / "navigation-editor-headless.json",
                        "--capture", OUT / "navigation-editor-headless.png"], cwd=case)
            inspect_frames("navigation-editor-headless", False, draw_count=12, expect_character=False)
            run_capture("audio-editor-headless", [editor, "--app-template", template, "--project", project,
                        "--products", products, "--scene", "audio.scene", "--headless", "--smoke",
                        "--report", OUT / "audio-editor-headless.json", "--capture", OUT / "audio-editor-headless.png"], cwd=case)
            inspect_frames("audio-editor-headless", False, draw_count=4, expect_character=False)
            editor_audio = json.loads((OUT / "audio-editor-headless.json").read_text())["audio"]
            assert editor_audio["state"] == "idle" and editor_audio["generations_started"] == 0
            run_capture("spatial-audio-editor-headless", [editor, "--app-template", template, "--project", project,
                        "--products", products, "--scene", "spatial_audio.scene", "--headless", "--smoke",
                        "--report", OUT / "spatial-audio-editor-headless.json", "--capture", OUT / "spatial-audio-editor-headless.png"], cwd=case)
            inspect_frames("spatial-audio-editor-headless", False, draw_count=4, expect_character=False)
            author_audio = json.loads((OUT / "spatial-audio-editor-headless.json").read_text())["audio"]
            assert author_audio["state"] == "idle" and author_audio["generations_started"] == 0
            run_capture("collect-editor-headless", [editor, "--app-template", template, "--project", project,
                        "--products", products, "--scene", "collect.scene", "--headless", "--smoke",
                        "--report", OUT / "collect-editor-headless.json", "--capture", OUT / "collect-editor-headless.png"], cwd=case)
            collect_author, _ = inspect_capture("collect-editor-headless", False, 7)
            assert not collect_author["application"]["dirty"] and collect_author["application"]["hud"] is None
            assert collect_author["audio"]["state"] == "idle"
            run_capture("animation-editor-headless", [editor, "--app-template", template, "--project", project,
                        "--products", products, "--scene", "animation.scene", "--headless", "--smoke",
                        "--report", OUT / "animation-editor-headless.json", "--capture", OUT / "animation-editor-headless.png"], cwd=case)
            inspect_frames("animation-editor-headless", False, draw_count=3, expect_character=False)
            for level_name, _, draw_count, _ in character_verification.LEVELS:
                name = f"{level_name}-editor-headless"
                run_capture(name, [editor, "--app-template", template, "--project", project,
                    "--products", products, "--scene", f"{level_name}.scene", "--headless", "--smoke",
                    "--report", OUT / f"{name}.json", "--capture", OUT / f"{name}.png"], cwd=case)
                value, _ = inspect_capture(name, False, draw_count)
                assert not value["application"]["dirty"] and value["application"]["hud"] is None
        built = case / ("package-build.app" if sys.platform == "darwin" else "package-build")
        run("editor_export", [editor, "--app-template", template, "--project", project, "--products", products,
                              "--export-game", built, "--include-scene", extra, "--include-scene", DEMO_JOINT_SCENE,
                              "--include-scene", TRAVEL_SCENES[0], "--include-scene", TRAVEL_SCENES[1],
                              "--include-scene", NAVIGATION_SCENE, "--include-scene", audio_verification.AUDIO_SCENE, "--include-scene", spatial_verification.SCENE,
                              "--include-scene", collect_verification.SCENE,
                              "--include-scene", animation_verification.SCENE,
                              "--include-scene", character_verification.SCENES[0],
                              "--include-scene", character_verification.SCENES[1],
                              "--include-scene", combat_verification.SCENE,
                              "--report", OUT / "editor-export.json"], cwd=case)
        exported = json.loads((OUT / "editor-export.json").read_text())
        assert exported["status"] == "passed"
        built = Path(exported["root"])
        executable_relative = Path(exported["executable"]).relative_to(built)
        products_relative = Path(exported["products"]).relative_to(built)
        manifest_relative = products_relative.parent / "package.json"
        manifest = json.loads((built / manifest_relative).read_text())
        project_settings = json.loads((project / "project.json").read_text())
        assert project_settings["version"] == 2 and uuid.UUID(project_settings["game_id"]).int != 0
        assert manifest["version"] == 4 and manifest["game_id"] == project_settings["game_id"]
        assert set(manifest["scenes"]) == {scene, extra, DEMO_JOINT_SCENE, NAVIGATION_SCENE, audio_verification.AUDIO_SCENE, spatial_verification.SCENE, collect_verification.SCENE, animation_verification.SCENE, *TRAVEL_SCENES, *character_verification.SCENES, combat_verification.SCENE} and manifest["entry_scene"] == scene
        assert manifest["runtime"]["build_id"] == programs["build_id"]
        template_manifest = json.loads(template.read_text())
        assert template_manifest["version"] == 4
        icon = built / manifest_relative.parent / "app.icns"
        assert icon.read_bytes().startswith(b"icns")
        assert icon.read_bytes() != (ROOT / "app/resources/icons/app.icns").read_bytes()
        project_icon = built / manifest_relative.parent / "game-icon.png"
        assert hashlib.sha256(project_icon.read_bytes()).hexdigest() == manifest["game_icon_sha256"]
        assert hashlib.sha256(Path(programs["app"]).read_bytes()).hexdigest() == template_manifest["app_sha256"]
        if sys.platform == "darwin":
            plist = plistlib.loads((built / "Contents/Info.plist").read_bytes())
            assert plist["CFBundleIconFile"] == "app.icns" and plist["LSMinimumSystemVersion"] == "14.2"
            assert plist["CFBundleIdentifier"] == f"org.comet.game.{manifest['game_id']}"
        checks["application_icons"] = {"status": "passed", "editor_icns_sha256": hashlib.sha256(editor_icon.read_bytes()).hexdigest(), "project_icns_sha256": hashlib.sha256(icon.read_bytes()).hexdigest(), "project_png_sha256": manifest["game_icon_sha256"], "template_unchanged": True}
        for name, expected in template_manifest["notices"].items():
            assert hashlib.sha256((built / manifest_relative.parent / "licenses" / name).read_bytes()).hexdigest() == expected
        original_manifest = (built / manifest_relative).read_bytes()
        run("existing_export_rejected", [editor, "--app-template", template, "--project", project, "--products", products, "--export-game", built], expect_failure=True)
        assert (built / manifest_relative).read_bytes() == original_manifest
        bad_template = case / "mismatched-template.json"
        bad = json.loads(template.read_text())
        bad["identity"]["build_id"] = "0" * 64
        bad_template.write_text(json.dumps(bad))
        rejected_output = case / "must-not-publish"
        run("template_mismatch_rejected", [editor, "--app-template", bad_template, "--project", project, "--products", products, "--export-game", rejected_output], expect_failure=True)
        assert not rejected_output.exists() and "start.sh" in (OUT / "template_mismatch_rejected.log").read_text()
        relocated = case / ("relocated-game.app" if sys.platform == "darwin" else "relocated-game")
        source_entries = json.loads((products / "index.json").read_text())["products"]
        expected_products, pending = set(), list(manifest["scenes"])
        while pending:
            asset = pending.pop()
            if asset not in expected_products:
                expected_products.add(asset)
                pending.extend(source_entries[asset]["dependencies"])
        built.rename(relocated)
        shutil.rmtree(project)
        shutil.rmtree(products)
        empty = case / "empty-cwd"
        empty.mkdir()
        run("standalone_products", [assetc, "inspect", relocated / products_relative, scene], cwd=empty)
        inspected = json.loads((OUT / "standalone_products.log").read_text())
        assert DEMO_SCRIPT in inspected["dependencies"]
        inspect_script_package(relocated / products_relative, scene, manifest["runtime"]["protocols"], [audio_verification.SCRIPT, spatial_verification.SCRIPT, collect_verification.SCRIPT, character_verification.SCRIPT, character_verification.FEEDBACK, combat_verification.SCRIPT])
        package_entries = json.loads((relocated / products_relative / "index.json").read_text())["products"]
        assert set(package_entries) == expected_products, "package differs from the declared scene dependency union"
        audio_ids = {asset for asset, entry in package_entries.items() if entry["kind"] == "audio"}
        assert audio_ids == {audio_verification.LOOP, audio_verification.CUE, character_verification.FOOTSTEP}
        checks["audio_product_inventory"] = {"status": "passed", "clips": sorted(audio_ids)}
        inspect_navigation_package(relocated / products_relative, manifest["runtime"]["protocols"])
        checks["audio_package"] = audio_verification.inspect_package(relocated / products_relative, manifest["runtime"]["protocols"], read_product, ROOT / "demo/assets")
        checks["spatial_audio_package"] = spatial_verification.inspect_package(relocated / products_relative, manifest["runtime"]["protocols"], read_product, ROOT / "demo/assets")
        checks["collect_package"] = collect_verification.inspect_package(relocated / products_relative, manifest["runtime"]["protocols"], read_product, ROOT / "demo/assets")
        checks["animation_package"] = animation_verification.inspect_package(relocated / products_relative, manifest["runtime"]["protocols"], read_product, ROOT / "demo/assets")
        checks["character_package"] = character_verification.inspect_package(relocated / products_relative, manifest["runtime"]["protocols"], read_product, ROOT / "demo/assets")
        checks["combat_package"] = combat_verification.inspect_package(relocated / products_relative, manifest["runtime"]["protocols"], read_product, ROOT / "demo/assets")
        app = relocated / executable_relative
        checks["persistent_game_identity"] = {"status": "passed", "game_id": manifest["game_id"],
            "package_version": manifest["version"], "project_settings_version": project_settings["version"]}
        for index, travel_scene in enumerate(TRAVEL_SCENES):
            run(f"standalone_travel_scene_{index}", [assetc, "inspect", relocated / products_relative,
                                                    travel_scene], cwd=empty)
            inspected = json.loads((OUT / f"standalone_travel_scene_{index}.log").read_text())
            assert TRAVEL_SCRIPT in inspected["dependencies"]
        joint_snapshot = OUT / "standalone-joints-scene.json"
        run("standalone_joint_scene", [assetc, "expand", relocated / products_relative, DEMO_JOINT_SCENE,
                                       "--output", joint_snapshot], cwd=empty)
        expanded = json.loads(joint_snapshot.read_text())
        assert expanded["version"] == 5 and manifest["runtime"]["protocols"]["scene"] >= 5
        for component in ("point_joint", "hinge_joint", "slider_joint"):
            assert sum(entity.get(component) is not None for entity in expanded["entities"]) == 1
        run("standalone_runtime_identity", [app, "--template-info"], cwd=empty)
        identity = json.loads((OUT / "standalone_runtime_identity.log").read_text())
        assert identity["identity"] == manifest["runtime"] and not identity["debug"]
        run("standalone_identity_env_ignored", [app, "--template-info"], cwd=empty,
            environment=dict(os.environ, COMET_BUILD_ID="0" * 64))
        assert json.loads((OUT / "standalone_identity_env_ignored.log").read_text()) == identity
        if args.gpu or args.native:
            for native in ([False, True] if args.native else [False]):
                name = "app-native" if native else "app-headless"
                command = [app, "--smoke", "--report", OUT / f"{name}.json", "--capture", OUT / f"{name}.png"]
                if not native:
                    command.append("--headless")
                run_capture(name, command, cwd=empty)
                inspect_frames(name, native)
            run_capture("joints-app-headless", [app, "--scene", DEMO_JOINT_SCENE, "--headless", "--smoke",
                                        "--report", OUT / "joints-app-headless.json",
                                        "--capture", OUT / "joints-app-headless.png"], cwd=empty)
            inspect_frames("joints-app-headless", False, draw_count=8, expect_character=False)
            value = json.loads((OUT / "joints-app-headless.json").read_text())
            runtime = value["application"]
            joints = {joint["kind"]: joint for joint in runtime["joints"]}
            assert set(joints) == {"point", "hinge", "slider"}
            assert all(joint["status"] == "active" for joint in joints.values())
            assert joints["slider"]["translation"][0] > 2.
            checks["joint_package_evidence"] = {"status": "passed", "scene": DEMO_JOINT_SCENE,
                                                "draw_count": 8, "joints": runtime["joints"]}
            run_capture("travel-app-headless", [app, "--scene", TRAVEL_SCENES[0], "--headless", "--smoke",
                                        "--report", OUT / "travel-app-headless.json",
                                        "--capture", OUT / "travel-app-headless.png"], cwd=empty)
            inspect_travel_frames("travel-app-headless")
            run_capture("navigation-app-headless", [app, "--scene", NAVIGATION_SCENE, "--headless", "--smoke",
                                        "--report", OUT / "navigation-app-headless.json",
                                        "--capture", OUT / "navigation-app-headless.png"], cwd=empty)
            inspect_navigation_frames("navigation-app-headless")
            run_capture("audio-app-headless", [app, "--scene", audio_verification.AUDIO_SCENE, "--headless", "--smoke",
                        "--report", OUT / "audio-app-headless.json", "--capture", OUT / "audio-app-headless.png"], cwd=empty)
            audio_report, _ = inspect_capture("audio-app-headless", False, 4)
            checks["audio_runtime_evidence"] = audio_verification.inspect_runtime(audio_report)
            run_capture("spatial-audio-app-headless", [app, "--scene", spatial_verification.SCENE, "--headless", "--smoke",
                        "--report", OUT / "spatial-audio-app-headless.json", "--capture", OUT / "spatial-audio-app-headless.png"], cwd=empty)
            spatial_report, _ = inspect_capture("spatial-audio-app-headless", False, 4)
            checks["spatial_audio_runtime_evidence"] = spatial_verification.inspect_runtime(spatial_report)
            run_capture("collect-app-headless", [app, "--scene", collect_verification.SCENE, "--headless", "--smoke",
                        "--report", OUT / "collect-app-headless.json", "--capture", OUT / "collect-app-headless.png"], cwd=empty)
            collect_report, _ = inspect_capture("collect-app-headless", False,
                {number: 7 if number == 0 else 5 for number in range(8)})
            assert {0, 1} <= {frame["number"] for frame in collect_report["frames"]}
            checks["collect_runtime_evidence"] = collect_verification.inspect_runtime(collect_report)
            run_capture("animation-app-headless", [app, "--scene", animation_verification.SCENE, "--headless", "--smoke",
                        "--report", OUT / "animation-app-headless.json", "--capture", OUT / "animation-app-headless.png"], cwd=empty)
            animation_report, _ = inspect_capture("animation-app-headless", False, 3)
            checks["animation_runtime_evidence"] = animation_verification.inspect_runtime(animation_report)
            for level_name, level_scene, draw_count, label in character_verification.LEVELS:
                for native in ((False, True) if args.native else (False,)):
                    mode = "native" if native else "headless"
                    name = f"{level_name}-app-{mode}"
                    run_capture(name, [app, "--scene", level_scene, *([] if native else ["--headless"]), "--smoke",
                        "--report", OUT / f"{name}.json", "--capture", OUT / f"{name}.png"], cwd=empty)
                    value, _ = inspect_capture(name, native, draw_count)
                    checks[f"{level_name}_{mode}_runtime_evidence"] = character_verification.inspect_runtime(value, level_scene, label, native)
            for native in ((False, True) if args.native else (False,)):
                mode = "native" if native else "headless"
                name = f"combat-app-{mode}"
                run_capture(name, [app, "--scene", combat_verification.SCENE, *([] if native else ["--headless"]), "--smoke",
                    "--report", OUT / f"{name}.json", "--capture", OUT / f"{name}.png"], cwd=empty)
                value, _ = inspect_capture(name, native, combat_verification.DRAW_COUNT)
                checks[f"combat_{mode}_runtime_evidence"] = combat_verification.inspect_runtime(value, native)
            data_directory = case / "isolated-checkpoint-data"
            run_capture("checkpoint-directory-headless", [app, "--scene", extra, "--headless", "--smoke",
                "--data-dir", data_directory, "--report", OUT / "checkpoint-directory-headless.json",
                "--capture", OUT / "checkpoint-directory-headless.png"], cwd=empty)
            storage_report, _ = inspect_capture("checkpoint-directory-headless", False, 0)
            assert storage_report["application"]["checkpoint"] == {
                "available": True, "persistent": True, "last_message": None}
            assert storage_report["application"]["progress_entries"] == 0 and not data_directory.exists()
            checks["checkpoint_directory_evidence"] = {"status": "passed", "game_id": manifest["game_id"],
                "explicit_data_directory": str(data_directory), "created_without_save": False,
                "scope": "Release packaged App; explicit directory only, no player storage"}
            settings_path = data_directory / manifest["game_id"] / "settings.json"
            settings_path.parent.mkdir(parents=True)
            settings_bytes = json.dumps({"version": 1, "game_id": manifest["game_id"],
                "master_volume": 0., "shadows": False}).encode()
            settings_path.write_bytes(settings_bytes)
            run_capture("settings-app-headless", [app, "--scene", audio_verification.AUDIO_SCENE,
                "--headless", "--smoke", "--data-dir", data_directory,
                "--report", OUT / "settings-app-headless.json", "--capture", OUT / "settings-app-headless.png"], cwd=empty)
            settings_report, _ = inspect_capture("settings-app-headless", False, 4)
            assert settings_report["application"]["settings"] == {"master_volume": 0., "shadows": False}
            assert settings_report["audio"]["master_gain"] == 0.
            audio_settings = audio_verification.inspect_runtime(settings_report)
            assert settings_path.read_bytes() == settings_bytes and not settings_path.with_name("checkpoint.json").exists()
            metadata, reference = decode_rgba_png(OUT / "audio-app-headless.png")
            current_metadata, current = decode_rgba_png(OUT / "settings-app-headless.png")
            assert (metadata["width"], metadata["height"]) == (current_metadata["width"], current_metadata["height"])
            changed = sum(any(abs(reference[offset + channel] - current[offset + channel]) > 8 for channel in range(3))
                          for offset in range(0, len(reference), 4))
            assert changed > 64, "disabling directional shadows did not change actual pixels"
            checks["settings_runtime_evidence"] = {"status": "passed", "settings": settings_report["application"]["settings"],
                "audio": audio_settings, "accepted_master_gain": 0., "shadow_difference_pixels": changed,
                "settings_file_unchanged": True, "checkpoint_created": False,
                "scope": "Release packaged App and temporary explicit data directory; no listening claim"}
        checks["package_isolation"] = {"status": "passed", "package": str(relocated), "authoring_exists": project.exists(), "development_cache_exists": products.exists(), "included_scenes": manifest["scenes"], "product_count": len(json.loads((relocated / products_relative / "index.json").read_text())["products"])}
        assert checks["package_isolation"]["product_count"] == len(expected_products)
        verify_empty_project(case, assetc, editor, template, args)
        verify_prefab_document(case, assetc, editor, template, args)
        checks["cubic_animation"] = animation_verification.verify_cubic(
            case, assetc, editor, template, args, run, run_capture, inspect_capture, read_product)
        verify_scaled_geometry(case, assetc, Path(programs["app"]), args)
        if args.linux:
            check_target("linux", "x86_64-unknown-linux-gnu")
        if args.windows:
            check_target("windows", "x86_64-pc-windows-msvc")
        if args.gpu or args.native:
            recovered = offscreen_test("editor_view_recovery_gpu",
                "session::tests::view_errors::headless_view_recovery_delivers_ui_and_returns_to_authoring",
                "editor-view-recovery.json")
            checks["editor_view_recovery_evidence"] = {"status": "passed", "submitted_frames": recovered["submitted_frames"],
                                                      "runtime_stopped": not recovered["application"]["play_active"],
                                                      "author_dirty": recovered["application"]["dirty"],
                                                      "error": recovered["application"]["error"],
                                                      "capture": str(OUT / "editor-view-recovery.png")}
            local = offscreen_test("editor_local_rotation_gpu",
                "ui::tests::gizmo::local_rotation::headless_local_rotation_draws_the_selected_object_and_keeps_authoring_unchanged",
                "editor-local-rotation.json")
            checks["editor_local_rotation_evidence"] = {"status": "passed", "submitted_frames": local["submitted_frames"],
                                                       "author_dirty": local["application"]["dirty"],
                                                       "error": local["application"]["error"],
                                                       "capture": str(OUT / "editor-local-rotation.png")}
            common = offscreen_test("editor_common_properties_gpu",
                "ui::tests::inspector::group::headless_common_properties_draws_mixed_transforms_without_authoring_changes",
                "editor-common-properties.json")
            assert common["common_properties"]["mixed_fields"] == ["位置（多值）", "旋转（多值）", "缩放（多值）"]
            assert common["common_properties"]["scalar_fields"] == ["粗糙度（多值）", "音量（多值）"]
            assert common["common_properties"]["mixed_flags"] == ["循环（多值）", "启动时播放（多值）", "空间音效（多值）"]
            assert common["common_properties"]["asset_fields"] == ["网格（多值）", "材质（多值）", "音频片段（多值）", "脚本（多值）"]
            assert common["common_properties"]["source_unchanged"] and len(common["application"]["selected"]) == 2
            checks["editor_common_properties_evidence"] = {"status": "passed", "submitted_frames": common["submitted_frames"],
                "mixed_fields": common["common_properties"]["mixed_fields"], "author_dirty": common["application"]["dirty"],
                "scalar_fields": common["common_properties"]["scalar_fields"],
                "mixed_flags": common["common_properties"]["mixed_flags"],
                "asset_fields": common["common_properties"]["asset_fields"],
                "png_sha256": inspect_png(OUT / "editor-common-properties.png")["sha256"],
                "capture": str(OUT / "editor-common-properties.png")}
            clips = offscreen_test("editor_common_animation_clips_gpu",
                "ui::tests::inspector::group::clips::headless_common_animation_clip_menu_keeps_bindings_and_authoring_unchanged",
                "editor-common-animation-clips.json")
            assert clips["common_properties"]["asset_fields"] == ["播放片段（多值）"]
            assert clips["common_properties"]["source_unchanged"] and len(clips["application"]["selected"]) == 2
            checks["editor_common_animation_clips_evidence"] = {"status": "passed", "submitted_frames": clips["submitted_frames"],
                "asset_fields": clips["common_properties"]["asset_fields"], "author_dirty": clips["application"]["dirty"],
                "png_sha256": inspect_png(OUT / "editor-common-animation-clips.png")["sha256"],
                "capture": str(OUT / "editor-common-animation-clips.png")}
            marquee = offscreen_test("editor_marquee_gpu",
                "ui::tests::viewport::headless_marquee_overlay_draws_without_changing_authoring",
                "editor-marquee.json")
            assert marquee["marquee"]["overlay_visible"] and marquee["marquee"]["source_unchanged"]
            checks["editor_marquee_evidence"] = {"status": "passed", "submitted_frames": marquee["submitted_frames"],
                                                "author_dirty": marquee["application"]["dirty"],
                                                "error": marquee["application"]["error"],
                                                "capture": str(OUT / "editor-marquee.png")}
            browser = offscreen_test("editor_project_browser_gpu",
                "ui::tests::project::headless_project_browser_draws_without_changing_authoring_or_user_preferences",
                "editor-project-browser.json")
            assert not browser["application"]["dirty"] and browser["application"]["error"] is None
            assert browser["submitted_frames"] == 8 and not browser["application"]["play_active"]
            checks["editor_project_browser_evidence"] = {"status": "passed", "submitted_frames": 8,
                "png_sha256": inspect_png(OUT / "editor-project-browser.png")["sha256"],
                "author_dirty": False, "user_preferences_enabled": False}
            data = offscreen_test("editor_data_preview_gpu",
                "ui::tests::resources::headless_data_preview_draws_typed_records_without_editing_authoring",
                "editor-data-preview.json")
            assert not data["application"]["dirty"] and data["application"]["error"] is None
            assert data["submitted_frames"] == 8 and not data["application"]["play_active"]
            checks["editor_data_preview_evidence"] = {"status": "passed", "submitted_frames": 8,
                "png_sha256": inspect_png(OUT / "editor-data-preview.png")["sha256"], "author_dirty": False}
            workspace = offscreen_test("editor_workspace_gpu",
                "ui::tests::workspace::headless_resized_workspace_draws_the_scene_and_preserves_authoring",
                "editor-workspace.json")
            assert workspace["application"]["error"] is None and not workspace["application"]["dirty"]
            assert workspace["application"]["history"] == 0 and workspace["submitted_frames"] == 8
            checks["editor_workspace_evidence"] = {"status": "passed", "submitted_frames": 8,
                "png_sha256": inspect_png(OUT / "editor-workspace.png")["sha256"], "author_dirty": False,
                "draw_count": 5, "layout": "three resized splitters"}
            resource_tree = offscreen_test("editor_resource_tree_gpu",
                "ui::resources::browser::tests::headless_project_tree_draws_filtered_subassets_without_editing",
                "editor-resource-tree.json")
            assert resource_tree["application"]["error"] is None and not resource_tree["application"]["dirty"]
            assert resource_tree["application"]["history"] == 0 and resource_tree["submitted_frames"] == 8
            checks["editor_resource_tree_evidence"] = {"status": "passed", "submitted_frames": 8,
                "png_sha256": inspect_png(OUT / "editor-resource-tree.png")["sha256"], "author_dirty": False,
                "query": "Amber", "contents": "directory, model source and matching material subasset"}
            menu = offscreen_test("app_pause_menu_gpu",
                "session::tests::menu::headless_pause_menu_draws_without_advancing_the_world_or_touching_player_data",
                "app-pause-menu.json", package="comet-app")
            assert menu["application"]["menu_open"] and menu["application"]["menu_blocks_frame"]
            assert menu["application"]["steps"] == 0 and menu["application"]["elapsed"] == 0
            assert not menu["application"]["checkpoint"]["persistent"]
            assert menu["audio"]["state"] == "idle" and menu["audio"]["generations_started"] == 0
            checks["app_pause_menu_evidence"] = {"status": "passed", "submitted_frames": menu["submitted_frames"],
                "steps": 0, "pending_audio_voices": menu["audio"]["pending_voices"],
                "persistent_storage": False, "png_sha256": inspect_png(OUT / "app-pause-menu.png")["sha256"]}
            skin_captures = [f"render-skinning-{case}{suffix}.png"
                             for case in ("regular", "mirrored", "singular", "extreme-nonuniform") for suffix in ("-static", "")]
            skin_captures.append("render-skinning-no-shadows.png")
            skin = offscreen_test("render_skinning_gpu",
                "frame::tests::skin::native_skinning_matches_static_geometry_normals_and_shadows",
                "render-skinning.json", package="comet-render", captures=skin_captures)
            assert skin["status"] == "passed" and skin["frames"] == 9
            assert {case["case"] for case in skin["comparisons"]} == {"regular", "mirrored", "singular", "extreme-nonuniform"}
            checks["render_skinning_evidence"] = {"status": "passed", "comparisons": skin["comparisons"],
                "captures": {name: inspect_png(OUT / name) for name in skin_captures}}
            run("gpu_capabilities", ["cargo", "run", "-p", "comet-gpu-check", "--locked", "--", "--output", case / "gpu"])
        freeze_sources()
        report("passed")
    except Exception:
        report("failed")
        raise
    finally:
        resources.close()
    print(f"Passed {len(checks)} checks. Evidence: {OUT}")


if __name__ == "__main__":
    main()
