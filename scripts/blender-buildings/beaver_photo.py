"""Model the roof courses, soft furnishings and trailing foliage of the furnished terrace."""
import math
import random


def tile_roof(emit, box, plaster, wood, paints):
    """Interlocking pans and tapered cover tiles on two planes sharing one ridge."""
    ridge, top = 2.87, 10.98
    front, rear = -.61, 13.22
    rng = random.Random(2107)
    lanes = 63
    pitch = (rear - front) / lanes

    def shell(material, rings, reverse):
        count = len(rings[0])
        faces = [(i, i+1, count+i+1, count+i) for i in range(count-1)]
        emit(material, rings[0] + rings[1], [f[::-1] for f in faces] if reverse else faces, True)
        # The fired, exposed end shows the thickness of the clay at every overlap.
        lip = rings[0] + [(x, y-.014, z) for x,y,z in rings[0]]
        emit(material, lip, faces)

    for edge, eave in ((-.15, 10.02), (7.15, 10.04)):
        run = ridge - edge
        surface = lambda x: eave + (top-eave) * (x-edge) / run
        emit(wood, [(edge,eave,front),(ridge,top,front),(ridge,top,rear),(edge,eave,rear)],[(0,1,2,3)])
        for z in (front+.18,rear-.17):
            emit(plaster, [(edge,9.97,z),(ridge,9.97,z),(ridge,top,z),(edge,eave,z)],[(0,1,2,3)])
        courses = math.ceil(abs(run) / .315)
        for course in range(courses):
            start = course/courses
            stop = min(1.0,(course+1.17)/courses)
            for lane in range(lanes):
                for cover in (False, True):
                    z = front + pitch * (lane + (.05 if cover else .55))
                    kiln = rng.choices(paints, weights=(11,25,15,6,24,19))[0]
                    warp = rng.uniform(-.005,.005)
                    offset = rng.uniform(-.012,.012) if course else 0.
                    rings = []
                    for end,s in enumerate((start,stop)):
                        x = edge + run*s + offset*(1-end)
                        scale = (1. if not end else .85) if cover else (1. if not end else 1.04)
                        rad = pitch*(.32 if cover else .57)*scale
                        arch = (.085 if cover else -.041)*scale
                        y = surface(x) + (.077 if cover else .068) + .024*(1-end)
                        rings.append([(x,y+arch*math.sin(k*math.pi/8),
                                       z+rad*math.cos(k*math.pi/8)+warp*end) for k in range(9)])
                    shell(kiln,rings,run>0)
        # A continuous bargeboard closes the shallow rear rake. Timber tails
        # are exposed at the front where the overhang actually shelters the loggia.
        x1,x2=edge,ridge
        for z in (rear-.155,rear-.100):
            emit(wood,[(x1,surface(x1)-.048,z),(x2,surface(x2)-.048,z),
                       (x2,surface(x2)+.018,z),(x1,surface(x1)+.018,z)],[(0,1,2,3)])
        for z in (front+.11,):
            for k in range(math.ceil(abs(run)/.54)):
                x = edge + (k+.34)*math.copysign(.54,run)
                if min(edge,ridge)<x<max(edge,ridge):
                    y = surface(x)
                    box(wood,(x-.046,y-.12,z-.08),(x+.046,y,z+.28),False,.009)
    for k in range(math.ceil((rear-front)/.31)):
        z = front+k*.31
        rings = []
        for end,depth in enumerate((z,min(rear,z+.36))):
            radius = .118-.013*end
            rings.append([(ridge+radius*math.cos(i*math.pi/10),
                           top+.068+radius*math.sin(i*math.pi/10),depth) for i in range(11)])
        shell(rng.choice(paints),rings,False)


def bunched_linen(u, t, x, h, z):
    """Bias-cut diagonal creases in the loose top, unfurling into the hanging hem."""
    edge = abs(2*u-1)
    # Each compression ridge has a long soft face beside a shorter, shaded return.
    bands = ((.15,.19,.052,.045,.22),(.32,-.18,.073,.055,.45),
             (.49,.29,.085,.037,.73),(.63,-.22,.08,.06,.26),(.79,.14,.05,.040,.72))
    lift = 0.
    for center,slope,amplitude,spread,end in bands:
        offset = t-center-slope*(u-.5)-.026*math.sin(7*u+center*11)
        length = math.exp(-((u-end)/.36)**2)
        lift += .73*amplitude*length*math.exp(-(offset/(spread*.76))**2)
    # Spread the hem into hanging folds with some slack left on the foot corners.
    pleat = math.sin(27*t+2.6*u+1.2*math.sin(8*t))
    x += math.copysign(.026*edge**4*pleat,u-.5)
    z -= .032*(1-t)**5*(.5+math.sin(19*u+1.7*t))
    # Closely spaced compression pleats fan out from the loose edge,
    # breaking at the quilt seams instead of repeating across the whole bed.
    for anchor,lean in ((.13,.31),(.33,-.21),(.72,.20),(.89,-.32)):
        line=u-anchor-lean*t-.021*math.sin(13*t+anchor*8)
        lift += .038*math.exp(-(line/.022)**2) * math.exp(-((t-.35)/.31)**2)
    h += lift + .011*math.sin(41*u+19*t)*math.sin(8*t)**2
    h -= .105*edge**5*(.55+.36*math.sin(9*t+u)) + .060*(1-t)**6
    # Small radial tension folds continue over the hanging edge at the foot.
    edge_t = max(0., (.38-t)/.38)
    seam = (math.sin(67*u + 9*t + 2.8*math.sin(8*u-7*t)) +
            .35*math.sin(113*u-19*t))
    z -= edge_t**2 * seam * .024
    h += edge_t * seam * .013 + .008*math.sin(72*u+32*t)*math.exp(-((t-.81)/.17)**2)
    return x,h,z


def upholstered_headboard(emit, box, finish, y):
    cloth = finish("flax-linen","ab8f69",.84)
    trim = finish("oiled-oak","805d3b",.48)
    box(trim,(.80,y+.08,12.40),(2.85,y+1.24,12.54),True,.074)
    box(cloth,(.83,y+.32,12.367),(2.82,y+1.205,12.442),False,.039)


def sleeping_pillow(emit, tube, mat, x, h, z):
    """A pillowcase made from two bowed panels sharing a sewn edge and folded corners."""
    points, faces, rim = [], [], []
    sections, rings = 56, 12
    for face in (-1,1):
        base = len(points)
        points.append((x,h+face*.106,z))
        for j in range(1,rings+1):
            t=j/rings
            for i in range(sections):
                a=i*math.tau/sections
                power=lambda n: math.copysign(abs(n)**.48,n)
                dx=.462*power(math.cos(a))*t
                dz=.301*power(math.sin(a))*t
                corners=abs(math.sin(2*a))
                crease=.013*math.sin(19*a+5*t)*corners*t**3
                y=h+.24*dz+face*(.004+.105*(1-t**3)**.62)+crease
                p=(x+dx,y,z+dz)
                points.append(p)
                if j==rings and face==1:
                    rim.append(p)
        for i in range(sections):
            a=base+1+i;b=base+1+(i+1)%sections
            faces.append((base,b,a) if face>0 else (base,a,b))
        for j in range(rings-1):
            for i in range(sections):
                a=base+1+j*sections+i;b=base+1+j*sections+(i+1)%sections
                q=(a,b,b+sections,a+sections)
                faces.append(q if face>0 else q[::-1])
    emit(mat,points,faces,True)
    tube(mat,rim+[rim[0]],.004,(0,1,0),6)


def trailing_plant(emit, turned, tube, clay, green, pale, bark, x, h, z):
    """Vines spill over the shelf and carry individual folded heart-shaped leaves."""
    rng = random.Random(217)
    turned(clay,x,h,z,[(.064,0),(.086,.105),(.091,.119),(.086,.128),(.074,.125)],24)
    for j in range(7):
        length=rng.uniform(.31,.72)
        azimuth=rng.uniform(-1.1,1.1)
        def stem(t):
            return (x+.19*(1-math.exp(-5*t))+.036*math.sin(7*t+j),
                    h+.12+.085*math.sin(min(1,t*3)*math.pi)-length*t**1.4,
                    z+.16*t*math.sin(azimuth)+.038*math.sin(5*t+j))
        tube(bark,[stem(i/14) for i in range(15)],.0021,(0,0,1),5)
        for k in range(9):
            t=(k+.48)/9
            a,b,c=stem(t)
            side=(-1)**(k+j)
            size=(.076-.031*t)*rng.uniform(.76,1.15)
            pts=[(a,b,c),(a+.25*size,b+.05*size,c+side*.37*size),
                 (a+.47*size,b-.16*size,c+side*.32*size),(a+.43*size,b-.77*size,c+side*size),
                 (a+.06*size,b-.27*size,c+side*.64*size),(a-.12*size,b+.08*size,c+side*.35*size)]
            emit(pale if k%4==0 else green,pts,[(0,1,2),(0,2,5),(2,3,4),(2,4,5)],True)


def stair_joinery(emit, box, turned, tube, oak, ivory, walnut, y, rise, ramps):
    """Closed painted string, tread noses and fitted spindles below a moulded rail."""
    start, end = 3., 9.
    slope = rise/(end-start)
    base = y-.012
    skirt_x = 5.07
    top = lambda z: y+(z-start)*slope+.076
    end_panel = 6.18
    side = [(skirt_x,base,start),(skirt_x,base,end_panel),
            (skirt_x,top(end_panel)-.11,end_panel),(skirt_x,base+.025,start)]
    far = [(x+.065,h,z) for x,h,z in side]
    emit(ivory,side+far,[(0,1,2,3),(7,6,5,4),(2,6,7,3),(1,5,6,2),(0,3,7,4)])
    ramps.append(side)
    box(ivory,(skirt_x-.008,base,start),(skirt_x+.078,y+.095,end_panel),False,.004)
    # The narrower timber string continues above the plaster to the upper
    # landing. The painted soffit closes the underside beyond the spandrel.
    for x in (5.061,5.103):
        q=[(x,top(start)-.16,start),(x,top(end)-.16,end),(x,top(end)-.012,end),(x,top(start)-.012,start)]
        emit(oak,q,[(0,1,2,3)])
    emit(ivory,[(5.10,top(end_panel)-.21,end_panel),(6.85,top(end_panel)-.21,end_panel),
                (6.85,top(end)-.21,end),(5.10,top(end)-.21,end)],[(0,1,2,3)])
    for i in range(20):
        z=start+.30*i
        tread=y+(i+1)*rise/20
        box(oak,(5.085,tread-.031,z-.023),(6.85,tread+.002,z+.025),False,.009)
        for k in range(3):
            pos=z+.052+.098*k
            length=y+(pos-start)*slope+.964-tread
            turned(ivory,5.17,tread,pos,[(.022,0),(.022,.104),(.015,.16),(.020,.20),
                (.014,.24),(.012,length-.19),(.022,length-.15),(.021,length)],12)
    for x, radius,height in ((5.17,.036,.995),(5.17,.025,.951)):
        tube(walnut,[(x,y+height,start-.03),(x,y+rise+height,end+.03)],radius,(1,0,0),16)
    box(walnut,(5.128,y,start-.063),(5.212,y+1.03,start+.023),False,.008)
    turned(walnut,5.17,y+1.026,start-.02,[(.064,0),(.062,.018),(.047,.029),(.022,.042)],20,True)


def shelf_still_life(emit, box, turned, mats, k, h):
    oak, cream, sage, clay, charcoal = mats
    if k in (1,3):
        # Rabbeted frame and paper mat. A low ridge and distant ridge are cut
        # separately so a strip of unprinted paper shows between the hills.
        z=5.04 if k==1 else 5.61
        size=.24 if k==1 else .30
        width=size*1.29
        def rect(mat,x,z0,z1,y0,y1):
            emit(mat,[(x,h+y0,z+z0),(x,h+y0,z+z1),(x,h+y1,z+z1),(x,h+y1,z+z0)],[(0,3,2,1)])
        box(oak,(.18,h,z),(.213,h+size,z+width),False,.006)
        rect(cream,.214,.014,width-.014,.014,size-.014)
        for distance,(color,alt,amp) in enumerate(((sage,.51,.12),(clay,.35,.09))):
            points=[]
            for i in range(9):
                s=i/8;zz=z+.042+(width-.084)*s
                hh=h+size*(alt+amp*math.sin(5*s+distance*2))
                points.extend([(.216+.001*distance,h+.044,zz),(.216+.001*distance,hh,zz)])
            emit(color,points,[(j,j+1,j+3,j+2) for j in range(0,16,2)])
    else:
        z=5.53 if k==0 else 4.69
        for j,(depth,width) in enumerate(((.165,.26),(.148,.238),(.155,.249))):
            a=h+j*.025
            box((charcoal,clay,sage)[j],(.21,a,z+.009*j),(.21+depth,a+.025,z+width),False,.003)
        turned(cream,.31,h+.075,z+.14,[(.025,0),(.055,.012),(.071,.049),(.065,.05),(.046,.024),(.024,.012)],24)


def metric_bake(objects, groups, assemblies, across, back):
    """Align light occluders with their installed furniture before the BVH is built."""
    from mathutils import Matrix, Vector
    # The bake uses Blender axes (across, negative depth, height). Keep the
    # shell fixed and express physical furniture axes in that same local frame.
    width = math.hypot(*across)
    skew = sum(x*z for x,z in zip(across,back))/width
    depth = abs(across[0]*back[1]-across[1]*back[0])/width
    site = Matrix(((width,-skew,0,0),(0,depth,0,0),(0,0,1,0),(0,0,0,1)))
    undo = []
    for ob in objects:
        component = ob.name.split(" - ")[0]
        if component not in groups:
            continue
        x,h,z=assemblies[groups[component]][0]
        pivot=Vector((x,-z,h))
        m=Matrix.Translation(pivot) @ site.inverted() @ Matrix.Translation(-pivot)
        ob.data.transform(m)
        ob.data.update()
        undo.append((ob.data,m.inverted()))
    return undo


def verge_clump(emit, grass, stone, x, y, z, scale, seed):
    """Irregular clumps of dry grass, small low leaves and loose mineral mulch."""
    rng=random.Random(seed)
    for k in range(round(54*scale)):
        angle=rng.random()*math.tau
        root=rng.random()*.095*scale
        reach=rng.uniform(.05,.25)*scale
        height=rng.uniform(.055,.28)*scale
        width=rng.uniform(.0024,.0072)*scale
        pts=[]
        for j in range(5):
            t=j*.25
            r=root+reach*t*t
            h=y+height*(1.8*t-1.04*t*t)
            for side in (-1,1):
                w=side*width*(1-.94*t)
                pts.append((x+r*math.cos(angle)-w*math.sin(angle),h,z+r*math.sin(angle)+w*math.cos(angle)))
        emit(grass[k%len(grass)],pts,[(j,j+1,j+3,j+2) for j in range(0,8,2)],True)
    for k in range(10):
        a=rng.random()*math.tau
        r=rng.uniform(.04,.23)*scale
        cx=x+r*math.cos(a);cz=z+r*math.sin(a);s=rng.uniform(.009,.024)*scale
        pts=[(cx,y+.47*s,cz)]+[(cx+s*math.cos(j*math.tau/6),y-.002,cz+.7*s*math.sin(j*math.tau/6)) for j in range(6)]
        emit(stone,pts,[(0,1+(j+1)%6,1+j) for j in range(6)],True)


def loggia_point(room, point, rise):
    """Open the three upper bays vertically while keeping the slab and roof datum."""
    x,h,z = point
    if room in ("workroom-storey", "upper-arched-loggia", "high-and-low-casements") and x < 4.84 and z < .78:
        v = h-2*rise
        for low,high,base,cap in ((0,.86,0,.72),(.86,2.55,.72,2.78),(2.55,rise,2.78,rise)):
            if low < v <= high:
                h = 2*rise+base+(v-low)*(cap-base)/(high-low)
                break
    return x,h,z

# The upper landing stays on its slab. Each tread goes 285 mm in the surveyed house.
STAIR_START = 9.0 - 20 * 0.285 / math.hypot(0.13461538, 1.28846154)


def stair_point(room, point):
    x, h, z = point
    if room.startswith(("stair-flight-", "landing-balustrade-")):
        z = 9.0 + (z - 9.0) * (9.0 - STAIR_START) / 6.0
    return x, h, z


def furniture_placement(name, point, group):
    x, h, z = point
    if name == "double-bed":
        # 1.93 m king mattress at the window. Keep the 2.03 m sleeping length.
        x = 1.825 + (x - 1.825) * (1.93 / (1.94 * .83))
    elif name in ("bedside-table", "bedside-lamp"):
        x += .16
    elif name == "potted-plant":
        x += .10
        z += 1.13
    elif group == 1 or name == "dining-pendant":
        z -= .70
        if name == "dining-chair": z += .18
    return x, h, z
