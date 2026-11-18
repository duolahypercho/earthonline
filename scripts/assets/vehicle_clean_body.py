"""Controlled sedan panels, apertures and glazing; metres, front -Y, up Z.

The original image design is interpreted as explicit surface profiles. No
generated-source triangles are used. All distance tiers sample the same
surfaces; opening panels keep their pivots at every tier.
"""
import argparse
import json
import math
import sys
from pathlib import Path

import bpy
import bmesh
from mathutils import Vector


CAPS=(65000,11000,3200)
FRONT=-2.27
REAR=2.27
AXLES=(-1.40,1.42)
ARCH_RADIUS=.40
DOOR_POLYGONS={
    'F':[(-.86,.285),(.195,.285),(.225,.978),(-.93,.978)],
    'R':[(.235,.285),(.78,.285),(1.04,.60),(1.19,.978),(.255,.978)],
}


def _mix(a,b,t):return a+(b-a)*t
def _smooth(t):return t*t*(3-2*t)


def _profile(keys,t):
    if t<=keys[0][0]:return keys[0][1]
    for (a,x),(b,y) in zip(keys,keys[1:]):
        if t<=b:return _mix(x,y,_smooth((t-a)/(b-a)))
    return keys[-1][1]


def shoulder_height(y):
    return _profile([(FRONT,.79),(-1.40,.94),(-.90,1.00),(.30,1.005),(1.25,1.01),(REAR,.94)],y)


def side_surface(y,z,side=1):
    """Outer side sheet, including restrained waist and wheel-arch flare."""
    width=_profile([(FRONT,.86),(-1.40,.909),(-.55,.892),(.55,.894),(1.42,.911),(REAR,.87)],y)
    h=shoulder_height(y);v=max(0,min(1,(z-.22)/(h-.22)))
    section=_profile([(0,-.047),(.08,-.018),(.20,-.028),(.47,-.021),(.70,.002),(.80,.003),(.88,-.004),(1,-.018)],v)
    # A slight rising scallop follows one full side rather than restarting on
    # every door. It stays a shallow 7mm depression under the shoulder.
    waist=-.007*math.exp(-((z-(.49+.022*y))/.11)**2)*math.exp(-(y/1.5)**6)
    flare=sum(.009*math.exp(-((math.hypot(y-a,z-.34)-.417)/.028)**2) for a in AXLES)
    return (side*(width+section+waist+flare),y,z)


def greenhouse_x(y,z):
    return .842-.495*(z-1.0)-.006*math.cos(y*1.4)


def roof_surface(u,v):
    y=_mix(-.18,.92,v);edge=1.39
    return (u*greenhouse_x(y,edge),y,edge+.055*(1-u*u)+.005*math.sin(math.pi*v)*(1-u*u))


def lid_surface(front,u,v):
    y=_mix(FRONT,-.90,v) if front else _mix(1.54,REAR,v)
    shoulder=shoulder_height(y);width=side_surface(y,shoulder)[0]
    crown=(.030 if front else .040)*(1-u*u)
    # Two shallow hood character ridges follow a coherent longitudinal line.
    ridge=(.009*math.exp(-((abs(u)-.70)/.09)**2)*math.sin(math.pi*v)) if front else 0
    return (u*width,y,shoulder+crown+ridge)


def end_profile(front=True,side_steps=8,top_steps=16):
    """Closed perimeter from right sill up/over top to left sill and underbody."""
    y=FRONT if front else REAR;h=shoulder_height(y)
    right=[side_surface(y,_mix(.22,h,i/side_steps)) for i in range(side_steps)]
    top=[lid_surface(front,_mix(1,-1,i/top_steps),0 if front else 1) for i in range(top_steps+1)]
    left=[side_surface(y,_mix(h,.22,i/side_steps),-1) for i in range(1,side_steps+1)]
    return [list(p) for p in right+top+left]


def _material(name,color,metallic=0,roughness=.4):
    material=bpy.data.materials.get(name)
    if material:return material
    material=bpy.data.materials.new(name);material.use_nodes=True
    bsdf=material.node_tree.nodes.get('Principled BSDF')
    bsdf.inputs['Base Color'].default_value=(*color,1)
    bsdf.inputs['Metallic'].default_value=metallic;bsdf.inputs['Roughness'].default_value=roughness
    if name=='VehiclePaint':
        bsdf.inputs['Coat Weight'].default_value=.7;bsdf.inputs['Coat Roughness'].default_value=.12
    if name=='VehicleGlass':
        bsdf.inputs['Transmission Weight'].default_value=.22;bsdf.inputs['IOR'].default_value=1.45
    return material


def _parent(ob,parent):
    if parent:
        bpy.context.view_layer.update()
        world=ob.matrix_world.copy();ob.parent=parent
        ob.matrix_parent_inverse=parent.matrix_world.inverted();ob.matrix_basis=world


def _empty(name,location=(0,0,0)):
    ob=bpy.data.objects.new(name,None);bpy.context.scene.collection.objects.link(ob);ob.location=location
    return ob


def _mesh(name,vertices,faces,material,parent=None,normal=None):
    mesh=bpy.data.meshes.new(name);mesh.from_pydata(vertices,[],faces);mesh.materials.append(material)
    ob=bpy.data.objects.new(name,mesh);bpy.context.scene.collection.objects.link(ob)
    bm=bmesh.new();bm.from_mesh(mesh)
    bmesh.ops.remove_doubles(bm,verts=list(bm.verts),dist=1e-7)
    bmesh.ops.dissolve_degenerate(bm,dist=1e-9,edges=list(bm.edges))
    bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(mesh);bm.free();mesh.update()
    for p in mesh.polygons:p.use_smooth=True
    if normal:
        normals=[normal(v.co).normalized() for v in mesh.vertices]
        if mesh.polygons and sum(mesh.polygons[0].normal[i]*normals[mesh.polygons[0].vertices[0]][i] for i in range(3))<0:mesh.flip_normals()
        mesh.normals_split_custom_set_from_vertices(normals)
    uv=mesh.uv_layers.new(name='UVMap')
    for loop in mesh.loops:
        p=mesh.vertices[loop.vertex_index].co;uv.data[loop.index].uv=(p.y/4.85+.5,p.z/1.45)
    _parent(ob,parent);return ob


def _clip(poly,a,b,inside=True,offset=0):
    """Clip a convex (y,z) polygon against a directed edge's half plane."""
    if not poly:return []
    dx=b[0]-a[0];dy=b[1]-a[1];length=math.hypot(dx,dy)
    def distance(p):return dx*(p[1]-a[1])-dy*(p[0]-a[0])-offset*length
    result=[]
    for p,q in zip(poly,poly[1:]+poly[:1]):
        dp=distance(p);dq=distance(q);ip=dp>=-1e-10 if inside else dp<=1e-10;iq=dq>=-1e-10 if inside else dq<=1e-10
        if ip:result.append(p)
        if ip!=iq:
            t=dp/(dp-dq);result.append((_mix(p[0],q[0],t),_mix(p[1],q[1],t)))
    return result


def _intersection(poly,boundary,inset=0):
    for a,b in zip(boundary,boundary[1:]+boundary[:1]):poly=_clip(poly,a,b,True,inset)
    return poly


def _difference(poly,boundary):
    # Convex clipping partitions the outside without crossing aperture edges.
    pieces=[]
    for a,b in zip(boundary,boundary[1:]+boundary[:1]):
        outside=_clip(poly,a,b,False)
        if len(outside)>=3:pieces.append(outside)
        poly=_clip(poly,a,b,True)
        if len(poly)<3:break
    return pieces


def _rounded(poly,radius=.025,steps=4):
    result=[]
    for i,p in enumerate(poly):
        prev=poly[i-1];nxt=poly[(i+1)%len(poly)]
        a=tuple(_mix(p[k],prev[k],min(.20,radius/math.dist(p,prev))) for k in (0,1))
        b=tuple(_mix(p[k],nxt[k],min(.20,radius/math.dist(p,nxt))) for k in (0,1))
        for j in range(steps+1):
            t=j/steps;result.append(tuple((1-t)**2*a[k]+2*(1-t)*t*p[k]+t*t*b[k] for k in (0,1)))
    return result


def _inside(p,poly):
    return all((b[0]-a[0])*(p[1]-a[1])-(b[1]-a[1])*(p[0]-a[0])>=-1e-8 for a,b in zip(poly,poly[1:]+poly[:1]))


def _surface_mesh(name,polys,sampler,material,parent=None,normal=None):
    vertices=[];faces=[]
    for poly in polys:
        if len(poly)<3:continue
        area=sum(p[0]*q[1]-p[1]*q[0] for p,q in zip(poly,poly[1:]+poly[:1]))*.5
        if abs(area)<1e-10:continue
        start=len(vertices);vertices.extend(sampler(*p) for p in poly);faces.append(tuple(range(start,start+len(poly))))
    return _mesh(name,vertices,faces,material,parent,normal)


def _grid(ys,zs):
    return [[(a,c),(b,c),(b,d),(a,d)] for a,b in zip(ys,ys[1:]) for c,d in zip(zs,zs[1:])]


def _ring(name,outer,inner,sampler,material,parent=None):
    if len(outer)!=len(inner):raise ValueError('Ring contour mismatch')
    n=len(outer);v=[sampler(*p) for p in outer]+[sampler(*p) for p in inner]
    return _mesh(name,v,[(i,(i+1)%n,(i+1)%n+n,i+n) for i in range(n)],material,parent)


def _inset(poly,amount):
    # Offset each convex edge, intersect adjacent shifted lines. Curved sampled
    # contours keep identical vertex correspondence for rims and shutlines.
    result=[]
    for i,p in enumerate(poly):
        a=Vector(poly[i-1]);b=Vector(p);c=Vector(poly[(i+1)%len(poly)])
        d=(b-a).normalized();e=(c-b).normalized();n=Vector((-d.y,d.x));m=Vector((-e.y,e.x))
        bis=n+m;denom=bis.dot(n)
        result.append(tuple(b+bis*(amount/max(1e-8,denom))))
    return result


def _patch(name,surface,u_values,v_values,material,parent=None):
    vertices=[surface(u,v) for v in v_values for u in u_values];n=len(u_values)
    faces=[(j*n+i,j*n+i+1,(j+1)*n+i+1,(j+1)*n+i) for j in range(len(v_values)-1) for i in range(n-1)]
    return _mesh(name,vertices,faces,material,parent)


def _series(a,b,count):return [_mix(a,b,i/count) for i in range(count+1)]


def build_clean_body(lod=0):
    if lod not in (0,1,2):raise ValueError('lod must be 0,1,2')
    if any(bpy.data.objects.get(n) for n in ('Body','Panel_FL','Roof')):raise ValueError('Build into a fresh assembly; existing body names would collide')
    paint=_material('VehiclePaint',(.018,.12,.36),.82,.25)
    glass=_material('VehicleGlass',(.026,.041,.055),.08,.13)
    rubber=_material('VehicleTrim',(.011,.014,.018),.03,.4)
    chrome=_material('VehicleChrome',(.40,.47,.54),.92,.20)
    inside=_material('VehicleInterior',(.025,.028,.034),0,.65)
    y_count,z_count,arc_count,corner_steps=(64,24,48,4) if lod==0 else (32,12,24,3) if lod==1 else (12,4,16,1)
    body_polys=[];panels={'F':[],'R':[]}
    doors={k:_rounded(p,.025,corner_steps) for k,p in DOOR_POLYGONS.items()}
    door_inner={k:_inset(p,.0035) for k,p in doors.items()}
    arches=[[(a+ARCH_RADIUS*math.cos(t),.34+ARCH_RADIUS*math.sin(t)) for t in _series(0,math.tau,arc_count)[:-1]] for a in AXLES]
    ys=sorted(set(_series(FRONT,REAR,y_count)+[FRONT,-1.80,-1.00,-.90,.22,1.02,1.82,REAR]))
    zs=sorted(set(_series(.22,1.015,z_count)+[.285,.34,.978,1.0]))
    # Each grid cell is partitioned by explicit convex apertures; the broad
    # regions remain quads and arch/shutline vertices belong to actual cuts.
    for cell in _grid(ys,zs):
        ymid=(cell[0][0]+cell[1][0])*.5
        # Piecewise shoulder follows the same analytic function as upper lids.
        top=[(cell[0][0],.0),(cell[1][0],.0),(cell[1][0],shoulder_height(cell[1][0])),(cell[0][0],shoulder_height(cell[0][0]))]
        cell=_intersection(cell,top)
        pieces=[cell] if len(cell)>=3 else []
        for arch in arches:
            if abs(ymid-sum(p[0] for p in arch)/len(arch))>ARCH_RADIUS+(REAR-FRONT)/y_count:continue
            pieces=[q for p in pieces for q in _difference(p,arch)]
        for piece in pieces:
            for key,outline in door_inner.items():
                cut=_intersection(piece,outline)
                if len(cut)>=3:panels[key].append(cut)
            rest=[piece]
            for outline in doors.values():rest=[q for p in rest for q in _difference(p,outline)]
            body_polys.extend(rest)
    objects=[]
    def side_normal(side):
        def normal(p):
            e=1e-4;dxdy=(side_surface(p.y+e,p.z)[0]-side_surface(p.y-e,p.z)[0])/(2*e);dxdz=(side_surface(p.y,p.z+e)[0]-side_surface(p.y,p.z-e)[0])/(2*e)
            return Vector((side,-dxdy,-dxdz))
        return normal
    for side,letter in ((-1,'L'),(1,'R')):
        sampler=lambda y,z,s=side:side_surface(y,z,s)
        objects.append(_surface_mesh('BodySide_'+letter,body_polys,sampler,paint,normal=side_normal(side)))
        for which in ('F','R'):
            key=which+letter;hinge=(-.90 if which=='F' else .22)
            door=_empty('Door_'+key,(side*.86,hinge,.72));door['hingeAxis']='Z';door['openingSign']=1 if side<0 else -1
            panel=_surface_mesh('Panel_'+key,panels[which],sampler,paint,door,side_normal(side));objects.append(panel)
            # Exposed return attaches the visible door skin to a real inner
            # edge, and a fixed dark jamb bounds the open passenger aperture.
            contour=door_inner[which];outer=[sampler(*p) for p in contour];inner=[(p[0]-side*.024,p[1],p[2]) for p in outer];n=len(outer)
            _mesh('DoorRim_'+key,outer+inner,[(i,(i+1)%n,(i+1)%n+n,i+n) for i in range(n)],paint,door)
            jamb=doors[which];v=[sampler(*p) for p in jamb];v+= [(p[0]-side*.065,p[1],p[2]) for p in v];n=len(jamb)
            _mesh('DoorJamb_'+key,v,[(i,(i+1)%n,(i+1)%n+n,i+n) for i in range(n)],rubber)
        for index,axle in enumerate(AXLES):
            angles=_series(0,math.pi,arc_count//2)
            outer=[side_surface(axle+ARCH_RADIUS*math.cos(t),.34+ARCH_RADIUS*math.sin(t),side) for t in angles]
            inner=[(side*.64,p[1],p[2]) for p in outer];n=len(outer)
            _mesh(f'WheelWell_{letter}{index}',outer+inner,[(i,i+1,i+1+n,i+n) for i in range(n-1)],inside)
    # A named static Body groups the two side skins without merging moving parts.
    body=_empty('Body')
    for ob in objects:
        if ob.name.startswith('BodySide_'):_parent(ob,body)
    resolution=(32,16) if lod==0 else (16,8) if lod==1 else (8,3)
    for front,name in ((True,'Hood'),(False,'Trunk')):
        pivot=_empty(name,(0,-.90 if front else 1.54,1.0));pivot['hingeAxis']='X'
        # Surface-parametric inset provides a visible 3.5mm shutline without a
        # second opaque lid below the hinged one.
        u=_series(-.994,.994,resolution[0]);v=_series(.0035,.9965,resolution[1])
        _patch(name+'Skin',lambda a,b,f=front:lid_surface(f,a,b),u,v,paint,pivot)
        _patch(name+'Inner',lambda a,b,f=front:tuple(x-(.014 if i==2 else 0) for i,x in enumerate(lid_surface(f,a,b))),u,v,inside,pivot)
    _patch('Roof',roof_surface,_series(-1,1,resolution[0]),_series(0,1,resolution[1]),paint)
    _patch('RoofInner',lambda u,v:(roof_surface(u,v)[0],roof_surface(u,v)[1],roof_surface(u,v)[2]-.018),_series(-1,1,resolution[0]//2),_series(0,1,max(2,resolution[1]//2)),inside)
    # Main panes and their surrounds use the SAME two-dimensional surface;
    # there are no independently fitted skirts or floating replacement glass.
    for front,name in ((True,'Windscreen'),(False,'RearWindow')):
        a,b=(-.90,-.18) if front else (.92,1.54)
        za,zb=(1.0,1.39) if front else (1.39,shoulder_height(1.54)+.015)
        def wind(u,v,a=a,b=b,za=za,zb=zb):
            y=_mix(a,b,v);z=_mix(za,zb,v)+.014*math.sin(math.pi*v)*(1-u*u)
            return (u*greenhouse_x(y,z),y,z+.01*(1-u*u))
        u=_series(-.954,.954,resolution[0]);v=_series(.045,.950,resolution[1])
        _patch(name,wind,u,v,glass)
        for label,ua,ub,va,vb in [('Left',-1,-.954,0,1),('Right',.954,1,0,1),('Bottom',-.954,.954,0,.045),('Top',-.954,.954,.950,1)]:
            _patch(name+'_Surround'+label,wind,_series(ua,ub,2 if abs(ub-ua)<.1 else resolution[0]),_series(va,vb,2 if abs(vb-va)<.1 else resolution[1]),paint)
    # Shared side greenhouse outline, fixed A/B/C pillars and separately moving panes.
    greenhouse=[(-.90,.988),(1.54,.988),(1.54,1.035),(.92,1.39),(-.18,1.39)]
    windows={
        'F':_rounded([(-.80,1.016),(.198,1.016),(.182,1.353),(-.173,1.353)],.018,corner_steps),
        'R':_rounded([(.270,1.016),(1.076,1.016),(.821,1.346),(.270,1.353)],.016,corner_steps),
        'Quarter':_rounded([(1.13,1.033),(1.395,1.045),(.914,1.321)],.014,corner_steps),
    }
    green_grid=_grid(_series(-.90,1.54,y_count//2),_series(.988,1.39,max(3,z_count//2)))
    green_polys=[];window_polys={k:[] for k in windows}
    for cell in green_grid:
        cell=_intersection(cell,greenhouse)
        if len(cell)<3:continue
        for key,p in windows.items():
            cut=_intersection(cell,p)
            if len(cut)>=3:window_polys[key].append(cut)
        rest=[cell]
        for p in windows.values():rest=[q for piece in rest for q in _difference(piece,p)]
        green_polys.extend(rest)
    for side,letter in ((-1,'L'),(1,'R')):
        sampler=lambda y,z,s=side:(s*greenhouse_x(y,z),y,z)
        normal=lambda p,s=side:Vector((s,.0,.495))
        _surface_mesh('Greenhouse_'+letter,green_polys,sampler,paint,normal=normal)
        for key,polys in window_polys.items():
            parent=bpy.data.objects.get('Door_'+key+letter) if key in ('F','R') else None
            name='Window_'+key+letter if key in ('F','R') else 'QuarterGlass_'+letter
            _surface_mesh(name,polys,sampler,glass,parent,normal)
            _ring(name+'_Seal',_inset(windows[key],-.010),windows[key],sampler,rubber,parent)
            if key in ('F','R'):
                _ring('DoorFrame_'+key+letter,_inset(windows[key],-.016),_inset(windows[key],-.010),sampler,chrome,parent)
    # Sill return and underside terminate at the fascia contract planes.
    _patch('Underbody',lambda u,v:(u*.74,_mix(FRONT,REAR,v),.205),[-1,1],[0,1],inside)
    triangles=0
    for ob in bpy.context.scene.objects:
        if ob.type=='MESH':ob.data.calc_loop_triangles();triangles+=len(ob.data.loop_triangles)
    if triangles>CAPS[lod]:raise ValueError(f'Clean body LOD{lod} exceeds {CAPS[lod]}: {triangles}')
    return {'version':'clean-body-v1','lod':lod,'triangles':triangles,'cap':CAPS[lod],
            'frontProfile':end_profile(True),'rearProfile':end_profile(False),'archRadius':ARCH_RADIUS,
            'archSegments':arc_count//2,'doorOutlines':DOOR_POLYGONS,
            'sourceTopology':'none; analytic surface reconstruction','visualStatus':'candidate; exact-export review required'}


if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--output',required=True);parser.add_argument('--lod',type=int,default=0)
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:]);output=Path(args.output);output.mkdir(parents=True,exist_ok=True)
    bpy.ops.wm.read_factory_settings(use_empty=True);bpy.context.scene.world=bpy.data.worlds.new('World')
    result=build_clean_body(args.lod)
    bpy.ops.wm.save_as_mainfile(filepath=str(output/'clean-body.blend'))
    bpy.ops.export_scene.gltf(filepath=str(output/'clean-body.glb'),export_format='GLB',export_apply=True,export_yup=True,export_extras=True)
    (output/'body-report.json').write_text(json.dumps(result,indent=2)+'\n')
