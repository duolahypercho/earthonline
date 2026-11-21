"""Editable, metre-scale dining props. All UV artwork is authored here."""

import math
from itertools import pairwise

import bpy
import numpy as np


def linear(rgb):
    return tuple(((c / 255 + 0.055) / 1.055) ** 2.4 for c in rgb)


def material(name, rgb, roughness, metal=0):
    mat = bpy.data.materials.new(name)
    mat.diffuse_color = (*linear(rgb), 1)
    mat.use_nodes = True
    bs = mat.node_tree.nodes["Principled BSDF"]
    bs.inputs["Base Color"].default_value = mat.diffuse_color
    bs.inputs["Metallic"].default_value = metal
    bs.inputs["Roughness"].default_value = roughness
    return mat


def image_map(mat, name, rgb, socket="Base Color"):
    h, w, _ = rgb.shape
    image = bpy.data.images.new(name, width=w, height=h, alpha=False)
    image.colorspace_settings.name = "sRGB" if socket == "Base Color" else "Non-Color"
    rgba = np.ones((h, w, 4), np.float32)
    rgba[:, :, :3] = np.clip(rgb, 0, 1)
    image.pixels.foreach_set(rgba.ravel())
    image.update()
    image.pack()
    node = mat.node_tree.nodes.new("ShaderNodeTexImage")
    node.image = image
    links = mat.node_tree.links
    bs = mat.node_tree.nodes["Principled BSDF"]
    if socket == "Normal":
        normal = mat.node_tree.nodes.new("ShaderNodeNormalMap")
        links.new(node.outputs["Color"], normal.inputs["Color"])
        links.new(normal.outputs["Normal"], bs.inputs[socket])
    else:
        links.new(node.outputs["Color"], bs.inputs[socket])


def mesh(group, name, points, faces, materials, material_ids=(), uv=(), smooth=True):
    data = bpy.data.meshes.new(name)
    data.from_pydata(points, [], faces)
    for mat in materials:
        data.materials.append(mat)
    data.update()
    if material_ids:
        data.polygons.foreach_set("material_index", material_ids)
    if smooth:
        data.polygons.foreach_set("use_smooth", np.ones(len(faces), dtype=bool))
    if len(uv):
        layer = data.uv_layers.new(name="UVMap")
        idx = np.empty(len(data.loops), np.int32)
        data.loops.foreach_get("vertex_index", idx)
        layer.uv.foreach_set("vector", np.asarray(uv, np.float32)[idx].ravel())
    obj = bpy.data.objects.new(name, data)
    group.objects.link(obj)
    return obj


def lathe(group, name, profile, materials, sides=80, wobble=0):
    """Revolve a closed section with material indices at ring starts."""
    points, faces, ids, uv = [], [], [], []
    length = [0.0]
    for a, b in pairwise(profile):
        length.append(length[-1] + math.dist(a[:2], b[:2]))
    for j, (r, z, _) in enumerate(profile):
        for i in range(sides + 1):
            u = i / sides
            t = 2 * math.pi * u
            radius = r * (
                1 + wobble * (math.sin(3 * t + 0.4) + 0.4 * math.sin(7 * t + z))
            )
            level = z + wobble * r * math.sin(2 * t + 0.9) * min(1, z / 0.04)
            points.append((radius * math.cos(t), radius * math.sin(t), level))
            uv.append((u, length[j] / length[-1]))
            if j and i:
                k = j * (sides + 1) + i
                faces.append((k - sides - 2, k - sides - 1, k, k - 1))
                ids.append(profile[j - 1][2])
    return mesh(group, name, points, faces, materials, ids, uv)


def ceramic_bowl(group):
    clay = material("Exposed stoneware foot", (149, 110, 78), 0.68)
    glaze = material("Fired sage ash glaze", (135, 144, 118), 0.25)
    glaze.node_tree.nodes["Principled BSDF"].inputs["Coat Weight"].default_value = 0.32
    rim = material("Iron wash at the lip", (137, 105, 65), 0.29)
    rng = np.random.default_rng(3237)
    v, u = np.mgrid[0:512, 0:1024]
    cloud = (
        np.sin(u * 2 * math.pi / 256 + 0.7 * np.sin(v / 30)) * 0.45
        + np.cos(v / 18 + u * 2 * math.pi / 512) * 0.35
    )
    iron = np.maximum(rng.normal(size=u.shape) - 2.02, 0)
    intensity = (
        1 + 0.032 * cloud - 0.30 * iron + 0.01 * np.cos(v * 0.45 + np.sin(u / 23))
    )
    # The iron in the clay shows through the lower glaze. Sgraffito just below the lip.
    color = intensity[:, :, None] * np.array([0.50, 0.55, 0.465])
    z_band = np.abs(v / 511 - 0.474)
    dash = (np.cos(u * 2 * math.pi * 54 / 1024 + v * 0.14) > 0.80) & (z_band < 0.009)
    color[dash] *= np.array([0.72, 0.70, 0.65])
    image_map(glaze, "Fired ash glaze - iron flecks and lip marks", color)
    profile = [
        (0.001, 0.009, 0),
        (0.062, 0.009, 0),
        (0.067, 0.002, 0),
        (0.072, 0, 0),
        (0.077, 0.002, 0),
        (0.079, 0.011, 0),
        (0.083, 0.016, 1),
        (0.092, 0.018, 1),
        (0.101, 0.022, 1),
        (0.118, 0.032, 1),
        (0.139, 0.052, 1),
        (0.159, 0.077, 1),
        (0.175, 0.105, 1),
        (0.187, 0.133, 1),
        (0.193, 0.154, 1),
        (0.196, 0.160, 2),
        (0.199, 0.162, 2),
        (0.200, 0.166, 2),
        (0.198, 0.169, 2),
        (0.194, 0.169, 1),
        (0.190, 0.166, 1),
        (0.188, 0.155, 1),
        (0.181, 0.135, 1),
        (0.168, 0.109, 1),
        (0.153, 0.083, 1),
        (0.136, 0.062, 1),
        (0.115, 0.046, 1),
        (0.091, 0.035, 1),
        (0.063, 0.030, 1),
        (0.032, 0.028, 1),
        (0.001, 0.028, 1),
    ]
    return lathe(
        group, "Hand-thrown mixing bowl", profile, [clay, glaze, rim], 112, 0.003
    )


def folded_linen(group):
    flax = material("Laundered flax with woven indigo borders", (194, 184, 155), 0.86)
    flax.node_tree.nodes["Principled BSDF"].inputs["Sheen Weight"].default_value = 0.27
    v, u = np.mgrid[0:1024, 0:512]
    u = u / 512
    v = v / 1024
    warp = np.sin(2 * math.pi * (u * 159 + 0.13 * np.sin(v * 25)))
    weft = np.cos(2 * math.pi * (v * 281 + 0.10 * np.cos(u * 32)))
    bright = 0.975 + 0.035 * warp + 0.029 * weft + 0.017 * np.cos(11 * u + 19 * v)
    stripe = np.zeros_like(u, dtype=bool)
    for edge in (0.078, 0.094, 0.127, 0.863, 0.901, 0.921):
        stripe |= abs(u - edge) < (0.008 if edge in (0.127, 0.863) else 0.003)
    rgb = np.where(
        stripe[:, :, None], np.array([0.32, 0.38, 0.37]), np.array([0.78, 0.745, 0.655])
    )
    image_map(flax, "Linen - indigo selvedges and flax weave", rgb * bright[:, :, None])
    nx, ny = 49, 173
    points, faces, uv = [], [], []
    for j in range(ny):
        v = j / (ny - 1)
        # One continuous sheet folds back on itself twice. Loops connect the layers.
        path = v * 3
        leg = min(2, int(path))
        along = path - leg if leg % 2 == 0 else 1 - (path - leg)
        for i in range(nx):
            u = i / (nx - 1)
            across = u - 0.5
            phase = math.pi * along
            crease = math.exp(-(((across + 0.13 - 0.21 * along) / 0.071) ** 2))
            near_fold = math.exp(-((min(along, 1 - along) / 0.095) ** 2))
            loft = 0.0028 + 0.0055 * leg + 0.018 * crease * math.sin(phase) ** 2
            loft += (
                0.007
                * (1 + math.sin(u * 21 + along * 13 + leg * 0.6))
                * 0.5
                * near_fold
            )
            loft += 0.003 * math.sin(u * 15 + along * 8) * math.sin(phase) ** 2
            side = (
                0.014 * math.sin(along * 4 + 0.7 * leg)
                + 0.012 * math.sin(phase + 1.1) * (2 * across) ** 2
            )
            length = 0.028 * across * math.sin(along * 4 + leg * 0.6) + 0.018 * leg * (
                u - 0.44
            )
            points.append(
                (across * 0.38 + side, (along - 0.5) * 0.245 + length, max(0.001, loft))
            )
            uv.append((u, v))
            if i and j:
                k = j * nx + i
                faces.append((k - nx - 1, k - nx, k, k - 1))
    cloth = mesh(group, "Folded striped kitchen linen", points, faces, [flax], uv=uv)
    solid = cloth.modifiers.new("Turned cloth edge", "SOLIDIFY")
    solid.thickness = 0.0012
    return cloth


def enamel_fixture(group, bulb_height, ceiling):
    black = material("Black stove enamel", (48, 46, 42), 0.31, 0.46)
    cream = material("Cream vitreous reflector", (232, 220, 193), 0.34, 0.19)
    brass = material("Darkened brass fittings", (103, 85, 61), 0.36, 0.79)
    cord = material("Braided brown flex", (50, 42, 33), 0.72)
    glass = material("Warm opal globe", (241, 226, 198), 0.26)
    shader = glass.node_tree.nodes["Principled BSDF"]
    shader.inputs["Emission Color"].default_value = (*linear((255, 211, 151)), 1)
    shader.inputs["Emission Strength"].default_value = 2.8
    lip = 0.039
    profile = [
        (0.013, 0.232, 0),
        (0.043, 0.230, 0),
        (0.051, 0.214, 0),
        (0.063, 0.207, 0),
        (0.066, 0.191, 0),
        (0.083, 0.184, 0),
        (0.102, 0.175, 0),
        (0.127, 0.160, 0),
        (0.148, 0.145, 0),
        (0.170, 0.125, 0),
        (0.188, 0.104, 0),
        (0.203, 0.082, 0),
        (0.213, 0.061, 0),
        (0.218, 0.045, 0),
        (0.221, lip + 0.004, 0),
        (0.223, lip + 0.002, 0),
        (0.223, lip, 1),
        (0.220, lip - 0.001, 1),
        (0.217, lip + 0.001, 1),
        (0.214, 0.058, 1),
        (0.200, 0.080, 1),
        (0.181, 0.107, 1),
        (0.150, 0.141, 1),
        (0.119, 0.163, 1),
        (0.078, 0.183, 1),
        (0.036, 0.191, 1),
    ]
    # Outer profile descends, so reverse the winding for the shade.
    shade = lathe(
        group, "Spun enamel shade and rolled lip", profile, [black, cream], 112
    )
    shade.data.flip_normals()
    cap = lathe(
        group,
        "Stepped socket cap",
        [
            (0.001, 0.216, 0),
            (0.052, 0.216, 0),
            (0.053, 0.224, 0),
            (0.050, 0.227, 0),
            (0.045, 0.231, 0),
            (0.044, 0.263, 0),
            (0.040, 0.271, 0),
            (0.028, 0.276, 1),
            (0.026, 0.297, 1),
            (0.015, 0.305, 0),
            (0.009, 0.315, 0),
        ],
        [black, brass],
    )
    globe = lathe(
        group,
        "Opal incandescent globe",
        [
            (0.001, 0, 0),
            (0.017, 0.003, 0),
            (0.034, 0.018, 0),
            (0.043, 0.042, 0),
            (0.041, 0.061, 0),
            (0.032, 0.077, 0),
            (0.019, 0.090, 0),
            (0.016, 0.128, 0),
            (0.019, 0.136, 1),
            (0.019, 0.163, 1),
        ],
        [glass, brass],
        64,
    )
    height = ceiling - bulb_height
    flex = lathe(
        group,
        "Flex and pressed ceiling rose",
        [
            (0.003, 0.309, 0),
            (0.003, height - 0.047, 0),
            (0.008, height - 0.044, 1),
            (0.010, height - 0.036, 1),
            (0.031, height - 0.034, 1),
            (0.054, height - 0.019, 1),
            (0.065, height - 0.012, 1),
            (0.066, height, 1),
            (0.001, height, 1),
        ],
        [cord, black],
        64,
    )
    return [shade, cap, globe, flex]
