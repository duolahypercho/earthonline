"""Original metre-scale sedan cabin and independently sampled wheel assemblies.

Blender Z-up, front -Y. Call build_clean_cabin(lod=0) in a fresh assembly.
No imported donor geometry, image pixels, or destructive LOD collapse is used.
"""
import math

import bmesh
import bpy
from mathutils import Matrix, Vector


def _material(name, color, metal=0, rough=.45):
    mat=bpy.data.materials.get(name) or bpy.data.materials.new(name)
    mat.use_nodes=True
    node=mat.node_tree.nodes.get('Principled BSDF')
    node.inputs['Base Color'].default_value=(*color,1)
    node.inputs['Metallic'].default_value=metal
    node.inputs['Roughness'].default_value=rough
    mat.diffuse_color=(*color,1)
    mat['envClass']='vehicle'
    return mat


def _parent(ob, parent):
    if parent is not None:
        bpy.context.view_layer.update()
        world=ob.matrix_world.copy();ob.parent=parent;ob.matrix_world=world
    return ob


def _empty(name, location):
    ob=bpy.data.objects.get(name)
    if ob is not None:
        if ob.type!='EMPTY':raise ValueError(name+' must be an empty pivot')
        return ob
    ob=bpy.data.objects.new(name,None);bpy.context.scene.collection.objects.link(ob)
    ob.location=location;ob.empty_display_size=.03
    return ob


def _mesh(name, verts, faces, mat, parent=None, smooth=True):
    mesh=bpy.data.meshes.new(name);mesh.from_pydata(verts,[],faces);mesh.materials.append(mat)
    bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
    bm.to_mesh(mesh);bm.free()
    for face in mesh.polygons:face.use_smooth=smooth
    ob=bpy.data.objects.new(name,mesh);bpy.context.scene.collection.objects.link(ob)
    return _parent(ob,parent)


def _box(name, location, dimensions, mat, lod, radius=.02, parent=None, rotation=None):
    bpy.ops.mesh.primitive_cube_add(size=1,location=location)
    ob=bpy.context.object;ob.name=name;ob.dimensions=dimensions
    bpy.ops.object.transform_apply(location=False,rotation=False,scale=True)
    if rotation is not None:ob.rotation_euler=rotation
    ob.data.materials.append(mat)
    if lod==1 and not (name.startswith('Seat_') or name in ['Dash','Console']):radius=0
    if radius and lod<2:
        mod=ob.modifiers.new('Authored edge radius','BEVEL')
        mod.width=min(radius,min(dimensions)*.46);mod.segments=2 if lod==0 else 1
        bpy.ops.object.modifier_apply(modifier=mod.name)
        if name.startswith('Seat_') and '_Back' in name and 'Rear' not in name:
            for vertex in ob.data.vertices:
                t=max(0,min(1,vertex.co.z/dimensions[2]+.5))
                vertex.co.x*=.89+.10*math.sin(math.pi*t)
        if name.startswith('DoorInsert_'):
            for vertex in ob.data.vertices:
                t=max(0,min(1,vertex.co.z/dimensions[2]+.5))
                vertex.co.y*=.84+.16*t
        if name in ['Dash','DashTop','DashFascia']:
            for vertex in ob.data.vertices:
                vertex.co.y+=.045*((vertex.co.x+location[0])/.715)**2
        for face in ob.data.polygons:face.use_smooth=True
        normal=ob.modifiers.new('Area weighted surface normals','WEIGHTED_NORMAL')
        normal.keep_sharp=True;normal.weight=40
        bpy.ops.object.modifier_apply(modifier=normal.name)
    return _parent(ob,parent)


def _ring(name, center, axis, profile, segments, mat, parent=None):
    """Closed revolved section; profile entries are (axial, radial)."""
    axis=Vector(axis).normalized();u=axis.orthogonal().normalized();v=axis.cross(u)
    center=Vector(center);verts=[];faces=[]
    for axial,radius in profile:
        for i in range(segments):
            angle=2*math.pi*i/segments
            verts.append(center+axis*axial+radius*(u*math.cos(angle)+v*math.sin(angle)))
    for j in range(len(profile)):
        for i in range(segments):
            a=j*segments+i;b=j*segments+(i+1)%segments
            c=((j+1)%len(profile))*segments+(i+1)%segments;d=((j+1)%len(profile))*segments+i
            faces.append((a,b,c,d))
    ob=_mesh(name,verts,faces,mat,parent)
    # Keep radial circles smooth but separate the barrel/annulus corners.
    # Without this, a planar brake rotor shades like an inflated doughnut.
    bm=bmesh.new();bm.from_mesh(ob.data)
    for edge in bm.edges:
        if not name.startswith('Tire_') and edge.is_manifold and edge.calc_face_angle()>.52:edge.smooth=False
    bm.to_mesh(ob.data);bm.free()
    return ob


def _tube(name, points, radius, mat, lod, parent=None, closed=False):
    sides=5 if lod==0 else 3
    points=[Vector(p)for p in points];verts=[];faces=[]
    for i,point in enumerate(points):
        tangent=points[(i+1)%len(points)]-points[i-1] if closed else points[min(i+1,len(points)-1)]-points[max(i-1,0)]
        tangent.normalize();u=tangent.orthogonal().normalized();v=tangent.cross(u)
        for j in range(sides):
            a=2*math.pi*j/sides;verts.append(point+radius*(u*math.cos(a)+v*math.sin(a)))
    for i in range(len(points) if closed else len(points)-1):
        for j in range(sides):faces.append((i*sides+j,i*sides+(j+1)%sides,((i+1)%len(points))*sides+(j+1)%sides,((i+1)%len(points))*sides+j))
    if not closed:
        faces.extend([tuple(reversed(range(sides))),tuple((len(points)-1)*sides+j for j in range(sides))])
    return _mesh(name,verts,faces,mat,parent)


def _wheel(key, sx, y, lod, mats):
    pivot=_empty('Wheel_'+key,(sx*.79,y,.34));center=Vector((sx*.79,y,.34))
    n=(48,16,8)[lod]
    if lod==0:
        profile=[(-.093,.241),(-.113,.266),(-.112,.298),(-.091,.327),(-.074,.338)]
        for x in [-.048,0,.048]:profile.extend([(x-.003,.34),(x-.0015,.3387),(x+.0015,.3387),(x+.003,.34)])
        profile.extend([(.074,.338),(.091,.327),(.112,.298),(.113,.266),(.093,.241)])
    elif lod==1:profile=[(-.10,.24),(-.11,.30),(-.075,.34),(.075,.34),(.11,.30),(.10,.24)]
    else:profile=[(-.10,.24),(-.08,.34),(.08,.34),(.10,.24)]
    _ring('Tire_'+key,center,(sx,0,0),profile,n,mats['tire'],pivot)
    if lod<2:
        profile=[(-.095,.214),(-.095,.239),(.097,.239),(.111,.232),(.107,.218),(.088,.217)] if lod==0 else [(-.095,.214),(-.095,.239),(.108,.239),(.108,.214)]
        _ring('RimBarrel_'+key,center,(sx,0,0),profile,n,mats['alloy'],pivot)
        if lod==0:_ring('BrakeDisc_'+key,center,(sx,0,0),[(.047,.067),(.047,.190),(.061,.190),(.061,.067)],32,mats['steel'],pivot)
    else:
        _ring('RimBarrel_'+key,center,(sx,0,0),[(.084,.204),(.108,.241),(.108,.204)],n,mats['alloy'],pivot)
    # Each spoke is an original swept solid with a shallow concave dish.
    verts=[];faces=[]
    for spoke in range(5):
        angle=spoke*2*math.pi/5+.14
        for fork in (-1,1) if lod==0 else (0,):
            rings=[]
            stations=[(.051,.110,.017,0),(.134,.092,.013,.015),(.223,.100,.011,.031)] if lod<2 else [(.0,.110,.020,0),(.224,.102,.023,0)]
            for radius,axial,halfwidth,spread in stations:
                radial=Vector((0,math.sin(angle),math.cos(angle)))
                tangent=Vector((0,math.cos(angle),-math.sin(angle)))
                mid=center+radial*radius+tangent*fork*spread+Vector((sx*axial,0,0))
                rings.append(len(verts))
                for depth,width in [(-.018,-halfwidth),(-.018,halfwidth),(0,halfwidth),(0,-halfwidth)]:
                    verts.append(mid+Vector((sx*depth,0,0))+tangent*width)
            faces.extend([tuple(rings[0]+j for j in (3,2,1,0)),tuple(rings[-1]+j for j in range(4))])
            for a,b in zip(rings,rings[1:]):
                for j in range(4):faces.append((a+j,a+(j+1)%4,b+(j+1)%4,b+j))
    spokes=_mesh('AlloySpokes_'+key,verts,faces,mats['alloy'],pivot,False)
    if lod==0:
        bpy.context.view_layer.objects.active=spokes
        bevel=spokes.modifiers.new('Machined spoke edge','BEVEL');bevel.width=.0014;bevel.segments=1
        bpy.ops.object.modifier_apply(modifier=bevel.name)
    if lod<2:_ring('WheelHub_'+key,center,(sx,0,0),[(.092,.001),(.092,.062),(.115,.062),(.119,.048),(.119,.001)],24 if lod==0 else 8,mats['alloy'],pivot)
    if lod==0:
        _ring('HubInset_'+key,center,(sx,0,0),[(.119,.001),(.119,.025),(.122,.025),(.122,.001)],16,mats['darkMetal'],pivot)
        for i in range(5):
            a=i*2*math.pi/5;bolt=center+Vector((0,math.sin(a)*.043,math.cos(a)*.043))
            _ring('WheelBolt_'+key+'_'+str(i),bolt,(sx,0,0),[(.118,.001),(.118,.006),(.124,.006),(.124,.001)],6,mats['steel'],pivot)
    if lod<2:
        # Caliper is chassis-mounted; unlike the disc it must not spin.
        brake=_empty('Brake_'+key,tuple(center))
        _box('BrakeCaliper_'+key,center+Vector((sx*.047,.149,.06)),(.065,.09,.158),mats['caliper'],lod,.025,brake,rotation=(.35,0,0))


def _upholstery(name,x,y,width,back,mats,parent):
    """One continuous dished pad: centre upholstery and bolsters share vertices."""
    n=12;verts=[];faces=[];insert_faces=[]
    def point(u,v,front=True):
        px=x+u*width*.5*math.sqrt(1-.18*v*v)
        if back:
            px=x+(px-x)*(.94-.055*v)
            z=.752+v*.235*math.sqrt(1-.09*u*u)
            py=y+.255+.17*(z-.752)
            py+=(-.050-.032*abs(u)**5+.009*abs(v)**5) if front else .048
            return (px,py,z)
        py=y+v*.235*math.sqrt(1-.16*u*u)
        z=.491+.037*abs(u)**5-.012*abs(v)**5 if front else .419
        return (px,py,z)
    for front in [True,False]:
        for j in range(n+1):
            for i in range(n+1):verts.append(point(-1+2*i/n,-1+2*j/n,front))
    count=(n+1)**2
    for j in range(n):
        for i in range(n):
            a=j*(n+1)+i;face=(a,a+1,a+n+2,a+n+1)
            if abs(-1+(2*i+1)/n)<.67 and abs(-1+(2*j+1)/n)<.84:insert_faces.append(len(faces))
            faces.append(face);faces.append(tuple(v+count for v in reversed(face)))
    boundary=list(range(n+1))+[j*(n+1)+n for j in range(1,n+1)]+[n*(n+1)+i for i in reversed(range(n))]+[j*(n+1)for j in reversed(range(1,n))]
    for a,b in zip(boundary,boundary[1:]+boundary[:1]):faces.append((b,a,a+count,b+count))
    ob=_mesh(name,verts,faces,mats['seat'],parent)
    origin=Vector((x,y+.255,.752)if back else(x,y,.459))
    for vertex in ob.data.vertices:vertex.co-=origin
    ob.matrix_world=Matrix.Translation(origin)
    ob.data.materials.append(mats['insert'])
    for index in insert_faces:ob.data.polygons[index].material_index=1
    for side in [-1,1]:
        points=[Vector(point(side*.68,-.80+i*.16))+Vector((0,-.0015,0)if back else(0,0,.0015))for i in range(11)]
        _tube(name+'_Piping'+str(side),points,.0015,mats['stitch'],0,parent)
    return ob


def _seat(key,x,y,width,lod,mats,bench=False):
    pivot=_empty('Seat_'+key,(x,y,.41));back=_empty('SeatBack_'+key,(x,y+.19,.52));_parent(back,pivot)
    if lod==0 and not bench:
        _upholstery('Seat_'+key+'_Base',x,y,width,False,mats,pivot)
    else:
        _box('Seat_'+key+'_Base',(x,y,.459),(width,.47,.086),mats['seat'],lod,.065,pivot)
        _box('Seat_'+key+'_BaseInsert',(x,y-.016,.501),(width*.66,.366,.024),mats['insert'],lod,.027,pivot)
    back_angle=-.17
    if lod==0 and not bench:_upholstery('Seat_'+key+'_Back',x,y,width,True,mats,back)
    else:
        _box('Seat_'+key+'_Back',(x,y+.255,.752),(width,.10,.47),mats['seat'],lod,.052,back,rotation=(back_angle,0,0))
        _box('Seat_'+key+'_BackInsert',(x,y+.196,.755),(width*.65,.026,.369),mats['insert'],lod,.036,back,rotation=(back_angle,0,0))
    heads=[x-width*.28,x+width*.28] if bench else [x]
    for i,hx in enumerate(heads):
        _box('Seat_'+key+'_Headrest'+str(i),(hx,y+.306,1.067),(.245,.109,.155),mats['seat'],lod,.04,back,rotation=(back_angle,0,0))
        if lod==0:
            for dx in (-.066,.066):_box('HeadPost_'+key+'_'+str(i)+'_'+str(dx),(hx+dx,y+.299,.977),(.012,.012,.046),mats['steel'],lod,.004,back)
    if lod==0:
        for sx in (-1,1) if bench else ():
            points=[(x+sx*width*.31,y-.172+i*.065,.515)for i in range(6)]
            _tube('SeatPiping_'+key+str(sx),points,.0018,mats['stitch'],lod,pivot)
            points=[(x+sx*width*.305,y+.17+i*.011,.60+i*.055)for i in range(6)]
            _tube('BackPiping_'+key+str(sx),points,.0018,mats['stitch'],lod,back)
        for k in range(5) if bench else ():
            px=x+((k/(2 if not bench else 4))-.5)*width*.46
            _tube('SeatSeam_'+key+str(k),[(px,y-.14,.514),(px,y+.135,.514)],.00085,mats['stitch'],lod,pivot)
        _box('Seat_'+key+'_Rail',(x,y,.382),(width*.68,.38,.05),mats['darkMetal'],lod,.008,pivot)
    return pivot


def _door_cards(lod,mats):
    for key in ['FL','FR','RL','RR']:
        sx=-1 if key.endswith('L')else 1
        yf,yr=(-.90,.22)if key.startswith('F')else(.22,1.10)
        door=_empty('Door_'+key,(sx*.88,yf,.40));mid=(yf+yr)/2;length=yr-yf-.085
        # A curved perimeter in the YZ plane is extruded into a moulded card.
        outline=[(mid-.50*length,.43),(mid-.46*length,.37),(mid+.40*length,.37),(mid+.49*length,.45),(mid+.49*length,.86),(mid+.42*length,.915),(mid-.47*length,.935),(mid-.50*length,.88)]
        if key.startswith('R'):
            outline=[(.265,.43),(.275,.37),(.775,.37),(.96,.62),(1.095,.915),(.275,.935),(.26,.87)]
        verts=[]
        for dx in [0,-.026]:
            verts.extend([(sx*(.789+dx),y,z)for y,z in outline])
        n=len(outline);faces=[tuple(reversed(range(n))),tuple(range(n,n*2))]
        for i in range(n):faces.append((i,(i+1)%n,(i+1)%n+n,i+n))
        _mesh('DoorCard_'+key,verts,faces,mats['dash'],door,False)
        _box('DoorInsert_'+key,(sx*.750,mid+.018,.763),(.020,length*.82,.17),mats['insert'],lod,.023,door)
        _box('DoorArmrest_'+key,(sx*.728,mid+.041,.674),(.096,length*.57,.060),mats['seat'],lod,.027,door)
        _box('DoorPocket_'+key,(sx*.748,mid-.10 if key.startswith('R') else mid+.08,.457),(.042,.37 if key.startswith('R') else length*.56,.062),mats['dash'],lod,.020,door)
        if lod==0:
            _box('DoorPull_'+key,(sx*.728,mid-.255,.814),(.022,.131,.019),mats['alloy'],lod,.008,door,rotation=(0,0,sx*.1))
            _tube('DoorTrim_'+key,[(sx*.746,mid-length*.43,.872),(sx*.746,mid,.861),(sx*.746,mid+length*.40,.844)],.003,mats['alloy'],lod,door)
            _tube('DoorStitch_'+key,[(sx*.737,mid-length*.34,.80),(sx*.737,mid+.025,.80),(sx*.737,mid+length*.35,.80)],.0016,mats['stitch'],lod,door)
            _box('WindowSwitch_'+key,(sx*.703,mid-.08,.707),(.033,.052,.010),mats['darkMetal'],lod,.003,door)
            # Small parallel slots read as a grille, without a high-poly field.
            for i in range(7):
                _box('DoorSpeaker_'+key+str(i),(sx*.745,mid-length*.33+i*.009,.501),(.003,.003,.092),mats['darkMetal'],lod,0,door)


def _cockpit(lod,mats):
    _box('FloorPan',(0,.20,.287),(1.45,2.16,.036),mats['carpet'],lod,.025)
    _box('RearShelf',(0,1.46,.839),(1.42,.32,.052),mats['carpet'],lod,.022)
    _box('Tunnel',(0,.23,.343),(.24,1.89,.084),mats['carpet'],lod,.039)
    _seat('FL',-.38,.05,.48,lod,mats);_seat('FR',.38,.05,.48,lod,mats)
    _seat('Rear',0,.91,1.30,lod,mats,True)
    # Low horizontal dashboard leaves the glass aperture and footwells clear.
    _box('Dash',(0,-.666,.785),(1.43,.30,.15),mats['dash'],lod,.055)
    _box('DashTop',(0,-.683,.873),(1.40,.268,.036),mats['seat'],lod,.016)
    _box('DashFascia',(0,-.508,.807),(1.34,.028,.071),mats['insert'],lod,.014)
    _box('GloveboxLid',(.395,-.520,.687),(.53,.042,.135),mats['dash'],lod,.02)
    _box('ScreenBezel',(.08,-.550,.939),(.357,.055,.165),mats['dash'],lod,.018)
    _box('CentreScreen',(.08,-.518,.939),(.333,.006,.141),mats['screen'],lod,.007)
    _box('Binnacle',(-.38,-.523,.907),(.375,.088,.122),mats['dash'],lod,.028)
    _box('Dials',(-.38,-.475,.911),(.319,.006,.076),mats['screen'],lod,.010)
    _box('Console',(0,-.027,.516),(.241,.729,.19),mats['dash'],lod,.042)
    _box('ConsoleTop',(0,-.114,.616),(.216,.427,.018),mats['darkMetal'],lod,.014)
    _box('ConsoleLid',(0,.228,.636),(.23,.252,.065),mats['seat'],lod,.028)
    _box('GearLever',(0,-.206,.646),(.038,.073,.052),mats['alloy'],lod,.017)
    if lod==0:
        _tube('DashTrim',[(x,-.485+.045*(x/.715)**2,.844)for x in [-.666,-.5,-.33,-.16,0,.16,.33,.5,.666]],.003,mats['alloy'],lod)
        for x in [-.603,.402,.577]:
            _box('Vent'+str(x),(x,-.485,.808),(.112,.008,.033),mats['darkMetal'],lod,.005)
            for i in range(3):_box('VentSlat'+str(x)+str(i),(x,-.479,.796+i*.010),(.100,.003,.002),mats['alloy'],lod,0)
        for y in [-.048,.071]:
            _ring('CupRecess'+str(y),(0,y,.629),(0,0,1),[(0,.036),(0,.049),(.002,.049),(.002,.036)],24,mats['darkMetal'])
            _ring('CupLip'+str(y),(0,y,.632),(0,0,1),[(0,.043),(0,.046),(.002,.046),(.002,.043)],24,mats['alloy'])
        # Minimal instrument markings are geometry, avoiding unreadable text.
        for i in range(11):
            a=math.radians(25+13*i)
            for cx in [-.465,-.305]:
                _box('DialTick'+str(cx)+str(i),(cx+.026*math.cos(a),-.471,.905+.026*math.sin(a)),(.002,.001,.005),mats['display'],lod,0)
        _box('ScreenRule',(.07,-.514,.935),(.26,.001,.002),mats['display'],lod,0)
        for i in range(4):_box('ScreenTile'+str(i),(-.023+i*.061,-.514,.906),(.039,.001,.017),mats['display'],lod,.001)
    # Local +Y is the steering axis, compatible with the authored driver rig.
    wheel=_empty('SteeringWheel',(-.38,-.327,.882));wheel.rotation_euler.x=-.35
    rot=Matrix.Rotation(-.35,3,'X');center=Vector(wheel.location)
    if lod==0:
        axis=rot@Vector((0,1,0))
        _ring('SteeringColumn',center-axis*.09,axis,[(-.09,.001),(-.09,.028),(.09,.028),(.09,.001)],12,mats['dash'])
    points=[center+rot@Vector((.167*math.cos(2*math.pi*i/(48 if lod==0 else 20)),0,.167*math.sin(2*math.pi*i/(48 if lod==0 else 20))))for i in range(48 if lod==0 else 20)]
    _tube('SteeringWheel_Rim',points,.018,mats['seat'],lod,wheel,True)
    hub=_box('SteeringWheel_Hub',center,(.131,.055,.096),mats['dash'],lod,.025,wheel,rotation=(-.35,0,0))
    for i,a in enumerate([20,160,270]):
        a=math.radians(a)
        points=[center+rot@Vector((math.cos(a)*r,.0,math.sin(a)*r))for r in [.048,.147]]
        _tube('SteeringWheel_Spoke'+str(i),points,.017 if i<2 else .013,mats['darkMetal'],lod,wheel)
    if lod==0:
        _box('SteeringAccent',center+rot@Vector((0,.031,-.019)),(.070,.006,.008),mats['alloy'],lod,.003,wheel,rotation=(-.35,0,0))
        for key,x in [('Brake',-.47),('Throttle',-.32)]:
            _box('Pedal'+key,(x,-.72,.39),(.055,.029,.090),mats['darkMetal'],lod,.009,rotation=(.28,0,0))


def build_clean_cabin(lod=0):
    """Create original mechanical/cabin meshes and stable interaction pivots."""
    if lod not in (0,1,2):raise ValueError('lod must be 0, 1, or 2')
    before=set(bpy.context.scene.objects)
    mats={
        'tire':_material('VehicleTire',(.017,.021,.023),0,.76),
        'alloy':_material('VehicleAlloy',(.40,.44,.48),.88,.23),
        'steel':_material('VehicleBrakeSteel',(.19,.22,.24),.82,.38),
        'darkMetal':_material('VehicleDarkMetal',(.024,.033,.041),.65,.30),
        'caliper':_material('VehicleBrakeCaliper',(.13,.105,.065),.72,.31),
        'dash':_material('VehicleInterior',(.031,.038,.045),0,.48),
        'seat':_material('VehicleLeather',(.065,.076,.084),0,.41),
        'insert':_material('VehicleSeatInsert',(.23,.255,.267),0,.58),
        'stitch':_material('VehicleStitch',(.37,.40,.41),0,.62),
        'carpet':_material('VehicleCarpet',(.022,.027,.031),0,.94),
        'screen':_material('VehicleDisplay',(.008,.023,.033),.15,.16),
        'display':_material('VehicleDisplayMark',(.20,.38,.42),.1,.4),
    }
    for key,sx,y in [('FL',-1,-1.40),('FR',1,-1.40),('RL',-1,1.42),('RR',1,1.42)]:_wheel(key,sx,y,lod,mats)
    # Keep semantic nodes at every LOD; far removes invisible upholstery.
    if lod<2:
        _cockpit(lod,mats);_door_cards(lod,mats)
    else:
        for key,x,y in [('FL',-.38,.05),('FR',.38,.05),('Rear',0,.91)]:
            seat=_empty('Seat_'+key,(x,y,.41))
            _parent(_empty('Seat_'+key+'_Base',(x,y,.459)),seat)
            _parent(_empty('Seat_'+key+'_Headrest0',(x,y+.306,1.067)),seat)
        for key in ['FL','FR','RL','RR']:
            sx=-1 if key.endswith('L')else 1;yf,yr=(-.90,.22)if key.startswith('F')else(.22,1.10)
            door=_empty('Door_'+key,(sx*.88,yf,.40))
            _parent(_empty('DoorCard_'+key,(sx*.77,(yf+yr)/2,.67)),door)
        _empty('SteeringWheel',(-.38,-.327,.882))
    bpy.context.view_layer.update()
    objects=[ob for ob in bpy.context.scene.objects if ob not in before]
    meshes=[ob for ob in objects if ob.type=='MESH']
    triangles=sum(sum(len(face.vertices)-2 for face in ob.data.polygons)for ob in meshes)
    return {'recipe':'original-clean-cabin-v1','lod':lod,'meshObjects':len(meshes),'triangles':triangles,
        'wheelRadiusM':.34,'rimRadiusM':.241,'wheelWidthM':.226,
        'seatHipHeightM':.50,'frontSeatX':[-.38,.38],'frontSeatY':.05,'rearSeatY':.91,
        'mechanicalNotes':'Brake calipers are chassis mounted; discs remain under wheel pivots.',
        'source':'Original analytic mesh construction; no donor geometry or image textures.'}
