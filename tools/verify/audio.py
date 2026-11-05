"""Inspect packaged PCM and real device reports without constructing an audio output."""
import array
import hashlib
import json
import math
import sys
import wave
from pathlib import Path

AUDIO_SCENE = "a0000000-0000-4000-8000-000000000008"
LOOP = "a0000000-0000-4000-8000-000000000009"
CUE = "a0000000-0000-4000-8000-00000000000a"
SCRIPT = "a0000000-0000-4000-8000-00000000000b"
CUE_ENTITY = "b2000000-0000-4000-8000-000000000002"
_LOOP_ENTITY = "b2000000-0000-4000-8000-000000000001"
_START = "音频示例开始：E 重播提示音，空格停止提示音。"
_STOP = "音频示例停止。"


def inspect_package(products: Path, protocols: dict, read_product, author_assets: Path) -> dict:
    index = json.loads((products / "index.json").read_text(encoding="utf-8"))
    assert index["version"] == protocols["index"]
    entries = index["products"]
    assert all("editor" not in entry for entry in entries.values()), "package retains editor provenance"
    for asset, kind in ((AUDIO_SCENE, "scene"), (LOOP, "audio"), (CUE, "audio"), (SCRIPT, "script")):
        assert entries[asset]["kind"] == kind, f"wrong audio dependency kind: {asset}"
    assert {LOOP, CUE, SCRIPT} <= set(entries[AUDIO_SCENE]["dependencies"])
    assert all(entries[asset]["dependencies"] == [] for asset in (LOOP, CUE, SCRIPT))
    source = read_product(products, entries, AUDIO_SCENE, protocols)["data"]
    assert protocols["scene"] >= 7
    assert source["version"] == 1 and source["base"]["version"] == 7, "2D author scene changed protocol"
    owners = {entity["id"]: entity for entity in source["base"]["entities"] if entity.get("audio_source")}
    assert set(owners) == {_LOOP_ENTITY, CUE_ENTITY}
    assert owners[_LOOP_ENTITY]["audio_source"] == {
        "clip": LOOP, "gain": 0.18, "looping": True, "autoplay": True}
    assert owners[CUE_ENTITY]["audio_source"] == {
        "clip": CUE, "gain": 0.65, "looping": False, "autoplay": False}
    binding = owners[CUE_ENTITY]["script"]
    assert binding["asset"] == SCRIPT and binding["enabled"] is True and binding["parameters"] == {}
    script = read_product(products, entries, SCRIPT, protocols)["data"]
    expected = (author_assets / "scripts/audio_demo.rhai").read_text(encoding="utf-8")
    assert script["version"] == 1 and script["source"] == expected
    clips = {}
    for asset, name in ((LOOP, "ambient_loop.wav"), (CUE, "play_chime.wav")):
        data = read_product(products, entries, asset, protocols)["data"]
        path = author_assets / "audio" / name
        with wave.open(str(path), "rb") as author:
            assert author.getcomptype() == "NONE" and author.getsampwidth() == 2
            rate, channels, frames = author.getframerate(), author.getnchannels(), author.getnframes()
            raw = author.readframes(frames)
        assert data["version"] == 1 and data["sample_rate"] == rate and data["channels"] == channels
        assert channels in (1, 2) and 8000 <= rate <= 192000
        samples = data["samples"]
        assert 0 < len(samples) <= 8_388_608 and len(samples) == frames * channels
        assert all(type(sample) is int and -32768 <= sample <= 32767 for sample in samples)
        decoded = array.array("h", samples)
        if sys.byteorder != "little":
            decoded.byteswap()
        assert decoded.tobytes() == raw and len(raw) == frames * channels * 2, "PCM differs from author WAV"
        clips[asset] = {"sample_rate": rate, "channels": channels, "frames": frames,
                        "pcm_sha256": hashlib.sha256(raw).hexdigest(),
                        "wav_sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
    package_root = products.parent
    if package_root.name == "Resources" and package_root.parent.name == "Contents":
        package_root = package_root.parent.parent
    assert not any(path.suffix.lower() in (".wav", ".meta") for path in package_root.rglob("*")), \
        "package retains author WAV or metadata files"
    return {"status": "passed", "scene": AUDIO_SCENE, "clips": clips,
            "script_sha256": hashlib.sha256(expected.encode()).hexdigest(),
            "editor_provenance_stripped": True, "authoring_sources_absent": True}


def inspect_runtime(value: dict) -> dict:
    runtime = value["application"]
    assert runtime["scene"] == AUDIO_SCENE and runtime["steps"] == 7
    assert runtime["character"] is None and math.isclose(runtime["elapsed"], 7 / 60, abs_tol=1e-9)
    assert value["audio_shutdown"] is True, "host did not shut down its audio output"
    world = runtime["world"]
    diagnostics = runtime["script_diagnostics"]
    assert len(diagnostics) == 2
    for entry, (phase, message, tick) in zip(diagnostics, (("on_start", _START, 0), ("on_stop", _STOP, 7))):
        assert entry["phase"] == phase and entry["message"] == message and entry["tick"] == tick
        assert entry["world"] == world and entry["entity"] == CUE_ENTITY and entry["asset"] == SCRIPT
        assert entry["failed"] is False
    assert diagnostics[0]["sequence"] < diagnostics[1]["sequence"]
    audio = value["audio"]
    assert audio["world"] in (None, world), "audio report belongs to a different world"
    started, voices = audio["generations_started"], audio["voices"]
    assert type(started) is int and 0 <= started <= 2
    assert len(voices) <= 2 and len({voice["entity"] for voice in voices}) == len(voices)
    for voice in voices:
        assert voice["entity"] in {_LOOP_ENTITY, CUE_ENTITY} and voice["generation"] == 1
        assert type(voice["playing"]) is bool and type(voice["pending"]) is bool
        assert math.isfinite(voice["position_seconds"]) and voice["position_seconds"] >= 0
    assert audio["active_voices"] == sum(voice["playing"] for voice in voices)
    assert audio["pending_voices"] == sum(voice["pending"] for voice in voices)
    progress = max((voice["position_seconds"] for voice in voices), default=0.)
    state = audio["state"]
    if state == "ready":
        assert audio["world"] == world and audio["error"] is None
        assert started == 2 and {voice["entity"] for voice in voices} == {_LOOP_ENTITY, CUE_ENTITY}
        assert audio["pending_voices"] == 0 and audio["cached_clips"] == 2
        assert progress > 0, "device opened but no audio callback progress was observed"
    else:
        assert state == "unavailable", "two requested voices cannot leave a successful idle output"
        assert isinstance(audio["error"], str) and 0 < len(audio["error"].encode()) <= 4096
        assert audio["active_voices"] == audio["pending_voices"] == audio["cached_clips"] == 0
    return {"status": "passed" if state == "ready" else "unavailable", "output_state": state,
            "world": world, "generations_started": started, "reported_voices": len(voices),
            "max_position_seconds": progress, "device_callback_observed": state == "ready" and progress > 0,
            "error": audio["error"], "audible_verification": "manual_pending"}
