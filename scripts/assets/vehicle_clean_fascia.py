"""Authored fascia, recessed lamps and grille for the multiview sedan design."""
import math
import bpy
from mathutils import Vector
from mathutils.geometry import delaunay_2d_cdt


def material(name, color, roughness=.3, metal=0, alpha=1):
    mat=bpy.data.materials.get(name)
    if mat:return mat
    mat=bpy.data.materials.new(name);mat.use_nodes=True
    shader=mat.node_tree.nodes.get('Principled BSDF')
    shader.inputs['Base Color'].default_value=(*color,1)
    shader.inputs['Roughness'].default_value=roughness
    shader.inputs['Metallic'].default_value=metal
    shader.inputs['Alpha'].default_value=alpha
    if alpha<1:
        shader.inputs['Transmission Weight'].default_value=.65
        shader.inputs['IOR'].default_value=1.46
        mat.surface_render_method='DITHERED'
    mat.diffuse_color=(*color,alpha)
    return mat


def mesh(name, verts, faces, mat, smooth=True):
    data=bpy.data.meshes.new(name);data.from_pydata(verts,[],faces);data.update()
    data.materials.append(mat)
    for face in data.polygons:face.use_smooth=smooth
    obj=bpy.data.objects.new(name,data);bpy.context.scene.collection.objects.link(obj)
    return obj


def rounded_polygon(points, count=4, rounding=.13):
    result=[]
    for i,p in enumerate(points):
        p=Vector(p);a=p.lerp(Vector(points[i-1]),rounding)
        b=p.lerp(Vector(points[(i+1)%len(points)]),rounding)
        for j in range(count+1):
            t=j/count
            result.append(a*(1-t)**2+p*(2*t*(1-t))+b*t*t)
    return result


def inside(point, polygon):
    x,y=point;answer=False
    for a,b in zip(polygon,polygon[1:]+polygon[:1]):
        if (a[1]>y)!=(b[1]>y) and x<(b[0]-a[0])*(y-a[1])/(b[1]-a[1])+a[0]:answer=not answer
    return answer


def face_point(front, x, z, depth=0):
    sign=-1 if front else 1
    # Broad convex fascia with a subtle tucked-under valance.
    y=sign*(2.425-.10*(abs(x)/.90)**3-.017*((z-.57)/.40)**2-depth)
    return Vector((x,y,z))


def _panel(name, front, outline, holes, mat, spacing):
    points=[];edges=[]
    for loop in [outline]+holes:
        offset=len(points);points.extend(Vector(p) for p in loop)
        edges.extend((offset+i,offset+(i+1)%len(loop)) for i in range(len(loop)))
    z=.23
    while z<1:
        x=-.86
        while x<.87:
            p=Vector((x,z))
            if inside(p,outline) and not any(inside(p,h) for h in holes):points.append(p)
            x+=spacing
        z+=spacing
    vertices,_,triangles,_,_,_=delaunay_2d_cdt(points,edges,[],0,1e-7,False)
    faces=[]
    for tri in triangles:
        center=sum((vertices[i] for i in tri),Vector((0,0)))/len(tri)
        if inside(center,outline) and not any(inside(center,h) for h in holes):
            faces.append(tuple(tri) if front else tuple(reversed(tri)))
    return mesh(name,[face_point(front,*p) for p in vertices],faces,mat)


def _tube(name, points, radius, mat, sides=6, closed=False):
    points=[Vector(p) for p in points];vertices=[];faces=[]
    for i,p in enumerate(points):
        if closed:tangent=(points[(i+1)%len(points)]-points[i-1]).normalized()
        else:tangent=(points[min(i+1,len(points)-1)]-points[max(0,i-1)]).normalized()
        axis=Vector((0,1,0))
        if abs(tangent.dot(axis))>.95:axis=Vector((1,0,0))
        a=tangent.cross(axis).normalized();b=tangent.cross(a).normalized()
        for k in range(sides):
            angle=math.tau*k/sides;vertices.append(p+radius*(math.cos(angle)*a+math.sin(angle)*b))
    for i in range(len(points) if closed else len(points)-1):
        j=(i+1)%len(points)
        for k in range(sides):faces.append((i*sides+(k+1)%sides,j*sides+(k+1)%sides,j*sides+k,i*sides+k))
    if not closed:
        faces.append(tuple(reversed(range(sides))))
        faces.append(tuple((len(points)-1)*sides+k for k in range(sides)))
    return mesh(name,vertices,faces,mat)


def _patch(name, front, loop, mat, depth=0):
    if sum(a[0]*b[1]-b[0]*a[1] for a,b in zip(loop,loop[1:]+loop[:1]))<0:loop=list(reversed(loop))
    center=sum((Vector(p) for p in loop),Vector((0,0)))/len(loop)
    verts=[face_point(front,*center,depth)]+[face_point(front,*p,depth) for p in loop]
    faces=[(0,i+1,(i+1)%len(loop)+1) for i in range(len(loop))]
    if not front:faces=[tuple(reversed(f)) for f in faces]
    return mesh(name,verts,faces,mat)


def _recess(name, front, loop, mat, depth=.025):
    if sum(a[0]*b[1]-b[0]*a[1] for a,b in zip(loop,loop[1:]+loop[:1]))<0:loop=list(reversed(loop))
    center=sum((Vector(p) for p in loop),Vector((0,0)))/len(loop)
    inset=[center+(Vector(p)-center)*.97 for p in loop];n=len(loop)
    verts=[face_point(front,*p,-.001) for p in loop]+[face_point(front,*p,depth) for p in inset]
    faces=[(i,(i+1)%n,(i+1)%n+n,i+n) for i in range(n)]
    if not front:faces=[tuple(reversed(f)) for f in faces]
    mesh(name+'_Reveal',verts,faces,mat)
    return inset


def _grille(name, front, loop, lod, black, metal):
    inset=_recess(name,front,loop,black,.035)
    _patch(name+'_Backing',front,inset,black,.039)
    _tube(name+'_Surround',[face_point(front,*p,-.002) for p in loop],.004,metal,4 if lod else 6,True)
    lo=min(p[1] for p in inset);hi=max(p[1] for p in inset)
    for row in range([5,4,2][lod]):
        z=lo+(hi-lo)*(row+1)/([5,4,2][lod]+1)
        hits=[]
        for a,b in zip(inset,inset[1:]+inset[:1]):
            if (a[1]>z)!=(b[1]>z):hits.append(a[0]+(b[0]-a[0])*(z-a[1])/(b[1]-a[1]))
        if len(hits)<2:continue
        a,b=min(hits),max(hits)
        pts=[face_point(front,a+(b-a)*i/8,z,.014) for i in range(9)]
        _tube(name+f'_Slat{row}',pts,.004 if name=='UpperGrille' else .0035,metal,4)
    if lod==0:
        for col in range(-4,5):
            x=col*.09;ends=[]
            for a,b in zip(inset,inset[1:]+inset[:1]):
                if (a[0]>x)!=(b[0]>x):ends.append(a[1]+(b[1]-a[1])*(x-a[0])/(b[0]-a[0]))
            if len(ends)>=2:
                _tube(name+f'_Fin{col}',[face_point(front,x,min(ends),.023),face_point(front,x,max(ends),.023)],.0028,black,4)


def _projector(name, front, x, z, radius, lod, reflector, emitter):
    count=[32,16,8][lod]
    # Independent concentric bowl, metal rim and slightly convex optical face.
    outer=[(x+radius*math.cos(math.tau*i/count),z+radius*.76*math.sin(math.tau*i/count)) for i in range(count)]
    _tube(name+'_Bezel',[face_point(front,*p,.009) for p in outer],.0025,reflector,4,True)
    inner=[(x+radius*.82*math.cos(math.tau*i/count),z+radius*.62*math.sin(math.tau*i/count)) for i in range(count)]
    _patch(name+'_Optic',front,inner,emitter,.011)


def build_clean_fascia(lod=0, profiles=None):
    paint=material('VehiclePaint',(.012,.070,.23),.27,.42)
    shader=paint.node_tree.nodes.get('Principled BSDF')
    shader.inputs['Coat Weight'].default_value=.8;shader.inputs['Coat Roughness'].default_value=.13
    black=material('VehicleTrimBlack',(.008,.010,.013),.43,.10)
    gloss=material('VehicleTrimGloss',(.015,.02,.028),.24,.48)
    metal=material('VehicleChrome',(.27,.32,.38),.22,.9)
    reflector=material('VehicleLampReflector',(.28,.31,.35),.16,.85)
    head=material('VehicleHeadlamp',(.64,.69,.74),.12,.25)
    tail=material('VehicleTaillamp',(.45,.003,.002),.19,.22)
    indicator=material('VehicleIndicator',(.5,.12,.004),.24,.10)
    cover=material('VehicleLampCover',(.20,.24,.28),.09,0,.17)
    redcover=material('VehicleRearLampCover',(.22,.004,.005),.17,.05,.38)
    for mat,channel in [(head,'head'),(tail,'tail')]:
        mat['installedLampChannel']=channel;mat['vehiclePart']='lampHead' if channel=='head' else 'lampTail'
        mat.use_backface_culling=True
    curve_samples=[5,3,1][lod]
    front_outline=[(-.79,.22),(-.86,.26),(-.865,.57),(-.86,.79),(-.60,.815),(0,.82),(.60,.815),(.86,.79),(.865,.57),(.86,.26),(.79,.22)]
    rear_outline=[(-.79,.22),(-.87,.27),(-.87,.74),(-.87,.94),(-.58,.975),(0,.98),(.58,.975),(.87,.94),(.87,.74),(.87,.27),(.79,.22)]
    if profiles:
        front_outline=[(p[0],p[2]) for p in profiles['front']]
        rear_outline=[(p[0],p[2]) for p in profiles['rear']]
    lamp_left=[(-.823,.77),(-.45,.698),(-.49,.647),(-.803,.652),(-.831,.683)]
    lamps=[rounded_polygon(lamp_left,curve_samples,.12),rounded_polygon([(-x,z) for x,z in reversed(lamp_left)],curve_samples,.12)]
    upper=rounded_polygon([(-.467,.697),(.467,.697),(.438,.620),(.350,.522),(-.350,.522),(-.438,.620)],curve_samples,.17)
    lower=rounded_polygon([(-.54,.405),(.54,.405),(.735,.268),(-.735,.268)],curve_samples,.10)
    side=rounded_polygon([(-.815,.453),(-.748,.445),(-.687,.365),(-.774,.281),(-.815,.294)],curve_samples,.18)
    side_right=[Vector((-p.x,p.y)) for p in reversed(side)]
    holes=lamps+[upper,lower,side,side_right]
    _panel('FrontFascia',True,front_outline,holes,paint,[.065,.12,.22][lod])
    _grille('UpperGrille',True,upper,lod,black,gloss)
    _grille('LowerGrille',True,lower,lod,black,gloss)
    for i,loop in enumerate([side,side_right]):
        inset=_recess(f'CornerIntake{i}',True,loop,black,.028)
        _patch(f'CornerIntake{i}_Backing',True,inset,black,.034)
    for index,loop in enumerate(lamps):
        key='L' if index==0 else 'R';sign=-1 if index==0 else 1
        inset=_recess('Headlamp_'+key,True,loop,gloss,.028)
        _patch('Headlamp_'+key+'_Housing',True,inset,reflector,.03)
        _tube('Headlamp_'+key+'_LED',[face_point(True,*p,.001) for p in inset],.0045,head,4 if lod else 6,True)
        for j,(x,z,r) in enumerate([(.772,.707,.026),(.680,.702,.023),(.595,.695,.018)]):
            if lod==2 and j>0:continue
            _projector(f'Headlamp_{key}_Module{j}',True,sign*x,z,r,lod,reflector,head)
        if lod<2:_patch('Headlamp_'+key+'_Cover',True,inset,cover,-.0015)
        _tube('Headlamp_'+key+'_Turn',[face_point(True,sign*.812,.683,.005),face_point(True,sign*.812,.725,.005)],.003,indicator,4)
    tail_left=[(-.834,.935),(-.362,.870),(-.445,.805),(-.834,.814)]
    tailloops=[rounded_polygon(tail_left,curve_samples,.13),rounded_polygon([(-x,z) for x,z in reversed(tail_left)],curve_samples,.13)]
    plate=rounded_polygon([(-.228,.745),(.228,.745),(.208,.58),(-.208,.58)],curve_samples,.1)
    _panel('RearFascia',False,rear_outline,tailloops+[plate],paint,[.065,.12,.22][lod])
    for index,loop in enumerate(tailloops):
        key='L' if index==0 else 'R';sign=-1 if index==0 else 1
        inset=_recess('Taillamp_'+key,False,loop,gloss,.027)
        _patch('Taillamp_'+key+'_Housing',False,inset,black,.03)
        for serial,chain in enumerate([
            [(.810,.838),(.801,.911),(.401,.864)],
            [(.805,.83),(.770,.875),(.45,.85)],
        ]):
            pts=[face_point(False,sign*x,z,.002) for x,z in chain]
            _tube(f'Taillamp_{key}_Signature{serial}',pts,.006,tail,4 if lod else 6)
        if lod<2:_patch('Taillamp_'+key+'_Cover',False,inset,redcover,-.001)
    inset=_recess('LicenseRecess',False,plate,paint,.035)
    _patch('LicenseRecess_Back',False,inset,black,.038)
    plate_mat=material('VehiclePlate',(.65,.67,.65),.55)
    plateloop=[(-.172,.701),(.172,.701),(.172,.617),(-.172,.617)]
    _patch('LicensePlate',False,plateloop,plate_mat,.018)
    valance=rounded_polygon([(-.83,.365),(-.72,.401),(.72,.401),(.83,.365),(.79,.222),(-.79,.222)],curve_samples,.13)
    _patch('RearValance',False,valance,black,-.002)
    for sign in (-1,1):
        strip=[face_point(False,sign*.73,.424,-.003),face_point(False,sign*.50,.424,-.003)]
        _tube('RearReflector_'+str(sign),strip,.010,redcover if lod else tail,4)
    if lod<2:
        loop=[face_point(False,.60+.070*math.cos(math.tau*i/24),.273+.031*math.sin(math.tau*i/24),-.013) for i in range(24)]
        _tube('ExhaustTip',loop,.005,metal,6,True)
    # Connect the fascia perimeter back to the body interface. The supplied
    # interface can later carry the body's exact independently sampled ring.
    for front,outline in [(True,front_outline),(False,rear_outline)]:
        sign=-1 if front else 1;n=len(outline);vertices=[]
        for row in range(4):
            w=row/3
            for x,z in outline:
                p=face_point(front,x,z)
                p.y=p.y*(1-w)+sign*2.27*w;vertices.append(p)
        faces=[]
        for row in range(3):
            for i in range(n):
                f=(row*n+i,row*n+(i+1)%n,(row+1)*n+(i+1)%n,(row+1)*n+i)
                faces.append(f if front else tuple(reversed(f)))
        mesh(('Front' if front else 'Rear')+'FasciaReturn',vertices,faces,paint)
    return {'construction':'original authored fascia guided by original multiview design',
            'lampChannels':['head','tail','indicator'],'lod':lod,'noSourceTopologyRetained':True}
