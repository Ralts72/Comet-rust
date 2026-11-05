"""Inspect the spatial demo's products and actual output observations without opening a device."""
import hashlib
import json
import math
from pathlib import Path

from audio import CUE, LOOP

SCENE = "a0000000-0000-4000-8000-00000000000c"
SCRIPT = "a0000000-0000-4000-8000-00000000000d"
EMITTER = "b3000000-0000-4000-8000-000000000001"
_START = "空间音频示例开始：E 开关移动声源，环绕相机比较方向和距离。"
_STOP = "空间音频示例停止。"


def inspect_package(products: Path, protocols: dict, read_product, author_assets: Path) -> dict:
    index = json.loads((products / "index.json").read_text(encoding="utf-8"))
    assert index["version"] == protocols["index"] and protocols["scene"] >= 8
    entries = index["products"]
    assert all("editor" not in entry for entry in entries.values()), "package retains editor provenance"
    assert entries[SCENE]["kind"] == "scene" and entries[SCRIPT]["kind"] == "script"
    assert {CUE, SCRIPT} <= set(entries[SCENE]["dependencies"])
    assert {asset for asset, entry in entries.items() if entry["kind"] == "audio"} == {LOOP, CUE}
    assert entries[CUE]["dependencies"] == entries[SCRIPT]["dependencies"] == []
    scene = read_product(products, entries, SCENE, protocols)["data"]
    assert scene["version"] == 1 and scene["base"]["version"] == 8
    owners = [entity for entity in scene["base"]["entities"] if entity.get("audio_source")]
    assert len(owners) == 1 and owners[0]["id"] == EMITTER
    source, binding = owners[0]["audio_source"], owners[0]["script"]
    assert source["clip"] == CUE and source["gain"] == 0.9
    assert source["spatial"] is True and source["looping"] is True and source["autoplay"] is True
    assert source.get("min_distance", 1.) == 1. and source.get("max_distance", 30.) == 30.
    assert owners[0]["transform"]["translation"] == [-3., 0.7, 0.]
    assert binding["asset"] == SCRIPT and binding["enabled"] is True and binding["parameters"] == {}
    script = read_product(products, entries, SCRIPT, protocols)["data"]
    expected = (author_assets / "scripts/spatial_audio.rhai").read_text(encoding="utf-8")
    assert script["version"] == 1 and script["source"] == expected
    package_root = products.parent
    if package_root.name == "Resources" and package_root.parent.name == "Contents":
        package_root = package_root.parent.parent
    assert not any(path.suffix.lower() in (".wav", ".meta") for path in package_root.rglob("*"))
    return {"status": "passed", "scene": SCENE, "emitter": EMITTER, "shared_clip": CUE,
            "script_sha256": hashlib.sha256(expected.encode()).hexdigest(),
            "editor_provenance_stripped": True, "authoring_sources_absent": True}


def _vector(value, size=3):
    assert len(value) == size and all(type(item) in (int, float) and math.isfinite(item) for item in value)
    return value


def _close(actual, expected):
    assert all(math.isclose(a, b, rel_tol=1e-6, abs_tol=2e-5) for a, b in zip(actual, expected))


def _cross(a, b):
    return [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]


def _unit(value):
    length = math.hypot(*value)
    assert length > 0
    return [item / length for item in value]


def _rotate(rotation, value):
    first = _cross(rotation[:3], value)
    second = _cross(rotation[:3], first)
    return [value[i] + 2 * (rotation[3] * first[i] + second[i]) for i in range(3)]


def inspect_runtime(value: dict) -> dict:
    runtime, audio = value["application"], value["audio"]
    assert runtime["scene"] == SCENE and runtime["steps"] == 7 and runtime["character"] is None
    assert math.isclose(runtime["elapsed"], 7 / 60, abs_tol=1e-9) and value["audio_shutdown"] is True
    world, diagnostics = runtime["world"], runtime["script_diagnostics"]
    assert len(diagnostics) == 2
    for entry, (phase, message, tick) in zip(diagnostics, (("on_start", _START, 0), ("on_stop", _STOP, 7))):
        assert entry["phase"] == phase and entry["message"] == message and entry["tick"] == tick
        assert entry["world"] == world and entry["entity"] == EMITTER and entry["asset"] == SCRIPT
        assert entry["failed"] is False
    assert diagnostics[0]["sequence"] < diagnostics[1]["sequence"]
    assert audio["world"] in (None, world)
    started, voices, listener = audio["generations_started"], audio["voices"], audio.get("listener")
    assert type(started) is int and 0 <= started <= 1 and len(voices) <= 1
    state = audio["state"]
    camera = runtime["camera"]
    eye, target = _vector(camera["eye"]), _vector(camera["target"])
    if listener is not None:
        _vector(listener["position"])
        rotation = _vector(listener["rotation"], 4)
        assert math.isclose(sum(item * item for item in rotation), 1., abs_tol=2e-5)
        if state == "ready":
            _close(listener["position"], eye)
            forward = _unit([target[i] - eye[i] for i in range(3)])
            _close(_rotate(rotation, [0., 0., -1.]), forward)
            _close(_rotate(rotation, [1., 0., 0.]), _unit(_cross(forward, [0., 1., 0.])))
    for voice in voices:
        assert voice["entity"] == EMITTER and voice["generation"] == 1
        assert type(voice["playing"]) is bool and type(voice["pending"]) is bool
        assert math.isfinite(voice["position_seconds"]) and voice["position_seconds"] >= 0
        if voice.get("spatial") is not None:
            _vector(voice["spatial"]["position"])
            minimum, maximum = voice["spatial"]["min_distance"], voice["spatial"]["max_distance"]
            assert math.isfinite(minimum) and math.isfinite(maximum) and 0 <= minimum < maximum
        if voice.get("relative_position") is not None:
            _vector(voice["relative_position"])
    assert audio["active_voices"] == sum(voice["playing"] for voice in voices)
    assert audio["pending_voices"] == sum(voice["pending"] for voice in voices)
    progress = max((voice["position_seconds"] for voice in voices), default=0.)
    if state == "ready":
        assert listener is not None and audio["world"] == world and audio["error"] is None
        assert started == len(voices) == audio["cached_clips"] == audio["active_voices"] == 1
        assert audio["pending_voices"] == 0 and progress > 0, "device callback progress is missing"
        spatial = voices[0]["spatial"]
        position = _vector(spatial["position"])
        assert spatial["min_distance"] == 1. and spatial["max_distance"] == 30.
        assert math.isclose(position[1], 0.7, abs_tol=2e-6)
        assert math.isclose(position[0] ** 2 + position[2] ** 2, 9., abs_tol=2e-5)
        assert math.hypot(position[0] + 3., position[2]) > 0.001, "spatial source did not move"
        _close(_vector(voices[0]["relative_position"]), [position[i] - listener["position"][i] for i in range(3)])
    else:
        assert state == "unavailable", "a spatial voice cannot leave a successful idle output"
        assert isinstance(audio["error"], str) and 0 < len(audio["error"].encode()) <= 4096
        assert audio["active_voices"] == audio["pending_voices"] == audio["cached_clips"] == 0
    return {"status": "passed" if state == "ready" else "unavailable", "output_state": state,
            "world": world, "generations_started": started, "reported_voices": len(voices),
            "max_position_seconds": progress, "device_callback_observed": state == "ready" and progress > 0,
            "listener_matches_camera": state == "ready", "error": audio["error"],
            "audible_verification": "manual_pending"}
