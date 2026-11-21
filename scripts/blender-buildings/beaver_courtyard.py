"""Authoring meshes for the courtyard-house variant, in house X/up/depth.
The City of Los Angeles SurveyLA Mediterranean and Indigenous Revival
Architecture (1893–1948) survey describes the stucco, recessed openings,
ironwork and low clay-tile roofs used as design references here.
"""
import math
import random


def arch(emit, mat, center, radius, spring, rise, head, front, back):
    """Solid spandrel with a curved soffit. The opening continues through it."""
    points, faces = [], []
    count = 28
    for i in range(count + 1):
        a = math.pi * i / count
        x = center - radius * math.cos(a)
        y = spring + rise * math.sin(a)
        points.extend([(x, y, front), (x, head, front), (x, head, back), (x, y, back)])
    for i in range(count):
        j = i * 4
        for k in range(4):
            n = (k+1) % 4
            faces.append((j+k, j+n, j+4+n, j+4+k))
    faces.extend([(0, 3, 2, 1), (count*4, count*4+1, count*4+2, count*4+3)])
    emit(mat, points, faces)


def arched_casements(emit, box, boxes, frame, glass, plaster, floor):
    """Three independently glazed arches, recessed behind the front arcade."""
    sill, spring = floor + .86, floor + 1.77
    rad, rise, rail = .595, .72, .047
    front, back = .408, .502
    centres = [1.315 + 1.31 * i for i in range(3)]
    # The backing wall closes the gaps between the casements from indoors.
    for left, right in zip([.66] + [c + rad for c in centres],
                           [c - rad for c in centres] + [4.645]):
        box(plaster, (left, sill, front), (right, floor + 2.55, back))
    for centre in centres:
        arch(emit, plaster, centre, rad, spring, rise, floor + 2.55, front, back)
        left, right = centre - rad, centre + rad
        for x in (left, right - rail):
            box(frame, (x, sill, front-.009), (x + rail, spring, back), bevel=.003)
        for h, width in ((sill, rail), (spring - .17, .032)):
            box(frame, (left + rail, h, front), (right - rail, h + width, back), bevel=.003)
        box(frame, (centre-.014, sill+rail, front+.005),
            (centre+.014, spring+rise-rail, back-.008), bevel=.002)
        # Concentric elliptical arcs define the shaped timber and the actual pane edge.
        rim, panes = [], []
        for i in range(33):
            a = math.pi * i / 32
            co, si = math.cos(a), math.sin(a)
            x, h = centre-rad*co, spring+rise*si
            u, v = centre-(rad-rail)*co, spring+(rise-rail)*si
            rim.extend(((x,h,front-.009), (u,v,front-.009), (u,v,back), (x,h,back)))
            panes.extend(((u,sill+rail,.459),(u,v,.459),(u,v,.465),(u,sill+rail,.465)))
        for mat, points in ((frame,rim), (glass,panes)):
            faces = [(4*i+k,4*i+(k+1)%4,4*i+4+(k+1)%4,4*i+4+k)
                     for i in range(32) for k in range(4)]
            emit(mat, points, faces)
    boxes.append([[.66,sill,front],[4.645,floor+2.55,back]])


def roof_pitch(emit, plaster, lining, clay, x0, y0, x1, y1, front, rear):
    """One roof slope, closed at the gable, with overlapping barrel tiles.
    Tiles alternate convex covers and concave pans. Every front course has a
    lip, giving the eave real thickness even from below street level.
    """
    emit(lining, [(x0,y0,front),(x1,y1,front),(x1,y1,rear),(x0,y0,rear)], [(0,1,2,3)])
    low = min(y0,y1) - 0.03
    for z in (front+0.18,rear-0.15):
        emit(plaster, [(x0,low,z),(x1,low,z),(x1,y1,z),(x0,y0,z)], [(0,1,2,3)])
    rng = random.Random(round((x0+6)*119+front*51))
    across = math.ceil(abs(x1-x0)/0.395)
    rows = math.ceil((rear-front)/0.26)
    dz = (rear-front)/rows
    for i in range(across):
        s0, s1 = i/across, min(1,(i+1.11)/across)
        for j in range(rows):
            tone = rng.randrange(len(clay))
            for pan in (False,True):
                center = front+(j+0.5)*dz + (0.50*dz if pan else 0)
                radius = dz*(0.57 if pan else 0.43)
                points = []
                for t in (s0,s1):
                    x = x0+(x1-x0)*t
                    base = y0+(y1-y0)*t
                    for k in range(8):
                        a = math.pi*k/7
                        yy = (0.047*(1-math.sin(a)) if pan else 0.060+0.075*math.sin(a))
                        step = 0.013 if t == s0 else 0
                        points.append((x,base+yy+step,center+radius*math.cos(a)))
                emit(clay[tone],points,[(k,k+1,k+9,k+8) for k in range(7)],True)
                if i == 0 and not pan:
                    lip = points[:8] + [(x,y-0.018,z) for x,y,z in points[:8]]
                    emit(clay[tone],lip,[(k,k+1,k+9,k+8) for k in range(7)],True)


def awning_tiles(emit, clay, left, right, front, rear, low, high):
    """A narrow lean-to; the channels drain out toward the street."""
    n = math.ceil((right-left)/0.19)
    pitch = (right-left)/n
    for j in range(n):
        x = left+(j+0.5)*pitch
        points = []
        for z, h, r in [(front,low,pitch*0.48),(rear,high,pitch*0.42)]:
            for k in range(9):
                a = k*math.pi/8
                points.append((x+r*math.cos(a),h+0.065*math.sin(a),z))
        emit(clay[(j*7+2)%len(clay)],points,[(k,k+1,k+10,k+9) for k in range(8)],True)


def stair_bay(emit, box, stucco, recess, wood, glass, tiles, rise, photo=False):
    """A half-round bay with two glazed arched openings and a radial clay cap."""
    cx = 5.955
    rx, rz = 1.00, 0.90
    edge = math.pi / 6
    segments = 48
    angles = [(i / segments - 0.5) * math.pi for i in range(segments + 1)]

    def curve(angle, height, depth=1.0):
        return (cx + rx * depth * math.sin(angle), height, 0.04 - rz * depth * math.cos(angle))

    def strip(material, low, high, smooth=False):
        points = [p for pair in zip(low, high) for p in pair]
        emit(material, points, [(i+0, i+1, i+3, i+2) for i in range(0, len(points)-2, 2)], smooth)

    for level in (1, 2):
        y = level * rise
        bottom = 2.98 if level == 1 else y
        sill, spring, head = y + 0.86, y + 1.82, y + 2.43
        top = 10.18 if level == 2 else y + rise
        in_opening = [a for a in angles if abs(a) < edge + 1e-8]
        def arch_height(a):
            return spring + (head-spring) * math.sqrt(max(0, 1 - (math.sin(a)/math.sin(edge))**2))
        for section in (angles[:17], angles[32:]):
            strip(stucco, [curve(a, bottom) for a in section], [curve(a, top) for a in section], True)
        strip(stucco, [curve(a,bottom) for a in in_opening], [curve(a,sill) for a in in_opening], True)
        strip(stucco, [curve(a,arch_height(a)) for a in in_opening], [curve(a,top) for a in in_opening], True)
        # Preserve the curved edge of the opening all the way back to its sash.
        for heights in ([sill]*len(in_opening), [arch_height(a) for a in in_opening]):
            lip = [curve(a,h) for a,h in zip(in_opening,heights)]
            strip(recess, lip, [(x,h,-0.46) for x,h,z in lip], True)
        for a in (-edge,edge):
            x,_,z = curve(a,sill)
            emit(recess, [(x,sill,z),(x,spring,z),(x,spring,-0.46),(x,sill,-0.46)], [(0,1,2,3)])
        left, right = cx - 0.51, cx + 0.51
        box(glass, (left,sill,-0.445),(right,head,-0.435), True)
        for x in (left,cx-0.018,right-0.038):
            box(wood,(x,sill,-0.47),(x+0.038,head,-0.44))
        for h in (sill, spring):
            box(wood,(left,h,-0.47),(right,h+0.035,-0.44))
    strip(wood, [curve(a,10.14) for a in angles], [curve(a,10.235,1.086) for a in angles], True)
    if photo:
        # A conical cap sheds water in pans, with separate cover tiles over the joints.
        # Shorter courses near the top let the clay follow the tighter circumference.
        rng = random.Random(2157)
        for j in range(17):
            pitch = math.pi / 17
            for outer, inner in ((1.10,.745), (.806,.478), (.544,.228), (.297,.025)):
                for pan in (True, False):
                    center = -math.pi/2 + (j + (.50 if pan else 0.)) * pitch
                    tone = rng.choice(tiles)
                    rings = []
                    for r, lift in ((outer,.012),(inner,0.)):
                        row = []
                        for k in range(9):
                            t = k/8
                            a = center + (t-.5) * pitch * (1.02 if pan else .51)
                            bulge = math.sin(math.pi*t)
                            h = 10.21 + (1.09-r)*.68 + lift
                            h += (.019*(1-bulge) if pan else .028 + .066*r**.65*bulge)
                            row.append(curve(a,h,r))
                        rings.append(row)
                    strip(tone,*rings,True)
                    strip(tone,[(x,h-.016,z) for x,h,z in rings[0]],rings[0],True)
        return
    # The channels taper toward the centre and overlap downhill in two courses.
    for j in range(22):
        start = -math.pi/2 + j*math.pi/22
        for outer,inner in ((1.09,0.54),(0.60,0.012)):
            rings = []
            for radius in (outer,inner):
                ring = []
                for k in range(9):
                    a = start + k/8*math.pi/22
                    height = 10.21 + (1.09-radius)*0.68 + 0.05*radius*math.sin(k*math.pi/8)
                    ring.append(curve(a,height,radius))
                rings.append(ring)
            strip(tiles[(j*5+int(outer*10))%len(tiles)], *rings, True)


def dry_garden(emit, surface, stone, earth, rock, foliage, grasses, ramps):
    """Terraced decomposed-granite bed with mineral mulch, agaves and bunchgrasses."""
    rng = random.Random(412)
    left, right = 0.14, 4.91
    def front(x):
        return -1.68 - 0.79 * max(0,math.sin(math.pi*(x-left)/(right-left)))**0.82
    def grade(x,z):
        return 0.07 - 0.093*x + 0.078*z
    def position(x,z,lift=0.0):
        return (x,grade(x,z)+lift,z)
    def ribbon(mat, line1, line2, smooth=False):
        points = [p for pair in zip(line1,line2) for p in pair]
        emit(mat,points,[(i,i+1,i+3,i+2) for i in range(0,len(points)-2,2)],smooth)
    # Keep the slender capstones individually jointed along the curved wall.
    stations = [left+(right-left)*i/31 for i in range(32)]
    near = [position(x,front(x)) for x in stations]
    far = [position(x,0.04) for x in stations]
    ribbon(earth,near,far)
    for a,b,c,d in zip(near,near[1:],far,far[1:]):
        ramps.append([a,b,d,c])
    ring = [(x,front(x)) for x in stations] + [(right,0.04),(left,0.04)]
    def inner(x,z):
        # Round the street side, with short returns beside the window and stoop.
        return (x + (2.5-x)*0.047, z+(0.02-z)*0.046)
    for i,((x,z),(u,w)) in enumerate(zip(ring,ring[1:]+ring[:1])):
        wall = [(x,-1.72,z),position(x,z,.056),position(u,w,.056),(u,-1.72,w)]
        emit(stone,wall,[(0,1,2,3)])
        ramps.append(wall)
        x0,z0=x+(u-x)*0.008,z+(w-z)*0.008
        x1,z1=x+(u-x)*0.992,z+(w-z)*0.992
        uv=[(x0,z0),(x1,z1),inner(x1,z1),inner(x0,z0)]
        emit(surface[i%len(surface)],[position(px,pz,0.071) for px,pz in uv],[(3,2,1,0)])
    def agave(x,z,size,angle):
        # Spiralling ranks grow from broad, recurved outer leaves to erect inner spears.
        for j in range(34):
            t=j/33
            a=angle+j*2.39996 + rng.uniform(-.10,.10)
            radius=size*(.97-.75*t)*rng.uniform(.83,1.10)
            height=size*(.32+.99*t)*rng.uniform(.88,1.12)
            pts=[]
            for k in range(11):
                s=k/10
                wide=(0.028*(1-s)+0.11*math.sin(math.pi*s)**.79)*size*(1-.43*t)
                radial=.017+radius*s**1.42
                h=grade(x,z)+height*(1.23*s-.23*s*s)+.12*(1-t)*size*math.sin(s*math.pi)
                for q in (-1,-.42,0,.42,1):
                    rib=(1-abs(q))*.032*size*math.sin(s*math.pi)
                    pts.append((x+math.cos(a)*radial-math.sin(a)*q*wide,h+rib,z+math.sin(a)*radial+math.cos(a)*q*wide))
            faces=[(k*5+i,k*5+i+1,k*5+i+6,k*5+i+5) for k in range(10) for i in range(4)]
            emit(foliage[(j//4+j)%len(foliage)],pts,faces,True)
    def grass(x,z,size):
        for j in range(63):
            a=rng.random()*math.tau
            spread=size*rng.uniform(.19,.68);height=size*rng.uniform(.32,1.1)
            width=rng.uniform(.004,.011);root=.05*rng.random();pts=[]
            for k in range(6):
                s=k/5;reach=root+s*s*spread
                h=grade(x,z)+height*(1.8*s-1.02*s*s)
                w=width*(1-s*.98)
                for side in (-1,1):
                    pts.append((x+reach*math.cos(a)-side*w*math.sin(a),h,z+reach*math.sin(a)+side*w*math.cos(a)))
            emit(grasses[j%len(grasses)],pts,[(2*k,2*k+1,2*k+3,2*k+2) for k in range(5)],True)
    for x,z,size in [(0.83,-.88,.60),(1.43,-1.47,.34),(3.08,-1.39,.73),(3.73,-1.82,.26),(4.48,-.62,.39)]:
        agave(x,z,size,rng.random()*math.tau)
    for x,z,size in [(.45,-.48,.34),(1.72,-.65,.66),(2.12,-1.55,.32),(2.49,-.86,.50),(3.71,-.69,.63),(4.49,-1.52,.48)]:
        grass(x,z,size)
    for i in range(620):
        x=rng.uniform(left+.08,right-.08);z=rng.uniform(front(x)+.13,-.04)
        size=rng.uniform(.008,.027) if i>10 else rng.uniform(.08,.17)
        a=rng.random()*math.tau;stretch=rng.uniform(.7,1.5);h=grade(x,z)+.003
        points=[(x,h+size*rng.uniform(.38,.8),z)]
        for k in range(7):
            t=a+k*math.tau/7;rad=size*rng.uniform(.79,1.12)
            points.append((x+rad*math.cos(t)*stretch,h+size*rng.uniform(-.08,.22),z+rad*math.sin(t)))
        emit(rock[i%len(rock)],points,[(0,1+(k+1)%7,1+k) for k in range(7)],True)
