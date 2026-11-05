import importlib.util
import json
import math
import os
from pathlib import Path
import struct
import sys
import tempfile
import unittest
from unittest import mock
import zlib

spec = importlib.util.spec_from_file_location("comet_verify", Path(__file__).resolve().parents[1] / "verify.py")
verify = importlib.util.module_from_spec(spec)
spec.loader.exec_module(verify)


def png(width, height, rgba, filters=(0,)):
    def chunk(kind, payload):
        return struct.pack(">I", len(payload)) + kind + payload + struct.pack(">I", zlib.crc32(kind + payload))

    stride = width * 4
    encoded = bytearray()
    for y in range(height):
        kind = filters[y % len(filters)]
        row = rgba[y * stride:(y + 1) * stride]
        previous = rgba[(y - 1) * stride:y * stride] if y else bytes(stride)
        encoded.append(kind)
        for x, sample in enumerate(row):
            left, above, upper_left = (row[x - 4] if x >= 4 else 0), previous[x], (previous[x - 4] if x >= 4 else 0)
            if kind == 4:
                prediction = left + above - upper_left
                predictor = min(enumerate((left, above, upper_left)), key=lambda item: (abs(prediction - item[1]), item[0]))[1]
            else:
                predictor = (0, left, above, (left + above) // 2, 0)[min(kind, 4)]
            encoded.append((sample - predictor) & 255)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(encoded)) + chunk(b"IEND", b""))


class CaptureEvidence(unittest.TestCase):
    def test_sparse_feedback_must_match_the_draw_schedule_at_each_recorded_frame(self):
        schedule = {number: 6 if number == 0 else 4 for number in range(8)}
        frames = [{"number": number, "draw_count": schedule[number],
                   "worker_thread": "comet-render", "presented": False}
                  for number in (0, 1, 2, 7)]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "capture.png").write_bytes(png(1, 1, bytes([8, 12, 18, 255])))
            path = root / "capture.json"
            with mock.patch.object(verify, "OUT", root):
                path.write_text(json.dumps({"submitted_frames": 8, "frames": frames}))
                verify.inspect_capture("capture", False, schedule)
                for number, wrong_count in ((0, 4), (1, 6), (7, 6)):
                    wrong = [dict(frame, draw_count=wrong_count) if frame["number"] == number else frame
                             for frame in frames]
                    path.write_text(json.dumps({"submitted_frames": 8, "frames": wrong}))
                    with self.subTest(number=number), self.assertRaises(AssertionError):
                        verify.inspect_capture("capture", False, schedule)

    def test_a_successful_process_must_replace_both_evidence_outputs(self):
        for existing in (False, True):
            for replaced in (0, 1, 2):
                with self.subTest(existing=existing, replaced=replaced), tempfile.TemporaryDirectory() as directory:
                    root = Path(directory)
                    outputs = [root / "capture.json", root / "capture.png"]
                    if existing:
                        for path in outputs:
                            path.write_bytes(b"old")
                            os.utime(path, ns=(1_000_000_000, 1_000_000_000))
                    command = [sys.executable, "-c",
                               "import sys\nfrom pathlib import Path\nfor name in sys.argv[1:]: Path(name).write_bytes(b'new')",
                               *outputs[:replaced]]
                    with mock.patch.object(verify, "OUT", root), mock.patch.object(verify, "checks", {}):
                        if replaced == 2:
                            verify.run_capture("capture", command, cwd=root)
                            self.assertTrue(all(path.read_bytes() == b"new" for path in outputs))
                        else:
                            with self.assertRaisesRegex(AssertionError, "new report and capture"):
                                verify.run_capture("capture", command, cwd=root)
                            if existing:
                                self.assertEqual(outputs[1].read_bytes(), b"old")

    def test_png_decode_reconstructs_every_filter_across_rows_and_rgba_channels(self):
        width, height = 17, 10
        pixels = bytes((index * 37 + index // 11) % 256 for index in range(width * height * 4))
        with tempfile.TemporaryDirectory() as directory:
            image = Path(directory) / "filtered.png"
            image.write_bytes(png(width, height, pixels, filters=(0, 1, 2, 3, 4)))
            metadata, decoded = verify.decode_rgba_png(image)
            self.assertEqual((metadata["width"], metadata["height"]), (width, height))
            self.assertEqual(decoded, pixels)
            self.assertEqual(verify.inspect_png(image), metadata)

    def test_invalid_crc_truncation_and_filter_cannot_be_accepted_as_capture_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            image = Path(directory) / "invalid.png"
            valid = png(2, 2, bytes([8, 12, 18, 255] * 4))
            damaged = bytearray(valid)
            damaged[-1] ^= 1
            for data in (valid[:-4], damaged, png(2, 2, bytes([8, 12, 18, 255] * 4), filters=(5,))):
                image.write_bytes(data)
                with self.assertRaises(AssertionError):
                    verify.decode_rgba_png(image)

    def test_triangle_roi_rejects_blank_or_corner_only_pixels_and_measures_real_foreground(self):
        width, height = 96, 64
        with tempfile.TemporaryDirectory() as directory:
            image = Path(directory) / "triangle.png"
            pixels = bytearray([8, 12, 18, 255] * width * height)
            for corner_only in (False, True):
                if corner_only:
                    for y in range(3):
                        for x in range(3):
                            offset = (y * width + x) * 4
                            pixels[offset:offset + 4] = bytes([190, 40, 16, 255])
                image.write_bytes(png(width, height, pixels))
                with self.assertRaisesRegex(AssertionError, "triangle is missing"):
                    verify.inspect_triangle_capture(image)
            for y in range(10, 55):
                half_width = int((y - 10) * 0.55)
                for x in range(width // 2 - half_width, width // 2 + half_width + 1):
                    offset = (y * width + x) * 4
                    pixels[offset:offset + 4] = bytes([190, 40, 16, 255])
            image.write_bytes(png(width, height, pixels, filters=(4, 3, 2, 1, 0)))
            evidence, mask = verify.inspect_triangle_capture(image)
            self.assertGreater(len(mask), 200)
            self.assertEqual(evidence["foreground_pixels"], len(mask))
            self.assertEqual(evidence["background_rgb"], [8, 12, 18])
            self.assertEqual(evidence["mean_foreground_rgb"], [190, 40, 16])

    def test_generated_projects_have_one_triangle_finite_normals_and_equivalent_f32_world_geometry(self):
        cases = (([1e-3] * 3, 1.), ([1e20] * 3, 1.), ([-1e20, 1e20, 1e20], 1.),
                 ([1.] * 3, 2. ** -149), ([-0.002, 3., 5e8], 1.))

        def f32(value):
            return struct.unpack("f", struct.pack("f", value))[0]

        with tempfile.TemporaryDirectory() as directory:
            for index, (scale, normal) in enumerate(cases):
                root = Path(directory) / str(index)
                authored = verify.scaled_geometry_project(root, scale, normal)
                assets = root / "assets"
                self.assertEqual(len(list(assets.glob("*.meta"))), 3)
                source = json.loads((assets / "main.scene").read_text())
                self.assertEqual(source["instances"], [])
                self.assertEqual(len(source["base"]["entities"]), 1)
                mesh = json.loads((assets / "triangle.mesh").read_text())
                self.assertEqual(mesh["indices"], [0, 1, 2])
                self.assertEqual(len(mesh["positions"]), 3)
                self.assertTrue(all(math.isfinite(f32(value)) for attribute in (mesh["positions"], mesh["normals"]) for vector in attribute for value in vector))
                self.assertTrue(all(any(f32(value) != 0 for value in vector) for vector in mesh["normals"]))
                self.assertTrue(all(point[2] == 0 for point in mesh["positions"]))
                transform = source["base"]["entities"][0]["transform"]
                self.assertEqual(transform["rotation"], authored["rotation"])
                self.assertNotEqual(transform["rotation"], [0., 0., 0., 1.])
                actual = verify.scaled_world_positions([[f32(value) for value in point] for point in mesh["positions"]],
                                                       [f32(value) for value in scale], [f32(value) for value in transform["rotation"]])
                for point, expected in zip(actual, authored["world_positions"]):
                    for value, target in zip(point, expected):
                        self.assertAlmostEqual(value, target, delta=1e-6)
                with self.assertRaises(FileExistsError):
                    verify.scaled_geometry_project(root, scale, normal)

    def test_rotated_anisotropic_case_uses_noncommuting_model_rotation_and_local_z_normal(self):
        with tempfile.TemporaryDirectory() as directory:
            scale = [-0.002, 3., 5e8]
            root = Path(directory) / "project"
            authored = verify.scaled_geometry_project(root, scale)
            mesh = json.loads((root / "assets/triangle.mesh").read_text())
            self.assertEqual(mesh["normals"], [[0., 0., 1.]] * 3)
            pitch, yaw = -0.25, 0.45
            for point, world in zip(mesh["positions"], authored["world_positions"]):
                x, y, _ = [value * factor for value, factor in zip(point, scale)]
                expected = [math.cos(yaw) * x + math.sin(yaw) * math.sin(pitch) * y,
                            math.cos(pitch) * y + 0.7,
                            -math.sin(yaw) * x + math.cos(yaw) * math.sin(pitch) * y]
                for actual, target in zip(world, expected):
                    self.assertAlmostEqual(actual, target, delta=1e-12)
            # Swapping S*R for R*S must produce a visibly different result for this fixture.
            wrong = verify.scaled_world_positions([[1., 0., 0.]], [1.] * 3, authored["rotation"])[0]
            wrong = [wrong[axis] * scale[axis] for axis in range(3)]
            correct = verify.scaled_world_positions([[1., 0., 0.]], scale, authored["rotation"])[0]
            self.assertGreater(max(abs(actual - target) for actual, target in zip(wrong, correct)), 1e6)
            normal = authored["world_normal"]
            self.assertAlmostEqual(sum(value * value for value in normal), 1.)
            self.assertGreater(abs(normal[0]), 0.1)
            self.assertGreater(abs(normal[1]), 0.1)

    def test_generated_gltf_cases_omit_normals_and_preserve_the_reference_world_triangle(self):
        def f32(value):
            return struct.unpack("f", struct.pack("f", value))[0]

        with tempfile.TemporaryDirectory() as directory:
            reference_root = Path(directory) / "reference"
            reference = verify.scaled_geometry_project(reference_root, [1.] * 3)
            material = json.loads((reference_root / "assets/red.material").read_text())
            for name, scale in (("large-source", 1e-20), ("small-source", 1e30)):
                root = Path(directory) / name
                authored = verify.scaled_geometry_project(root, [scale] * 3, gltf_missing_normals=True)
                assets = root / "assets"
                self.assertEqual({path.name for path in assets.iterdir()}, {
                    "main.scene", "main.scene.meta", "triangle.gltf", "triangle.gltf.meta", "triangle.bin",
                })
                source = json.loads((assets / "main.scene").read_text())
                self.assertEqual(source["base"]["entities"], [])
                self.assertEqual(source["instances"], [{"id": authored["instance"], "prefab": authored["prefab"],
                                                       "parent": None, "overrides": []}])
                self.assertEqual(json.loads((assets / "triangle.gltf.meta").read_text())["id"], authored["prefab"])
                gltf = json.loads((assets / "triangle.gltf").read_text())
                primitive = gltf["meshes"][0]["primitives"][0]
                self.assertEqual(primitive["attributes"], {"POSITION": 0})
                self.assertEqual(len(gltf["accessors"]), 2)
                self.assertEqual(gltf["nodes"][0]["scale"], [scale] * 3)
                self.assertEqual(gltf["nodes"][0]["rotation"], reference["rotation"])
                self.assertEqual(gltf["nodes"][0]["translation"], [0., 0.7, 0.])
                pbr = gltf["materials"][0]["pbrMetallicRoughness"]
                self.assertEqual(pbr["baseColorFactor"], material["base_color"])
                self.assertEqual(pbr["roughnessFactor"], material["roughness"])
                self.assertEqual(pbr["metallicFactor"], material["metallic"])
                binary = (assets / gltf["buffers"][0]["uri"]).read_bytes()
                self.assertEqual(len(binary), gltf["buffers"][0]["byteLength"])
                positions = list(struct.iter_unpack("<3f", binary[:36]))
                self.assertEqual(struct.unpack_from("<3H", binary, 36), (0, 1, 2))
                self.assertTrue(all(math.isfinite(value) for point in positions for value in point))
                actual = verify.scaled_world_positions(positions, [f32(scale)] * 3,
                                                       [f32(value) for value in authored["rotation"]])
                for point, expected in zip(actual, reference["world_positions"]):
                    for value, target in zip(point, expected):
                        self.assertAlmostEqual(value, target, delta=1e-6)
                self.assertEqual(authored["normal_source"], "generated")
                self.assertEqual(authored["world_normal"], reference["world_normal"])


if __name__ == "__main__":
    unittest.main()
