"""Verify the two playable collection levels in a source-free exported App."""
import hashlib
import json
import math

from animation import CLIP

SCENES = ("451b1764-57d9-5ab3-9f16-bcd2ef37ff6a", "4e06a93f-1f43-56f9-b72c-48c197ef7271")
SCRIPT = "3b84299b-156d-5435-9588-7e3d9270aa5d"
LEVELS = (("character", SCENES[0], 19, "第一关·庭院"),
          ("character_second", SCENES[1], 17, "第二关·方塔"))


def inspect_package(products, protocols, read_product, author_assets):
    entries = json.loads((products / "index.json").read_text())["products"]
    for name, scene, _, _ in LEVELS:
        content = read_product(products, entries, scene, protocols)["data"]
        assert len(content["instances"]) == 3
        targets = [e for e in content["base"]["entities"] if e["name"].startswith("收集目标 ")]
        assert len(targets) == 3 and len({e["id"] for e in targets}) == 3
        assert all(e["physics"]["sensor"] and e["script"]["parameters"]["standalone"]["value"] is False for e in targets)
        coordinator = next(e["script"] for e in content["base"]["entities"] if (e.get("script") or {}).get("asset") == SCRIPT)
        assert coordinator["parameters"]["next_scene"]["value"] == SCENES[1 - SCENES.index(scene)]
        assert {coordinator["parameters"][f"target_{i}"]["value"]["entity"] for i in (1, 2, 3)} == {e["id"] for e in targets}
    clip = read_product(products, entries, CLIP, protocols)["data"]
    assert clip["version"] == 3 and clip["events"] == [{"phase": 1.0, "name": "动作完成"}]
    source = (author_assets / "scripts/collection_level.rhai").read_text()
    assert read_product(products, entries, SCRIPT, protocols)["data"]["source"] == source
    return {"status": "passed", "scenes": list(SCENES), "independent_targets": 6,
            "clip_event_version": 3, "script_sha256": hashlib.sha256(source.encode()).hexdigest()}


def inspect_runtime(report, scene, label, native=False):
    runtime = report["application"]
    assert runtime["scene"] == scene
    assert type(runtime["steps"]) is int and runtime["steps"] >= 0
    if not native:
        assert runtime["steps"] == 7
    assert math.isclose(runtime["elapsed"], runtime["steps"] / 60, abs_tol=1e-9)
    assert runtime["character"] is not None
    assert all(not entry["failed"] for entry in runtime["script_diagnostics"])
    assert len(runtime["animations"]) == 3
    assert all(animation["time"] == 0 and animation["playing"] is False for animation in runtime["animations"])
    assert len(runtime["hud"]["labels"]) == 1
    text = runtime["hud"]["labels"][0]["text"]
    assert label in text and "收集目标：0 / 3" in text
    assert report["audio"]["generations_started"] == 0 and report["audio_shutdown"] is True
    return {"status": "passed", "scene": scene, "steps": runtime["steps"], "native": native, "paused_animations": 3, "hud": text,
            "scope": "packaged App startup; full collection/return/checkpoint chain covered by editor CPU regression"}
