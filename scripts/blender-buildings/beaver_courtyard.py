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


def stair_bay(emit, box, stucco, recess, wood, glass, tiles, rise):
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
