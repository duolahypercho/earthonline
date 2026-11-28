"""Build the v1 street signals: the vehicle and pedestrian signal heads from published models.

Usage (headless, Blender 4.5):
  blender -b --factory-startup --python scripts/assets/build-street-signals-v1.py -- \
      [--out public/assets/street-signals] [--seed 11]

The heads' geometry is two published models (SOURCES), each licensed CC BY 4.0 and read from
the git-ignored `external/sketchfab/<uid>/model.glb` (`EO_SKETCHFAB_DIR` overrides the folder);
the file's digest is checked before it is read. They are fetched with a Sketchfab account
(the download endpoint needs one), so `scripts/setup-assets.py` does not fetch them; the
built files below are what the game reads. Only their geometry is used: the photographs the
sources carry as lens and face textures are not (the pedestrian ones are Flickr photographs
under CC BY-NC 2.0; the others have no record of who took them). The lens and face images
are drawn here.

  * vehicle head: the McCain three-section 12 in polycarbonate head of `mccainLeotek12`
    (housing, doors, gaskets, wing nuts, tunnel visors), scaled so its lens pitch is the
    game's (LENS_PITCH_M), on a louvred backplate with the yellow retroreflective border of
    the street-standards entry `signal-backplate-border`; its tunnel visors cut away underneath
    (VISOR_CUT_MOUTH) as SF's are; its three lenses are kept as their own primitives
    (materials `lens_red`, `lens_amber`, `lens_green`), mapped 0..1 across each, their domes
    deepened (LENS_DOME_SAG_M) and their faces turned out of the head;
  * pedestrian head: the McCain 16 in countdown head of `mccainPed16` (hooded housing), its
    face kept as its own primitive (material `ped_face`), mapped 0..1 across the face;
  * the housings' flat source materials are baked with Cycles into one albedo and one
    metallic-roughness atlas per head, weathered here (sun-faded polycarbonate, dust on the
    up-facing surfaces, grime in the crevices, rain streaks down the sides): matte, aged
    hardware as the city's is;
  * `signal-lenses-v1.png` (drawn, `lens_images`): a cell per lens, red, amber and green unlit
    (top left, top right, bottom left): the tinted lens over the module's LEDs and pillow
    lenslets; in the bottom right cell the lit modules' pattern (red, amber, green in R, G, B;
    its mean over the lit area 0.5): the game lights a lens in its own calibrated colour
    through it;
  * `ped-faces-v1.png` (drawn, `ped_images`): the countdown face unlit (top left), and in the
    top right cell the lit symbols' LED masks (the upraised hand in R, the walking person in G);
  * every file's digest is written in `street-signals-v1.provenance.json` with the sources,
    their licence, authors and links, and what was changed (CC BY 4.0 section 3(a)(1)(B)).

Frames (glTF Y up): vehicle head origin at the middle lens centre, lenses toward +Z (the
`signalHead-v2` slot); pedestrian head origin at the face centre, face toward +Z, the
housing behind (the game's `pedHead` box: z from -depth to 0).
"""

import bpy
import bmesh
import hashlib
import json
import math
import os
import struct
import sys

import numpy as np
from mathutils import Matrix, Vector

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), '..', '..'))
SCRIPT = os.path.abspath(__file__)
STANDARDS = os.path.join(ROOT, 'scripts', 'world-data', 'sources', 'california-street-standards.json')
IN = 0.0254

SOURCES = {
    'mccainLeotek12': {
        'uid': '5d1544f61cc74047a2453d55791882ba',
        'title': 'McCain Leotek 12 Inch Signal Set',
        'author': 'Signalrenders',
        'authorUrl': 'https://sketchfab.com/Signalrenders',
        'url': 'https://sketchfab.com/3d-models/mccain-leotek-12-inch-signal-set-5d1544f61cc74047a2453d55791882ba',
        'license': 'CC-BY-4.0',
        'licenseUrl': 'https://creativecommons.org/licenses/by/4.0/',
        'file': 'model.glb (Sketchfab download, glTF binary)',
        'sha256': '0ad03a2247621b0b60759db9efe24a0c745a321744e411e1ac47c11e5075cea7',
        'bytes': 2555948,
        'used': 'geometry only; its lens textures (photographs, no record of who took them) are not used',
    },
    'mccainPed16': {
        'uid': '8c245f9477594831908c570c4f9a1385',
        'title': 'McCain LED Pedestrian Signals Pack',
        'author': 'Signalrenders',
        'authorUrl': 'https://sketchfab.com/Signalrenders',
        'url': 'https://sketchfab.com/3d-models/mccain-led-pedestrian-signals-pack-8c245f9477594831908c570c4f9a1385',
        'license': 'CC-BY-4.0',
        'licenseUrl': 'https://creativecommons.org/licenses/by/4.0/',
        'file': 'model.glb (Sketchfab download, glTF binary)',
        'sha256': '5aad569ac424ba8881d19d28d23727e695bfdfce4745110b9a5b509629f7794c',
        'bytes': 2267292,
        'used': 'geometry only; its face textures are not used (the lit ones are Flickr photographs by Citylight74 under CC BY-NC 2.0, '
                'e.g. https://www.flickr.com/photos/123654821@N05/35599766721/; the unlit one has no record of who took it)',
    },
}

# The game's vehicle head slot (signalHead-v2, eo-render props_lib): lens centres 0.35 m apart.
LENS_PITCH_M = 0.35
# The source head the game head is built from: the object-name group whose bottom lens is lit
# green (its housing, visors, gaskets and wing nuts), and the source materials per lens state.
HEAD_GROUP = 'Cylinder.857'
LENS_MATS = {
    '12inchleotekredoff': ('red', 'off'),
    'Material.105': ('amber', 'off'),
    'Material.106': ('green', 'off'),
    'leotek_12inch_red_ON': ('red', 'on'),
    '12inch_leotek_Amber_ON': ('amber', 'on'),
    '12inch_Leotek_green_ON': ('green', 'on'),
}
COLOURS = ('red', 'amber', 'green')
# The pedestrian source: the hooded housing group and its face.
PED_GROUP = 'Circle.003'
PED_FACE_MAT = 'Ped_OFF'
PED_HOUSING_MAT = 'MCCAIN_16_PED_SIGNAL'

BAKE_PX = 1024
LENS_PX = 512
CELL_INSET = 0.02  # each lens / face sits in its cell inset by this share of the layer (mip gutter)
# The drawn lens: a module's LEDs (a hex array, LENS_LEDS_ACROSS across the lens) behind
# its pillow lenslets (LENS_LENSLETS_ACROSS across, one per colour so the modules differ).
# Modelling values, from the 12 in LED modules' look: no primary source read.
LENS_LEDS_ACROSS = 7.5
LENS_LENSLETS_ACROSS = (44, 46, 42)
# The drawn pedestrian face: its symbols' LEDs on a square pitch (a modelling value, from
# the 16 in countdown heads' look), each a disc PED_LED_R_M across its flat top.
PED_LED_PITCH_M = 0.0075
PED_LED_R_M = (0.0027, 0.0037)
# Its egg-crate visor (the anti-phantom lattice over countdown faces, in photographs a diagonal
# mesh ~13 mm across): the vanes' pitch, their thickness and their edges' albedo. Modelling values.
PED_MESH_PITCH_M = 0.013
PED_MESH_VANE_M = 0.0016
PED_MESH_VANE_ALBEDO = 0.05
# The lens's dome: its apex this far in front of its rim (the source's ~14 mm read flat; a
# 12 in LED module's polycarbonate dome stands well out of the door in photographs of SF
# heads from below and beside). INFERRED from those photographs.
LENS_DOME_SAG_M = 0.028
# The tunnel visors cut away underneath, as SF's are: a slanted plane from the visor's bottom
# at the door (no opening there) up to a chord VISOR_CUT_MOUTH x the visor's inside radius
# under the lens axis at the mouth (+-60 deg open there), so a lens shows from below. The
# visor's inside radius and the door and mouth planes are the source's (measured below).
# INFERRED from photographs of SF heads from below.
VISOR_CUT_MOUTH = 0.5
# The visors' rings: the source's 16 segments (22.5 deg) read as flat facets on the visors' mouths
# and cut edges at a few metres; each segment split this many times more, the new points on the
# circle (rounded, not flat). A modelling value: 32 segments put the facets' sagitta under
# 0.75 mm, under a pixel at 3 m in a 13 deg view at 800 px.
VISOR_RING_SPLIT = 1
SS = 3  # the drawn images' supersampling (per axis)


def args():
    out = {'out': os.path.join(ROOT, 'public', 'assets', 'street-signals'), 'renders': None, 'hdri': None, 'samples': 96, 'seed': 11}
    argv = sys.argv[sys.argv.index('--') + 1:] if '--' in sys.argv else []
    i = 0
    while i < len(argv):
        k = argv[i].lstrip('-')
        if k not in out:
            raise SystemExit(f'unknown argument {argv[i]}')
        out[k] = argv[i + 1]
        i += 2
    out['samples'] = int(out['samples'])
    out['seed'] = int(out['seed'])
    return out


def sha256(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        h.update(f.read())
    return h.hexdigest()


def source_path(key):
    s = SOURCES[key]
    base = os.environ.get('EO_SKETCHFAB_DIR') or os.path.join(ROOT, 'external', 'sketchfab')
    path = os.path.join(base, s['uid'], 'model.glb')
    if not os.path.exists(path):
        raise SystemExit(f'{key}: {path} missing (download {s["url"]} as glTF binary into it)')
    got = sha256(path)
    if got != s['sha256']:
        raise SystemExit(f'{key}: {path} has sha256 {got}, expected {s["sha256"]}')
    return path


def standards_border_in():
    """The backplate border (in): 2 in, inside the street-standards entry's range."""
    d = json.load(open(STANDARDS))
    e = next(e for e in d['entries'] if e.get('id') == 'signal-backplate-border')
    lo, hi = e['value']['min'], e['value']['max']
    border = 2.0
    assert lo <= border <= hi, (lo, hi)
    return border


# ---------------------------------------------------------------- scene helpers

def reset():
    bpy.ops.wm.read_factory_settings(use_empty=True)
    sc = bpy.context.scene
    sc.render.engine = 'CYCLES'
    sc.cycles.device = 'CPU'
    sc.cycles.samples = 1
    sc.render.bake.margin = 6


def import_glb(path):
    before = set(bpy.data.objects)
    bpy.ops.import_scene.gltf(filepath=path)
    new = [o for o in bpy.data.objects if o not in before]
    # every mesh into world space, unparented (the source nodes carry the set's layout)
    for o in new:
        if o.type == 'MESH':
            o.data = o.data.copy()
            o.data.transform(o.matrix_world)
    for o in new:
        if o.type == 'MESH':
            o.parent = None
            o.matrix_world = Matrix.Identity(4)
    for o in new:
        if o.type != 'MESH':
            bpy.data.objects.remove(o, do_unlink=True)
    return [o for o in bpy.data.objects if o.type == 'MESH']


def group(o):
    return o.name.split('_', 1)[0]


def mat_name(o):
    m = o.data.materials[0] if o.data.materials else None
    return m.name if m else ''


def bounds(objs):
    pts = [v.co for o in objs for v in o.data.vertices]
    mn = Vector((min(p[i] for p in pts) for i in range(3)))
    mx = Vector((max(p[i] for p in pts) for i in range(3)))
    return mn, mx


def select_only(objs, active=None):
    bpy.ops.object.select_all(action='DESELECT')
    for o in objs:
        o.select_set(True)
    bpy.context.view_layer.objects.active = active or objs[0]


def join(objs, name):
    objs = [o for o in objs]
    select_only(objs)
    bpy.ops.object.join()
    j = bpy.context.view_layer.objects.active
    j.name = name
    j.data.name = name
    return j


def duplicate(o, name):
    d = o.copy()
    d.data = o.data.copy()
    d.name = name
    bpy.context.scene.collection.objects.link(d)
    return d


def transform(objs, m):
    for o in objs:
        o.data.transform(m)
        o.data.update()


def principled(name, base=(0.8, 0.8, 0.8), rough=0.5, metal=0.0):
    m = bpy.data.materials.new(name)
    m.use_nodes = True
    b = next(n for n in m.node_tree.nodes if n.type == 'BSDF_PRINCIPLED')
    b.inputs['Base Color'].default_value = (*base, 1.0)
    b.inputs['Roughness'].default_value = rough
    b.inputs['Metallic'].default_value = metal
    return m


# ---------------------------------------------------------------- planar maps and bakes

def planar_uv(o, name, origin, ax_u, ax_v, size_u, size_v):
    """A UV map `name` on o: u along ax_u from origin over size_u, v along ax_v over size_v (0..1)."""
    me = o.data
    uv = me.uv_layers.get(name) or me.uv_layers.new(name=name)
    for loop in me.loops:
        p = me.vertices[loop.vertex_index].co - origin
        uv.data[loop.index].uv = (p.dot(ax_u) / size_u + 0.5, p.dot(ax_v) / size_v + 0.5)
    return uv


def new_float_image(name, px):
    img = bpy.data.images.new(name, px, px, alpha=True, float_buffer=True)
    img.colorspace_settings.name = 'Non-Color'
    img.generated_color = (0, 0, 0, 1)
    return img


def pixels(img):
    w, h = img.size
    buf = np.empty(w * h * 4, dtype=np.float32)
    img.pixels.foreach_get(buf)
    return buf.reshape(h, w, 4)  # row 0 = bottom (Blender)


def srgb_encode(x):
    x = np.clip(x, 0.0, 1.0)
    return np.where(x <= 0.0031308, x * 12.92, 1.055 * np.power(x, 1 / 2.4) - 0.055)


def save_png(rgb_linear_or_srgb, path, encode):
    """Write an (h, w, 3) array (row 0 = bottom, Blender's order) as an 8-bit RGB PNG; encode:
    sRGB-encode it first. Lossless: the packed patterns and masks keep their channels apart."""
    a = srgb_encode(rgb_linear_or_srgb) if encode else np.clip(rgb_linear_or_srgb, 0, 1)
    h, w = a.shape[:2]
    img = bpy.data.images.new(os.path.basename(path), w, h, alpha=False)
    img.colorspace_settings.name = 'Non-Color'  # the values are written as given
    rgba = np.concatenate([a, np.ones((h, w, 1))], axis=2).astype(np.float32)
    img.pixels.foreach_set(rgba.ravel())
    img.filepath_raw = path
    img.file_format = 'PNG'
    bpy.context.scene.render.image_settings.color_mode = 'RGB'
    bpy.context.scene.render.image_settings.compression = 100
    img.save()
    bpy.data.images.remove(img)


def save_jpeg(rgb_linear_or_srgb, path, encode, quality=92):
    """Write an (h, w, 3) array (row 0 = bottom, Blender's order) as a JPEG; encode: sRGB-encode it first."""
    a = srgb_encode(rgb_linear_or_srgb) if encode else np.clip(rgb_linear_or_srgb, 0, 1)
    h, w = a.shape[:2]
    img = bpy.data.images.new(os.path.basename(path), w, h, alpha=False)
    img.colorspace_settings.name = 'Non-Color'  # the values are written as given
    rgba = np.concatenate([a, np.ones((h, w, 1))], axis=2).astype(np.float32)
    img.pixels.foreach_set(rgba.ravel())
    img.filepath_raw = path
    img.file_format = 'JPEG'
    img.save(quality=quality)
    bpy.data.images.remove(img)


def cell_place(dst, src, cell, px):
    """Copy src (s x s, Blender row order) into dst's cell (cx, cy) in IMAGE order (cy 0 = top), inset."""
    cx, cy = cell
    half = px // 2
    inset = int(round(CELL_INSET * px))
    s = half - 2 * inset
    from_img = resize(src, s)
    x0 = cx * half + inset
    y0_top = cy * half + inset
    # Blender rows run from the bottom: image row r (from the top) is array row px - 1 - r.
    for r in range(s):
        dst[px - 1 - (y0_top + r), x0:x0 + s] = from_img[s - 1 - r]


def resize(a, s):
    h, w = a.shape[:2]
    ys = (np.arange(s) + 0.5) * h / s - 0.5
    xs = (np.arange(s) + 0.5) * w / s - 0.5
    y0 = np.clip(np.floor(ys).astype(int), 0, h - 1)
    x0 = np.clip(np.floor(xs).astype(int), 0, w - 1)
    y1 = np.clip(y0 + 1, 0, h - 1)
    x1 = np.clip(x0 + 1, 0, w - 1)
    fy = np.clip(ys - y0, 0, 1)[:, None, None]
    fx = np.clip(xs - x0, 0, 1)[None, :, None]
    t = a[y0][:, x0] * (1 - fx) + a[y0][:, x1] * fx
    b = a[y1][:, x0] * (1 - fx) + a[y1][:, x1] * fx
    return t * (1 - fy) + b * fy


# ---------------------------------------------------------------- the drawn lens and face images

def grid_coords(px, w_m, h_m):
    """Sample coordinates (metres; x right, y up, the origin at the cell's centre) of a px x px
    cell spanning w_m x h_m, SS x SS samples a pixel, image order (row 0 = top)."""
    n = px * SS
    c = (np.arange(n) + 0.5) / n
    u, v = np.meshgrid(c, c)
    return (u - 0.5) * w_m, (0.5 - v) * h_m


def down(a):
    h, w = a.shape[:2]
    return a.reshape(h // SS, SS, w // SS, SS, *a.shape[2:]).mean(axis=(1, 3))


def smoothstep(e0, e1, x):
    t = np.clip((x - e0) / (e1 - e0), 0.0, 1.0)
    return t * t * (3 - 2 * t)


def capsule(x, y, pts, r):
    """Signed distance to a polyline of radius r."""
    d = np.full(x.shape, np.inf)
    for (ax, ay), (bx, by) in zip(pts[:-1], pts[1:]):
        px, py, vx, vy = x - ax, y - ay, bx - ax, by - ay
        t = np.clip((px * vx + py * vy) / max(vx * vx + vy * vy, 1e-12), 0, 1)
        d = np.minimum(d, np.hypot(px - vx * t, py - vy * t))
    return d - r


def rbox(x, y, c, half, r):
    qx = np.abs(x - c[0]) - (half[0] - r)
    qy = np.abs(y - c[1]) - (half[1] - r)
    return np.hypot(np.maximum(qx, 0), np.maximum(qy, 0)) + np.minimum(np.maximum(qx, qy), 0) - r


def hex_distance(x, y, p):
    """Distance to the nearest point of a hex lattice of pitch p (rows p * sqrt 3 / 2 apart)."""
    rh = p * math.sqrt(3) / 2
    j0 = np.round(y / rh)
    d = np.full(x.shape, np.inf)
    for dj in (-1, 0, 1):
        j = j0 + dj
        off = np.where(np.mod(j, 2) == 1, p / 2, 0.0)
        i = np.round((x - off) / p)
        d = np.minimum(d, np.hypot(x - (i * p + off), y - j * rh))
    return d


def lens_images(lens_r, k):
    """Lens k (red, amber, green), filling a LENS_PX / 2 cell, image order: (the unlit lens's albedo,
    linear; the lit module's pattern, its mean over the lit area 0.5). The LEDs behind the pillow
    lenslets of a 12 in LED module, a faint Fresnel ring structure, the rim gasket dark."""
    x, y = grid_coords(LENS_PX // 2, 2 * lens_r, 2 * lens_r)
    r = np.hypot(x, y) / lens_r
    g = 2 * lens_r * 0.9 / LENS_LENSLETS_ACROSS[k]
    fx = np.abs(np.mod(x / g + 0.5, 1.0) - 0.5)
    fy = np.abs(np.mod(y / g + 0.5, 1.0) - 0.5)
    lenslet = smoothstep(0.5, 0.32, np.maximum(fx, fy)) * (0.72 + 0.28 * (1 - 2 * (fx * fx + fy * fy)))
    p = 2 * lens_r / LENS_LEDS_ACROSS
    d = hex_distance(x, y, p)
    hot = np.exp(-(d / (0.3 * p)) ** 2)
    die = np.exp(-(d / (0.05 * p)) ** 2)
    area = smoothstep(0.93, 0.89, r)
    rings = 0.93 + 0.07 * np.cos(r * 2 * math.pi * 9)
    lit = down(area * lenslet * (0.6 + 0.4 * hot) * rings * (1 - 0.12 * r * r))
    rd = down(r)
    lit = np.clip(lit / max(float(lit[rd < 0.85].mean()), 1e-4) * 0.5, 0, 1)
    tint = np.array([(0.085, 0.010, 0.008), (0.11, 0.050, 0.006), (0.010, 0.055, 0.045)][k], dtype=np.float32)
    glass = (0.55 + 0.45 * lenslet) * (1 - 0.35 * r * r)
    alb = tint[None, None, :] * glass[..., None] + 0.05 * die[..., None] * np.array([1.0, 0.95, 0.85])
    rim = smoothstep(0.93, 0.97, r)[..., None]
    alb = alb * (1 - rim) + 0.02 * rim
    return down(alb).astype(np.float32), lit.astype(np.float32)


def hand_sdf(x, y, c):
    """The upraised hand (a right hand, palm toward the viewer, the thumb on the right), centred at c."""
    X, Y = x - c[0], y - c[1]
    # the fingers 21 mm wide with gaps wider than PED_LED_PITCH_M (an LED column dark between them)
    d = rbox(X, Y, (0.0, -0.040), (0.058, 0.058), 0.016)
    d = np.minimum(d, rbox(X, Y, (-0.004, -0.112), (0.044, 0.030), 0.010))
    for base, tip in (((-0.045, -0.005), (-0.052, 0.062)), ((-0.015, 0.000), (-0.018, 0.098)),
                      ((0.015, 0.000), (0.017, 0.108)), ((0.045, -0.005), (0.052, 0.088))):
        d = np.minimum(d, capsule(X, Y, [base, tip], 0.0105))
    return np.minimum(d, capsule(X, Y, [(0.052, -0.075), (0.082, -0.042), (0.096, -0.008)], 0.0115))


def walk_sdf(x, y, c):
    """The walking person (striding to the left), centred at c."""
    X, Y = x - c[0], y - c[1]
    d = np.hypot(X - 0.004, Y - 0.100) - 0.020
    d = np.minimum(d, capsule(X, Y, [(0.0, 0.072), (0.006, -0.012)], 0.019))
    for pts, r in (([(-0.004, 0.060), (-0.035, 0.025), (-0.040, -0.015)], 0.0085),
                   ([(0.006, 0.060), (0.034, 0.028), (0.052, -0.004)], 0.0085),
                   ([(0.0, -0.020), (-0.030, -0.070), (-0.050, -0.125), (-0.066, -0.130)], 0.0115),
                   ([(0.010, -0.020), (0.032, -0.075), (0.040, -0.130), (0.058, -0.132)], 0.0115)):
        d = np.minimum(d, capsule(X, Y, pts, r))
    return d


def digits_sdf(x, y, centres, w=0.032, h=0.082, gap=0.009, r=0.0075):
    """Seven-segment '8's (the countdown's two digits, every segment)."""
    d = np.full(x.shape, np.inf)
    for cx in centres:
        P = {'tl': (-w, h), 'tr': (w, h), 'ml': (-w, 0), 'mr': (w, 0), 'bl': (-w, -h), 'br': (w, -h)}
        for a, b in (('tl', 'tr'), ('tr', 'mr'), ('mr', 'br'), ('bl', 'br'), ('ml', 'bl'), ('tl', 'ml'), ('ml', 'mr')):
            (ax, ay), (bx, by) = P[a], P[b]
            L = math.hypot(bx - ax, by - ay)
            ux, uy = (bx - ax) / L, (by - ay) / L
            seg = [(cx + ax + ux * gap, ay + uy * gap), (cx + bx - ux * gap, by - uy * gap)]
            d = np.minimum(d, capsule(x, y, seg, r))
    return d


def ped_images(fw, fh):
    """The 16 in countdown face (fw x fh m) filling a LENS_PX / 2 cell, image order: (its unlit albedo,
    linear; {'hand', 'walk'}: the lit symbols' masks; the masks' mean over their symbols). The
    hand and the walking person overlaid in the left half, the countdown's two digits in the
    right, each an array of LEDs; unlit, the LEDs a little paler than the black face, under the
    egg-crate visor's diagonal lattice."""
    x, y = grid_coords(LENS_PX // 2, fw, fh)
    P = PED_LED_PITCH_M
    cx = (np.floor(x / P) + 0.5) * P
    cy = (np.floor(y / P) + 0.5) * P
    led = smoothstep(PED_LED_R_M[1], PED_LED_R_M[0], np.hypot(x - cx, y - cy))
    sym_c = (-0.105, 0.0)
    hand = (hand_sdf(cx, cy, sym_c) < 0).astype(np.float32)
    walk = (walk_sdf(cx, cy, sym_c) < 0).astype(np.float32)
    digs = (digits_sdf(cx, cy, (0.062, 0.150)) < 0).astype(np.float32)
    # the egg-crate visor over the face: a diagonal lattice of thin vanes (PED_MESH_PITCH_M apart,
    # PED_MESH_VANE_M thick) whose edges catch the light over the black face behind
    u, v = (x + y) / math.sqrt(2.0), (x - y) / math.sqrt(2.0)
    du = np.abs(np.mod(u / PED_MESH_PITCH_M + 0.5, 1.0) - 0.5) * PED_MESH_PITCH_M
    dv = np.abs(np.mod(v / PED_MESH_PITCH_M + 0.5, 1.0) - 0.5) * PED_MESH_PITCH_M
    vane = smoothstep(PED_MESH_VANE_M, 0.5 * PED_MESH_VANE_M, np.minimum(du, dv))
    face = 0.018 + PED_MESH_VANE_ALBEDO * vane
    edge = smoothstep(0.0, 0.012, -rbox(x, y, (0.0, 0.0), (fw / 2, fh / 2), 0.02))
    face = face * (0.6 + 0.4 * edge)
    ghost = 0.045 * led * np.clip(hand + walk + digs, 0, 1)
    alb = face[..., None] + ghost[..., None] * np.array([1.0, 0.96, 0.88])
    m_hand, m_walk = down(led * hand), down(led * walk)
    inside = down(hand) > 0.5
    cover = float(m_hand[inside].mean()) if inside.any() else 0.0
    return down(alb).astype(np.float32), {'hand': m_hand.astype(np.float32), 'walk': m_walk.astype(np.float32)}, cover


# ---------------------------------------------------------------- weathering

def weather_material(src, kind, seed):
    """A bake material for a housing part: kind 'poly' (black polycarbonate), 'visor' (inside of
    the visors, matte), 'rubber', 'zinc' (plated hardware), 'yellow' (retroreflective sheeting),
    'plate' (painted aluminium backplate). Outputs: 'albedo' and 'orm' (1, roughness, metal)."""
    m = bpy.data.materials.new(f'w-{src}-{kind}')
    m.use_nodes = True
    nt = m.node_tree
    n, L = nt.nodes, nt.links
    for x in list(n):
        if x.type == 'BSDF_PRINCIPLED':
            n.remove(x)
    base, rough, metal = {
        # weathered, chalked polycarbonate / painted aluminium (UV-aged, matte): at 0.55 a street
        # lamp's glint ran along the visors' cylinders at 2.2 stops over the night's white and the
        # sky's sheen greyed them by day, where photographs show them about as dark as the housing
        # ("Chinatown San Francisco", 2014), glinting only on the lips; and chalked a little paler
        # (was 0.030): sunlit housings read 0.10-0.12 of a clear sky ("Stevens Creek Blvd traffic
        # light"), the game's 0.08 of a paler one
        'poly': ((0.045, 0.046, 0.048), 0.75, 0.0),
        'visor': ((0.016, 0.016, 0.017), 0.85, 0.0),
        'rubber': ((0.022, 0.021, 0.020), 0.9, 0.0),
        'zinc': ((0.30, 0.30, 0.29), 0.5, 0.75),
        'yellow': ((0.62, 0.42, 0.02), 0.45, 0.0),
        'plate': ((0.039, 0.040, 0.042), 0.75, 0.0),  # chalked and matte as the housing (was 0.026, 0.6)
    }[kind]
    geo = n.new('ShaderNodeNewGeometry')
    coord = n.new('ShaderNodeTexCoord')
    # dust: on up-facing surfaces, patchy
    sep = n.new('ShaderNodeSeparateXYZ')
    L.new(geo.outputs['Normal'], sep.inputs['Vector'])
    up = n.new('ShaderNodeMapRange')
    up.inputs['From Min'].default_value = 0.15
    up.inputs['From Max'].default_value = 0.85
    L.new(sep.outputs['Z'], up.inputs['Value'])
    patches = n.new('ShaderNodeTexNoise')
    patches.inputs['Scale'].default_value = 18.0
    patches.inputs['Detail'].default_value = 6.0
    patches.noise_dimensions = '4D'
    patches.inputs['W'].default_value = seed * 0.37
    L.new(coord.outputs['Object'], patches.inputs['Vector'])
    pr = n.new('ShaderNodeMapRange')
    pr.inputs['From Min'].default_value = 0.38
    pr.inputs['From Max'].default_value = 0.68
    L.new(patches.outputs['Fac'], pr.inputs['Value'])
    dust = n.new('ShaderNodeMath')
    dust.operation = 'MULTIPLY'
    L.new(up.outputs['Result'], dust.inputs[0])
    L.new(pr.outputs['Result'], dust.inputs[1])
    # rain streaks down the sides: noise stretched along z
    mp = n.new('ShaderNodeMapping')
    mp.inputs['Scale'].default_value = (60.0, 60.0, 2.5)
    L.new(coord.outputs['Object'], mp.inputs['Vector'])
    streak = n.new('ShaderNodeTexNoise')
    streak.inputs['Scale'].default_value = 1.0
    streak.inputs['Detail'].default_value = 2.0
    L.new(mp.outputs['Vector'], streak.inputs['Vector'])
    sr = n.new('ShaderNodeMapRange')
    sr.inputs['From Min'].default_value = 0.5
    sr.inputs['From Max'].default_value = 0.75
    L.new(streak.outputs['Fac'], sr.inputs['Value'])
    # grime in the crevices
    ao = n.new('ShaderNodeAmbientOcclusion')
    ao.inputs['Distance'].default_value = 0.04
    ao.samples = 16
    # colour: base, faded / streaked, dusted, grimed
    rgb = n.new('ShaderNodeRGB')
    rgb.outputs[0].default_value = (*base, 1.0)
    sk = n.new('ShaderNodeMath')
    sk.operation = 'MULTIPLY'
    sk.inputs[1].default_value = 0.6
    L.new(sr.outputs['Result'], sk.inputs[0])
    streaked = mix_rgb(n, L, sk.outputs[0], rgb.outputs[0], (base[0] * 1.6 + 0.012, base[1] * 1.6 + 0.011, base[2] * 1.6 + 0.010))
    dk = n.new('ShaderNodeMath')
    dk.operation = 'MULTIPLY'
    dk.inputs[1].default_value = 0.55 if kind in ('poly', 'plate', 'visor') else 0.3
    L.new(dust.outputs[0], dk.inputs[0])
    dusted = mix_rgb(n, L, dk.outputs[0], rgba_out(streaked), (0.105, 0.098, 0.088))
    inv = n.new('ShaderNodeMath')
    inv.operation = 'SUBTRACT'
    inv.inputs[0].default_value = 1.0
    L.new(ao.outputs['AO'], inv.inputs[1])
    grime = mix_rgb(n, L, inv.outputs[0], rgba_out(dusted), (0.55, 0.5, 0.45), blend='MULTIPLY')
    albedo = grime
    # roughness: dust and streaks matte it; metal stays on the plated hardware
    rr = n.new('ShaderNodeMath')
    rr.operation = 'MULTIPLY_ADD'
    L.new(dust.outputs[0], rr.inputs[0])
    rr.inputs[1].default_value = 0.3
    rr.inputs[2].default_value = rough
    rc = n.new('ShaderNodeClamp')
    L.new(rr.outputs[0], rc.inputs['Value'])
    comb = n.new('ShaderNodeCombineColor')
    comb.inputs['Red'].default_value = 1.0
    L.new(rc.outputs[0], comb.inputs['Green'])
    comb.inputs['Blue'].default_value = metal
    m['albedo_socket'] = albedo.name
    m['orm_socket'] = comb.name
    return m


def rgba_out(node):
    return next(s for s in node.outputs if s.type == 'RGBA')


def mix_rgb(n, L, fac, a, b, blend='MIX'):
    """A colour Mix node: fac a socket, a a socket, b an (r, g, b) value (its sockets found by type:
    the node carries float, vector and colour sockets of the same names)."""
    x = n.new('ShaderNodeMix')
    x.data_type = 'RGBA'
    x.blend_type = blend
    cols = [s for s in x.inputs if s.type == 'RGBA']
    facs = [s for s in x.inputs if s.name == 'Factor' and s.type == 'VALUE']
    L.new(fac, facs[0])
    L.new(a, cols[0])
    cols[1].default_value = (*b, 1.0)
    return x


def bake_atlas(o, seed, kinds, px):
    """o's atlas (map 'atlas'): (albedo linear, orm) arrays, Blender row order. kinds: slot material
    name -> weathering kind."""
    o.data.uv_layers.active = o.data.uv_layers['atlas']
    weathered = {}
    for slot in o.material_slots:
        weathered[slot.material.name] = weather_material(slot.material.name, kinds[slot.material.name], seed)
    originals = [slot.material for slot in o.material_slots]
    for slot in o.material_slots:
        slot.material = weathered[slot.material.name]
    out = {}
    for what in ('albedo', 'orm'):
        img = new_float_image(f'{o.name}-{what}', px)
        for slot in o.material_slots:
            m = slot.material
            nt = m.node_tree
            for x in [x for x in nt.nodes if x.type in ('EMISSION', 'TEX_IMAGE')]:
                nt.nodes.remove(x)
            outn = next(x for x in nt.nodes if x.type == 'OUTPUT_MATERIAL')
            em = nt.nodes.new('ShaderNodeEmission')
            src = rgba_out(nt.nodes[m['albedo_socket']]) if what == 'albedo' else nt.nodes[m['orm_socket']].outputs['Color']
            nt.links.new(src, em.inputs['Color'])
            nt.links.new(em.outputs['Emission'], outn.inputs['Surface'])
            tgt = nt.nodes.new('ShaderNodeTexImage')
            tgt.image = img
            for x in nt.nodes:
                x.select = False
            tgt.select = True
            nt.nodes.active = tgt
        select_only([o])
        bpy.context.scene.cycles.samples = 16 if what == 'albedo' else 1
        bpy.ops.object.bake(type='EMIT', use_clear=True, margin=8)
        out[what] = pixels(img)[..., :3].copy()
        bpy.data.images.remove(img)
    for slot, m in zip(o.material_slots, originals):
        slot.material = m
    for m in weathered.values():
        bpy.data.materials.remove(m)
    return out['albedo'], out['orm']


def smooth(o, deg=40.0):
    """Smooth shading split at creases over deg (the source's normals are per face on the visors)."""
    select_only([o])
    bpy.ops.object.mode_set(mode='EDIT')
    bpy.ops.mesh.select_all(action='SELECT')
    bpy.ops.mesh.customdata_custom_splitnormals_clear()
    bpy.ops.object.mode_set(mode='OBJECT')
    bpy.ops.object.shade_smooth_by_angle(angle=math.radians(deg), keep_sharp_edges=False)


def atlas_uv(o):
    uv = o.data.uv_layers.new(name='atlas')
    o.data.uv_layers.active = uv
    select_only([o])
    bpy.ops.object.mode_set(mode='EDIT')
    bpy.ops.mesh.select_all(action='SELECT')
    bpy.ops.uv.smart_project(angle_limit=math.radians(66), island_margin=0.006, area_weight=0.0, scale_to_bounds=True)
    bpy.ops.object.mode_set(mode='OBJECT')


def game_body_material(name, albedo, orm, tdir):
    """The body material: albedo (sRGB) and metallic-roughness maps written as JPEGs."""
    pa = os.path.join(tdir, f'{name}-albedo.jpg')
    po = os.path.join(tdir, f'{name}-orm.jpg')
    save_jpeg(albedo, pa, encode=True)
    save_jpeg(orm, po, encode=False)
    m = principled('body')
    nt = m.node_tree
    b = next(n for n in nt.nodes if n.type == 'BSDF_PRINCIPLED')
    ta = nt.nodes.new('ShaderNodeTexImage')
    ta.image = bpy.data.images.load(pa)
    to = nt.nodes.new('ShaderNodeTexImage')
    to.image = bpy.data.images.load(po)
    to.image.colorspace_settings.name = 'Non-Color'
    uvn = nt.nodes.new('ShaderNodeUVMap')
    uvn.uv_map = 'atlas'
    nt.links.new(uvn.outputs['UV'], ta.inputs['Vector'])
    nt.links.new(uvn.outputs['UV'], to.inputs['Vector'])
    nt.links.new(ta.outputs['Color'], b.inputs['Base Color'])
    sep = nt.nodes.new('ShaderNodeSeparateColor')
    nt.links.new(to.outputs['Color'], sep.inputs['Color'])
    nt.links.new(sep.outputs['Green'], b.inputs['Roughness'])
    nt.links.new(sep.outputs['Blue'], b.inputs['Metallic'])
    return m


# ---------------------------------------------------------------- export

def canonical_indices(path):
    """Every primitive's index buffer in a canonical order (each triangle from its smallest index,
    winding kept; triangles sorted), so the digests do not change from run to run."""
    with open(path, 'rb') as f:
        b = bytearray(f.read())
    jlen = struct.unpack_from('<I', b, 12)[0]
    gj = json.loads(bytes(b[20:20 + jlen]))
    boff = 20 + jlen + 8
    for mesh in gj['meshes']:
        for prim in mesh['primitives']:
            acc = gj['accessors'][prim['indices']]
            view = gj['bufferViews'][acc['bufferView']]
            fmt = {5121: 'B', 5123: 'H', 5125: 'I'}[acc['componentType']]
            start = boff + view.get('byteOffset', 0) + acc.get('byteOffset', 0)
            n = acc['count']
            idx = list(struct.unpack_from(f'<{n}{fmt}', b, start))
            tris = []
            for t in range(0, n, 3):
                a = idx[t:t + 3]
                k = a.index(min(a))
                tris.append(tuple(a[k:] + a[:k]))
            tris.sort()
            struct.pack_into(f'<{n}{fmt}', b, start, *[i for t in tris for i in t])
    with open(path, 'wb') as f:
        f.write(b)


def export(objs, path):
    """objs joined (materials in slot order: `body` first) and written as a GLB with one UV map."""
    j = join([duplicate(o, o.name + '-x') for o in objs], os.path.splitext(os.path.basename(path))[0])
    me = j.data
    for uv in [uv for uv in me.uv_layers if uv.name != 'atlas']:
        me.uv_layers.remove(uv)
    me.uv_layers['atlas'].active = True
    me.uv_layers['atlas'].active_render = True
    material_order_fix(j)
    assert me.materials[0].name == 'body'
    select_only([j])
    bpy.ops.export_scene.gltf(filepath=path, use_selection=True, export_format='GLB', export_yup=True, export_apply=True,
                              export_image_format='JPEG', export_jpeg_quality=90, export_texcoords=True, export_normals=True,
                              export_tangents=False, export_materials='EXPORT', export_extras=False)
    canonical_indices(path)
    tris = sum(len(p.vertices) - 2 for p in me.polygons)
    mn, mx = bounds([j])
    bpy.data.objects.remove(j, do_unlink=True)
    # glTF frame: (x, z, -y) of Blender's
    return tris, ([mn.x, mn.z, -mx.y], [mx.x, mx.z, -mn.y])


def material_order_fix(o):
    """Move the `body` slot first (the game reads the first material's maps)."""
    me = o.data
    names = [m.name for m in me.materials]
    if names[0] == 'body':
        return
    k = names.index('body')
    mats = list(me.materials)
    perm = [k] + [i for i in range(len(mats)) if i != k]
    inv = {old: new for new, old in enumerate(perm)}
    idx = [p.material_index for p in me.polygons]
    me.materials.clear()
    for i in perm:
        me.materials.append(mats[i])
    for p, i in zip(me.polygons, idx):
        p.material_index = inv[i]


# ---------------------------------------------------------------- vehicle head

def shape_lens(o, centre, lens_r):
    """A lens in the head's frame (its normal along y, the visors toward -y): its dome deepened to
    LENS_DOME_SAG_M in front of its rim (the rim kept), its faces turned out of the head (the
    source's face into it: the front toward -y, the back toward +y, the side outward), smooth.
    Returns the apex's and the rim's forward offsets (m, toward -y) from the centre's plane."""
    bm = bmesh.new()
    bm.from_mesh(o.data)
    back = max(v.co.y for v in bm.verts)
    front = [v for v in bm.verts if v.co.y < back - 0.003]
    rr = lambda v: math.hypot(v.co.x - centre.x, v.co.z - centre.z)
    apex = min(front, key=rr).co.y
    rim = max(v.co.y for v in front if rr(v) > 0.93 * lens_r)
    extra = LENS_DOME_SAG_M - (rim - apex)
    for v in front:
        v.co.y -= extra * max(1.0 - (rr(v) / lens_r) ** 2, 0.0)
    for f in bm.faces:
        c = f.calc_center_median()
        side = Vector((c.x - centre.x, 0.0, c.z - centre.z))
        if c.y < centre.y - 0.002:
            want = Vector((0.0, -1.0, 0.0))
        elif c.y > centre.y + 0.006:
            want = Vector((0.0, 1.0, 0.0))
        else:
            want = side
        if f.normal.dot(want) < 0.0:
            f.normal_flip()
    bm.normal_update()
    bm.to_mesh(o.data)
    bm.free()
    o.data.update()
    smooth(o, 40.0)
    apex = min(v.co.y for v in o.data.vertices)
    return centre.y - apex, centre.y - rim


def round_rings(o, centres, r_lo, r_hi, y_max):
    """The rings of o round the lens axes (y) rounded: for each centre, the faces whose points
    all lie nearest its axis, r_lo..r_hi from it and behind y_max (toward -y) joined back into
    quads, their edges round the axis (ends at one radius, over 10 deg apart) split
    VISOR_RING_SPLIT times, each new point put on the circle (its radius about the axis the
    edge's ends', interpolated) rather than on the chord. Returns the number of points added."""
    bm = bmesh.new()
    bm.from_mesh(o.data)
    rl = bm.verts.layers.float.new('ring_r')
    rr = lambda p, q: math.hypot(p.x - q.x, p.z - q.z)
    owner = lambda p: min(centres, key=lambda q: rr(p, q))
    for v in bm.verts:
        v[rl] = rr(v.co, owner(v.co))
    added = 0
    for c in centres:
        def ring_faces():
            return [f for f in bm.faces
                    if all(r_lo <= rr(v.co, c) < r_hi and v.co.y < y_max
                           and all(rr(v.co, c) <= rr(v.co, q) for q in centres) for v in f.verts)]
        faces = ring_faces()
        if not faces:
            continue
        bmesh.ops.join_triangles(bm, faces=faces, angle_face_threshold=math.radians(5.0), angle_shape_threshold=math.radians(180.0))
        faces = ring_faces()

        def ring(e):
            a = [math.atan2(v.co.z - c.z, v.co.x - c.x) for v in e.verts]
            d = abs(a[0] - a[1])
            same_r = abs(rr(e.verts[0].co, c) - rr(e.verts[1].co, c)) < 0.004
            return same_r and min(d, 2 * math.pi - d) > math.radians(10.0)
        bm.edges.index_update()
        # (in index order: a set's order varies from run to run, and with it the new points')
        edges = sorted((e for e in {e for f in faces for e in f.edges} if ring(e)), key=lambda e: e.index)
        res = bmesh.ops.subdivide_edges(bm, edges=edges, cuts=VISOR_RING_SPLIT, use_grid_fill=True)
        bm.verts.index_update()
        new = {v.index: v for v in res['geom_split'] + res['geom_inner'] if isinstance(v, bmesh.types.BMVert)}
        for _, v in sorted(new.items()):
            assert v[rl] > 0.5 * r_lo, 'the radius interpolated onto the new point'
            a = math.atan2(v.co.z - c.z, v.co.x - c.x)
            v.co.x = c.x + v[rl] * math.cos(a)
            v.co.z = c.z + v[rl] * math.sin(a)
            added += 1
    bm.verts.layers.float.remove(rl)
    bm.to_mesh(o.data)
    bm.free()
    o.data.update()
    return added


def cut_visors(o, centres, r_in, door_y, mouth_y):
    """The tunnel visors of o (the housing's outside or the visors' inside) cut away underneath:
    for each lens centre, the visor faces (in the annulus about its axis, under it, reaching
    forward of the door) bisected by a plane from the visor's bottom at the door to a chord
    VISOR_CUT_MOUTH x its inside radius under the axis at the mouth, what lies below removed."""
    bm = bmesh.new()
    bm.from_mesh(o.data)
    for c in centres:
        rr = lambda p, q=c: math.hypot(p.x - q.x, p.z - q.z)
        # (each face cut only for the lens whose axis it is nearest: a visor's top lies in the
        # annulus of the lens above)
        faces = [f for f in bm.faces
                 if all(0.12 < rr(v.co) < 0.215 and v.co.z < c.z and v.co.y < door_y + 0.01
                        and all(rr(v.co) <= rr(v.co, q) for q in centres) for v in f.verts)
                 and min(v.co.y for v in f.verts) < door_y - 0.1]
        if not faces:
            continue
        p0 = Vector((0.0, door_y, c.z - r_in))
        p1 = Vector((0.0, mouth_y, c.z - VISOR_CUT_MOUTH * r_in))
        t = p1 - p0
        n = Vector((0.0, t.z, -t.y)).normalized()
        if n.z < 0.0:
            n = -n
        bm.edges.index_update()
        bm.verts.index_update()
        # (in index order: a set's order varies from run to run, and with it the cut's new points)
        geom = (sorted({e for f in faces for e in f.edges}, key=lambda e: e.index)
                + sorted({v for f in faces for v in f.verts}, key=lambda v: v.index) + faces)
        bmesh.ops.bisect_plane(bm, geom=geom, dist=1e-6, plane_co=p0, plane_no=n, clear_inner=True)
    bmesh.ops.delete(bm, geom=[v for v in bm.verts if not v.link_faces], context='VERTS')
    bm.to_mesh(o.data)
    bm.free()
    o.data.update()


def build_head(a, tdir):
    reset()
    objs = import_glb(source_path('mccainLeotek12'))
    head = [o for o in objs if group(o) == HEAD_GROUP]
    lenses = {LENS_MATS[mat_name(o)][0]: o for o in head if mat_name(o) in LENS_MATS}
    assert set(lenses) == set(COLOURS), sorted(lenses)
    body = [o for o in head if mat_name(o) not in LENS_MATS]
    source_objects = sorted(group(o) + ' / ' + mat_name(o) for o in head)
    # frame: lens centres (top red), the lens normal, the front (toward the visor mouths)
    cen = {}
    for c, o in lenses.items():
        mn, mx = bounds([o])
        cen[c] = (mn + mx) / 2
    assert cen['red'].z > cen['amber'].z > cen['green'].z
    pitch = (cen['red'].z - cen['green'].z) / 2
    thin = min(range(3), key=lambda i: bounds([lenses['amber']])[1][i] - bounds([lenses['amber']])[0][i])
    assert thin == 1, 'lens normal along y'
    visor = next(o for o in body if mat_name(o).startswith('inner_visor'))
    vmn, vmx = bounds([visor])
    # the source faces -Y (Blender), the glTF +Z the game's frame wants: the visors' mouths lie toward -Y
    assert (vmn.y + vmx.y) / 2 < cen['amber'].y, 'visors behind the lenses: source frame changed'
    s = LENS_PITCH_M / pitch
    m = Matrix.Scale(s, 4) @ Matrix.Translation(-cen['amber'])
    others = [o for o in objs if o not in head]
    transform(head, m)
    # the lens images, drawn (the source's lens photographs are not used: no record of who took them)
    lens_r = max((v.co - (bounds([lenses['amber']])[0] + bounds([lenses['amber']])[1]) / 2).xz.length for v in lenses['amber'].data.vertices)
    # the visors (measured on the middle one's inside): radius, door and mouth planes; then the
    # lenses' domes and normals (shape_lens) and the visors cut away underneath (cut_visors)
    vv = [v.co for v in visor.data.vertices if 0.12 < math.hypot(v.co.x, v.co.z) < 0.2 and v.co.y < 0.1]
    r_in = sorted(math.hypot(p.x, p.z) for p in vv)[len(vv) // 10]
    door_y, mouth_y = max(p.y for p in vv), min(p.y for p in vv)
    cen_t = {c: Vector((0.0, (bounds([lenses[c]])[0].y + bounds([lenses[c]])[1].y) / 2, LENS_PITCH_M * (1 - k))) for k, c in enumerate(COLOURS)}
    # (the rings rounded first: the visors' shells (their radius band, forward of the door; the
    # door's frame and flanges, straight and further out, keep their chords) and the lenses'
    # outer half, whose rims meet the visors')
    for c in COLOURS:
        round_rings(lenses[c], [cen_t[c]], 0.5 * lens_r, 1.05 * lens_r, 1.0)
    dome = {c: shape_lens(lenses[c], cen_t[c], lens_r) for c in COLOURS}
    for o in body:
        if mat_name(o).startswith(('Aluminum', 'inner_visor')):
            round_rings(o, [cen_t[c] for c in COLOURS], 0.13, 0.175, door_y + 0.002)
            cut_visors(o, [cen_t[c] for c in COLOURS], r_in, door_y, mouth_y)
    atlas = np.zeros((LENS_PX, LENS_PX, 3), dtype=np.float32)
    pat = np.zeros((LENS_PX // 2, LENS_PX // 2, 3), dtype=np.float32)
    for k, (c, cell) in enumerate(zip(COLOURS, ((0, 0), (1, 0), (0, 1)))):
        alb, lit = lens_images(lens_r, k)
        cell_place(atlas, np.flipud(alb), cell, LENS_PX)
        pat[..., k] = np.flipud(lit)
    cell_place(atlas, pat, (1, 1), LENS_PX)
    lens_jpg = os.path.join(a['out'], 'signal-lenses-v1.png')
    # the unlit cells are colour (sRGB-encoded); the pattern cell is data, encoded alike (the game's
    # albedo array decodes sRGB, so the shader reads the linear pattern back)
    save_png(atlas, lens_jpg, encode=True)
    for o in others:
        bpy.data.objects.remove(o, do_unlink=True)

    # the body: housing, visors, gaskets, wing nuts, and the backplate
    border = standards_border_in() * IN
    bmn, bmx = bounds(body)
    housing = next(o for o in body if mat_name(o).startswith('Aluminum'))
    hmn, hmx = bounds([housing])
    hw = max(abs(hmn.x), abs(hmx.x))
    # the backplate: 0.11 m past the housing's sides, top and bottom as the v2 slot's, 6 cm behind the lens plane
    W, H = hw + 0.11, LENS_PITCH_M + 0.2672
    hx, hy = hw + 0.003, LENS_PITCH_M + 0.1705
    y0, y1 = 0.060, 0.066
    bp_mat = principled('backplate', (0.03, 0.03, 0.03), 0.6)
    yel = principled('border_yellow', (0.62, 0.42, 0.02), 0.45)
    bm = bmesh.new()

    def box(x0, z0, x1, z1, ya, yb, mi):
        vs = [bm.verts.new((x, y, z)) for x in (x0, x1) for y in (ya, yb) for z in (z0, z1)]
        for f in ((0, 1, 3, 2), (4, 6, 7, 5), (0, 4, 5, 1), (2, 3, 7, 6), (0, 2, 6, 4), (1, 5, 7, 3)):
            face = bm.faces.new([vs[i] for i in f])
            face.material_index = mi
    box(-W, hy, W, H, y0, y1, 0)
    box(-W, -H, W, -hy, y0, y1, 0)
    box(-W, -hy, -hx, hy, y0, y1, 0)
    box(hx, -hy, W, hy, y0, y1, 0)
    # behind the housing, the plate closed: the source's body is narrower than its doors, and
    # the opening round it showed the sky through slots down both sides of the head
    box(-hx, -hy, hx, hy, y0, y1, 0)
    yb = y0 - 0.0015
    box(-W, H - border, W, H, yb, y0, 1)
    box(-W, -H, W, -H + border, yb, y0, 1)
    box(-W, -H + border, -W + border, H - border, yb, y0, 1)
    box(W - border, -H + border, W, H - border, yb, y0, 1)
    # louvres: horizontal slats in the side panels, as a louvred backplate's
    for side in (-1, 1):
        for k in range(14):
            z = -hy + 0.05 + k * (2 * hy - 0.1) / 13
            xa, xb = sorted((side * (hx + 0.015), side * (W - border - 0.012)))
            box(xa, z - 0.006, xb, z + 0.006, y0 - 0.006, y0, 0)
    bm.normal_update()
    me = bpy.data.meshes.new('backplate')
    bm.to_mesh(me)
    bm.free()
    me.materials.append(bp_mat)
    me.materials.append(yel)
    bp = bpy.data.objects.new('backplate', me)
    bpy.context.scene.collection.objects.link(bp)
    body_j = join(body + [bp], 'head_body')
    smooth(body_j)
    kinds = {}
    for mm in body_j.data.materials:
        n = mm.name
        kinds[n] = ('visor' if n.startswith('inner_visor') else 'rubber' if n.startswith('rubber_rim') else 'zinc' if n.startswith('wingnut')
                    else 'yellow' if n == 'border_yellow' else 'plate' if n == 'backplate' else 'poly')
    atlas_uv(body_j)
    albedo, orm = bake_atlas(body_j, a['seed'], kinds, BAKE_PX)
    # a gentle occlusion term in the albedo (the game's maps carry no occlusion channel)
    bodymat = game_body_material('signalHead-v1', albedo, orm, tdir)
    body_j.data.materials.clear()
    body_j.data.materials.append(bodymat)
    # the lenses: their own primitives, mapped 0..1 across each lens
    lens_objs = []
    for c in COLOURS:
        o = lenses[c]
        oc = (bounds([o])[0] + bounds([o])[1]) / 2
        planar_uv(o, 'atlas', oc, Vector((1, 0, 0)), Vector((0, 0, 1)), 2 * lens_r, 2 * lens_r)
        o.data.materials.clear()
        o.data.materials.append(principled(f'lens_{c}', {'red': (0.05, 0.003, 0.002), 'amber': (0.06, 0.025, 0.002), 'green': (0.004, 0.04, 0.03)}[c], 0.15))
        lens_objs.append(o)
    path = os.path.join(a['out'], 'signalHead-v1.glb')
    tris, bb = export([body_j] + lens_objs, path)
    lens_front = max(-v.co.y for o in lens_objs for v in o.data.vertices)
    row = {
        'path': rel(path), 'sha256': sha256(path), 'bytes': os.path.getsize(path), 'triangles': tris,
        'boundsM': {'min': r4(bb[0]), 'max': r4(bb[1])}, 'dimensionsM': r4([bb[1][i] - bb[0][i] for i in range(3)]),
        'materials': ['body', 'lens_red', 'lens_amber', 'lens_green'],
        'lensPitchM': LENS_PITCH_M, 'lensRadiusM': round(lens_r, 4), 'lensFaceZ': round(lens_front, 4),
        'lensCentres': {f'lens_{c}': [0.0, round(LENS_PITCH_M * (1 - k), 4), 0.0] for k, c in enumerate(COLOURS)},
        'lensDomeSagM': round(dome['amber'][0] - dome['amber'][1], 4),
        'visorInsideRadiusM': round(r_in, 4), 'visorDoorZM': round(-door_y, 4), 'visorMouthZM': round(-mouth_y, 4), 'visorCutMouthOfRadius': VISOR_CUT_MOUTH, 'ringSplit': VISOR_RING_SPLIT,
        'scaleFromSource': round(s, 5), 'backplateBorderIn': standards_border_in(), 'backplateBackZM': -y1, 'backplateFrontZM': -y0,
        'source': 'mccainLeotek12', 'sourceObjects': source_objects,
        'changes': 'one head of the set kept (its housing, visors, gaskets and wing nuts), scaled to the game lens pitch and moved to the game frame; '
                   'a louvred backplate with a 2 in yellow border added (closed behind the housing); the flat materials replaced by a baked, weathered albedo and metallic-roughness atlas; '
                   'the lenses re-mapped 0..1 across each lens and their photographs dropped (the drawn lens image in their place); '
                   'the lenses\' domes deepened and their faces turned out of the head (the source\'s face into it); the visors\' and the lenses\' rings rounded (each segment split, '
                   'the new points on the circle); the tunnel visors cut away underneath',
    }
    lens_row = {'path': rel(lens_jpg), 'sha256': sha256(lens_jpg), 'bytes': os.path.getsize(lens_jpg), 'px': LENS_PX, 'cellInset': CELL_INSET,
                'cells': {'red_off': [0, 0], 'amber_off': [1, 0], 'green_off': [0, 1], 'lit_pattern_rgb': [1, 1]},
                'source': 'drawn by this script (lens_images): no photograph', 'ledsAcross': LENS_LEDS_ACROSS, 'lensletsAcross': LENS_LENSLETS_ACROSS}
    return row, lens_row


# ---------------------------------------------------------------- pedestrian head

def build_ped(a, tdir):
    reset()
    objs = import_glb(source_path('mccainPed16'))
    grp = [o for o in objs if group(o) == PED_GROUP]
    face = next(o for o in grp if mat_name(o) == PED_FACE_MAT)
    housing = next(o for o in grp if mat_name(o) == PED_HOUSING_MAT)
    fmn, fmx = bounds([face])
    fc = (fmn + fmx) / 2
    fw, fh = fmx.x - fmn.x, fmx.z - fmn.z
    hmn, hmx = bounds([housing])
    # the source faces -Y (Blender), glTF +Z: the housing's back lies toward +Y, past the face
    assert hmx.y - fc.y > 0.15, 'housing not behind the face: source frame changed'
    m = Matrix.Translation(-fc)

    # the face image, drawn (the source's face photographs are not used: the lit ones are
    # Flickr photographs under CC BY-NC 2.0, the unlit one has no record of who took it)
    off, masks, cover = ped_images(fw, fh)
    atlas = np.zeros((LENS_PX, LENS_PX, 3), dtype=np.float32)
    cell_place(atlas, np.flipud(off), (0, 0), LENS_PX)
    mk = np.zeros((LENS_PX // 2, LENS_PX // 2, 3), dtype=np.float32)
    mk[..., 0] = np.flipud(masks['hand'])
    mk[..., 1] = np.flipud(masks['walk'])
    cell_place(atlas, mk, (1, 0), LENS_PX)
    faces_jpg = os.path.join(a['out'], 'ped-faces-v1.png')
    save_png(atlas, faces_jpg, encode=True)
    for o in objs:
        if o not in (face, housing):
            bpy.data.objects.remove(o, do_unlink=True)
    transform([face, housing], m)
    housing.name = 'ped_body'
    smooth(housing)
    kinds = {housing.data.materials[0].name: 'poly'}
    atlas_uv(housing)
    albedo, orm = bake_atlas(housing, a['seed'] + 1, kinds, BAKE_PX)
    housing.data.materials.clear()
    housing.data.materials.append(game_body_material('pedHead-v1', albedo, orm, tdir))
    planar_uv(face, 'atlas', Vector((0, 0, 0)), Vector((1, 0, 0)), Vector((0, 0, 1)), fw, fh)
    face.data.materials.clear()
    face.data.materials.append(principled('ped_face', (0.03, 0.03, 0.03), 0.3))
    path = os.path.join(a['out'], 'pedHead-v1.glb')
    tris, bb = export([housing, face], path)
    face_z = max(-v.co.y for v in face.data.vertices)
    row = {
        'path': rel(path), 'sha256': sha256(path), 'bytes': os.path.getsize(path), 'triangles': tris,
        'boundsM': {'min': r4(bb[0]), 'max': r4(bb[1])}, 'dimensionsM': r4([bb[1][i] - bb[0][i] for i in range(3)]),
        'materials': ['body', 'ped_face'], 'faceSizeM': r4([fw, fh]), 'faceZ': round(face_z, 4),
        'source': 'mccainPed16', 'sourceObjects': [f'{PED_GROUP} / {PED_HOUSING_MAT}', f'{PED_GROUP} / {PED_FACE_MAT}'],
        'changes': 'one hooded head of the pack kept (housing and face) and moved to the game frame; the flat housing material replaced by a baked, '
                   'weathered albedo and metallic-roughness atlas; the face re-mapped 0..1 across it and its photographs dropped (the drawn face image, under an egg-crate lattice, in their place)',
    }
    faces_row = {'path': rel(faces_jpg), 'sha256': sha256(faces_jpg), 'bytes': os.path.getsize(faces_jpg), 'px': LENS_PX, 'cellInset': CELL_INSET,
                 'cells': {'face_off': [0, 0], 'lit_masks_rg': [1, 0]}, 'masks': {'R': 'hand', 'G': 'walking person'},
                 'source': 'drawn by this script (ped_images): no photograph', 'ledPitchM': PED_LED_PITCH_M, 'maskCoverOfSymbol': round(cover, 4)}
    return row, faces_row


def rel(p):
    return os.path.relpath(p, ROOT).replace(os.sep, '/')


def r4(v):
    return [round(float(x), 4) for x in v]


def main():
    a = args()
    os.makedirs(a['out'], exist_ok=True)
    tdir = os.path.join(a['out'], '.textures')
    os.makedirs(tdir, exist_ok=True)
    head, lenses = build_head(a, tdir)
    ped, faces = build_ped(a, tdir)
    for f in os.listdir(tdir):
        os.remove(os.path.join(tdir, f))
    os.rmdir(tdir)
    prov = {
        'version': 'street-signals-v1',
        'note': 'San Francisco signal heads: the geometry of two published models (sources), licensed CC BY 4.0, modified as each row says '
                '(cut to one head, rescaled, re-framed, a backplate added, rebaked and weathered). The photographs the sources carry as lens and face '
                'textures are not used (the pedestrian ones are Flickr photographs under CC BY-NC 2.0, the others have no record of who took them): '
                'the lens and face images are drawn by the script. Built by the script below in Blender; '
                'frames: glTF Y up; signalHead origin at the middle lens centre, lenses toward +Z; pedHead origin at the face centre, face toward +Z.',
        'script': rel(SCRIPT), 'scriptSha256': sha256(SCRIPT), 'blender': bpy.app.version_string,
        'sources': SOURCES,
        'attribution': [f'"{s["title"]}" by {s["author"]} ({s["url"]}), licensed CC BY 4.0 ({s["licenseUrl"]}); modified' for s in SOURCES.values()],
        'assets': {'signalHead': head, 'pedHead': ped},
        'images': {'signalLenses': lenses, 'pedFaces': faces},
    }
    with open(os.path.join(a['out'], 'street-signals-v1.provenance.json'), 'w') as f:
        json.dump(prov, f, indent=2)
        f.write('\n')
    print(json.dumps({k: (v['triangles'], v['bytes']) for k, v in prov['assets'].items()}))


if __name__ == '__main__':
    main()
