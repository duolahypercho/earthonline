"""Filter incident panoramas for a dielectric floor viewed from above."""

import math

import numpy as np

GRID = (4, 8)
PLAN_METRES = (7, 13)
TILE = (64, 16)
SOURCES = (
    "fixtures",
    "uniform-sky",
    "clear-sky",
    "front-25",
    "front-57",
    "rear-25",
    "rear-57",
)


def angular_filter(roughness, panorama_size=(128, 64)):
    """Bilinear panorama samples weighted by the visible GGX distribution.

    The outgoing view is mirrored across the geometric normal to address
    the stored hemisphere. Sampling visible facets accounts for the long,
    anisotropic reflection footprint seen at a standing player's grazing angle.
    Normalize by the energy for a uniform environment: the game supplies the
    Fresnel and masking magnitude separately through its integrated BRDF.
    """
    width, height = panorama_size
    nx, ny = TILE[0] - 2, TILE[1] - 2
    azimuth = ((np.arange(nx) + 0.5) / nx - 0.5) * 2 * math.pi
    elevation = (np.arange(ny) + 0.5) / ny * math.pi / 2
    az, el = np.meshgrid(azimuth, elevation)
    view = np.stack(
        (-np.sin(az) * np.cos(el), -np.cos(az) * np.cos(el), np.sin(el)), axis=-1
    )
    alpha = roughness**2

    def unit(v):
        return v / np.linalg.norm(v, axis=-1, keepdims=True)

    stretched = unit(view * np.array([alpha, alpha, 1]))
    tangent = unit(np.stack((-stretched[..., 1], stretched[..., 0], 0 * el), axis=-1))
    bitangent = np.cross(stretched, tangent)
    blend = (1 + stretched[..., 2]) * 0.5
    columns, weights = [], []
    for i in range(256):
        radius = math.sqrt((i + 0.5) / 256)
        angle = 2 * math.pi * int(f"{i:08b}"[::-1], 2) / 256
        x = radius * math.cos(angle)
        y = (1 - blend) * math.sqrt(1 - x * x) + blend * radius * math.sin(angle)
        normal = (
            tangent * x
            + bitangent * y[..., None]
            + stretched * np.sqrt(np.maximum(0, 1 - x * x - y * y))[..., None]
        )
        half = unit(normal * np.array([alpha, alpha, 1]))
        vh = np.sum(view * half, axis=-1).clip(0, 1)
        ray = 2 * vh[..., None] * half - view
        nl = np.maximum(0, ray[..., 2])
        nv = view[..., 2]
        smith_v = np.sqrt(alpha**2 + (1 - alpha**2) * nv**2)
        smith_l = np.sqrt(alpha**2 + (1 - alpha**2) * nl**2)
        weight = nl * (smith_v + nv) / np.maximum(nl * smith_v + nv * smith_l, 1e-8)
        weight *= 0.04 + 0.96 * (1 - vh) ** 5
        u = (np.arctan2(ray[..., 0], ray[..., 1]) / (2 * math.pi) + 0.5) * width - 0.5
        v = np.clip(
            (np.arcsin(ray[..., 2].clip(-1, 1)) / math.pi + 0.5) * height - 0.5,
            0,
            height - 1,
        )
        left, bottom = np.floor(u).astype(int), np.floor(v).astype(int)
        fx, fy = u - left, v - bottom
        for side, wx in enumerate((1 - fx, fx)):
            for up, wy in enumerate((1 - fy, fy)):
                columns.append(
                    (
                        np.minimum(height - 1, bottom + up) * width
                        + (left + side) % width
                    ).ravel()
                )
                weights.append((weight * wx * wy).ravel())
    index = np.stack(columns, axis=1)
    w = np.stack(weights, axis=1)
    w /= w.sum(axis=1, keepdims=True)
    return index, w.astype(np.float32)


def store_panorama(atlas, panorama, source_index, position_index, kernel):
    index, weights = kernel
    rgb = panorama.reshape(-1, 3)
    radiance = np.einsum("ijk,ij->ik", rgb[index], weights, optimize=False)
    field = np.maximum(0, radiance.reshape(TILE[1] - 2, TILE[0] - 2, 3))
    encoded = np.rint(255 * np.sqrt(field / (1 + field))).astype(np.uint8)
    wrapped = np.pad(encoded, ((0, 0), (1, 1), (0, 0)), mode="wrap")
    padded = np.pad(wrapped, ((1, 1), (0, 0), (0, 0)), mode="edge")
    x = position_index % 8 * TILE[0]
    y = (source_index * 4 + position_index // 8) * TILE[1]
    atlas[y : y + TILE[1], x : x + TILE[0], :3] = padded
