"""Preserve board-atlas channels and linear wood stains in a standard glTF material."""

from pathlib import Path
from tempfile import TemporaryDirectory

import numpy as np
from beaver_dining_oak import pack, read_plate


def source_and_stain(socket):
    if not socket.is_linked:
        return None, np.ones(3)
    node = socket.links[0].from_node
    if node.type == "TEX_IMAGE":
        return node, np.ones(3)
    if node.type == "MIX_RGB" and node.blend_type == "MULTIPLY":
        amount, base, dye = node.inputs
        if not amount.is_linked and not dye.is_linked:
            image, stain = source_and_stain(base)
            return image, stain * (
                1
                - amount.default_value
                + amount.default_value * np.array(dye.default_value[:3])
            )
    if node.type == "VECT_MATH" and node.operation == "MULTIPLY":
        a, b = node.inputs[:2]
        if a.is_linked and not b.is_linked:
            image, stain = source_and_stain(a)
            return image, stain * np.array(b.default_value[:3])
    return None, np.ones(3)


def prepare_oak(materials):
    pages = {}
    factors = {}
    with TemporaryDirectory() as folder:
        tmp = Path(folder)
        for mat in materials:
            if not mat or not mat.use_nodes:
                continue
            tree = mat.node_tree
            shader = next((n for n in tree.nodes if n.type == "BSDF_PRINCIPLED"), None)
            if not shader:
                continue
            color = shader.inputs["Base Color"]
            if mat.name.startswith("Dining tabletop · plank "):
                for node in tree.nodes:
                    if node.type != "TEX_IMAGE" or not node.inputs[0].is_linked:
                        continue
                    coord = node.inputs[0].links[0].from_node
                    if coord.type != "VECT_MATH" or coord.operation != "MULTIPLY_ADD":
                        continue
                    if not np.allclose(coord.inputs[1].default_value[:2], (1, 1 / 3)):
                        raise ValueError("Unexpected board-atlas projection")
                    page = 2 - round(3 * coord.inputs[2].default_value[1])
                    if page not in pages:
                        archive = tmp / "source.png"
                        archive.write_bytes(node.image.packed_file.data)
                        plate = read_plate(archive)
                        size = plate.shape[1]
                        if plate.shape[0] != 3 * size:
                            raise ValueError("Expected three square oak-atlas pages")
                        label = ("colour", "normal", "roughness")[page]
                        pages[page] = pack(
                            "Table boards - " + label,
                            plate[size * page : size * (page + 1), :, :3],
                            tmp / (label + ".png"),
                            "sRGB" if page == 0 else "Non-Color",
                        )
                    node.image = pages[page]
                    tree.links.new(coord.inputs[0].links[0].from_socket, node.inputs[0])
            image, stain = source_and_stain(color)
            if image and not np.allclose(stain, 1):
                # glTF multiplies the linear sampled base colour by this factor.
                # Factors above one must be baked into the texture to preserve them.
                if np.any(stain > 1):
                    key = mat.name.replace("/", "-")
                    source = tmp / "fibre.png"
                    source.write_bytes(image.image.packed_file.data)
                    plate = read_plate(source)[:, :, :3] / 255.0
                    linear = np.where(
                        plate <= 0.04045,
                        plate / 12.92,
                        ((plate + 0.055) / 1.055) ** 2.4,
                    )
                    corrected = np.minimum(1, linear * stain)
                    rgb = np.where(
                        corrected <= 0.0031308,
                        12.92 * corrected,
                        1.055 * corrected ** (1 / 2.4) - 0.055,
                    )
                    image.image = pack(
                        key + " - oiled wood",
                        np.uint8(np.rint(rgb * 255)),
                        tmp / (key + ".png"),
                        "sRGB",
                    )
                else:
                    factors[mat.name] = stain.tolist() + [1.0]
                tree.links.new(image.outputs["Color"], color)
            elif (
                mat.name in ("oiled-oak@ae906f", "oiled-oak@785338") and color.is_linked
            ):
                # These secondary parts use object-space timber noise. Preserve
                # the central stain colour for viewers without procedural nodes.
                ramp = color.links[0].from_node
                if ramp.type == "VALTORGB":
                    color.default_value = ramp.color_ramp.evaluate(0.5)
                    tree.links.remove(color.links[0])
    return factors
