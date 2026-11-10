"""Inspect the shared SimpleSkin products and actual runtime poses without a glTF SDK."""
import hashlib
import json
import math
import shutil
import struct
import sys
import uuid
from pathlib import Path

SCENE = "a0000000-0000-4000-8000-000000000011"
SOURCE = "a0000000-0000-4000-8000-000000000010"
INSTANCE = "b5000000-0000-4000-8000-000000000002"


def _child(key):
    return str(uuid.uuid5(uuid.UUID(SOURCE), key))


def _instanced(source, instance=INSTANCE):
    digest = hashlib.sha1(uuid.UUID(instance).bytes + uuid.UUID(source).bytes).digest()
    return str(uuid.UUID(bytes=digest[:16], version=5))


NODES = tuple(_child(f"node:index:{index}") for index in range(3))
SKELETON = _child("skeleton:scene")
SKIN = _child("skin:index:0")
CLIP = _child("animation:index:0")
MESH = _child("mesh:index:0/primitive:0")
MATERIAL = _child("material:default")
CONTROLLER = _instanced(_child("animation:controller"))


def _close(actual, expected):
    assert len(actual) == len(expected)
    assert all(type(a) in (int, float) and math.isfinite(a) and
               math.isclose(a, b, abs_tol=3e-6, rel_tol=1e-6) for a, b in zip(actual, expected))


def _unit(values):
    length = math.hypot(*values)
    assert length > 0 and math.isfinite(length)
    return [value / length for value in values]


def _curve(author_assets):
    root = author_assets / "models/simple_skin"
    data = (root / "SimpleSkin_animation.bin").read_bytes()
    assert len(data) == 240
    times = list(struct.unpack_from("<12f", data))
    rotations = [_unit(struct.unpack_from("<4f", data, 48 + index * 16)) for index in range(12)]
    assert times[0] == 0 and times[1] == 0.5 and times[-1] == 5.5
    return times, rotations


def inspect_package(products: Path, protocols: dict, read_product, author_assets: Path) -> dict:
    index = json.loads((products / "index.json").read_text(encoding="utf-8"))
    assert index["version"] == protocols["index"] and protocols["scene"] >= 9
    entries = index["products"]
    assert all("editor" not in entry for entry in entries.values()), "package retains editor provenance"
    kinds = {SOURCE: "prefab", SKELETON: "skeleton", SKIN: "skin", CLIP: "animation_clip",
             MESH: "mesh", MATERIAL: "material"}
    for asset, kind in kinds.items():
        assert entries[asset]["kind"] == kind
    assert SOURCE in entries[SCENE]["dependencies"] and entries[SCENE]["kind"] == "scene"
    assert set(entries[SOURCE]["dependencies"]) == set(kinds) - {SOURCE}
    assert entries[SKIN]["dependencies"] == entries[CLIP]["dependencies"] == [SKELETON]
    read = lambda asset: read_product(products, entries, asset, protocols)["data"]
    scene, prefab = read(SCENE), read(SOURCE)
    assert scene["base"]["version"] == prefab["base"]["version"] == 9
    assert len(scene["instances"]) == 1 and scene["instances"][0]["id"] == INSTANCE
    assert scene["instances"][0]["prefab"] == SOURCE and scene["instances"][0]["overrides"] == []
    skeleton, skin, clip, mesh = (read(asset) for asset in (SKELETON, SKIN, CLIP, MESH))
    assert skeleton["version"] == skin["version"] == 1 and clip["version"] == 3 and mesh["version"] == 2
    assert clip["events"] == [{"phase": 1.0, "name": "动作完成"}]
    nodes = {node["id"]: node for node in skeleton["nodes"]}
    assert set(nodes) == set(NODES) and skin["skeleton"] == clip["skeleton"] == SKELETON
    assert nodes[NODES[0]]["parent"] is None and nodes[NODES[1]]["parent"] is None
    assert nodes[NODES[2]]["parent"] == NODES[1]
    assert [joint["node"] for joint in skin["joints"]] == [NODES[1], NODES[2]]
    assert len(mesh["positions"]) == len(mesh["joints"]) == len(mesh["weights"]) == 10
    assert len(mesh["indices"]) == 24
    root = author_assets / "models/simple_skin"
    geometry, influences, inverse = ((root / name).read_bytes() for name in (
        "SimpleSkin_geometry.bin", "SimpleSkin_skinningData.bin", "SimpleSkin_inverseBindMatrices.bin"))
    assert len(geometry) == 168 and len(influences) == 320 and len(inverse) == 128
    assert mesh["indices"] == list(struct.unpack_from("<24H", geometry))
    for vertex in range(10):
        _close(mesh["positions"][vertex], struct.unpack_from("<3f", geometry, 48 + vertex * 12))
        assert mesh["joints"][vertex] == list(struct.unpack_from("<4H", influences, vertex * 16))
        weights = struct.unpack_from("<4f", influences, 160 + vertex * 16)
        _close(mesh["weights"][vertex], [weight / sum(weights) for weight in weights])
    for joint in range(2):
        for column in range(4):
            _close(skin["joints"][joint]["inverse_bind"][column], struct.unpack_from("<4f", inverse, joint * 64 + column * 16))
    times, rotations = _curve(author_assets)
    assert clip["duration"] == 5.5 and len(clip["channels"]) == 1
    channel = clip["channels"][0]
    assert channel["node"] == NODES[2] and channel["interpolation"] == "linear"
    assert channel["times"] == times and channel["values"]["type"] == "rotation"
    assert len(channel["values"]["data"]) == 12
    for actual, expected in zip(channel["values"]["data"], rotations):
        _close(actual, expected)
    owner = next(entity for entity in prefab["base"]["entities"] if entity.get("animator"))
    animator = owner["animator"]
    assert owner["id"] == _child("animation:controller")
    assert animator["skeleton"] == SKELETON and animator["clip"] == CLIP
    assert animator["looping"] is True and animator["speed"] == 1
    assert animator["bindings"] == {node: {"scope": "local", "entity": node} for node in NODES}
    package_root = products.parent
    if package_root.name == "Resources" and package_root.parent.name == "Contents":
        package_root = package_root.parent.parent
    assert not any(path.suffix.lower() in (".gltf", ".glb", ".bin", ".meta") for path in package_root.rglob("*"))
    return {"status": "passed", "scene": SCENE, "source": SOURCE, "source_products": kinds,
            "vertices": 10, "joints": 2, "source_nodes": 3, "rotation_keys": 12,
            "duration": 5.5, "authoring_sources_absent": True, "editor_provenance_stripped": True}


def inspect_runtime(value: dict) -> dict:
    runtime, audio = value["application"], value["audio"]
    assert runtime["scene"] == SCENE and runtime["steps"] == 7 and runtime["character"] is None
    assert runtime["script_diagnostics"] == [] and runtime["hud"] is None and value["audio_shutdown"] is True
    assert math.isclose(runtime["elapsed"], 7 / 60, abs_tol=1e-9)
    assert audio["state"] == "idle" and audio["error"] is None
    assert audio["generations_started"] == audio["active_voices"] == audio["pending_voices"] == audio["cached_clips"] == 0
    assert audio["voices"] == []
    animations = runtime["animations"]
    assert len(animations) == 1
    animation = animations[0]
    assert animation["entity"] == CONTROLLER and animation["skeleton"] == SKELETON and animation["clip"] == CLIP
    assert animation["bindings"] == 3 and math.isclose(animation["time"], 7 / 60, abs_tol=1e-9)
    assert animation["duration"] == 5.5 and animation["playing"] is True and animation["speed"] == 1
    times, rotations = _curve(Path(__file__).resolve().parents[2] / "demo/assets")
    a, b = rotations[:2]
    dot = sum(x * y for x, y in zip(a, b))
    if dot < 0:
        b, dot = [-value for value in b], -dot
    angle = math.acos(min(1., max(-1., dot)))
    amount = (animation["time"] - times[0]) / (times[1] - times[0])
    assert 0 < amount < 1
    expected = _unit([math.sin((1 - amount) * angle) / math.sin(angle) * x +
                      math.sin(amount * angle) / math.sin(angle) * y for x, y in zip(a, b)])
    poses = {pose["source"]: pose for pose in runtime["animation_poses"]}
    assert len(runtime["animation_poses"]) == len(poses) == 3 and set(poses) == set(NODES)
    for index, node in enumerate(NODES):
        pose = poses[node]
        assert pose["entity"] == _instanced(node)
        _close(pose["translation"], [0., 1. if index == 2 else 0., 0.])
        _close(pose["scale"], [1., 1., 1.])
        _close(pose["rotation"], expected if index == 2 else [0., 0., 0., 1.])
    return {"status": "passed", "world": runtime["world"], "animators": 1,
            "source_nodes": 3, "sample_time": animation["time"], "quaternion_matches_source_slerp": True,
            "playing": animation["playing"], "duration": animation["duration"], "speed": animation["speed"]}


def verify_cubic(case, assetc, editor, template, args, run, run_capture, inspect_capture, read_product):
    """Export a generated cubic variant; keep the sole upstream asset unchanged."""
    out = case.parent
    project, products = case / "cubic-project", case / "cubic-products"
    run("cubic_create_project", [assetc, "create-project", project])
    model = project / "assets/model"
    shutil.copytree(Path(__file__).resolve().parents[2] / "demo/assets/models/simple_skin", model)
    path = model / "SimpleSkin.gltf"
    source = json.loads(path.read_text())
    values = [[0.] * 4, [0., 0., 0., 1.], [0., 0., 4., 0.],
              [0.] * 4, [0., 0., 0., -1.], [0.] * 4]
    binary = struct.pack("<2f", 0., 2.) + b"".join(struct.pack("<4f", *value) for value in values)
    (model / "SimpleSkin_animation.bin").write_bytes(binary)
    source["buffers"][3]["byteLength"] = source["bufferViews"][4]["byteLength"] = len(binary)
    source["accessors"][5].update(count=2, max=[2.])
    source["accessors"][6].update(byteOffset=8, count=6)
    for bound in ("min", "max"):
        source["accessors"][6].pop(bound, None)
    source["animations"][0]["samplers"][0]["interpolation"] = "CUBICSPLINE"
    source["animations"][0].pop("extras", None)  # Retain event-free v2 compatibility coverage.
    path.write_text(json.dumps(source))
    scene_path = project / "assets/main.scene"
    scene = json.loads(scene_path.read_text())
    scene["instances"] = [{"id": INSTANCE, "prefab": SOURCE, "parent": None, "overrides": []}]
    scene_path.write_text(json.dumps(scene))
    run("cubic_prepare", [assetc, "prepare", project, "--output", products])
    built = case / ("cubic-game.app" if sys.platform == "darwin" else "cubic-game")
    export_report = out / "cubic-export.json"
    run("cubic_export", [editor, "--app-template", template, "--project", project, "--products", products,
                         "--export-game", built, "--report", export_report])
    exported = json.loads(export_report.read_text())
    assert exported["status"] == "passed"
    built = Path(exported["root"])
    executable = Path(exported["executable"]).relative_to(built)
    packaged_products = Path(exported["products"]).relative_to(built)
    relocated = case / ("relocated-cubic.app" if sys.platform == "darwin" else "relocated-cubic")
    built.rename(relocated)
    shutil.rmtree(project)
    shutil.rmtree(products)
    index = json.loads((relocated / packaged_products / "index.json").read_text())
    # Inspect through the product index; the exported runtime never needs the glTF source.
    run("cubic_inspect", [assetc, "inspect", relocated / packaged_products, CLIP])
    protocols = json.loads(template.read_text())["identity"]["protocols"]
    clip = read_product(relocated / packaged_products, index["products"], CLIP, protocols)["data"]
    assert clip["version"] == 2 and clip["duration"] == 2.
    assert clip["channels"][0]["interpolation"] == "cubic_spline"
    assert clip["channels"][0]["values"]["data"] == values
    captures = []
    if args.gpu or args.native:
        for native in ([False, True] if args.native else [False]):
            name = "cubic-app-native" if native else "cubic-app-headless"
            command = [relocated / executable, "--smoke", "--report", out / f"{name}.json",
                       "--capture", out / f"{name}.png"]
            if not native:
                command.append("--headless")
            run_capture(name, command, cwd=case)
            value, png = inspect_capture(name, native, 1)
            runtime = value["application"]
            assert 0 < runtime["steps"] <= 7 and runtime["script_diagnostics"] == []
            if not native:
                assert runtime["steps"] == 7
            time = runtime["animations"][0]["time"]
            assert math.isclose(time, runtime["steps"] / 60, abs_tol=1e-9)
            assert math.isclose(runtime["elapsed"], time, abs_tol=1e-9)
            t = time / 2.
            expected = _unit([0., 0., 8 * (t**3 - 2 * t**2 + t), 4 * t**3 - 6 * t**2 + 1])
            pose = next(pose for pose in runtime["animation_poses"] if pose["source"] == NODES[2])
            _close(pose["rotation"], expected)
            captures.append({"name": name, "sample_time": time, "png_sha256": png["sha256"]})
    return {"status": "passed", "clip_version": 2, "package": str(relocated),
            "authoring_sources_absent": not project.exists(), "runtime_captures": captures}
