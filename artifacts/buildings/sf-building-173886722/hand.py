# Bay Arms: the shop's fit-out is mounted at runtime (src/world/buildings/authored/shops).
# The between-tenants dressing (left-behind boxes, a work lamp) would stand in its aisles.
b._vacant = lambda rx0, rx1, kz, z1, top: None

# The fit-out hangs a real pair of doors that swing as you reach them. The shell's own
# leaves stand permanently open inward against the jambs, so they come out here: the
# frame, the head and the opening stay exactly as built.
import bmesh

DOOR_X0, DOOR_X1 = -1.886, -0.55
LEAF_REACH_M = 0.12


def _in_leaf(co):
    depth = abs(co.y)
    if not 23.1 <= depth <= 24.475 or co.z > 2.46:
        return False
    return DOOR_X0 - 0.02 <= co.x <= DOOR_X0 + LEAF_REACH_M or DOOR_X1 - LEAF_REACH_M <= co.x <= DOOR_X1 + 0.02


_removed = 0
for _obj in list(b.exterior.all_objects):
    if _obj.type != 'MESH' or not _obj.name.startswith(('storefront', 'glazing')):
        continue
    _mesh = bmesh.new()
    _mesh.from_mesh(_obj.data)
    _faces = [face for face in _mesh.faces if all(_in_leaf(vert.co) for vert in face.verts)]
    if _faces:
        _removed += len(_faces)
        bmesh.ops.delete(_mesh, geom=_faces, context='FACES')
        _mesh.to_mesh(_obj.data)
    _mesh.free()
if _removed == 0:
    raise RuntimeError('hand.py: no door leaf faces found to remove; the shell or this frame changed')
print(f'BAY_ARMS_DOOR_LEAVES_REMOVED faces={_removed}')
