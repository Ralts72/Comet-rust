"""Verify the two playable collection levels in a source-free exported App."""
import hashlib
import json
import math

from animation import CLIP

SCENES = ("451b1764-57d9-5ab3-9f16-bcd2ef37ff6a", "4e06a93f-1f43-56f9-b72c-48c197ef7271")
SCRIPT = "3b84299b-156d-5435-9588-7e3d9270aa5d"
MODEL = "331e3889-e9ac-527e-ac27-0ae02585657f"
CONTROLLER = "412ab660-dc19-5a08-95f2-8e0439de26ca"
FOOTSTEP = "b064757d-3011-544d-bafc-8756cc8561cd"
FEEDBACK = "e4266474-5bd6-5c3b-b0a7-a83ac42133ba"
LEVELS = (("character", SCENES[0], 19, "第一关·庭院"),
          ("character_second", SCENES[1], 17, "第二关·方塔"))


def inspect_package(products, protocols, read_product, author_assets):
    entries = json.loads((products / "index.json").read_text())["products"]
    for name, scene, _, _ in LEVELS:
        content = read_product(products, entries, scene, protocols)["data"]
        assert len(content["instances"]) == 4 and content["base"]["version"] == protocols["scene"]
        avatar = next(instance for instance in content["instances"] if instance["prefab"] == MODEL)
        driver = next(e for e in content["base"]["entities"] if e.get("character_animation"))
        assert driver["character_animation"]["animator"]["instance"] == avatar["id"]
        assert driver["audio_source"]["clip"] == FOOTSTEP and driver["audio_source"]["spatial"] is True
        assert driver["script"]["asset"] == FEEDBACK
        assert {override["field"]["field"] for override in avatar["overrides"]} == {2, 6}
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
    controller = read_product(products, entries, CONTROLLER, protocols)["data"]
    assert entries[CONTROLLER]["kind"] == "animation_controller" and controller["version"] == 1
    assert controller["parameters"] == {"speed": 0., "grounded": True, "rising": False, "crouched": False}
    states = {state["name"]: state for state in controller["states"]}
    assert set(states) == {"ground", "crouch", "jump", "fall"}
    clips = {motion["clip"] for state in states.values() for motion in state["motions"]}
    assert len(clips) == 7 and {controller["skeleton"], *clips} == set(entries[CONTROLLER]["dependencies"])
    for clip in clips:
        data = read_product(products, entries, clip, protocols)["data"]
        assert data["skeleton"] == controller["skeleton"] and data["duration"] > 0
    assert len(read_product(products, entries, controller["skeleton"], protocols)["data"]["nodes"]) == 67
    assert read_product(products, entries, FEEDBACK, protocols)["data"]["source"] == (author_assets / "scripts/character_feedback.rhai").read_text()
    return {"status": "passed", "scenes": list(SCENES), "independent_targets": 6,
            "clip_event_version": 3, "controller_version": 1, "locomotion_clips": len(clips),
            "script_sha256": hashlib.sha256(source.encode()).hexdigest()}


def inspect_runtime(report, scene, label, native=False):
    runtime = report["application"]
    assert runtime["scene"] == scene
    assert type(runtime["steps"]) is int and runtime["steps"] >= 0
    if not native:
        assert runtime["steps"] == 7
    assert math.isclose(runtime["elapsed"], runtime["steps"] / 60, abs_tol=1e-9)
    assert runtime["character"] is not None
    assert all(not entry["failed"] for entry in runtime["script_diagnostics"])
    assert len(runtime["animations"]) == 4
    targets = [animation for animation in runtime["animations"] if animation["controller"] is None]
    assert len(targets) == 3 and all(animation["time"] == 0 and animation["playing"] is False for animation in targets)
    avatar = next(animation for animation in runtime["animations"] if animation["controller"] == CONTROLLER)
    assert avatar["state"] in {"ground", "fall"} and avatar["playing"] is True and avatar["bindings"] == 67
    assert len(runtime["hud"]["labels"]) == 1
    text = runtime["hud"]["labels"][0]["text"]
    assert label in text and "收集目标：0 / 3" in text
    assert report["audio"]["generations_started"] == 0 and report["audio_shutdown"] is True
    return {"status": "passed", "scene": scene, "steps": runtime["steps"], "native": native, "paused_animations": 3, "avatar_state": avatar["state"], "hud": text,
            "scope": "packaged App startup; full collection/return/checkpoint chain covered by editor CPU regression"}
