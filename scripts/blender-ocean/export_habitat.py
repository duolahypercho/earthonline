"""Cook the original SF habitat study into small, instanced, Y-up mesh prototypes.

blender -b "/path/to/Aquatic Park habitats.blend" --python-exit-code 1 --python scripts/blender-ocean/export_habitat.py
"""
import hashlib
import json
import math
import struct
from pathlib import Path

import bpy
from mathutils import Vector, noise

root = Path(__file__).resolve().parents[2]
out = root / "public/assets/ocean"
out.mkdir(parents=True, exist_ok=True)
source = Path(bpy.data.filepath)
scene = bpy.context.scene
scene.frame_set(1)
gltf = dict(asset=dict(version="2.0", generator="Earth Online habitat cook"),
            scenes=[dict(nodes=[])], scene=0, nodes=[], meshes=[], bufferViews=[], accessors=[])
blob = bytearray()


def accessor(values, component, kind, fmt, target, bounds=False, normalized=False):
    while len(blob) % 4:
        blob.append(0)
    start = len(blob)
    for value in values:
        blob.extend(struct.pack("<" + fmt, *value))
    view = len(gltf["bufferViews"])
    gltf["bufferViews"].append(dict(buffer=0, byteOffset=start, byteLength=len(blob)-start, target=target))
    acc = dict(bufferView=view, componentType=component, count=len(values), type=kind)
    if bounds:
        acc.update(min=list(map(min, zip(*values))), max=list(map(max, zip(*values))))
    if normalized:
        acc["normalized"] = True
    gltf["accessors"].append(acc)
    return len(gltf["accessors"]) - 1


def write_mesh(name, coords, faces, colors, target):
    mesh = bpy.data.meshes.new(name)
    mesh.from_pydata(coords, [], faces)
    mesh.update()
    mesh.polygons.foreach_set("use_smooth", [True] * len(mesh.polygons))
    color = mesh.color_attributes.new(name="Pigment", type="FLOAT_COLOR", domain="POINT")
    color.data.foreach_set("color", [x for rgb in colors for x in (*rgb, 1.)])
    ob = bpy.data.objects.new(name, mesh)
    scene.collection.objects.link(ob)
    mesh.calc_loop_triangles()
    if target and len(mesh.loop_triangles)>target:
        mod = ob.modifiers.new("Game silhouette", "DECIMATE")
        mod.ratio = target / len(mesh.loop_triangles)
    ev = ob.evaluated_get(bpy.context.evaluated_depsgraph_get())
    geom = ev.to_mesh()
    geom.calc_loop_triangles()
    pig = geom.color_attributes["Pigment"]
    vertices = [(v.co.x, v.co.z, -v.co.y) for v in geom.vertices]
    normals = [(v.normal.x, v.normal.z, -v.normal.y) for v in geom.vertices]
    rgba = [tuple(round(255 * min(1., max(0., c))) for c in entry.color) for entry in pig.data]
    indices = [(idx,) for tri in geom.loop_triangles for idx in tri.vertices]
    pos = accessor(vertices, 5126, "VEC3", "3f", 34962, bounds=True)
    norm = accessor(normals, 5126, "VEC3", "3f", 34962)
    col = accessor(rgba, 5121, "VEC4", "4B", 34962, normalized=True)
    idx = accessor(indices, 5123, "SCALAR", "H", 34963)
    gltf["meshes"].append(dict(name=name, primitives=[dict(attributes=dict(POSITION=pos,NORMAL=norm,COLOR_0=col),indices=idx,mode=4)]))
    gltf["scenes"][0]["nodes"].append(len(gltf["nodes"]))
    gltf["nodes"].append(dict(mesh=len(gltf["meshes"])-1, name=name))
    print(name, len(vertices), len(indices)//3)
    ev.to_mesh_clear()
    bpy.data.objects.remove(ob, do_unlink=True)
    bpy.data.meshes.remove(mesh)


def sample_ribbons(points, colors, polygons, kind):
    """Follow the study's body rings, fin rays and leaf ribbons at a coarser spacing."""
    parents = list(range(len(points)))
    def root(i):
        while parents[i] != i:
            parents[i] = parents[parents[i]]
            i = parents[i]
        return i
    for face in polygons:
        owner = root(face[0])
        for i in face[1:]:
            parents[root(i)] = owner
    islands = {}
    for face in polygons:
        islands.setdefault(root(face[0]), []).append(face)
    out, pigment, faces = [], [], []
    def stations(count, keep, closed=False):
        n = min(keep,count)
        if closed:
            return [round(i*count/n) % count for i in range(n)]
        return [round(i*(count-1)/(n-1)) for i in range(n)]
    for island in islands.values():
        ids = sorted({i for face in island for i in face})
        first = min(ids)
        quad = next((f for f in island if len(f)==4), None)
        if not quad: continue
        across = max(quad)-min(quad)-1
        strips = len(ids)//across
        if across<2 or strips<2 or len(ids)%across or ids!=list(range(first,first+len(ids))):
            raise ValueError(f"unexpected {kind} surface layout at vertex {first}")
        closed = any(first in f and first+across-1 in f and len(f)==4 for f in island)
        if kind in ("perch","topsmelt","sculpin"):
            if first==0: along,around=21,14
            elif across==22: along,around=5,8  # stacked iris, pupil and cornea
            elif closed: along,around=7,4        # jaw and gill folds
            else: along,around=9,3              # translucent fin membranes
        elif kind=="eelgrass": along,around=11,3
        elif kind=="red-algae": along,around=6,5
        elif kind=="rockweed": along,around=7,5
        else: along,around=12,8
        rows=stations(strips,along)
        cols=stations(across,around,closed)
        start=len(out); width=len(cols)
        for r in rows:
            for c in cols:
                idx=first+r*across+c
                out.append(points[idx]); pigment.append(colors[idx])
        for r in range(len(rows)-1):
            for c in range(width if closed else width-1):
                a=start+r*width+c
                b=start+r*width+(c+1)%width
                face=(a,b,b+width,a+width)
                faces.append(face if quad[1]-quad[0]==1 else face[::-1])
        if closed:
            for end in (0,len(rows)-1):
                cap=tuple(start+end*width+c for c in range(width))
                faces.append(cap if end else cap[::-1])
    return out,faces,pigment


def prototype(name, prefix, limit, budget, color, normalize=False):
    ob = next(o for o in scene.objects if o.name.startswith(prefix))
    m = ob.data
    count = min(limit or len(m.vertices), len(m.vertices))
    pts = [v.co.copy() for v in m.vertices[:count]]
    faces = [tuple(p.vertices) for p in m.polygons if max(p.vertices)<count]
    attr = m.color_attributes.get("Tissue pigment")
    colors = [tuple(attr.data[i].color)[:3] if attr else color for i in range(count)]
    if normalize:
        # The study collects each shoot's ribbons into one world-space mesh.
        # Keep a complete clump. Put its holdfast at local zero for terrain placement.
        base = pts[0].copy()
        for pt in pts:
            pt -= base
        height = max(p.z for p in pts)
        for pt in pts:
            pt /= max(.01, height)
    if name.startswith("stone"):
        colors = [tuple(t*(.80+.40*noise.noise(p*3.3)+.17*noise.noise(p*14.1)) for t in color) for p in pts]
    if name=="shell":
        centre=sum(pts,Vector())/len(pts)
        radius=max((p-centre).length for p in pts)
        pts=[(p-centre)/radius for p in pts]
    if name in ("perch","topsmelt","sculpin","eelgrass","sea-lettuce","red-algae","rockweed"):
        pts,faces,colors=sample_ribbons(pts,colors,faces,name)
        budget=None
    write_mesh(name, pts, faces, colors, budget)


for name, prefix in [("perch","Shiner surfperch • 01"),("topsmelt","Topsmelt • 01"),("sculpin","Pacific staghorn sculpin • 01")]:
    prototype(name,prefix,None,850,(.27,.31,.22))
prototype("eelgrass","Eelgrass • ribbon-leaved shoots",5*48,195,(.13,.19,.047),True)
prototype("sea-lettuce","Sea lettuce • ruffled Ulva sheets",300,255,(.16,.26,.04),True)
prototype("red-algae","Red seaweed • branched Gracilaria and drift",735,425,(.15,.049,.033),True)
prototype("rockweed","Rockweed colonies • forked blades",378,355,(.148,.126,.034),True)
for name,prefix,tint in [("stone-ledge","Split ledge",(.14,.135,.102)),("stone-block","Fractured block",(.105,.135,.116)),("stone-cobble","Water-worn cobble",(.21,.167,.129))]:
    prototype(name,prefix,None,180,tint)
prototype("shell","Silt and shell lag",75,110,(.43,.38,.247))
# A normalized bull-kelp stipe, float and the divided crown at its tip.
# This original Pacific-coast prototype complements the cove's short algae.
def bull_kelp():
    verts, faces, colors = [], [], []
    def rows(point, along, across, closed, color):
        first=len(verts)
        for j in range(along):
            for i in range(across):
                verts.append(point(j/(along-1), i/(across if closed else across-1)))
                colors.append(color)
                if j>0 and (closed or i<across-1):
                    a=first+(j-1)*across+i
                    b=first+(j-1)*across+(i+1)%across
                    faces.append((a,b,b+across,a+across))
    def centre(t): return Vector((.08*t*t,.021*math.sin(3*t),t))
    def stipe(t,u):
        radius=.004+.014*math.exp(-((t-.985)/.019)**2)
        return centre(t)+Vector((radius*math.cos(2*math.pi*u),radius*math.sin(2*math.pi*u),0))
    rows(stipe,26,8,True,(.17,.153,.054))
    for k in range(12):
        az=k*2.39996
        d=Vector((math.cos(az),math.sin(az),0))
        side=Vector((-d.y,d.x,0))
        def blade(t,u):
            width=.011*(.04+math.sin(math.pi*t)**.66)
            return centre(.996)+d*(.17*t)+Vector((.18*t*t,0,.045*math.sin(math.pi*t)-.095*t*t))+side*((2*u-1)*width+.013*math.sin(t*7+k)*t)
        rows(blade,11,3,False,(.18+.018*(k%3),.146+.011*(k%3),.038))
    write_mesh("bull-kelp",verts,faces,colors,None)

def sanddab():
    # Horizontal flatfish, both eyes on the pigmented upper side, fin rays along both margins.
    verts, faces, colors = [], [], []
    def body(center, radii, tint, length=16, around=14):
        start = len(verts)
        for j in range(length+1):
            t = j/length
            radial = .001+math.sin(t*math.pi)**.79
            for k in range(around):
                theta = 2*math.pi*k/around
                x = center[0]+(2*t-1)*radii[0]
                y = center[1]+radii[1]*radial*math.cos(theta)
                z = center[2]+radii[2]*radial*math.sin(theta)
                verts.append((x,y,z))
                grain = .83+.27*noise.noise(Vector((x,y,z))*43)+.14*math.sin(x*91+y*36)
                colors.append(tuple(c*grain for c in tint))
                if j:
                    a = start+(j-1)*around+k
                    b = start+(j-1)*around+(k+1)%around
                    faces.append((a,b,b+around,a+around))
    body((.07,0,.022),(.43,.164,.042),(.23,.202,.137))
    body((-.407,0,.014),(.093,.082,.014),(.19,.149,.088),10,14)
    for sign in [-1,1]:
        for j in range(20):
            t=(j+.5)/20
            x=-.31+.68*t
            w=.155*(.18+math.sin(math.pi*t)**.69)
            a=len(verts)
            verts.extend([(x-.024,sign*w*.7,.018),(x+.02,sign*w*.7,.018),(x-.019,sign*(w+.022),.005)])
            colors.extend([(.16,.14,.08),(.22,.20,.12),(.25,.21,.13)])
            faces.append((a,a+1,a+2) if sign>0 else (a+2,a+1,a))
    for x,y in [(.35,.036),(.308,-.046)]:
        body((x,y,.059),(.021,.020,.010),(.24,.194,.101),7,10)
        body((x+.002,y,.066),(.011,.010,.006),(.019,.017,.010),6,10)
    write_mesh("sanddab",verts,faces,colors,None)

def sea_pen():
    # The tapered rachis carries paired ranks of polyp leaves; the peduncle is embedded in mud.
    verts, faces, colors = [], [], []
    for j in range(21):
        t=j/20
        radius=.026*(.12+math.sin(math.pi*t)**.6)
        for k in range(7):
            a=2*math.pi*k/7
            verts.append((.075*t*t+radius*math.cos(a),radius*math.sin(a),t))
            colors.append((.37+.24*t,.20+.11*t,.12+.06*t))
            if j:
                i=(j-1)*7+k
                n=(j-1)*7+(k+1)%7
                faces.append((i,n,n+7,i+7))
    for j in range(22):
        t=.19+.77*j/22
        extent=.18*math.sin((t-.15)/.85*math.pi)**.7
        for sign in [-1,1]:
            for k in range(4):
                a=len(verts)
                reach=extent*(.71+.09*k)
                verts.extend([(.075*t*t,0,t),(.075*t*t,0,t+.035),
                    (.075*t*t+sign*reach,.017*(k-1.5),t+.075+.012*k),
                    (.075*t*t+sign*reach*.78,.022*(k-1.5),t+.043)])
                colors.extend([(.46,.23,.14),(.58,.32,.17),(.65,.39,.22),(.58,.31,.16)])
                faces.append((a,a+1,a+2,a+3))
    write_mesh("sea-pen",verts,faces,colors,None)

bull_kelp()
sanddab()
sea_pen()
while len(blob)%4: blob.append(0)
gltf["buffers"]=[dict(byteLength=len(blob))]
header=json.dumps(gltf,separators=(",",":"),allow_nan=False).encode()
header+=b" "*(-len(header)%4)
glb=struct.pack("<4sII",b"glTF",2,28+len(blob)+len(header))+struct.pack("<I4s",len(header),b"JSON")+header+struct.pack("<I4s",len(blob),b"BIN\0")+blob
asset=out/"sf-habitat.glb"
asset.write_bytes(glb)
(out/"sf-habitat.provenance.json").write_text(json.dumps(dict(
    url="/assets/ocean/sf-habitat.glb",bytes=len(glb),sha256=hashlib.sha256(glb).hexdigest(),
    source="artifacts/ocean/sf-habitats.blend",sourceSha256=hashlib.sha256(source.read_bytes()).hexdigest(),
    author="Earth Online contributors",license="Project license (LICENSE)",builder="scripts/blender-ocean/export_habitat.py",
    blender=bpy.app.version_string,description="Original aquatic animal, plant, shell and stone geometry and vertex pigments. No third-party texture or geometry is embedded."
),indent=2)+"\n")
