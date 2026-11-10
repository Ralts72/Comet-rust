"""Inspect the actual collect demo and its isolated, player-overlap smoke package."""
import hashlib
import json
import math
from pathlib import Path

from animation import CLIP, MATERIAL, MESH, NODES, SKELETON, SKIN, SOURCE, _child, _instanced
from audio import CUE

SCENE = "a0000000-0000-4000-8000-00000000000e"
SCRIPT = "a0000000-0000-4000-8000-00000000000f"
OWNER = "b4000000-0000-4000-8000-000000000001"
PLAYER = "b4000000-0000-4000-8000-000000000002"
VISUAL = "b4000000-0000-4000-8000-000000000003"
RIG_PARENT = "b4000000-0000-4000-8000-00000000000b"
RIG_INSTANCE = "b4000000-0000-4000-8000-000000000010"
SOURCE_CONTROLLER = _child("animation:controller")
ANIMATOR = _instanced(SOURCE_CONTROLLER, RIG_INSTANCE)
RIG_NODES = tuple(_instanced(source, RIG_INSTANCE) for source in NODES)
NEXT_SCENE = "a0000000-0000-4000-8000-000000000003"
INITIAL = "收集目标：0 / 1\n走近金色方块收集。"
PLAYING = "收集目标：1 / 1\n动作播放中，请稍候。"
COMPLETE = "收集目标：1 / 1\n动作完成！重新按 E 前往第二关。"
_START = "收集示例开始：走近金色方块，动作结束后重新按 E 前往第二关。"
_COLLECTED = "已收集目标：1 / 1。"
_STOP = "收集示例停止。"


def prepare_smoke_project(project: Path) -> None:
    assert project.resolve() != Path(__file__).resolve().parents[2] / "demo", "smoke requires an isolated copy"
    path = project / "assets/collect.scene"
    scene = json.loads(path.read_text(encoding="utf-8"))
    entities = {entity["id"]: entity for entity in scene["base"]["entities"]}
    assert entities[OWNER]["physics"]["sensor"] is True and entities[OWNER]["script"]["asset"] == SCRIPT
    entities[PLAYER]["transform"]["translation"] = list(entities[OWNER]["transform"]["translation"])
    path.write_text(json.dumps(scene, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def inspect_package(products: Path, protocols: dict, read_product, author_assets: Path) -> dict:
    index = json.loads((products / "index.json").read_text(encoding="utf-8"))
    assert index["version"] == protocols["index"] and protocols["scene"] >= 10
    entries = index["products"]
    assert all("editor" not in entry for entry in entries.values()), "package retains editor provenance"
    assert entries[SCENE]["kind"] == "scene" and entries[SCRIPT]["kind"] == "script"
    dependencies = set(entries[SCENE]["dependencies"])
    assert {CUE, SCRIPT, SOURCE} <= dependencies
    assert entries[SOURCE]["kind"] == "prefab"
    for asset, kind in ((SKELETON, "skeleton"), (SKIN, "skin"), (CLIP, "animation_clip"),
                        (MESH, "mesh"), (MATERIAL, "material")):
        assert entries[asset]["kind"] == kind
    assert entries[CUE]["kind"] == "audio" and entries[SCRIPT]["dependencies"] == []
    assert entries[NEXT_SCENE]["kind"] == "scene"
    scene = read_product(products, entries, SCENE, protocols)["data"]
    assert scene["version"] == 1 and scene["base"]["version"] == 10
    assert scene["instances"] == [{"id": RIG_INSTANCE, "prefab": SOURCE, "parent": RIG_PARENT,
        "overrides": [{"entity": SOURCE_CONTROLLER, "field": {"component": 13, "field": 4},
                       "value": {"type": "bool", "value": False}}]}]
    entities = {entity["id"]: entity for entity in scene["base"]["entities"]}
    assert len(entities) == 11 and len([entity for entity in entities.values() if entity.get("model")]) == 6
    assert all(entity.get("animator") is None and entity.get("skin") is None for entity in entities.values())
    owner, player, visual = (entities[asset] for asset in (OWNER, PLAYER, VISUAL))
    assert owner["parent"] is None and owner["physics"]["sensor"] is True
    assert owner["physics"]["motion"] == "fixed" and owner["physics"]["shape"]["kind"] == "box"
    assert player["character"]["player_controlled"] is True and player["physics"]["sensor"] is False
    assert player["transform"]["translation"] == owner["transform"]["translation"], "smoke player is not in the real sensor"
    assert visual["parent"] == OWNER and visual.get("physics") is None
    assert all(component > 0 for component in visual["transform"]["scale"])
    assert sum(entity["parent"] == VISUAL and entity.get("model") is not None for entity in entities.values()) == 2
    audio, binding = owner["audio_source"], owner["script"]
    assert audio["clip"] == CUE and audio["gain"] == 0.65 and not audio["looping"] and not audio["autoplay"]
    assert audio.get("spatial", False) is False and binding["asset"] == SCRIPT and binding["enabled"] is True
    assert binding["parameters"] == {
        "player": {"type": "entity", "value": {"scope": "local", "entity": PLAYER}},
        "visual": {"type": "entity", "value": {"scope": "local", "entity": VISUAL}},
        "next_scene": {"type": "text", "value": NEXT_SCENE},
        "animator": {"type": "entity", "value": {"scope": "in_instance", "instance": RIG_INSTANCE,
                                                   "entity": SOURCE_CONTROLLER}},
    }
    container = entities[RIG_PARENT]
    assert container["parent"] is None and container["transform"]["translation"] == [1.8, 0., -0.5]
    prefab = read_product(products, entries, SOURCE, protocols)["data"]
    assert prefab["base"]["version"] == 9 and prefab["instances"] == []
    rig = {entity["id"]: entity for entity in prefab["base"]["entities"]}
    assert len(rig) == 5
    controller = rig[SOURCE_CONTROLLER]
    assert controller["parent"] is None
    assert controller["animator"] == {
        "skeleton": SKELETON, "clip": CLIP, "looping": True, "speed": 1.,
        "bindings": {source: {"scope": "local", "entity": source} for source in NODES},
    }
    for index, node in enumerate(NODES):
        assert rig[node]["parent"] == (NODES[1] if index == 2 else SOURCE_CONTROLLER)
        assert rig[node]["transform"]["translation"] == [0., 1. if index == 2 else 0., 0.]
    mesh = rig[_child("node:index:0/primitive:0")]
    assert mesh["parent"] == NODES[0]
    assert mesh["model"]["mesh"] == MESH and mesh["model"]["surface"]["material"] == MATERIAL
    assert mesh["skin"] == {"skin": SKIN, "animator": {"scope": "local", "entity": SOURCE_CONTROLLER}}
    original = json.loads((author_assets / "collect.scene").read_text(encoding="utf-8"))
    original = {entity["id"]: entity for entity in original["base"]["entities"]}
    assert math.dist(original[PLAYER]["transform"]["translation"], original[OWNER]["transform"]["translation"]) > 2.
    script = read_product(products, entries, SCRIPT, protocols)["data"]
    expected = (author_assets / "scripts/collect.rhai").read_text(encoding="utf-8")
    assert script["version"] == 1 and script["source"] == expected
    package_root = products.parent
    if package_root.name == "Resources" and package_root.parent.name == "Contents":
        package_root = package_root.parent.parent
    assert not any(path.suffix.lower() in (".wav", ".meta", ".rhai", ".scene", ".gltf", ".bin")
                   for path in package_root.rglob("*"))
    return {"status": "passed", "scene": SCENE, "owner": OWNER, "shared_clip": CUE,
            "script_sha256": hashlib.sha256(expected.encode()).hexdigest(),
            "smoke_player_in_sensor": True, "normal_author_requires_movement": True,
            "shared_animation_clip": CLIP, "prefab_instance": RIG_INSTANCE, "shared_rig_nodes": 3,
            "editor_provenance_stripped": True, "authoring_sources_absent": True}


def inspect_runtime(value: dict) -> dict:
    runtime, audio = value["application"], value["audio"]
    assert runtime["scene"] == SCENE and runtime["steps"] == 7 and runtime["character"] is not None
    assert runtime["progress_entries"] == 1
    assert math.isclose(runtime["elapsed"], 7 / 60, abs_tol=1e-9) and value["audio_shutdown"] is True
    world, hud, diagnostics = runtime["world"], runtime["hud"], runtime["script_diagnostics"]
    assert hud == {"world": world, "labels": [{"owner": OWNER, "text": PLAYING}]}
    animations = runtime["animations"]
    assert len(animations) == 1
    animation = animations[0]
    assert animation["entity"] == ANIMATOR and animation["skeleton"] == SKELETON and animation["clip"] == CLIP
    assert animation["bindings"] == 3 and math.isclose(animation["time"], 5 / 60, abs_tol=1e-9)
    assert animation["duration"] == 5.5 and animation["playing"] is True and animation["speed"] == 1
    poses = {pose["source"]: pose for pose in runtime["animation_poses"]}
    assert len(runtime["animation_poses"]) == len(poses) == 3 and set(poses) == set(NODES)
    for index, (source, target) in enumerate(zip(NODES, RIG_NODES)):
        pose = poses[source]
        assert pose["entity"] == target and pose["translation"] == [0., 1. if index == 2 else 0., 0.]
        assert pose["scale"] == [1., 1., 1.]
        rotation = pose["rotation"]
        assert len(rotation) == 4 and all(math.isfinite(component) for component in rotation)
        assert math.isclose(math.hypot(*rotation), 1., abs_tol=1e-6)
        if index == 2:
            assert rotation[0] == rotation[1] == 0. and 0. < rotation[2] < 0.1 and rotation[3] > 0.99
        else:
            assert rotation == [0., 0., 0., 1.]
    assert len(diagnostics) == 3
    for entry, (phase, message, tick) in zip(diagnostics, (
            ("on_start", _START, 0), ("on_trigger", _COLLECTED, 1), ("on_stop", _STOP, 7))):
        assert entry["phase"] == phase and entry["message"] == message and entry["tick"] == tick
        assert entry["world"] == world and entry["entity"] == OWNER and entry["asset"] == SCRIPT
        assert entry["failed"] is False
    assert all(first["sequence"] < second["sequence"] for first, second in zip(diagnostics, diagnostics[1:]))
    started, voices, state = audio["generations_started"], audio["voices"], audio["state"]
    assert audio["world"] in (None, world) and type(started) is int and 0 <= started <= 1 and len(voices) <= 1
    for voice in voices:
        assert voice["entity"] == OWNER and voice["generation"] == 1 and voice.get("spatial") is None
        assert type(voice["playing"]) is bool and type(voice["pending"]) is bool
        assert math.isfinite(voice["position_seconds"]) and voice["position_seconds"] >= 0
    assert audio["active_voices"] == sum(voice["playing"] for voice in voices)
    assert audio["pending_voices"] == sum(voice["pending"] for voice in voices)
    progress = max((voice["position_seconds"] for voice in voices), default=0.)
    if state == "ready":
        assert audio["world"] == world and audio["error"] is None
        assert started == len(voices) == audio["cached_clips"] == 1
        assert audio["pending_voices"] == 0 and progress > 0, "collect cue has no device callback progress"
    else:
        assert state == "unavailable", "a collected cue cannot leave an idle output"
        assert isinstance(audio["error"], str) and 0 < len(audio["error"].encode()) <= 4096
        assert audio["active_voices"] == audio["pending_voices"] == audio["cached_clips"] == 0
    return {"status": "passed" if state == "ready" else "unavailable", "world": world,
            "hud": hud, "collected_once": True, "progress_entries": 1, "animation_time": animation["time"],
            "animation_playing": True, "generations_started": started,
            "max_position_seconds": progress, "device_callback_observed": state == "ready" and progress > 0,
            "error": audio["error"], "audible_verification": "manual_pending"}
