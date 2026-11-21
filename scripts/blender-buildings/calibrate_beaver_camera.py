"""Measure a Blender sRGB view, then fit the native camera's colour response.

Open the reference house in Blender and pass -- OUTPUT to capture the chart.
Run Python with --fit OUTPUT --neutral-display VALUE --shader FILE to generate
the WGSL coefficients. Fitting uses NumPy, SciPy and OpenCV offline.
"""

import argparse
import hashlib
import json
import sys
from pathlib import Path

import numpy as np


def measure(output):
    import bpy

    if output.is_relative_to(Path(__file__).resolve().parents[2]):
        raise ValueError("Choose an external directory for the camera measurements")
    scene = bpy.context.scene
    view = scene.view_settings
    if (
        view.view_transform != "AgX"
        or view.look != "None"
        or scene.display_settings.display_device != "sRGB"
    ):
        raise ValueError("Expected the house's AgX sRGB view with no additional look")
    view.exposure = 0
    view.gamma = 1
    scene.render.dither_intensity = 0
    settings = scene.render.image_settings
    settings.file_format = "PNG"
    settings.color_mode = "RGB"
    settings.color_depth = "16"
    width, height = 512, 128
    rng = np.random.default_rng(19628)
    colors = rng.uniform(size=(width * height, 3))
    colors[::4] **= 4
    colors[1::4] **= 0.30
    colors /= colors.max(1, keepdims=True)
    colors *= np.exp2(rng.uniform(-15, 11, (len(colors), 1)))
    colors[:width] = np.exp2(np.linspace(-18, 14, width)[:, None])
    colors[-1] = 0
    chart = bpy.data.images.new(
        "Measured linear camera chart", width, height, float_buffer=True, alpha=False
    )
    rgba = np.ones((height, width, 4), dtype=np.float32)
    rgba[:, :, :3] = colors.reshape(height, width, 3)
    chart.pixels.foreach_set(rgba[::-1].copy().ravel())
    chart.update()
    output.mkdir(parents=True, exist_ok=True)
    chart.save_render(str(output / "color-chart.png"), scene=scene)
    np.save(output / "color-input.npy", colors.astype(np.float32))
    metadata = {
        "blender": bpy.app.version_string,
        "view": view.view_transform,
        "look": view.look,
        "display": scene.display_settings.display_device,
        "exposure_stops": 0,
        "samples": len(colors),
        "dither": 0,
    }
    (output / "chart-settings.json").write_text(json.dumps(metadata, indent=2) + "\n")


def neutral_preserving_matrix(parameters):
    a, b, c, d, e, f = parameters
    return np.array([[1 - a - b, a, b], [c, 1 - c - d, d], [e, f, 1 - e - f]])


def fit(output, neutral_display, shader):
    import cv2
    from scipy.interpolate import CubicHermiteSpline, PchipInterpolator
    from scipy.optimize import least_squares

    inputs = np.load(output / "color-input.npy").astype(float)
    bgr = cv2.imread(str(output / "color-chart.png"), cv2.IMREAD_UNCHANGED)
    if bgr is None or bgr.shape != (128, 512, 3) or bgr.dtype != np.uint16:
        raise ValueError("Expected the original 512 by 128, 16-bit RGB camera chart")
    if inputs.shape != (128 * 512, 3) or not np.isfinite(inputs).all():
        raise ValueError("Invalid linear camera stimuli")
    if not 0.0 < neutral_display < 1.0:
        raise ValueError("The measured display luminance must lie between zero and one")
    display = bgr.reshape(-1, 3)[:, ::-1].astype(float) / 65535
    # The first row is a neutral exposure wedge. A monotone interpolation of
    # its values and slopes supplies one cubic per stop for the shader.
    ramp = PchipInterpolator(np.log2(inputs[:512, 0]), display[:512].mean(1))
    stops = np.arange(-13, 6)
    curve = CubicHermiteSpline(stops, ramp(stops), ramp.derivative()(stops))

    def predict(colors, parameters):
        wide = np.einsum(
            "...j,ij->...i", colors, neutral_preserving_matrix(parameters[:6])
        )
        measured = curve(np.clip(np.log2(np.maximum(wide, 1e-9)), stops[0], stops[-1]))
        linear = np.where(
            measured <= 0.04045, measured / 12.92, ((measured + 0.055) / 1.055) ** 2.4
        )
        mixed = np.einsum(
            "...j,ij->...i", linear, neutral_preserving_matrix(parameters[6:])
        )
        return np.clip(
            np.where(
                mixed <= 0.0031308,
                12.92 * mixed,
                1.055 * np.maximum(mixed, 0) ** (1 / 2.4) - 0.055,
            ),
            0,
            1,
        )

    rng = np.random.default_rng(451)
    training = rng.choice(np.arange(512, len(inputs) - 1, 2), size=3900, replace=False)
    start = [
        0.1,
        0.06,
        0.03,
        0.04,
        0.04,
        0.10,
        -0.23,
        -0.10,
        -0.06,
        -0.06,
        -0.04,
        -0.18,
    ]
    solution = least_squares(
        lambda p: (predict(inputs[training], p) - display[training]).ravel(),
        start,
        ftol=1e-10,
    )
    if not solution.success:
        raise RuntimeError(solution.message)
    held_error = 255 * np.abs(predict(inputs[513::2], solution.x) - display[513::2])
    gain = np.interp(neutral_display, display[:512].mean(1), inputs[:512, 0]) / 0.18
    s = {
        "input": neutral_preserving_matrix(solution.x[:6]).tolist(),
        "output": neutral_preserving_matrix(solution.x[6:]).tolist(),
        "curve": curve.c.T.tolist(),
        "knots": stops.tolist(),
        "chart_sha256": hashlib.sha256(
            (output / "color-chart.png").read_bytes()
        ).hexdigest(),
        "input_sha256": hashlib.sha256(
            (output / "color-input.npy").read_bytes()
        ).hexdigest(),
        "middle_gray": float(neutral_display),
        "meter_gain": float(gain),
        "training_colors": len(training),
        "held_out_colors": len(inputs[513::2]),
        "mean_display_error": float(held_error.mean()),
        "p95_display_error": float(np.quantile(held_error, 0.95)),
        "max_display_error": float(held_error.max()),
    }
    (output / "camera-response.json").write_text(json.dumps(s, indent=2) + "\n")

    def vec(v):
        return f"vec{len(v)}<f32>(" + ", ".join(f"{value:.9f}" for value in v) + ")"

    lines = [
        "// Earth Online camera measurements. Generated by calibrate_beaver_camera.py.",
        "// The curve covers -13 to +5 stops relative to unit scene-linear Rec.709.",
        f"const PHOTO_METER_GAIN: f32 = {gain:.9f};",
        "const PHOTO_RESPONSE = array<vec4<f32>, 18>(",
    ]
    lines.extend("    " + vec(v) + "," for v in curve.c.T)
    lines.append(");")
    for name, matrix in [("PHOTO_INPUT", s["input"]), ("PHOTO_OUTPUT", s["output"])]:
        lines.append(f"const {name} = mat3x3<f32>(")
        lines.extend("    " + vec(column) + "," for column in np.array(matrix).T)
        lines.append(");")
    shader.write_text("\n".join(lines) + "\n")


if __name__ == "__main__":
    if "--" in sys.argv:
        measure(Path(sys.argv[sys.argv.index("--") + 1]).resolve())
    else:
        parser = argparse.ArgumentParser(description=__doc__)
        parser.add_argument("--fit", type=Path, required=True)
        parser.add_argument(
            "--neutral-display",
            type=float,
            required=True,
            help="Display luminance of 18%% gray from eo_look::tone::display at exposure 1.",
        )
        parser.add_argument("--shader", type=Path, required=True)
        args = parser.parse_args()
        fit(args.fit, args.neutral_display, args.shader)
