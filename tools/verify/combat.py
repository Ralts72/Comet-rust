"""Inspect the event-driven arena in a relocated, source-free game package."""
import json
import math

SCENE = "799d31eb-db7a-560e-badd-593eddaa4d89"
CONTROLLER = "171b6088-5cbc-5822-9396-573149ea3b4e"
SCRIPT = "c76d234d-3469-58c4-bf1c-a84eb5cb73e4"
NAVIGATION = "8517122b-7685-5fa9-b8bd-a751c0c322db"
DATA = "771c1eeb-1922-5fe5-a992-2244052ab3c7"
DRAW_COUNT = 10


def inspect_package(products, protocols, read_product, author_assets):
    entries = json.loads((products / "index.json").read_text())["products"]
    scene = read_product(products, entries, SCENE, protocols)["data"]
    assert scene["base"]["version"] == 12 and len(scene["instances"]) == 3
    actors = [entity for entity in scene["base"]["entities"] if entity.get("character")]
    assert len(actors) == 3 and sum(actor["character"]["player_controlled"] for actor in actors) == 1
    assert sum(bool(actor.get("navigation_agent")) for actor in actors) == 2
    assert entries[NAVIGATION]["kind"] == "navigation"
    assert entries[DATA]["kind"] == "data" and DATA in entries[SCENE]["dependencies"]
    data = read_product(products, entries, DATA, protocols)["data"]
    author = json.loads((author_assets / "data/combat.data.json").read_text())
    assert data == {"version": 1, "fields": author["fields"], "rows": {row["id"]: row for row in author["rows"]}}
    coordinator = next(entity for entity in scene["base"]["entities"] if entity.get("script", {}).get("asset") == SCRIPT)
    assert coordinator["script"]["parameters"]["stats"] == {"type": "data", "value": DATA}
    controller = read_product(products, entries, CONTROLLER, protocols)["data"]
    assert controller["parameters"]["attacking"] is False
    attack = next(state for state in controller["states"] if state["name"] == "attack")
    assert attack["looping"] is False and attack["events"] == [
        {"phase": 0.43, "name": "combat.strike"}, {"phase": 0.95, "name": "combat.recover"}]
    assert read_product(products, entries, SCRIPT, protocols)["data"]["source"] == (author_assets / "scripts/combat.rhai").read_text()
    return {"status": "passed", "scene": SCENE, "actors": 3, "navigation_agents": 2,
            "attack_events": attack["events"], "data_asset": DATA, "scope": "partial combat migration; no particles or defeat animation"}


def inspect_runtime(report, native=False):
    runtime = report["application"]
    assert runtime["scene"] == SCENE and runtime["character"] is not None
    assert type(runtime["steps"]) is int and runtime["steps"] >= 0
    if not native:
        assert runtime["steps"] == 7
    assert math.isclose(runtime["elapsed"], runtime["steps"] / 60, abs_tol=1e-9)
    assert all(not item["failed"] for item in runtime["script_diagnostics"])
    assert len(runtime["animations"]) == 3
    assert all(item["controller"] == CONTROLLER and item["bindings"] == 67 for item in runtime["animations"])
    text = runtime["hud"]["labels"][0]["text"]
    assert "战斗样例" in text and "练习者：120" in text and "击败：0 / 2" in text
    assert report["audio"]["generations_started"] == 0 and report["audio_shutdown"] is True
    return {"status": "passed", "scene": SCENE, "native": native, "steps": runtime["steps"], "hud": text,
            "scope": "packaged startup; combat hit/miss, navigation, victory/defeat, fade/despawn and restart use the real-scene CPU regression"}
