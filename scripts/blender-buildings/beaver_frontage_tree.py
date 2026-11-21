"""Build the inventoried broadleaf tree at the private edge of the Beaver frontage."""
import gzip
import json
import math
import random

from mathutils import Vector

RECORD = 101727


def inventory(root):
    data_file = root / "native/eo-props/data/props-js.json.gz"
    with gzip.open(data_file, "rt") as stream:
        data = json.load(stream)["trees"]
    stride = data["stride"]
    x, z, form, width, height, *_ = data["data"][RECORD*stride:(RECORD+1)*stride]
    if data["forms"][form] != "mature" or (x, z) != (-12916, 14433):
        raise ValueError("Beaver frontage tree changed in the street-tree inventory")
    return [x/10, z/10], width/10, height/10, data["source"]["dataset"]


def broadleaf(emit, finish, pivot, diameter, height):
    """An open scaffold with bending leaders, terminal sprays and individual leaf blades."""
    rng = random.Random(RECORD)
    bark = [finish("street-tree-bark", c, .96) for c in ("665a47", "817461", "776850", "5a5143")]
    leaves = [finish("street-tree-foliage", c, .76) for c in
              ("53673b", "657947", "819057", "495e36", "65763c", "7e874d")]
    width = diameter/2
    up = Vector((0, 1, 0))
    def placed(p):
        return tuple(p[i]+pivot[i] for i in range(3))
    def branch(points, radii):
        faces = [[] for _ in bark]
        verts = []
        sides = 18 if radii[0] > .14 else 9 if radii[0] > .055 else 6
        ring_jitter = [rng.uniform(.90,1.13) for _ in range(sides)]
        stripe = [rng.choice((0,0,0,1,2,3)) for _ in range(sides)]
        for i, (point, radius) in enumerate(zip(points, radii)):
            tangent = (points[min(i+1,len(points)-1)]-points[max(0,i-1)]).normalized()
            cross = tangent.cross(Vector((0,0,1)) if abs(tangent.z)<.92 else up).normalized()
            along = cross.cross(tangent).normalized()
            for k in range(sides):
                angle = math.tau*k/sides
                verts.append(placed(point+radius*ring_jitter[k]*(cross*math.cos(angle)+along*math.sin(angle))))
            if i:
                base = (i-1)*sides
                for k in range(sides):
                    a, b = base+k, base+(k+1)%sides
                    faces[stripe[k]].append((a,b,b+sides,a+sides))
        for material, group in zip(bark, faces):
            if group: emit(material,verts,group,True)

    def spray(root, tip, outward):
        step = tip-root
        for level in range(5):
            origin = root + step*((level+.5)/5)
            for side in (-1,1):
                axis=(outward*.28 + Vector((step.z*side,rng.uniform(-.30,.20),-step.x*side))).normalized()
                span=axis.cross(up).normalized()
                length=rng.uniform(.07,.15)
                mid=origin+axis*length*.50
                end=origin+axis*length-up*.025
                blade=[origin,mid+span*length*.22,mid+up*.009,mid-span*length*.22,end]
                emit(rng.choice(leaves),[placed(p) for p in blade],[(0,1,2),(0,2,3),(1,4,2),(2,4,3)],True)
    # A dominant trunk and four unequal scaffold limbs carry the outer crown.
    # Sample points on real parent segments so even the fine growth is connected.
    trunk = [Vector(p) for p in ((0,0,0),(.035,.36,.025),(-.08,1.5,.05),
             (-.17,2.63,.12),(-.04,3.71,.16),(.13,4.62,-.05),
             (.05,5.51,-.16),(-.16,6.20,-.24))]
    branch(trunk,[.29,.21,.173,.162,.145,.119,.086,.043])
    for azimuth in (.32,1.61,2.83,4.10,5.36):
        direction = Vector((math.cos(azimuth),0,math.sin(azimuth)))
        branch([direction*.46-up*.075, direction*.27+up*.045, trunk[1]],
               [.057,.093,.133])
    scaffolds = []
    for i,(joint,azimuth) in enumerate(((3,.39),(4,3.67),(5,1.97),(6,5.28))):
        axis = Vector((math.cos(azimuth),0,math.sin(azimuth)))
        across = Vector((-axis.z,0,axis.x))
        start = trunk[joint]
        knee = start + axis*(.56+.08*i) + up*.89
        elbow = knee + axis*(.36+.10*i) + across*rng.uniform(-.26,.28) + up*1.27
        end = elbow + across*.24 + axis*.25 + up*(1.20+.14*i)
        backbone = [start,knee,elbow,end]
        branch(backbone,[.133-.008*i,.112-.006*i,.084-.005*i,.051])
        scaffolds.append((backbone,azimuth))
    for i in range(17):
        backbone,azimuth = scaffolds[i%4]
        rank = i//4
        u = i/16
        angle = azimuth + (-1 if rank%2 else 1)*(.28+.32*rank) + rng.uniform(-.17,.17)
        out = Vector((math.cos(angle),0,math.sin(angle)))
        side = Vector((-out.z,0,out.x))
        segment = min(rank,2)
        attach = backbone[segment].lerp(backbone[segment+1], .54 if rank<3 else .94)
        h = max(attach.y+.87, height*(.57+.41*u)+rng.uniform(-.30,.19))
        r = width*(.94-.61*u*u)*rng.uniform(.82,1.02)
        leader = out*r + up*h
        knee = attach.lerp(leader,.37)+side*rng.uniform(-.25,.24)+out*.14
        elbow = attach.lerp(leader,.71)+side*rng.uniform(-.21,.22)-up*.18
        branch([attach,knee,elbow,leader],[.077*(1-.34*u),.064,.037,.009])
        for j in range(10):
            t=(j+.6)/10
            base = knee.lerp(elbow,t*2) if t<.5 else elbow.lerp(leader,t*2-1)
            heading=(out*rng.uniform(.25,.66)+side*((-1)**j)*rng.uniform(.5,1.2)).normalized()
            bud=base+heading*width*rng.uniform(.20,.40)+up*rng.uniform(.25,.75)
            bend=base.lerp(bud,.57)+up*.10
            branch([base,bend,bud],[.029,.018,.006])
            for k in range(6):
                corner=heading.cross(up)
                divergent=(heading*.48+corner*((-1)**k)*rng.uniform(.42,1.1)).normalized()
                stem=bend.lerp(bud,k/7)
                end=stem+divergent*rng.uniform(.28,.55)+up*rng.uniform(-.12,.25)
                branch([stem,end],[.006,.002])
                for q in range(2):
                    origin=stem.lerp(end,q*.44)
                    spray(origin,end+corner*rng.uniform(-.10,.10),divergent)
