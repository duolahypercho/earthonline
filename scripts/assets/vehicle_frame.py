"""Vehicle builder frame and its conversion to the runtime frame.

THE AUTHORING FRAME IS MIRRORED. The vehicle builders (fleet-v1/v2,
project-sedan-v1, vehicle-v3) describe their frame as "X right, -Y nose, Z up"
and name every sided part from it: `Door_FL`, `Wheel_FL`, `Seat_FL` and the
steering wheel all sit at -X. That frame is left-handed. With the nose at -Y
and Z up, +X is the car's LEFT flank, so each of those parts landed on the
car's physical right and every car in the city was right-hand drive.

Everything a builder places is consistent inside its own frame, so the builders
keep authoring in it. `export_runtime_frame` converts at export by mirroring X on
the whole car: mesh data, every node transform, and the hinge descriptors in
node extras. After the mirror, the left-named parts are on the physical left
and the driver sits on the left, as in a US car. The glTF exporter's Y-up
conversion is untouched, so the car's nose is still +Z in the GLB and its left
flank is +X.

A converted GLB carries `vehicleFrame: FRAME` in its scene extras. Code that
re-imports a GLB into a builder (vehicle-v3 `carry_cabin`) reads the marker and
mirrors the parts back into the authoring frame first.
"""
import bpy
from mathutils import Matrix

FRAME = 'lhd-v1'
FRAME_KEY = 'vehicleFrame'
MIRROR_X = Matrix.Scale(-1.0, 4, (1.0, 0.0, 0.0))


def _mirror_hinge(ob):
    """A mirror reverses the sense of any rotation about an axis it flips.

    For the mirror S = diag(-1, 1, 1), S R(a, t) S = R(-S a, t). A named axis
    is a node-local axis, and conjugation keeps the node's local axes, so a
    turn about local X (the axis S flips) keeps its sense while a turn about
    local Y or Z reverses: that becomes a sign flip on `openSign`. A vector
    axis is rewritten as -S a so `openSign` is unchanged. Vector axes are
    already in glTF node axes (x, y, z) -> (x, z, -y) of Blender, where the
    mirrored axis is still x, so the same rule applies.
    """
    axis = ob.get('hingeAxis')
    if axis is None:
        return
    if isinstance(axis, str):
        if axis.upper() != 'X' and 'openSign' in ob:
            ob['openSign'] = -ob['openSign']
        return
    x, y, z = (float(v) for v in axis)
    ob['hingeAxis'] = [x, -y if y else 0.0, -z if z else 0.0]


def _mirror(objects):
    """Mirror X on `objects`; the mirror is its own inverse.

    Each object's transform is conjugated by the mirror (S M S), so every local
    frame stays right-handed and no node carries a negative scale into the GLB.
    Mesh data is mirrored once per datablock and its winding reversed so faces
    still point out.
    """
    objects = list(objects)
    meshes = {ob.data for ob in objects if ob.type == 'MESH' and ob.data is not None}
    for mesh in meshes:
        mesh.transform(MIRROR_X)
        mesh.flip_normals()
        mesh.update()
    for ob in objects:
        ob.matrix_parent_inverse = MIRROR_X @ ob.matrix_parent_inverse @ MIRROR_X
        ob.matrix_basis = MIRROR_X @ ob.matrix_basis @ MIRROR_X
        _mirror_hinge(ob)
    bpy.context.view_layer.update()


def export_runtime_frame(objects, export, scene=None):
    """Run `export()` with `objects` (a whole car) in the runtime frame.

    The car is mirrored, stamped, exported and mirrored back, so the builder's
    scene is still in the authoring frame afterwards: the next LOD, a saved
    .blend and any review render see exactly what they saw before.
    """
    scene = scene or bpy.context.scene
    objects = list(objects)
    _mirror(objects)
    scene[FRAME_KEY] = FRAME
    try:
        return export()
    finally:
        del scene[FRAME_KEY]
        _mirror(objects)


def carried_to_authoring_frame(objects, scene=None):
    """Bring parts imported from a vehicle GLB into the authoring frame.

    The glTF importer copies the file's scene extras onto the importing scene.
    A GLB stamped with FRAME is in the runtime frame, so its parts are mirrored
    back; an unstamped GLB predates the conversion and is already in the
    authoring frame. The stamp is removed so it cannot leak into this scene's
    own export. Returns the frame the parts arrived in.
    """
    scene = scene or bpy.context.scene
    arrived = scene.get(FRAME_KEY)
    if FRAME_KEY in scene:
        del scene[FRAME_KEY]
    if arrived == FRAME:
        _mirror(objects)
    elif arrived is not None:
        raise ValueError(f'unknown vehicle frame {arrived!r}')
    return arrived
